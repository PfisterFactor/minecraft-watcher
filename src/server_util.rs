use anyhow::{anyhow, Result};
use aws_sdk_ec2::Client;
use aws_sdk_ec2::types::{Filter, Instance, InstanceStateName};
use craftio_rs::{CraftAsyncReader, CraftAsyncWriter, CraftIo, CraftTokioConnection};
use mcproto_rs::protocol::State;
use mcproto_rs::v1_15_2::{HandshakeNextState, HandshakeSpec, Packet578, RawPacket578, StatusPingSpec, StatusRequestSpec};
use tokio::net::TcpStream;

use crate::types::ServerStatus;

/// Information about the Minecraft server running on an EC2 instance
#[derive(Clone)]
pub struct EC2MinecraftServerStatus {
    pub ec2_instance_id: String,
    pub public_ip: Option<String>,
    pub ec2_state: InstanceStateName,
    pub server_status: ServerStatus,
    pub player_count: u32
}
impl EC2MinecraftServerStatus {
    /// Pings the EC2 instance and minecraft server and returns the status
    pub async fn get_server_status(client: &Client, instance_id: &str) -> Result<EC2MinecraftServerStatus> {
        // Grab EC2 instance state from AWS API
        let instance = get_ec2_instance(client, instance_id).await?;
        let instance_state = (||{instance.state()?.name()})().ok_or(anyhow!("AWS API Error"))?;
        let public_ip = instance.public_ip_address().map(|x| {x.to_string()});

        // Set server status based on whether EC2 instance is running
        let mut server_status = match &instance_state {
            InstanceStateName::Stopped => ServerStatus::Offline,
            InstanceStateName::Stopping => ServerStatus::ShuttingDown,
            InstanceStateName::Pending => ServerStatus::StartingEC2,
            InstanceStateName::ShuttingDown => ServerStatus::ShuttingDown,
            _ => ServerStatus::Unknown
        };

        // EC2 sometimes after stopping a spot instance won't let you provision another one until the spot request is finished updating
        // So we try to detect that here
        if server_status == ServerStatus::Offline {
            let res = client.describe_spot_instance_requests()
                .filters(
                    Filter::builder()
                        .set_name(Some("instance-id".to_string()))
                        .set_values(Some(vec!(instance_id.to_string()))).build()
                ).send().await?;
            if let Some(spot_instance_requests) = res.spot_instance_requests {
                let spot_request_status = spot_instance_requests.first().unwrap().status().unwrap().code().unwrap();
                if spot_request_status == "marked-for-stop" {
                    server_status = ServerStatus::ShuttingDown;
                }
            }
        }

        // Return early if we definitively know the state of the server
        // i.e. if the EC2 instance isn't running, we know the server isn't running
        if server_status != ServerStatus::Unknown || public_ip.is_none() {
            return Ok(EC2MinecraftServerStatus {
                ec2_instance_id: instance_id.to_string(),
                public_ip,
                ec2_state: instance_state.clone(),
                server_status,
                player_count: 0
            })
        }
        let public_ip = public_ip.unwrap();

        // If the EC2 instance is up, we have to ping the server to see if the Minecraft server is running
        let server_ping = ping_server(&public_ip).await;
        server_status = server_ping.unwrap_or(ServerStatus::Unknown);
        let player_count = get_player_count(&public_ip).await.unwrap_or(0);
        Ok(EC2MinecraftServerStatus {
            ec2_instance_id: instance_id.to_string(),
            public_ip: Some(public_ip),
            ec2_state: instance_state.clone(),
            server_status,
            player_count
        })
    }
}

/// Get details about the EC2 instance
async fn get_ec2_instance(client: &Client, instance_id: &str) -> Result<Instance> {
    let instance_statuses = client.describe_instances().instance_ids(instance_id).send().await?;
    (||{instance_statuses.reservations().first()?.instances().first()})().ok_or(anyhow!("AWS API Error")).cloned()
}
/// Ping the minecraft server and report the ServerStatus of it
async fn ping_server(public_ip: &str) -> Result<ServerStatus> {
    let public_ip_with_port = public_ip.to_string() + ":25565";
    {
        let tcp_ping = TcpStream::connect(&public_ip_with_port).await;
        if tcp_ping.is_err() {
            return Ok(ServerStatus::StartingEC2);
        }
    }
    let conn = CraftTokioConnection::connect_server_tokio(&public_ip_with_port).await;
    if conn.is_err() {
        return Ok(ServerStatus::StartingUp);
    }
    let mut conn = conn.unwrap();
    conn.write_packet_async(Packet578::Handshake(HandshakeSpec {
        version: 5.into(),
        server_address: public_ip.to_string(),
        server_port: 25565,
        next_state: HandshakeNextState::Status,
    })).await?;
    conn.set_state(State::Status);
    conn.write_packet_async(Packet578::StatusPing(StatusPingSpec { payload: 0 })).await?;
    match conn.read_packet_async::<RawPacket578>().await? {
        Some(Packet578::StatusPong(_)) => Ok(ServerStatus::Online),
        _ => Ok(ServerStatus::Unknown)
    }
}

/// Send a command to AWS to start up the EC2 instance hosting the minecraft server
pub async fn start_ec2_instance(client: &Client, instance_id: &str) -> Result<()> {
    let res = client.start_instances().instance_ids(instance_id).send().await;
    res?;
    Ok(())
}

/// Send a command to AWS to shut down the EC2 instance hosting the minecraft server
pub async fn stop_ec2_instance(client: &Client, instance_id: &str) -> Result<()> {
    client.stop_instances().instance_ids(instance_id).send().await?;
    Ok(())
}

/// Fetch the active player count on the minecraft server
pub async fn get_player_count(public_ip: &str) -> Result<u32> {
    let public_ip_with_port = public_ip.to_string() + ":25565";
    {
        let tcp_ping = TcpStream::connect(&public_ip_with_port).await;
        if tcp_ping.is_err() {
            return Err(anyhow!("Server not started."));
        }
    }
    let conn = CraftTokioConnection::connect_server_tokio(&public_ip_with_port).await;
    if conn.is_err() {
        return Err(anyhow!("Server not started."));
    }
    let mut conn = conn.unwrap();
    conn.write_packet_async(Packet578::Handshake(HandshakeSpec {
        version: 5.into(),
        server_address: public_ip.to_string(),
        server_port: 25565,
        next_state: HandshakeNextState::Status,
    })).await?;
    conn.set_state(State::Status);
    conn.write_packet_async(Packet578::StatusRequest(StatusRequestSpec {})).await?;
    let server_response = conn.read_packet_async::<RawPacket578>().await?;
    match server_response {
        Some(Packet578::StatusResponse(payload)) => Ok(u32::try_from(payload.response.players.online.max(0)).unwrap()),
        _ => Err(anyhow!("Server didn't respond correctly to Status Request"))
    }
}
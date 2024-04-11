use std::net::SocketAddr;
use anyhow::anyhow;
use aws_config::BehaviorVersion;
use aws_sdk_ec2::Client;
use craftio_rs::{CraftAsyncReader, CraftAsyncWriter, CraftIo, CraftTokioConnection};
use mcproto_rs::protocol::{PacketDirection, State};
use mcproto_rs::types::Chat;
use mcproto_rs::v1_15_2::{LoginDisconnectSpec};
use tokio::io::BufReader;
use tokio::net::TcpStream;
use crate::{CLI_ARGS, server_util};
use crate::types::ServerStatus;

use mcproto_rs::v1_15_2 as proto;
use proto::Packet578 as Packet;
use crate::server_util::EC2MinecraftServerStatus;

use anyhow::Result;

/// Shorthand to load the AWS config and get the server status
async fn get_server_status() -> Result<EC2MinecraftServerStatus> {
    let aws_credentials = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let ec2_client = Client::new(&aws_credentials);
    EC2MinecraftServerStatus::get_server_status(&ec2_client, &CLI_ARGS.get().unwrap().ec2_instance).await
}

/// Handle a connection from the Minecraft client
pub async fn handle_connection(socket: TcpStream, addr: &SocketAddr) -> anyhow::Result<()> {
    let (read, write) = socket.into_split();
    let buf_reader = BufReader::new(read);
    let mut craft_connect = CraftTokioConnection::from_async_with_state((buf_reader, write), PacketDirection::ServerBound, State::Handshaking);
    let handshake = match craft_connect.read_packet_async::<proto::RawPacket578>().await? {
        Some(Packet::Handshake(body)) => {
            body
        }
        other => {
            return Err(anyhow!("Unexpected Packet {:?}", other));
        }
    };
    let next_state = handshake.next_state.clone();
    match next_state {
        proto::HandshakeNextState::Status => handle_status(craft_connect, addr).await,
        proto::HandshakeNextState::Login => handle_login(craft_connect, addr).await
    }
}

/// Handle a status request from the Minecraft client
async fn handle_status(mut craft_connect: CraftTokioConnection, addr: &SocketAddr) -> anyhow::Result<()> {
    craft_connect.set_state(State::Status);
    log::info!("Serving status to {}", addr);

    use Packet::{StatusRequest, StatusResponse};
    use proto::{StatusResponseSpec};
    use mcproto_rs::status::*;

    let status = get_server_status().await?.server_status;
    log::info!("Server Status: {}",status.as_str());
    let response = StatusSpec {
        players: StatusPlayersSpec {
            max: 0,
            online: 0,
            sample: vec!(),
        },
        description: Chat::from_traditional(&("&lStatus:&r ".to_string() + status.get_motd()), true),
        favicon: None,
        version: Some(StatusVersionSpec {
            name: "phofidd-server-booter".to_owned(),
            protocol: 5,
        }),
    };

    craft_connect.write_packet_async(StatusResponse(StatusResponseSpec { response })).await?;
    if let Some(packet) = craft_connect.read_packet_async::<proto::RawPacket578>().await? {
        match packet {
            StatusRequest(_) => {}
            other => {
                return Err(anyhow!("Unexpected Packet {:?}", other));
            }
        }
    }
    log::info!("Status complete for {}", addr);
    Ok(())
}

/// Handle a login request from the Minecraft client
async fn handle_login(mut craft_connect: CraftTokioConnection, addr: &SocketAddr) -> anyhow::Result<()> {
    craft_connect.set_state(State::Login);
    log::info!("Serving login to {}", addr);
    use Packet::LoginStart;
    let player_name: String = match craft_connect.read_packet_async::<proto::RawPacket578>().await? {
        Some(LoginStart(body)) => {
            body.name
        }
        other => {
            return Err(anyhow!("Unexpected Packet {:?}", other));
        }
    };
    let server_status = get_server_status().await?.server_status;
    let players_allowed_to_start_server = &CLI_ARGS.get().unwrap().usernames_allowed_to_start_server;
    let message: Chat = match &server_status {
        ServerStatus::Offline => {
            let is_allowed_to_start_server =
                players_allowed_to_start_server.contains(&"*".to_string()) ||
                    players_allowed_to_start_server.contains(&player_name);


            if server_status == ServerStatus::Offline && is_allowed_to_start_server {
                let aws_credentials = aws_config::load_defaults(BehaviorVersion::latest()).await;
                let ec2_client = Client::new(&aws_credentials);
                server_util::start_ec2_instance(&ec2_client, &CLI_ARGS.get().unwrap().ec2_instance).await?;
                Chat::from_traditional("&lLogin acknowledged: &6Starting server up...", true)
            } else {
                Chat::from_traditional("&lLogin denied: &4Server is offline", true)
            }
        }
        ServerStatus::StartingEC2 => Chat::from_traditional("&6&lServer is still spinning up &7&o(give it a few minutes)", true),
        ServerStatus::StartingUp => Chat::from_traditional("&6&lServer is still spinning up &7&o(give it a few minutes)", true),
        ServerStatus::Online => Chat::from_traditional("&2&lServer is online&r, but DNS hasn't updated yet\n&7&o(wait a minute, then try again)", true),
        ServerStatus::ShuttingDown => Chat::from_traditional("&c&lServer is shutting down...", true),
        ServerStatus::Unknown => Chat::from_traditional("&b&lServer status isn't known\n&r&o(usually it's just starting up the EC2 instance, but it could be an error)", true)
    };

    craft_connect.write_packet_async(Packet::LoginDisconnect(LoginDisconnectSpec { message })).await?;
    Ok(())
}
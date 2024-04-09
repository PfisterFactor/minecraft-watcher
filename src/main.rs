mod types;
mod server_watcher;

use std::cell::{Cell, RefCell};
use anyhow::{Result, anyhow};
use std::net::SocketAddr;
use std::os::macos::raw::stat;
use aws_config::{BehaviorVersion, SdkConfig};
use aws_sdk_ec2::Client;
use craftio_rs::{CraftAsyncReader, CraftAsyncWriter, CraftIo, CraftTokioConnection};
use lazy_static::lazy_static;
use mcproto_rs::protocol::{PacketDirection, State};
use tokio::io::BufReader;
use tokio::net::{TcpListener, TcpStream};
use mcproto_rs::{v1_15_2  as proto};
use mcproto_rs::types::Chat;
use mcproto_rs::v1_15_2::LoginDisconnectSpec;
use mcproto_rs::v1_15_2::Packet578Kind::LoginDisconnect;
use proto::Packet578 as Packet;
use tokio::sync::{Mutex, OnceCell};
use tokio_cron_scheduler::{Job, JobScheduler};
use crate::server_watcher::EC2MinecraftServerStatus;
use crate::types::ServerStatus;

const MINECRAFT_PORT: u32 = 25565;
const EC2_INSTANCE_ID: &str = "i-06766df1a51b9415c";

const MINUTES_TO_WAIT_BEFORE_SHUTTING_DOWN: u32 = 20u32;

async fn get_server_status() -> Result<EC2MinecraftServerStatus> {
    let aws_credentials = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let ec2_client = Client::new(&aws_credentials);
    Ok(EC2MinecraftServerStatus::get_server_status(&ec2_client,EC2_INSTANCE_ID).await?)
}

async fn shutdown_server_if_inactive_task(inactivity_counter: &mut u32) {
    log::info!("[PERIODIC SERVER CHECK START]");
    let aws_credentials = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let ec2_client = Client::new(&aws_credentials);
    let server_status = EC2MinecraftServerStatus::get_server_status(&ec2_client,EC2_INSTANCE_ID).await;
    if server_status.is_err() {
        log::info!("Server Status Failed to fetch");
    }
    let status = server_status.as_ref().map(|x| x.server_status).unwrap_or(ServerStatus::Unknown);
    let player_count = server_status.as_ref().map(|x| x.player_count).unwrap_or(0);
    log::info!("Current Server Status: {status}");
    log::info!("Player count: {player_count}");
    if status == ServerStatus::Online && player_count == 0 {
        *inactivity_counter = (*inactivity_counter+1).min(MINUTES_TO_WAIT_BEFORE_SHUTTING_DOWN);
    }
    else {
        *inactivity_counter = 0;
    }
    log::info!("Inactivity Counter: {} min", MINUTES_TO_WAIT_BEFORE_SHUTTING_DOWN - *inactivity_counter);

    if *inactivity_counter == MINUTES_TO_WAIT_BEFORE_SHUTTING_DOWN {
        log::info!("Server has been inactive for {} minutes, shutting down...", {MINUTES_TO_WAIT_BEFORE_SHUTTING_DOWN});
        let shutdown_result = server_watcher::stop_ec2_instance(&ec2_client,EC2_INSTANCE_ID).await;
        match shutdown_result {
            Ok(_) => {log::info!("EC2 Shutdown request confirmed")},
            Err(e) => {log::error!("EC2 Shutdown request Error: {:?}",e);}
        }
        *inactivity_counter = 0;
    }
    log::info!("[PERIODIC SERVER CHECK END]");

}
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    log::info!("Initializing Phofidd.net status watcher");

    let mut sched = JobScheduler::new().await?;
    sched.add(
        Job::new_async("0 * * * * *", |_uuid, _l| {
            Box::pin(async move {
                static mut INACTIVITY_COUNTER: u32 = 0;
                unsafe { shutdown_server_if_inactive_task(&mut INACTIVITY_COUNTER).await }
            })
        })?
    ).await?;

    sched.start().await?;
    let mut listener = TcpListener::bind("127.0.0.1:".to_string() + &MINECRAFT_PORT.to_string()).await?;
    log::info!("Bound to TCP port {}", MINECRAFT_PORT);

    loop {
        let (socket,addr) = listener.accept().await?;
        log::info!("Received connection from: {}", addr);
        match handle_connection(socket, &addr).await {
            Ok(()) => {},
            Err(e) => {
                let stacktrace = e.backtrace();
                log::error!("Error serving {addr}\n{e}\n{stacktrace}")
            }
        }
        log::info!("Finished serving: {addr}");
    }

}

async fn handle_connection(mut socket: TcpStream, addr: &SocketAddr) -> Result<()> {
    let (read,write) = socket.into_split();
    let buf_reader = BufReader::new(read);
    let mut craft_connect = CraftTokioConnection::from_async_with_state((buf_reader, write), PacketDirection::ServerBound, State::Handshaking);
    let handshake = match craft_connect.read_packet_async::<proto::RawPacket578>().await? {
        Some(Packet::Handshake(body)) => {
            body
        },
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

async fn handle_status(mut craft_connect: CraftTokioConnection, addr: &SocketAddr) -> Result<()> {
    craft_connect.set_state(State::Status);
    log::info!("Serving status to {}", addr);

    use Packet::{StatusRequest, StatusResponse, StatusPing, StatusPong};
    use proto::{StatusResponseSpec, StatusPongSpec};
    use mcproto_rs::status::*;

    let status = get_server_status().await?.server_status;
    log::info!("Server Status: {}",status.as_str());
    let response = StatusSpec {
        players: StatusPlayersSpec {
            max: 0,
            online: 0,
            sample: vec!()
        },
        description: Chat::from_traditional(&("&lStatus:&r ".to_string() + status.get_motd()), true),
        favicon: None,
        version: Some(StatusVersionSpec {
            name: "phofidd-server-booter".to_owned(),
            protocol: 5
        }),
    };

    craft_connect.write_packet_async(StatusResponse(StatusResponseSpec {response})).await?;
    if let Some(packet) = craft_connect.read_packet_async::<proto::RawPacket578>().await? {
        match packet {
            StatusRequest(body) => {},
            other => {
                return Err(anyhow!("Unexpected Packet {:?}", other));
            }
        }
    }
    log::info!("Status complete for {}", addr);
    Ok(())
}

async fn handle_login(mut craft_connect: CraftTokioConnection, addr: &SocketAddr) -> Result<()> {
    craft_connect.set_state(State::Login);
    log::info!("Serving login to {}", addr);
    use Packet::LoginStart;
    match craft_connect.read_packet_async::<proto::RawPacket578>().await? {
        Some(LoginStart(body)) => {},
        other => {
            return Err(anyhow!("Unexpected Packet {:?}", other));
        }
    }
    let server_status = get_server_status().await?.server_status;

    if server_status == ServerStatus::Offline {
        let aws_credentials = aws_config::load_defaults(BehaviorVersion::latest()).await;
        let ec2_client = Client::new(&aws_credentials);
        server_watcher::start_ec2_instance(&ec2_client,EC2_INSTANCE_ID).await?;
    }

    let message = match &server_status {
        ServerStatus::Offline => {Chat::from_traditional("&lLogin acknowledged: &6Starting server up...",true)},
        ServerStatus::StartingEC2 => Chat::from_traditional("&6&lServer is still spinning up &7&o(give it a few minutes)", true),
        ServerStatus::StartingUp => Chat::from_traditional("&6&lServer is still spinning up &7&o(give it a few minutes)", true),
        ServerStatus::Online => Chat::from_traditional("&2&lServer is online&r, but DNS hasn't updated yet\n&7&o(Wait a minute, then try again)", true),
        ServerStatus::ShuttingDown => Chat::from_traditional("&c&lServer is shutting down...", true),
        ServerStatus::Unknown => Chat::from_traditional("&b&lServer status isn't known\n&r&o(usually it's just starting up the EC2 instance, but it could be an error)", true)
    };
    craft_connect.write_packet_async(Packet::LoginDisconnect(LoginDisconnectSpec {message })).await?;
    Ok(())
}
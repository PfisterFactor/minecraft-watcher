#![feature(never_type)]
mod types;
mod server_util;
mod status_reporter;
mod server_watcher;

use anyhow::{Result};
use clap::Parser;
use lazy_static::lazy_static;
use tokio::net::{TcpListener};
use tokio::sync::{OnceCell};

lazy_static! {
    /// Global variable containing the CLI arguments
    static ref CLI_ARGS: OnceCell<Args> = OnceCell::new();
}

#[derive(Clone, Parser, Debug)]
#[command(about, long_about = None)]
struct Args {
    /// EC2 Instance ID to monitor
    #[arg(long,required = true)]
    ec2_instance: String,

    /// TCP Port to have the watcher listen on
    #[arg(long,default_value_t = 25565)]
    watcher_port: u32,

    /// TCP Port the remote minecraft server is running on
    #[arg(long,default_value_t = 25565)]
    server_port: u32,

    /// Minutes to wait before considering the server as inactive and shutting it down
    #[arg(long, default_value_t = 20)]
    inactivity_timer: u32,

    /// List of usernames allowed to start the server seperated by commas, or '*' for everyone allowed
    #[arg(long, value_parser, value_delimiter = ',')]
    usernames_allowed_to_start_server: Vec<String>
}
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    CLI_ARGS.get_or_try_init(|| async {Ok::<Args,!>(Args::parse())}).await?;
    log::info!("Initializing Minecraft server status watcher");
    server_watcher::start_watcher().await?;
    log::info!("Initializing Minecraft server status reporter");
    let port = CLI_ARGS.get().unwrap().watcher_port;
    let listener = TcpListener::bind("0.0.0.0:".to_string() + &port.to_string()).await?;
    log::info!("Minecraft status reporter bound to TCP port {}", port);
    loop {
        let (socket,addr) = listener.accept().await?;
        log::info!("Received connection from: {}", addr);
        match status_reporter::handle_connection(socket, &addr).await {
            Ok(()) => {},
            Err(e) => {
                let stacktrace = e.backtrace();
                log::error!("Error serving {addr}\n{e}\n{stacktrace}")
            }
        }
        log::info!("Finished serving: {addr}");
    }
}
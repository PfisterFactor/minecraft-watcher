use std::cell::Cell;
use aws_config::BehaviorVersion;
use aws_sdk_ec2::Client;
use lazy_static::lazy_static;
use tokio::sync::Mutex;
use tokio_cron_scheduler::{Job, JobScheduler};
use crate::server_util::EC2MinecraftServerStatus;
use crate::{CLI_ARGS, server_util};
use crate::types::ServerStatus;
use anyhow::Result;

async fn shutdown_server_if_inactive_task(inactivity_counter: &mut u32) {
    log::info!("[PERIODIC SERVER CHECK START]");
    let aws_credentials = aws_config::load_defaults(BehaviorVersion::latest()).await;
    let ec2_client = Client::new(&aws_credentials);
    let server_status = EC2MinecraftServerStatus::get_server_status(&ec2_client,&CLI_ARGS.get().unwrap().ec2_instance).await;
    if server_status.is_err() {
        log::info!("Server Status Failed to fetch");
    }
    let status = server_status.as_ref().map(|x| x.server_status).unwrap_or(ServerStatus::Unknown);
    let player_count = server_status.as_ref().map(|x| x.player_count).unwrap_or(0);
    let inactivity_timer_max = CLI_ARGS.get().unwrap().inactivity_timer;
    log::info!("Current Server Status: {status}");
    log::info!("Player count: {player_count}");
    if status == ServerStatus::Online && player_count == 0 {
        *inactivity_counter = (*inactivity_counter+1).min(inactivity_timer_max);
    }
    else {
        *inactivity_counter = 0;
    }
    log::info!("Inactivity Counter: {} min", inactivity_timer_max - *inactivity_counter);

    if *inactivity_counter == inactivity_timer_max {
        log::info!("Server has been inactive for {} minutes, shutting down...", {inactivity_timer_max});
        let shutdown_result = server_util::stop_ec2_instance(&ec2_client, &CLI_ARGS.get().unwrap().ec2_instance).await;
        match shutdown_result {
            Ok(_) => {log::info!("EC2 Shutdown request confirmed")},
            Err(e) => {log::error!("EC2 Shutdown request Error: {:?}",e);}
        }
        *inactivity_counter = 0;
    }
    log::info!("[PERIODIC SERVER CHECK END]");

}

pub async fn start_watcher() -> Result<()> {
    let sched = JobScheduler::new().await?;
    sched.add(
        Job::new_async("0 * * * * *", |_uuid, _l| {
            Box::pin(async move {
                lazy_static! {
                    /// Time that the running server has been in an inactive state in minutes
                    static ref INACTIVITY_COUNTER: Mutex<Cell<u32>> = Mutex::new(Cell::new(0));
                }
                let mut mutex_guard = INACTIVITY_COUNTER.lock().await;
                shutdown_server_if_inactive_task(mutex_guard.get_mut()).await;
            })
        })?
    ).await?;

    sched.start().await?;
    Ok(())
}
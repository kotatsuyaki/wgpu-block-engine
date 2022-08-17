use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use anyhow::Result;
use time::macros::format_description;
use tokio::{runtime, signal::ctrl_c, spawn};
use tracing::{info, warn};
use tracing_subscriber::fmt::time::UtcTime;

mod core;
mod frontend;
mod network;

type SyncSender<T> = crossbeam_channel::Sender<T>;
type SyncReceiver<T> = crossbeam_channel::Receiver<T>;
type AsyncSender<T> = tokio::sync::mpsc::UnboundedSender<T>;
type AsyncReceiver<T> = tokio::sync::mpsc::UnboundedReceiver<T>;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .compact()
        .with_line_number(true)
        .with_file(true)
        .with_target(false)
        .with_timer(UtcTime::new(format_description!(
            "[hour]:[minute]:[second].[subsecond digits:6]"
        )))
        .init();

    let runtime = runtime::Builder::new_multi_thread().enable_all().build()?;
    let _enter_guard = runtime.enter();

    let should_stop = Arc::new(AtomicBool::new(false));
    spawn(listen_ctrl_c(should_stop.clone()));

    let network_system = network::NetworkSystem::new();
    core::run(network_system.handle(), &should_stop);
    runtime.block_on(network_system.shutdown());

    info!("Exiting");

    Ok(())
}

async fn listen_ctrl_c(should_stop: Arc<AtomicBool>) {
    if let Err(e) = ctrl_c().await {
        warn!(?e);
    }
    info!("Received shutdown signal");
    should_stop.store(true, Ordering::SeqCst);
}

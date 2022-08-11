use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use anyhow::Result;
use futures::FutureExt;
use time::macros::format_description;
use tokio::{runtime, signal::ctrl_c, sync::mpsc};
use tracing::info;
use tracing_subscriber::fmt::time::UtcTime;

mod core;
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

    let should_stop = Arc::new(AtomicBool::new(false));
    {
        let should_stop = should_stop.clone();
        runtime.spawn(ctrl_c().then(|_| async move {
            info!("Received shutdown signal");
            should_stop.store(true, Ordering::SeqCst);
        }));
    }

    // channel for **incoming** messages from the clients
    let (in_tx, in_rx) = crossbeam_channel::unbounded();
    // channel for **outgoing** messages to be sent to the clients
    let (out_tx, out_rx) = mpsc::unbounded_channel();

    let network_task = runtime.spawn(network::run((in_tx, out_rx)));
    core::run((out_tx, in_rx), &should_stop);

    runtime.block_on(network_task)?;
    info!("Exiting");

    Ok(())
}

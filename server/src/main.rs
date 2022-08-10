use anyhow::Result;
use tokio::{runtime, sync::mpsc};
use tracing::info;

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
        .init();

    let runtime = runtime::Builder::new_multi_thread().enable_all().build()?;

    // channel for **incoming** messages from the clients
    let (in_tx, in_rx) = crossbeam_channel::unbounded();
    // channel for **outgoing** messages to be sent to the clients
    let (out_tx, out_rx) = mpsc::unbounded_channel();

    let network_task = runtime.spawn(network::run((in_tx, out_rx)));
    core::run((out_tx, in_rx));

    runtime.block_on(network_task)?;
    info!("Exiting");

    Ok(())
}

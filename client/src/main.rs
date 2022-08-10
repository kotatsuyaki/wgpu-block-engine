use anyhow::Result;
use tokio::sync::mpsc;

mod core;
mod network;

fn main() -> Result<()> {
    init_tracing();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    // channel for **incoming** messages from the server
    let (in_tx, in_rx) = crossbeam_channel::unbounded();
    // channel for **outgoing** messages to be sent to the server
    let (out_tx, out_rx) = mpsc::unbounded_channel();

    // start network task
    let network_task = runtime.spawn(network::run((in_tx, out_rx)));

    // start main loop
    core::run(runtime.handle().clone(), (out_tx, in_rx));

    runtime.block_on(network_task)??;

    Ok(())
}

fn init_tracing() {
    use std::str::FromStr;
    use tracing_subscriber::*;

    const PKG_NAME: &str = env!("CARGO_PKG_NAME");
    fmt()
        .compact()
        .with_line_number(true)
        .with_target(true)
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            let pkg_name = PKG_NAME.replace("-", "_");
            EnvFilter::from_str(&format!("warn,{pkg_name}=info"))
                .expect("Failed to parse env-filter string")
        }))
        .init();
}

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use hashbrown::HashMap;
use quinn::{Endpoint, Incoming, NewConnection, ServerConfig};
use serde::{Deserialize, Serialize};
use spin_sleep::LoopHelper;
use tokio::{runtime, select, signal::ctrl_c, spawn};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tracing::{info, warn};
use wgpu_block_shared::chunk::Chunk;

mod network;

fn main() -> Result<()> {
    tracing_subscriber::fmt().init();
    let _chunk_collection = ChunkCollection::new();

    let mut loop_helper = LoopHelper::builder()
        .report_interval_s(0.5)
        .build_with_target_rate(20.0);

    let runtime = runtime::Builder::new_multi_thread().enable_all().build()?;
    let network_task = runtime.spawn(network::run());

    runtime.block_on(network_task)?;
    info!("Exiting");

    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
enum Message {
    Ping { data: i64 },
    Pong { data: i64 },
}

#[allow(dead_code)]
pub struct ChunkCollection {
    chunks: HashMap<(i64, i64), Chunk>,
}

impl ChunkCollection {
    fn new() -> Self {
        Self {
            chunks: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod test {}

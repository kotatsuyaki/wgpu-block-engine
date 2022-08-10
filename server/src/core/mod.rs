use glam::Vec3;
use hashbrown::{HashMap, HashSet};
use itertools::Itertools;
use noise::{NoiseFn, OpenSimplex};
use spin_sleep::LoopHelper;

use thiserror::Error;
use tracing::{info, warn};
use uuid::Uuid;
use wgpu_block_shared::{
    chunk::{BlockId, Chunk},
    protocol::{ClientMessage, ServerMessage},
};

use crate::{
    network::{InboundMessage, OutboundDestination, OutboundMessage},
    AsyncSender, SyncReceiver,
};

pub fn run((mut out_tx, in_rx): (AsyncSender<OutboundMessage>, SyncReceiver<InboundMessage>)) {
    let mut chunk_collection = ChunkCollection::new();
    let mut clients = Clients::new();

    let mut loop_helper = LoopHelper::builder()
        .report_interval_s(2.0)
        .build_with_target_rate(20.0);

    loop {
        let _delta = loop_helper.loop_start();

        // process inbound messages
        for in_msg in in_rx.try_iter() {
            if let Err(e) = handle_inbound_message(
                InboundHandlerContext {
                    clients: &mut clients,
                    chunks: &mut chunk_collection,
                    out_tx: &mut out_tx,
                },
                in_msg,
            ) {
                warn!("Error while handling inbound message: {:?}", e);
            }
        }

        // TODO: tick game

        // send newly-entered chunks to the clients
        let server_chunks: HashSet<(i64, i64)> = chunk_collection.chunks.keys().cloned().collect();
        for (&uuid, client) in clients.clients.iter_mut() {
            if client.logined == false {
                continue;
            }

            // TODO: Restrict view radius
            let new_chunks: Vec<(i64, i64)> = server_chunks
                .difference(&client.loaded_chunks)
                .cloned()
                .collect_vec();

            if new_chunks.is_empty() == false {
                info!("Sending new chunks at {new_chunks:?} to client {uuid}");
            }

            for &(cx, cz) in new_chunks.iter() {
                let chunk = chunk_collection.get((cx, cz)).expect("Failed to get chunk");
                let server_msg = ServerMessage::LoadChunk {
                    cx,
                    cz,
                    chunk: chunk.clone(),
                };
                out_tx
                    .send(OutboundMessage {
                        dest: OutboundDestination::Client(uuid),
                        server_msg,
                    })
                    .expect("Sending to out_tx failed");
            }
            client.loaded_chunks.extend(new_chunks);
        }

        // TODO: tell clients to unload faraway chunks

        if let Some(tps) = loop_helper.report_rate() {
            info!("TPS = {tps}");
        }

        loop_helper.loop_sleep();
    }
}

struct InboundHandlerContext<'cl, 'ch, 'tx> {
    clients: &'cl mut Clients,
    chunks: &'ch mut ChunkCollection,
    out_tx: &'tx mut AsyncSender<OutboundMessage>,
}

fn handle_inbound_message(
    ctx: InboundHandlerContext,
    in_msg: InboundMessage,
) -> Result<(), HandleInboundMessageError> {
    let (uuid, client_msg) = match in_msg {
        InboundMessage::Message { uuid, client_msg } => (uuid, client_msg),
        InboundMessage::AddClient { uuid } => return ctx.clients.add_client(uuid, Client::new()),
        InboundMessage::RemoveClient { uuid } => return ctx.clients.remove_client(uuid),
    };

    match client_msg {
        ClientMessage::Login => {
            let client = ctx.clients.get_client_mut(uuid)?;
            client.logined = true;

            let server_msg = ServerMessage::SetClientInfo { uuid };
            ctx.out_tx
                .send(OutboundMessage {
                    dest: OutboundDestination::Client(uuid),
                    server_msg,
                })
                .expect("Sending to out_tx failed");
        }
        ClientMessage::SetPlayerPos { eye, pitch, yaw } => {
            let client = ctx.clients.get_client_mut(uuid)?;
            client.player_pos.eye = eye.into();
            client.player_pos.pitch = pitch;
            client.player_pos.yaw = yaw;
        }
        client_msg => {
            warn!("Unhandled message {client_msg:?}");
        }
    }

    Ok(())
}

struct ChunkCollection {
    chunks: HashMap<(i64, i64), Chunk>,
}

impl ChunkCollection {
    fn new() -> Self {
        let mut chunks = HashMap::new();
        let simplex = OpenSimplex::new(0);

        let mut maxheight = 0;
        for cx in -3..=3_i64 {
            for cz in -3..=3_i64 {
                info!("Generating chunk ({cx}, {cz})");

                let mut chunk = Chunk::default();
                for lx in 0..16 {
                    for lz in 0..16 {
                        let height = (simplex
                            .get([(cx * 16 + lx) as f64 / 16.0, (cz * 16 + lz) as f64 / 16.0])
                            + 1.0)
                            * 10.0
                            + 26.0;
                        let height = height as usize;
                        maxheight = maxheight.max(height);
                        for h in 0..height {
                            chunk.set((lx as usize, h, lz as usize), BlockId::Grass);
                        }
                    }
                }
                chunks.insert((cx, cz), chunk);
            }
        }

        info!(maxheight);
        Self { chunks }
    }

    fn get(&self, (cx, cz): (i64, i64)) -> Option<&Chunk> {
        self.chunks.get(&(cx, cz))
    }
}

struct Clients {
    clients: HashMap<Uuid, Client>,
}

impl Clients {
    fn new() -> Self {
        Self {
            clients: HashMap::new(),
        }
    }

    /// Errors with [`HandleInboundMessageError::MissingClient`].
    fn get_client(&self, uuid: Uuid) -> Result<&Client, HandleInboundMessageError> {
        if let Some(client) = self.clients.get(&uuid) {
            Ok(client)
        } else {
            Err(HandleInboundMessageError::MissingClient)
        }
    }

    /// Errors with [`HandleInboundMessageError::MissingClient`].
    fn get_client_mut(&mut self, uuid: Uuid) -> Result<&mut Client, HandleInboundMessageError> {
        if let Some(client) = self.clients.get_mut(&uuid) {
            Ok(client)
        } else {
            Err(HandleInboundMessageError::MissingClient)
        }
    }

    /// Errors with [`HandleInboundMessageError::RepeatedClient`].
    fn add_client(&mut self, uuid: Uuid, client: Client) -> Result<(), HandleInboundMessageError> {
        use hashbrown::hash_map::Entry;
        match self.clients.entry(uuid) {
            Entry::Occupied(_) => return Err(HandleInboundMessageError::RepeatedClient),
            Entry::Vacant(e) => e.insert(client),
        };
        Ok(())
    }

    /// Errors with [`HandleInboundMessageError::MissingClient`].
    fn remove_client(&mut self, uuid: Uuid) -> Result<(), HandleInboundMessageError> {
        if self.clients.remove(&uuid).is_some() {
            Ok(())
        } else {
            Err(HandleInboundMessageError::MissingClient)
        }
    }
}

struct Client {
    logined: bool,
    player_pos: PlayerPosition,
    loaded_chunks: HashSet<(i64, i64)>,
}

#[derive(Debug, Default)]
struct PlayerPosition {
    eye: Vec3,
    pitch: f32,
    yaw: f32,
}

impl Client {
    fn new() -> Self {
        Self {
            logined: false,
            player_pos: PlayerPosition::default(),
            loaded_chunks: HashSet::new(),
        }
    }
}

#[derive(Debug, Error)]
enum HandleInboundMessageError {
    #[error("Client repeatedly added")]
    RepeatedClient,

    #[error("Attempt to get or remove a missing client")]
    MissingClient,
}

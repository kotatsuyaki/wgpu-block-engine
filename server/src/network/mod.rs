use std::sync::Arc;

use anyhow::{Context, Result};
use futures::StreamExt;
use quinn::{Endpoint, Incoming, NewConnection, ServerConfig};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use uuid::Uuid;

use crate::frontend::{Frontend, InboundMessage};
use crate::{AsyncReceiver, AsyncSender, SyncReceiver, SyncSender};
use wgpu_block_shared::asyncutils::{Shared, SpawnFutureExt};
use wgpu_block_shared::protocol;
use wgpu_block_shared::protocol::ServerMessage;
use wgpu_block_shared::protocol::{Rx, Tx};

mod client;
use client::*;

#[derive(Clone)]
pub struct NetworkHandle {
    out_tx: AsyncSender<OutboundMessage>,
    in_rx: Arc<SyncReceiver<InboundMessage>>,
}

#[must_use]
pub struct NetworkSystem {
    forward_outbound_handle: JoinHandle<()>,
    out_tx: AsyncSender<OutboundMessage>,
    in_rx: Arc<SyncReceiver<InboundMessage>>,
}

impl NetworkSystem {
    pub fn new() -> Self {
        let (in_tx, in_rx) = crossbeam_channel::unbounded();
        let (out_tx, out_rx) = mpsc::unbounded_channel();
        let (_endpoint, incoming) = create_endpoint();
        let clients = NetworkClients::new();

        let _dispatch_incomings_handle =
            dispatch_incomings(incoming, in_tx.clone(), clients.clone()).spawn();
        let forward_outbound_handle = forward_outbound_messages(
            in_tx.clone(),
            out_rx, // moved
            clients.clone(),
        )
        .spawn();

        Self {
            forward_outbound_handle,
            out_tx,
            in_rx: Arc::new(in_rx),
        }
    }

    pub fn handle(&self) -> NetworkHandle {
        NetworkHandle {
            out_tx: self.out_tx.clone(),
            in_rx: self.in_rx.clone(),
        }
    }

    pub async fn shutdown(self) {
        let Self {
            forward_outbound_handle,
            out_tx,
            in_rx,
        } = self;
        drop(out_tx);
        drop(in_rx);
        forward_outbound_handle.await.expect("Failed to join");
    }
}

impl Frontend for NetworkHandle {
    fn iter_messages(&self) -> Box<dyn Iterator<Item = InboundMessage> + '_> {
        Box::new(self.in_rx.try_iter())
    }

    fn broadcast(&self, server_msg: ServerMessage) {
        self.out_tx
            .send(OutboundMessage {
                dest: OutboundMessageDestination::Broadcast,
                server_msg,
            })
            .expect("Failed to send outbound message to out_tx");
    }

    fn send(&self, uuid: Uuid, server_msg: ServerMessage) {
        self.out_tx
            .send(OutboundMessage {
                dest: OutboundMessageDestination::Client(uuid),
                server_msg,
            })
            .expect("Failed to send outbound message to out_tx");
    }
}

/// # Panics
fn create_endpoint() -> (Endpoint, Incoming) {
    let (cert, key) = generate_self_signed_cert().expect("Failed to generate self-signed cert");
    let server_config =
        ServerConfig::with_single_cert(vec![cert], key).expect("Failed to create server config");
    let (endpoint, incoming) = Endpoint::server(server_config, "127.0.0.1:5000".parse().unwrap())
        .expect("Failed to construct server");
    (endpoint, incoming)
}

async fn dispatch_incomings(
    mut incoming: Incoming,
    in_tx: SyncSender<InboundMessage>,
    clients: Shared<NetworkClients>,
) {
    while let Some(connecting) = incoming.next().await {
        let mut newconn = match connecting.await {
            Ok(newconn) => newconn,
            Err(e) => {
                warn!("Failed new connection {e:?}");
                continue;
            }
        };

        let (tx, rx) = match wait_for_framed_stream(&mut newconn).await {
            Ok(val) => val,
            Err(e) => {
                warn!("Failed to wait for framed stream: {e:?}");
                continue;
            }
        };

        let uuid = Uuid::new_v4();

        if let Err(e) = in_tx.send(InboundMessage::AddClient { uuid }) {
            warn!(?e);
            break;
        }

        let new_client =
            NetworkClient::new((tx, rx), newconn.connection.clone(), in_tx.clone(), uuid);
        clients.write().await.insert_client(uuid, new_client);
    }
}

fn generate_self_signed_cert() -> Result<(rustls::Certificate, rustls::PrivateKey)> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let key = rustls::PrivateKey(cert.serialize_private_key_der());
    Ok((rustls::Certificate(cert.serialize_der()?), key))
}

async fn wait_for_framed_stream(newconn: &mut NewConnection) -> Result<(Tx, Rx)> {
    // wait for the bidir stream from the client
    let (tx, rx) = newconn
        .bi_streams
        .next()
        .await
        .context("bi_streams ended before the first bidirectional stream is opened")?
        .context("Connection error")?;
    Ok(protocol::make_framed(tx, rx))
}

async fn forward_outbound_messages(
    in_tx: SyncSender<InboundMessage>,
    mut out_rx: AsyncReceiver<OutboundMessage>,
    connections: Shared<NetworkClients>,
) {
    // This loop breaks once the sender half of `out_rx` is dropped.
    // Since the sender `out_tx` is held by the game logic, this is the first receiver loop to be
    // broken (by the time the game logic is already halted).
    while let Some(out_msg) = out_rx.recv().await {
        let OutboundMessage { dest, server_msg } = out_msg;
        match dest {
            OutboundMessageDestination::Client(uuid) => {
                // get the client tx and send
                let res = {
                    let connections = connections.read().await;
                    let client_tx = if let Some(client_tx) = connections.get_client_tx_for(uuid) {
                        client_tx
                    } else {
                        warn!("Missing client with uuid {uuid}");
                        continue;
                    };

                    client_tx.send(server_msg)
                };

                // check for error (and conditionally remove the client tx)
                if let Err(e) = res {
                    // The `client_rx` end i.e. the sender loop has stopped
                    warn!(?e);

                    let mut connections = connections.write().await;
                    connections.remove_client(uuid).await;
                    if let Err(e) = in_tx.send(InboundMessage::RemoveClient { uuid }) {
                        warn!(?e);
                        break;
                    }
                }
            }
            OutboundMessageDestination::Broadcast => {
                let client_txs = connections.read().await;
                for client_tx in client_txs.iter_client_txs() {
                    if let Err(e) = client_tx.send(server_msg.clone()) {
                        warn!(?e);
                    }
                }
            }
        }
    }

    // Properly shutdown all client connections
    info!("Shutting down client connections");
    let mut connections = connections.write().await;
    connections.close_all().await;
    info!("Shutted down all client connections");
}

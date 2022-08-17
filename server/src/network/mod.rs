use std::sync::Arc;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use hashbrown::HashMap;
use quinn::{Connection, Endpoint, Incoming, NewConnection, ServerConfig, VarInt};
use tokio::sync::{mpsc, RwLock};
use tokio::task::{spawn, JoinHandle};
use tracing::{info, warn};
use uuid::Uuid;

use crate::frontend::{Frontend, InboundMessage};
use crate::{AsyncReceiver, AsyncSender, SyncReceiver, SyncSender};
use wgpu_block_shared::protocol;
use wgpu_block_shared::protocol::{ClientMessage, Rx, ServerMessage, Tx};

type ClientTx = AsyncSender<ServerMessage>;
type Shared<T> = Arc<RwLock<T>>;

#[derive(Clone)]
pub struct NetworkHandle {
    out_tx: AsyncSender<OutboundMessage>,
    in_rx: Arc<SyncReceiver<InboundMessage>>,
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

/// Wrapper around [`ServerMessage`] that includes the destination information.
#[derive(Debug)]
pub struct OutboundMessage {
    pub dest: OutboundMessageDestination,
    pub server_msg: ServerMessage,
}

#[derive(Debug)]
pub enum OutboundMessageDestination {
    /// The message is to be sent to a particular client identified by an [`Uuid`].
    Client(Uuid),
    /// The message is to be sent to all connected clients.
    Broadcast,
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
            spawn(dispatch_incomings(incoming, in_tx.clone(), clients.clone()));
        let forward_outbound_handle = spawn(forward_outbound_messages(
            in_tx.clone(),
            out_rx, // moved
            clients.clone(),
        ));

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
        let (client_tx, client_rx) = mpsc::unbounded_channel();

        if let Err(e) = in_tx.send(InboundMessage::AddClient { uuid }) {
            warn!(?e);
            break;
        }

        let client_connection = start_client_communicator(
            newconn,
            uuid,
            (tx, rx),
            (client_tx, client_rx),
            in_tx.clone(),
        )
        .await;
        clients.write().await.insert_client(uuid, client_connection);
    }
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

struct NetworkClients {
    clients: HashMap<Uuid, NetworkClient>,
}

impl NetworkClients {
    fn new() -> Shared<Self> {
        Arc::new(RwLock::new(Self {
            clients: HashMap::new(),
        }))
    }

    fn get_client_tx_for(&self, uuid: Uuid) -> Option<&ClientTx> {
        self.clients.get(&uuid).map(|c| &c.client_tx)
    }

    fn insert_client(&mut self, uuid: Uuid, connection: NetworkClient) {
        self.clients.insert(uuid, connection);
    }

    async fn remove_client(&mut self, uuid: Uuid) {
        let client = self.clients.remove(&uuid);
        if client.is_none() {
            warn!("Attempted to remove an already-removed network client");
            return;
        }
        client.unwrap().close().await;
    }

    fn iter_client_txs(&self) -> impl Iterator<Item = &ClientTx> {
        self.clients.iter().map(|(_key, client)| &client.client_tx)
    }

    async fn close_all(&mut self) {
        for (_uuid, connection) in self.clients.drain() {
            connection.close().await;
        }
    }
}

/// Instances should be properly [`ClientConnection::close`]d.
#[must_use]
struct NetworkClient {
    // for sending server messages to client task
    client_tx: ClientTx,
    // for closing
    connection: Connection,
    // must be awaited to ensure that clients are notified of the disconnection
    sender_task: JoinHandle<Result<()>>,
}

impl NetworkClient {
    async fn close(self) {
        let NetworkClient {
            client_tx,
            connection,
            sender_task,
        } = self;
        // drop sender
        drop(client_tx);
        // wait for the sender task to send out the final `ServerMessage::Disconnect`
        if let Err(e) = sender_task.await {
            warn!(?e);
        }
        // close the rx
        connection.close(VarInt::from_u32(1), b"Server shutdown");
    }
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

async fn start_client_communicator(
    newconn: NewConnection,
    uuid: Uuid,
    (tx, rx): (Tx, Rx),
    (client_tx, client_rx): (AsyncSender<ServerMessage>, AsyncReceiver<ServerMessage>),
    in_tx: SyncSender<InboundMessage>,
) -> NetworkClient {
    info!("Starting client communicator with client uuid {uuid:?}");

    let connection = newconn.connection.clone();
    let sender_task = spawn(start_client_communicator_sender(uuid, tx, client_rx));
    // dangling receivers returns once the client connections are closed
    let _receiver_task = spawn(start_client_communicator_receiver(uuid, rx, in_tx));

    NetworkClient {
        client_tx,
        connection,
        sender_task,
    }
}

/// This returns once the sender half of `client_rx` is closed.
///
/// * `tx`: The outgoing sender to a particular client.
/// * `client_rx`: Receiver getting the [`ServerMessage`]s to be sent to a particular client.
async fn start_client_communicator_sender(
    uuid: Uuid,
    mut tx: Tx,
    mut client_rx: AsyncReceiver<ServerMessage>,
) -> Result<()> {
    while let Some(server_msg) = client_rx.recv().await {
        let server_msg = server_msg.serialize()?;
        tx.send(server_msg.into()).await?;
    }

    info!("Stopping client sender for {uuid}");

    tx.send(ServerMessage::Disconnect.serialize().unwrap().into())
        .await?;
    tx.flush().await?;
    tx.close().await?;

    info!("Stopped client sender for {uuid}");

    Ok(())
}

/// This returns once the client connection is closed.
async fn start_client_communicator_receiver(
    uuid: Uuid,
    mut rx: Rx,
    in_tx: SyncSender<InboundMessage>,
) -> Result<()> {
    while let Some(client_msg) = rx.next().await {
        // Unpack the framed result and deserialize.
        // If these errors, the connection is bad.
        let client_msg = client_msg?;
        let client_msg = ClientMessage::deserialize(client_msg)?;

        info!("Received client message: {client_msg:?}");

        // If this errors, the receiver half of `in_tx` is already dropped.
        if let Err(e) = in_tx.send(InboundMessage::Message { uuid, client_msg }) {
            warn!(?e);
            break;
        }
    }

    info!("Stopped client receiver for {uuid}");

    Ok(())
}

fn generate_self_signed_cert() -> Result<(rustls::Certificate, rustls::PrivateKey)> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let key = rustls::PrivateKey(cert.serialize_private_key_der());
    Ok((rustls::Certificate(cert.serialize_der()?), key))
}

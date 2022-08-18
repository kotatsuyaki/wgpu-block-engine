use anyhow::Result;
use futures::{SinkExt, StreamExt, TryFutureExt};
use hashbrown::HashMap;
use quinn::{Connection, VarInt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use uuid::Uuid;

use crate::frontend::InboundMessage;
use crate::{AsyncReceiver, AsyncSender, SyncSender};
use wgpu_block_shared::asyncutils::{make_shared, Shared, SpawnFutureExt};
use wgpu_block_shared::protocol::{ClientMessage, ServerMessage};
use wgpu_block_shared::protocol::{Rx, Tx};

type ClientTx = AsyncSender<ServerMessage>;

#[derive(Debug)]
pub struct OutboundMessage {
    pub dest: OutboundMessageDestination,
    pub server_msg: ServerMessage,
}

#[derive(Debug)]
pub enum OutboundMessageDestination {
    Client(Uuid),
    Broadcast,
}

pub struct NetworkClients {
    clients: HashMap<Uuid, NetworkClient>,
}

pub struct NetworkClient {
    client_tx: ClientTx,
    connection: Connection,
    sender_handle: JoinHandle<Result<()>>,
}

impl NetworkClients {
    pub fn new() -> Shared<Self> {
        make_shared(Self {
            clients: HashMap::new(),
        })
    }

    pub fn get_client_tx_for(&self, uuid: Uuid) -> Option<&ClientTx> {
        self.clients.get(&uuid).map(|c| &c.client_tx)
    }

    pub fn insert_client(&mut self, uuid: Uuid, connection: NetworkClient) {
        self.clients.insert(uuid, connection);
    }

    pub async fn remove_client(&mut self, uuid: Uuid) {
        let client = self.clients.remove(&uuid);
        if client.is_none() {
            warn!("Attempted to remove an already-removed network client");
            return;
        }
        client.unwrap().close().await;
    }

    pub fn iter_client_txs(&self) -> impl Iterator<Item = &ClientTx> {
        self.clients.iter().map(|(_key, client)| &client.client_tx)
    }

    pub async fn close_all(&mut self) {
        for (_uuid, connection) in self.clients.drain() {
            connection.close().await;
        }
    }
}

impl NetworkClient {
    pub async fn close(self) {
        let NetworkClient {
            client_tx,
            connection,
            sender_handle: sender_task,
        } = self;
        drop(client_tx);
        if let Err(e) = sender_task.await {
            warn!(?e);
        }
        connection.close(VarInt::from_u32(1), b"Server shutdown");
    }

    pub fn new(
        (tx, rx): (Tx, Rx),
        connection: Connection,
        in_tx: SyncSender<InboundMessage>,
        uuid: Uuid,
    ) -> Self {
        let (client_tx, client_rx) = mpsc::unbounded_channel();
        let sender_handle = send_messages_to_client(uuid, client_rx, tx)
            .and_then(send_disconnect_and_close)
            .spawn();
        let _receiver_handle = receive_messages_from_client(uuid, rx, in_tx).spawn();

        Self {
            client_tx,
            connection,
            sender_handle,
        }
    }
}

/// client_rx => tx
async fn send_messages_to_client(
    uuid: Uuid,
    mut client_rx: AsyncReceiver<ServerMessage>,
    mut tx: Tx,
) -> Result<Tx> {
    while let Some(server_msg) = client_rx.recv().await {
        let server_msg = server_msg.serialize()?;
        tx.send(server_msg.into()).await?;
    }

    info!("Stopped sending server messages to {uuid}");
    Ok(tx)
}

async fn send_disconnect_and_close(mut tx: Tx) -> Result<()> {
    tx.send(ServerMessage::Disconnect.serialize().unwrap().into())
        .await?;
    tx.flush().await?;
    tx.close().await?;

    Ok(())
}

/// rx => in_tx
async fn receive_messages_from_client(
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

    info!("Stopped receiving client messages from {uuid}");
    Ok(())
}

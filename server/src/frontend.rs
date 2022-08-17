use uuid::Uuid;

use wgpu_block_shared::protocol::{ClientMessage, ServerMessage};

pub trait Frontend: Clone {
    fn iter_messages(&self) -> Box<dyn Iterator<Item = InboundMessage> + '_>;
    fn broadcast(&self, server_msg: ServerMessage);
    fn send(&self, uuid: Uuid, server_msg: ServerMessage);
}

/// Wrapper around [`ClientMessage`] that includes the one-time client uuid.
pub enum InboundMessage {
    Message {
        uuid: Uuid,
        client_msg: ClientMessage,
    },
    AddClient {
        uuid: Uuid,
    },
    RemoveClient {
        uuid: Uuid,
    },
}

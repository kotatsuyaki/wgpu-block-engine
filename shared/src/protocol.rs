use quinn::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use uuid::Uuid;

use crate::chunk::{BlockId, Chunk};
use crate::Error;

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ClientMessage {
    Ping,
    Login,
    SetPlayerPos {
        eye: (f32, f32, f32),
        pitch: f32,
        yaw: f32,
    },
    DestroyBlock {
        x: i64,
        y: i64,
        z: i64,
    },
    PlaceBlock {
        x: i64,
        y: i64,
        z: i64,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[non_exhaustive]
pub enum ServerMessage {
    Pong {
        data: i64,
    },
    LoadChunk {
        cx: i64,
        cz: i64,
        chunk: Chunk,
    },
    UnloadChunk {
        cx: i64,
        cz: i64,
    },
    UpdateBlock {
        x: i64,
        y: i64,
        z: i64,
        block: BlockId,
    },
    SetClientInfo {
        uuid: Uuid,
    },
    Disconnect,
}

impl ClientMessage {
    pub fn serialize(&self) -> Result<Vec<u8>, Error> {
        Ok(bincode::serialize(self)?)
    }

    pub fn deserialize<T: AsRef<[u8]>>(bytes: T) -> Result<Self, Error> {
        Ok(bincode::deserialize(bytes.as_ref().into())?)
    }
}

impl ServerMessage {
    pub fn serialize(&self) -> Result<Vec<u8>, Error> {
        Ok(bincode::serialize(self)?)
    }

    pub fn deserialize<T: AsRef<[u8]>>(bytes: T) -> Result<Self, Error> {
        Ok(bincode::deserialize(bytes.as_ref().into())?)
    }
}

pub type Tx = FramedWrite<SendStream, LengthDelimitedCodec>;
pub type Rx = FramedRead<RecvStream, LengthDelimitedCodec>;

pub fn make_framed(tx: SendStream, rx: RecvStream) -> (Tx, Rx) {
    (
        FramedWrite::new(tx, LengthDelimitedCodec::new()),
        FramedRead::new(rx, LengthDelimitedCodec::new()),
    )
}

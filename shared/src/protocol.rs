use crate::Error;
use quinn::{RecvStream, SendStream};
use serde::{Deserialize, Serialize};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ClientMessage {
    Ping,
}

impl ClientMessage {
    pub fn serialize(&self) -> Result<Vec<u8>, Error> {
        Ok(bincode::serialize(self)?)
    }

    pub fn deserialize<T: AsRef<[u8]>>(bytes: T) -> Result<Self, Error> {
        Ok(bincode::deserialize(bytes.as_ref().into())?)
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ServerMessage {
    Pong { data: i64 },
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

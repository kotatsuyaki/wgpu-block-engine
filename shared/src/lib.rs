pub mod chunk;
pub mod protocol;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Serialization error")]
    Bincode(#[from] bincode::Error),
}

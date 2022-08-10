pub mod chunk;
pub mod protocol;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Serialization error")]
    Bincode(#[from] bincode::Error),
}

#[macro_export]
macro_rules! catch {
    ($e: expr) => {
        if let Err(e) = $e {
            ::tracing::warn!(?e);
        }
    };
}

#[macro_export]
macro_rules! catch_return {
    ($e: expr, $v: expr) => {
        if let Err(e) = $e {
            ::tracing::warn!(?e);
            return $v;
        }
    };
}

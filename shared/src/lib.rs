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

pub mod asyncutils {
    use std::future::Future;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use tokio::task::JoinHandle;

    pub type Shared<T> = Arc<RwLock<T>>;

    pub fn make_shared<T>(value: T) -> Shared<T> {
        Arc::new(RwLock::new(value))
    }

    /// Trailing `.spawn()` for futures
    pub trait SpawnFutureExt: Future {
        fn spawn(self) -> JoinHandle<Self::Output>;
    }

    impl<T> SpawnFutureExt for T
    where
        T: Future + Send + 'static + Sized,
        T::Output: Send + 'static,
    {
        fn spawn(self) -> JoinHandle<Self::Output> {
            tokio::spawn(self)
        }
    }
}

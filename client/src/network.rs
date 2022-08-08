use std::{net::AddrParseError, sync::Arc};

use futures::SinkExt;
use quinn::{ClientConfig, ConnectError, ConnectionError, Endpoint, NewConnection};
use thiserror::Error;

use tokio_stream::StreamExt;
use tracing::info;
use wgpu_block_shared::protocol::{self, Message};

pub async fn run() -> Result<(), Error> {
    let mut endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;
    let client_config = make_client_config();
    endpoint.set_default_client_config(client_config);

    let NewConnection {
        connection: conn, ..
    } = endpoint
        .connect("127.0.0.1:5000".parse()?, "localhost")?
        .await?;
    let (tx, rx) = conn.open_bi().await?;
    let (mut tx, mut rx) = protocol::make_framed(tx, rx);

    let ping_msg = Message::Ping;
    tx.send(ping_msg.serialize()?.into()).await?;

    while let Some(msg) = rx.next().await {
        let msg = msg?;
        let msg = Message::deserialize(msg)?;
        info!("Got message from server: {msg:?}");
    }

    info!("Server closed bidirectional stream");
    tx.close().await?;

    Ok(())
}

fn make_client_config() -> ClientConfig {
    let crypto = rustls::ClientConfig::builder()
        .with_safe_defaults()
        .with_custom_certificate_verifier(SkipServerVerification::new())
        .with_no_client_auth();

    ClientConfig::new(Arc::new(crypto))
}

struct SkipServerVerification;

impl SkipServerVerification {
    fn new() -> Arc<Self> {
        Arc::new(Self)
    }
}

impl rustls::client::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::Certificate,
        _intermediates: &[rustls::Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    IO(#[from] std::io::Error),

    #[error(transparent)]
    AddrParse(#[from] AddrParseError),

    #[error(transparent)]
    NewConnectionFail(#[from] ConnectError),

    #[error(transparent)]
    ConnectionLost(#[from] ConnectionError),

    #[error(transparent)]
    Shared(#[from] wgpu_block_shared::Error),
}

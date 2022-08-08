use std::sync::Arc;

use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use quinn::{ClientConfig, Endpoint, NewConnection};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};
use tracing::info;
use wgpu_block_shared::protocol::Message;

fn main() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run())?;
    Ok(())
}

async fn run() -> Result<()> {
    tracing_subscriber::fmt().init();
    let client_config = configure_client();
    client(client_config).await?;

    Ok(())
}

async fn client(client_config: ClientConfig) -> Result<()> {
    let mut endpoint = Endpoint::client("0.0.0.0:0".parse().unwrap())?;
    endpoint.set_default_client_config(client_config);

    let new_connection = endpoint
        .connect("127.0.0.1:5000".parse().unwrap(), "localhost")?
        .await?;
    let NewConnection { connection, .. } = new_connection;
    let (tx, rx) = connection.open_bi().await?;
    let mut tx = FramedWrite::new(tx, LengthDelimitedCodec::new());
    let mut rx = FramedRead::new(rx, LengthDelimitedCodec::new());
    info!("Opened bidir");

    let msg = Message::Ping;
    let msg = bincode::serialize(&msg)?;
    tx.send(msg.into()).await?;
    info!("Sent message");

    let resp_msg = rx.next().await.context("rx.next() failed")??;
    let resp_msg: Message = bincode::deserialize(&resp_msg)?;
    info!("Received pong: {:?}", resp_msg);

    Ok(())
}

fn configure_client() -> ClientConfig {
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

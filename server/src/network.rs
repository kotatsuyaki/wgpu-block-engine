use anyhow::{Context, Result};
use futures::{SinkExt, StreamExt};
use quinn::{Endpoint, NewConnection, ServerConfig};
use tokio::{select, signal::ctrl_c, spawn};
use tracing::{info, trace, warn};

use wgpu_block_shared::protocol::{self, Message};

pub async fn run() {
    let (cert, key) = generate_self_signed_cert().expect("Failed to generate self-signed cert");
    let server_config =
        ServerConfig::with_single_cert(vec![cert], key).expect("Failed to create server config");
    let (_endpoint, mut incoming) =
        Endpoint::server(server_config, "127.0.0.1:5000".parse().unwrap())
            .expect("Failed to construct server");

    loop {
        select! {
            maybe_connecting = incoming.next() => {
                if let Some(connecting) = maybe_connecting {
                    let newconn = match connecting.await {
                        Ok(newconn) => newconn,
                        Err(e) => {
                            warn!("Failed connecting.await: {e:?}");
                            continue;
                        }
                    };

                    // handle the client in a new task
                    spawn(async {
                        if let Err(e) = handle_client_newconn(newconn).await {
                            warn!("Client newconn returned an error: {e:?}");
                        }
                    });
                } else {
                    info!("No more incoming connections");
                    return;
                }
            }
            // TODO: Move signal handling to parent system
            _ = async { ctrl_c().await.expect("failed to listen to ctrl-c") } => {
                info!("Received SIGINT");
                return;
            }
        }
    }
}

async fn handle_client_newconn(mut newconn: NewConnection) -> Result<()> {
    info!("Client newconn {newconn:?}");

    // wait for the bidir stream from the client
    let (tx, rx) = newconn
        .bi_streams
        .next()
        .await
        .context("bi_streams ended before the first bidirectional stream is opened")?
        .context("Connection error")?;
    let (mut tx, mut rx) = protocol::make_framed(tx, rx);

    while let Some(msg_raw) = rx.next().await {
        let msg_raw = msg_raw?;
        let msg = Message::deserialize(msg_raw)?;
        match msg {
            Message::Ping => {
                // respond with pong
                let pong = Message::Pong { data: 42 };
                let pong_raw = bincode::serialize(&pong)?;
                tx.send(pong_raw.into()).await?;

                // terminate connection
                info!("Terminating connection to client");
                tx.close().await?;
                break;
            }
            unhandled_msg => {
                trace!("Unhandled incoming message: {unhandled_msg:?}");
            }
        }
    }

    Ok(())
}

fn generate_self_signed_cert() -> Result<(rustls::Certificate, rustls::PrivateKey)> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let key = rustls::PrivateKey(cert.serialize_private_key_der());
    Ok((rustls::Certificate(cert.serialize_der()?), key))
}

/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::config_parser::ServerConfig;
use futures_util::StreamExt;
use native_tls::Identity;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio_native_tls::TlsAcceptor;
use tokio_stream::wrappers::WatchStream;

mod attachments;
mod chat_server;
mod client_connection;
mod manager;

pub use chat_server::ChatServer;

pub type Sender = mpsc::Sender<ServerCommandResponse>;

#[derive(Debug)]
pub enum ServerCommandResponse {
    Text(String),
    Disconnect(crate::commands::CommandError),
}

pub async fn start(config: &ServerConfig, shutdown_receiver: watch::Receiver<bool>) {
    // TODO: Remove these unwraps and propagate the errors to the caller.
    let pkcs12 = Identity::from_pkcs12(&config.cert, &config.cert_password).unwrap();
    let acceptor = {
        let a = TlsAcceptor::from(native_tls::TlsAcceptor::builder(pkcs12).build().unwrap());
        Arc::new(a)
    };

    let listener = TcpListener::bind(format!("{}:{}", &config.bind_ip, &config.port))
        .await
        .unwrap();
    let (sender, receiver) = mpsc::channel::<manager::Message>(32); // TODO: What should this be?
    let mut tasks = Vec::new();

    let db_path = config.db_path.clone();
    let worker_sd_recv = shutdown_receiver.clone();
    tasks.push(tokio::spawn(async move {
        manager::start(receiver, &db_path, worker_sd_recv).await;
    }));

    let mut shutdown_stream = WatchStream::new(shutdown_receiver.clone());
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (socket, addr) = result.expect("accept error");
                let tls_acceptor = acceptor.clone();
                let tx = sender.clone();
                let client_sd_recv = shutdown_receiver.clone();
                tasks.push(tokio::spawn(async move {
                    let websocket = {
                        let tls_stream = tls_acceptor.accept(socket).await.expect("accept error");
                        tokio_tungstenite::accept_async(tls_stream).await.expect("accept error")
                    };

                    // This sets up the appropriate channels so the manager can communicate with this new client.
                    if let Err(_) = client_connection::process_client(addr, websocket, tx, client_sd_recv).await {
                        // Just logging these errors for now. This may end up being too noisy
                        // yes, these are too noisy
                        //println!("Error in process_client: {}", e);
                    }
                }));
            }
            result = shutdown_stream.next() => {
                if let Some(shutdown) = result {
                    if shutdown {
                        break;
                    }
                }
            }
        }
    }

    // wait for everything else to shutdown before returning.
    futures::future::join_all(tasks).await;
}

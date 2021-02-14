/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use super::channel::Channel;
use super::client_connection;
use super::commands;
use super::commands::CommandError;
use super::config_parser::ServerConfig;
use super::db_interaction;
use super::user::{UnauthedUser, User};
use futures_util::StreamExt;
use native_tls::Identity;
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_native_tls::{TlsAcceptor, TlsStream};

pub type WebSocketStream = tokio_tungstenite::WebSocketStream<TlsStream<TcpStream>>;
pub type Sender = mpsc::Sender<ServerCommandResponse>;

#[derive(Debug)]
pub enum Message {
    NewConnection((SocketAddr, mpsc::Sender<ServerCommandResponse>)),
    NewData((SocketAddr, String)),
    Disconnected(SocketAddr),
    CheckUnauthUsers,
}

#[derive(Debug)]
pub enum ServerCommandResponse {
    Text(String),
    Disconnect(commands::CommandError),
}

struct WorkerStream {
    receiver: Pin<Box<dyn tokio_stream::Stream<Item = Message> + Send>>,
    timer: tokio::time::Interval,
}

impl tokio_stream::Stream for WorkerStream {
    type Item = Message;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Poll::Ready(Some(v)) = Pin::new(&mut self.receiver).poll_next(cx) {
            Poll::Ready(Some(v))
        } 
        else if let Poll::Ready(_) = Pin::new(&mut self.timer).poll_tick(cx) {
            Poll::Ready(Some(Message::CheckUnauthUsers))
        } 
        else {
            Poll::Pending
        }
    }
}

#[derive(Clone)]
pub struct ChatServer {
    pub db_path: String,
    pub unauth_connections: HashMap<SocketAddr, UnauthedUser>,
    pub connections: HashMap<SocketAddr, String>,
    pub users: HashMap<String, User>,
    pub channels: HashMap<String, Channel>,
}

impl ChatServer {
    fn new(db_path: &str) -> Self {
        let users = db_interaction::get_users(db_path).unwrap();
        let channels = db_interaction::get_channels(db_path, &users).unwrap();

        ChatServer {
            db_path: db_path.to_string(),
            unauth_connections: HashMap::new(),
            connections: HashMap::new(),
            users: users,
            channels: channels,
        }
    }

    pub fn add_unauth_connection(&mut self, addr: SocketAddr, sender: Sender) {
        self.unauth_connections.insert(addr, UnauthedUser::new(sender));
    }

    pub fn remove_unauth_connection(&mut self, addr: SocketAddr) {
        self.unauth_connections.remove(&addr);
    }

    pub fn get_unauthed_connection(&self, addr: SocketAddr) -> Option<UnauthedUser> {
        if self.unauth_connections.contains_key(&addr) {
            Some(self.unauth_connections[&addr].clone())
        }
        else {
            None
        }
    }

    // Disconnects any unauthenticated users that haven't identified themselves in the right amount of time
    pub async fn disconnect_unauth_users(&self) {
        let now = Instant::now();
        let max_dur = std::time::Duration::from_secs(30);
        for client in self.unauth_connections.values().filter(|c| now.duration_since(c.time) > max_dur) {
            if let Err(e) = client.tx.send(ServerCommandResponse::Disconnect(CommandError::DidNotAuth)).await {
                println!("Couldn't send error to client who didn't auth in time: {}", e);
            } 
            else {
                println!("Disconnecting client who didn't auth in time");
            }
        }
    }

    pub async fn add_connection(&mut self, addr: SocketAddr, username: String, tx: Sender) {
        let mut status_changed = false;
        if let Some(user) = self.users.get_mut(&username) {
            status_changed = user.add_connection(addr, tx);
        }

        if status_changed {
            if let Some(user) = self.users.get(&username) {
                Self::send_user_status_update(&self.channels, &self.users, &user).await;
            }
        }
        self.connections.insert(addr, username);
    }

    async fn send_user_status_update(channels: &HashMap<String, Channel>, users: &HashMap<String, User>, user: &User) {
        for channel in channels.values().filter(|chan| chan.users.contains(&user.username)) {
            let json = json!({
                "cmd": "STATUS",
                "user": user,
            });
            channel.broadcast_filter(|username| users.get(username).cloned(), |u| u != user, &json.to_string()).await;
        }
    }

    // Removes the specified connection, and returns the connection's associated username if present
    pub async fn remove_connection(&mut self, addr: SocketAddr) {
        if self.connections.contains_key(&addr) {
            let username = &self.connections[&addr];
            let mut status_changed = false;
            if let Some(user) = self.users.get_mut(username) {
                status_changed = user.remove_connection(&addr);
            }

            // Hopefully in a future version of Rust this can be expressed in one if block.
            if status_changed {
                if let Some(user) = self.users.get(username) {
                    Self::send_user_status_update(&self.channels, &self.users, &user).await;
                }
            }

            self.connections.remove(&addr);
        }
    }

    pub fn add_user(&mut self, user: User) {
        self.users.insert(user.username.to_string(), user);
    }

    pub fn get_user(&self, addr: &SocketAddr) -> Option<&User> {
        if let Some(username) = self.connections.get(addr) {
            self.users.get(username)
        }
        else {
            None
        }
    }
}

async fn server_worker_impl(mut receiver: mpsc::Receiver<Message>, db_path: &str) {
    let mut server = ChatServer::new(db_path);

    // Construct a stream that receives data from either the client thread, or receives a
    // notification from a timer to check for non-reponsive unauthenticated users.
    let mut worker_stream = {
        let rs = Box::pin(async_stream::stream! {
            while let Some(item) = receiver.recv().await {
                yield item;
            }
        });

        WorkerStream {
            receiver: rs,
            timer: tokio::time::interval(std::time::Duration::from_secs(30)),
        }
    };

    let mut all_connections: HashMap<SocketAddr, Sender> = HashMap::new();

    while let Some(message) = worker_stream.next().await {
        match message {
            Message::NewConnection((addr, tx)) => {
                println!("New connection from: {}", addr);
                all_connections.insert(addr, tx.clone());
                server.add_unauth_connection(addr, tx);
            }
            Message::NewData((addr, data)) => {
                println!("Received data from {}: {}", addr, data);
                if let Err(e) = commands::process_command(&mut server, addr, &data).await {
                    println!("Disconnecting client due to invalid command: {}", e);
                    if let Err(e2) = all_connections[&addr].send(ServerCommandResponse::Disconnect(e)).await {
                        println!("Failed to disconnect client?! {}", e2); // should never happen, hopefully.
                    }
                }
            }
            Message::Disconnected(addr) => {
                println!("{} disconnected", addr);
                server.remove_unauth_connection(addr);
                server.remove_connection(addr).await;
                all_connections.remove(&addr);
            }
            Message::CheckUnauthUsers => {
                server.disconnect_unauth_users().await;
            }
        }
    }

    println!("Manager exiting.");
}

pub async fn start_server(config: &ServerConfig) {
    // TODO: Remove these unwraps and propagate the errors to the caller.
    let pkcs12 = Identity::from_pkcs12(&config.cert, &config.cert_password).unwrap();
    let acceptor = {
        let a = TlsAcceptor::from(native_tls::TlsAcceptor::builder(pkcs12).build().unwrap());
        Arc::new(a)
    };

    db_interaction::setup_database(&config.db_path).unwrap();

    let listener = TcpListener::bind(format!("{}:{}", &config.bind_ip, &config.port)).await.unwrap();
    let (sender, receiver) = mpsc::channel::<Message>(32); // TODO: What should this be?
    let db_path = config.db_path.clone();
    tokio::spawn(async move {
        server_worker_impl(receiver, &db_path).await;
    });

    loop {
        let (socket, addr) = listener.accept().await.unwrap();
        let tls_acceptor = acceptor.clone();
        let tx = sender.clone();
        tokio::spawn(async move {
            let websocket = {
                let tls_stream = tls_acceptor.accept(socket).await.expect("accept error");
                tokio_tungstenite::accept_async(tls_stream).await.expect("accept error")
            };

            // This sets up the appropriate channels so the manager can communicate with this new client.
            if let Err(_) = client_connection::process_client(addr, websocket, tx).await {
                // Just logging these errors for now. This may end up being too noisy
                // yes, these are too noisy
                //println!("Error in process_client: {}", e);
            }
        });
    }
}

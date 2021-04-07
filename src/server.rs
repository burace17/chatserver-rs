/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use super::channel::Channel;
use super::client_connection;
use super::commands;
use super::commands::CommandError;
use super::config_parser::ServerConfig;
use super::db::{DatabaseError, ChatDatabase};
use super::db::sqlite::SqliteChatDatabase;
use super::user::{UnauthedUser, User};
use super::attachments::AttachmentInfo;
use futures_util::StreamExt;
use multi_map::MultiMap;
use native_tls::Identity;
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio_native_tls::{TlsAcceptor, TlsStream};
use tokio_stream::wrappers::WatchStream;
use crate::attachments::start_attachment_manager;

pub type WebSocketStream = tokio_tungstenite::WebSocketStream<TlsStream<TcpStream>>;
pub type Sender = mpsc::Sender<ServerCommandResponse>;
type PinnedStream<T> = Pin<Box<dyn tokio_stream::Stream<Item = T> + Send>>;

#[derive(Debug)]
pub enum Message {
    NewConnection((SocketAddr, mpsc::Sender<ServerCommandResponse>)),
    NewData((SocketAddr, String)),
    Disconnected(SocketAddr),
    CheckUnauthUsers,
    GotAttachment(AttachmentInfo),
    ShutdownStatus(bool)
}

#[derive(Debug)]
pub enum ServerCommandResponse {
    Text(String),
    Disconnect(commands::CommandError),
}

struct WorkerStream {
    receiver: PinnedStream<Message>,
    timer: tokio::time::Interval,
    attachment_receiver: PinnedStream<Message>,
    shutdown_receiver: WatchStream<bool>
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
        else if let Poll::Ready(Some(v)) = Pin::new(&mut self.attachment_receiver).poll_next(cx) {
            Poll::Ready(Some(v))
        }
        else if let Poll::Ready(Some(shutdown)) = Pin::new(&mut self.shutdown_receiver).poll_next(cx) {
            Poll::Ready(Some(Message::ShutdownStatus(shutdown)))
        }
        else {
            Poll::Pending
        }
    }
}

pub struct ChatServer {
    pub db: Box<dyn ChatDatabase>,
    pub unauth_connections: HashMap<SocketAddr, UnauthedUser>,
    pub connections: HashMap<SocketAddr, String>,
    pub users: MultiMap<String, i64, User>,
    pub channels: HashMap<String, Channel>,
    url_sender: mpsc::Sender<AttachmentInfo>
}

impl ChatServer {
    async fn new(db_path: &str, url_sender: mpsc::Sender<AttachmentInfo>) -> Self {
        // In the future we will of course want to generalize this so it doesn't just make a sqlite pool.
        let pool = sqlx::SqlitePool::connect(db_path).await.unwrap();
        let db = SqliteChatDatabase::new(pool);

        // These cannot fail.
        db.setup_database().await.expect("Database setup failed");
        let users = db.get_users().await.expect("Could not read the users table");
        let channels = db.get_channels(&users).await.expect("Could not read the channels table");

        ChatServer {
            db: Box::new(db),
            unauth_connections: HashMap::new(),
            connections: HashMap::new(),
            users,
            channels,
            url_sender
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

    async fn send_user_status_update(channels: &HashMap<String, Channel>, users: &MultiMap<String, i64, User>, user: &User) {
        for channel in channels.values().filter(|chan| chan.users.contains(&user.username)) {
            let json = json!({
                "cmd": "STATUS",
                "user": user,
            });
            channel.broadcast_filter(|username| users.get(&username.to_owned()).cloned(), |u| u != user, &json.to_string()).await;
        }
    }

    // Removes the specified connection, and returns the connection's associated username if present
    pub async fn remove_connection(&mut self, addr: SocketAddr) {
        if self.connections.contains_key(&addr) {
            let username = &self.connections[&addr];
            let mut status_changed = false;
            let mut no_viewers: Option<Vec<String>> = None;
            if let Some(user) = self.users.get_mut(username) {
                status_changed = user.remove_connection(&addr);
                user.clear_viewing(&addr);
                no_viewers = Some(user.get_and_clear_no_viewers());
            }

            // Hopefully in a future version of Rust this can be expressed in one if block.
            if status_changed {
                if let Some(user) = self.users.get(username) {
                    Self::send_user_status_update(&self.channels, &self.users, &user).await;
                }
            }

            if let Some(no_viewer_channels) = no_viewers {
                if let Some(user) = self.users.get(username) {
                    if let Err(e) = self.send_no_viewer_notifications(&no_viewer_channels, &user).await {
                        println!("remove_connection(): failed to send no viewer notification: {}", e);
                    }
                }
            }

            self.connections.remove(&addr);
        }
    }

    pub fn add_user(&mut self, user: User) {
        self.users.insert(user.username.to_string(), user.id, user);
    }

    pub fn get_user(&self, addr: &SocketAddr) -> Option<&User> {
        if let Some(username) = self.connections.get(addr) {
            self.users.get(username)
        }
        else {
            None
        }
    }
    pub fn get_user_mut(&mut self, addr: &SocketAddr) -> Option<&mut User> {
        if let Some(username) = self.connections.get(addr) {
            self.users.get_mut(username)
        }
        else {
            None
        }
    }

    pub async fn send_no_viewer_notifications(&self, channels: &Vec<String>, user: &User) -> Result<(), DatabaseError> {
        for channel in channels.iter().filter_map(|name| self.channels.get(name.as_str())) {
            let msg_id = self.db.set_last_message_read(user.id, channel.id).await?;
            let json = json!({
                "cmd": "NOVIEWERS",
                "channel": channel.name,
                "message_id": msg_id
            });
            user.send_to_all(&json.to_string()).await;
        }
        Ok(())
    }

    async fn add_attachment_to_message(&self, info: &AttachmentInfo) -> Result<(), DatabaseError> {
        self.db.add_message_attachment(info.message_id, &info.url, &info.mime).await?;
        if let Some(channel) = self.channels.values().find(|c| c.id == info.channel_id) {
            let json = json!({
                "cmd" : "ADDATTACHMENT",
                "channel" : channel.name,
                "message_id" : info.message_id,
                "url" : info.url,
                "mime" : info.mime
            });

            channel.broadcast(|username| self.users.get(&username.to_owned()).cloned(), &json.to_string()).await;
        }
        Ok(())
    }

    pub async fn query_for_attachments(&self, channel_id: i64, message_id: i64, url: &str) -> Result<(), mpsc::error::SendError<AttachmentInfo>> {
        let info = AttachmentInfo::new(channel_id, message_id, url);
        self.url_sender.send(info).await?;
        Ok(())
    }
}

async fn server_worker_impl(mut receiver: mpsc::Receiver<Message>, db_path: &str, shutdown_receiver: watch::Receiver<bool>) {
    let shutdown_clone = shutdown_receiver.clone();
    let shutdown_stream = WatchStream::new(shutdown_receiver);
    let (mut attachment_receiver, url_sender, attachment_task) = start_attachment_manager(shutdown_clone);
    let mut server = ChatServer::new(db_path, url_sender).await;

    // Construct a stream that receives data from either the client thread, or a
    // notification from the timer to check for non-responsive unauthenticated users,
    // or an attachment from the attachment manager task.
    let mut worker_stream = {
        let receiver_pin = Box::pin(async_stream::stream! {
            while let Some(item) = receiver.recv().await {
                yield item;
            }
        });
        let attachment_receiver_pin = Box::pin(async_stream::stream! {
            while let Some(item) = attachment_receiver.recv().await {
                yield item;
            }
        });

        WorkerStream {
            receiver: receiver_pin,
            timer: tokio::time::interval(std::time::Duration::from_secs(30)),
            attachment_receiver: attachment_receiver_pin,
            shutdown_receiver: shutdown_stream
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
            Message::GotAttachment(media_info) => {
                if let Err(e) = server.add_attachment_to_message(&media_info).await {
                    println!("Failed to send ADDATTACHMENT command: {}", e);
                }
            }
            Message::ShutdownStatus(do_it) => {
                if do_it {
                    break;
                }
            }
        }
    }

    attachment_task.await.expect("Attachment manager task panicked before shutdown.");
}

pub async fn start_server(config: &ServerConfig, shutdown_receiver: watch::Receiver<bool>) {
    // TODO: Remove these unwraps and propagate the errors to the caller.
    let pkcs12 = Identity::from_pkcs12(&config.cert, &config.cert_password).unwrap();
    let acceptor = {
        let a = TlsAcceptor::from(native_tls::TlsAcceptor::builder(pkcs12).build().unwrap());
        Arc::new(a)
    };

    let listener = TcpListener::bind(format!("{}:{}", &config.bind_ip, &config.port)).await.unwrap();
    let (sender, receiver) = mpsc::channel::<Message>(32); // TODO: What should this be?
    let mut tasks = Vec::new();

    let db_path = config.db_path.clone();
    let worker_sd_recv = shutdown_receiver.clone();
    tasks.push(tokio::spawn(async move {
        server_worker_impl(receiver, &db_path, worker_sd_recv).await;
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

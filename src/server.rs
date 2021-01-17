use native_tls::Identity;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::net::SocketAddr;
use std::time::Instant;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_native_tls::{TlsAcceptor, TlsStream};
use super::config_parser::ServerConfig;
use super::client_connection;
use super::commands;

pub type WebSocketStream = tokio_tungstenite::WebSocketStream<TlsStream<TcpStream>>;
pub type Sender = mpsc::Sender<ServerCommandResponse>;

/* pub struct User {
    username: String,
    nickname: String,
    connections: HashSet<SocketAddr>
} */

#[derive(Clone)]
pub struct UnauthedUser {
    pub tx: Sender,
    pub time: Instant
}

impl UnauthedUser {
    fn new(tx: Sender, time: Instant) -> Self {
        UnauthedUser{ tx, time }
    }
}

pub struct ChatServer {
    unauth_connections: HashMap<SocketAddr, UnauthedUser>,
    connections: HashMap<SocketAddr, String>,
    //users: HashMap<String, User>
}

impl ChatServer {
    fn new() -> Self {
        ChatServer{
            unauth_connections: HashMap::new(),
            connections: HashMap::new(),
            //users: HashMap::new()
        }
    }

    pub fn add_unauth_connection(&mut self, addr: SocketAddr, sender: Sender) {
        println!("Added unauth connection: {}", addr);
        self.unauth_connections.insert(addr, UnauthedUser::new(sender, Instant::now()));
    }

    pub fn remove_unauth_connection(&mut self, addr: SocketAddr) {
        println!("Removed unauth connection: {}", addr);
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

    pub fn add_connection(&mut self, addr: SocketAddr, username: String) {
        println!("Added connection: {}", addr);
        self.connections.insert(addr, username);
    }

    pub fn remove_connection(&mut self, addr: SocketAddr) {
        println!("Removed connection: {}", addr);
        // TODO: update the sockets stored in users.
        self.connections.remove(&addr);
    }
}

#[derive(Debug)]
pub enum Message {
    NewConnection((SocketAddr, mpsc::Sender<ServerCommandResponse>)),
    NewData((SocketAddr, String)),
    Disconnected(SocketAddr)
}

#[derive(Debug)]
pub enum ServerCommandResponse {
    Text(String),
    Disconnect(String)
}

async fn server_worker_impl(mut receiver: mpsc::Receiver<Message>) {
    let mut server = ChatServer::new();
    let mut all_connections: HashMap<SocketAddr, Sender> = HashMap::new();

    while let Some(message) = receiver.recv().await {
        match message {
            Message::NewConnection((addr, tx)) => {
                println!("New connection from: {}", addr);
                all_connections.insert(addr, tx.clone());
                server.add_unauth_connection(addr, tx);
            },
            Message::NewData((addr, data)) => {
                println!("Received data from {}: {}", addr, data);
                if let Err(e) = commands::process_command(&mut server, addr, &data).await {
                    println!("Disconnecting client due to invalid command: {}", e);
                    if let Err(e2) = all_connections[&addr].send(ServerCommandResponse::Disconnect(e.to_string())).await {
                        println!("Failed to disconnect client?! {}", e2); // should never happen, hopefully.
                    }
                }
            },
            Message::Disconnected(addr) => {
                println!("{} disconnected", addr);
                server.remove_unauth_connection(addr);
                server.remove_connection(addr);
                all_connections.remove(&addr);
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

    let listener = TcpListener::bind(format!("{}:{}", &config.bind_ip, &config.port)).await.unwrap();
    let (sender, receiver) = mpsc::channel::<Message>(32); // TODO: What should this be?
    tokio::spawn(async move {
         server_worker_impl(receiver).await;
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
            if let Err(e) = client_connection::process_client(addr, websocket, tx).await {
                // Just logging these errors for now. This may end up being too noisy
                println!("Error in process_client: {}", e);
            }
        });
    }
}

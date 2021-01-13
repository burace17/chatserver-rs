use native_tls::Identity;
use std::sync::Arc;
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_native_tls::{TlsAcceptor, TlsStream};
use super::config_parser::ServerConfig;
use super::client_connection;

pub type WebSocketStream = tokio_tungstenite::WebSocketStream<TlsStream<TcpStream>>;

struct ChatServer {

}

#[derive(Debug)]
pub enum Message {
    NewConnection((SocketAddr, mpsc::Sender<String>)),
    NewData((SocketAddr, String))
}

async fn server_worker_impl(mut receiver: mpsc::Receiver<Message>) {
    let mut _server = ChatServer{};

    while let Some(_message) = receiver.recv().await {

    }
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

use tungstenite::accept;
use native_tls::{Identity, TlsAcceptor};
use std::net::{TcpListener};
use std::sync::Arc;
use std::thread;
use super::config_parser::ServerConfig;

pub struct ChatServer {

}

impl ChatServer {
    pub fn new() -> ChatServer {
        ChatServer{}
    }

    pub fn start(&self, config: &ServerConfig) {
        let pkcs12 = Identity::from_pkcs12(&config.cert, &config.cert_password).unwrap();
        let acceptor = TlsAcceptor::new(pkcs12).unwrap();
        let acceptor = Arc::new(acceptor);
        let listener = TcpListener::bind(format!("{}:{}", &config.bind_ip, &config.port)).unwrap();

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let acceptor = acceptor.clone();
                    thread::spawn(move || {
                        let stream = acceptor.accept(stream).unwrap();
                        let mut websocket = accept(stream).unwrap();

                        loop {
                            let msg = websocket.read_message().unwrap();
                            if msg.is_text() {
                                websocket.write_message(msg).unwrap();
                            }
                        }

                    });
                }
                Err(_e) => { /* connection failed */ }
            }
        }
    }
}

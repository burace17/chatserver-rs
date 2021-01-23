use std::collections::HashSet;
use std::net::SocketAddr;
use std::time::Instant;
use super::server::Sender;

pub struct User {
    pub id: i64,
    pub username: String,
    pub nickname: String,
    connections: HashSet<SocketAddr>
}

impl User {
    pub fn new(id: i64, username: &str, nickname: &str) -> Self {
        User{
            id: id,
            username: username.to_string(),
            nickname: nickname.to_string(),
            connections: HashSet::new()
        }
    }

    pub fn add_connection(&mut self, addr: SocketAddr) {
        self.connections.insert(addr);
    }

    pub fn remove_connection(&mut self, addr: &SocketAddr) {
        self.connections.remove(&addr);
    }
}

#[derive(Clone)]
pub struct UnauthedUser {
    pub tx: Sender,
    pub time: Instant
}

impl UnauthedUser {
    pub fn new(tx: Sender) -> Self {
        UnauthedUser{ tx, time: Instant::now() }
    }
}
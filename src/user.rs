/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
 
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Instant;
use super::server::Sender;
use super::server::ServerCommandResponse;
use serde::{Serialize, Serializer};
use serde::ser::SerializeStruct;

#[derive(Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub nickname: String,
    connections: HashMap<SocketAddr, Sender>,
}

impl User {
    pub fn new(id: i64, username: &str, nickname: &str) -> Self {
        User{
            id: id,
            username: username.to_string(),
            nickname: nickname.to_string(),
            connections: HashMap::new(),
        }
    }

    pub fn add_connection(&mut self, addr: SocketAddr, tx: Sender) {
        self.connections.insert(addr, tx);
    }

    pub fn remove_connection(&mut self, addr: &SocketAddr) {
        self.connections.remove(&addr);
    }

    // Sends a message to all connections this user has.
    pub async fn send_to_all(&self, data: &str) {
        for tx in self.connections.values() {
            if let Err(e) = tx.send(ServerCommandResponse::Text(data.to_string())).await {
                println!("user:send_to_all(): failed: {}", e);
            }
        }
    }
}

impl std::hash::Hash for User {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id.hash(state);
        self.username.hash(state);
        self.nickname.hash(state);
    }
}

impl Serialize for User {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error> where S: Serializer {
        let mut s = serializer.serialize_struct("User", 3)?;
        s.serialize_field("id", &self.id)?;
        s.serialize_field("username", &self.username)?;
        s.serialize_field("nickname", &self.nickname)?;
        s.end()
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
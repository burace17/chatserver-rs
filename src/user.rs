/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Instant;
use super::server::Sender;
use super::server::ServerCommandResponse;
use serde::{Serialize, Serializer, Deserialize};
use serde::ser::SerializeStruct;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub enum OnlineStatus {
    Online,
    Away,
    Offline
}

#[derive(Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub nickname: String,
    connections: HashMap<SocketAddr, Sender>,
    status: OnlineStatus
}

impl User {
    pub fn new(id: i64, username: &str, nickname: &str) -> Self {
        User{
            id,
            username: username.to_string(),
            nickname: nickname.to_string(),
            connections: HashMap::new(),
            status: OnlineStatus::Offline
        }
    }

    // Returns whether the online status of the user changed as a result of this.
    pub fn add_connection(&mut self, addr: SocketAddr, tx: Sender) -> bool {
        let status_changed = self.status != OnlineStatus::Online;
        self.connections.insert(addr, tx);
        if status_changed {
            self.status = OnlineStatus::Online;
        }
        status_changed
    }

    // Returns whether the online status of the user changed as a result of this.
    pub fn remove_connection(&mut self, addr: &SocketAddr) -> bool {
        self.connections.remove(&addr);
        let going_offline = self.connections.len() == 0;
        if going_offline {
            self.status = OnlineStatus::Offline;
        }
        going_offline
    }

    // Sends a message to all connections this user has.
    pub async fn send_to_all(&self, data: &str) {
        for tx in self.connections.values() {
            if let Err(e) = tx.send(ServerCommandResponse::Text(data.to_string())).await {
                println!("user:send_to_all(): failed: {}", e);
            }
        }
    }

    pub async fn send_to(&self, addr: &SocketAddr, data: &str) {
        if let Some(tx) = self.connections.get(addr) {
            if let Err(e) = tx.send(ServerCommandResponse::Text(data.to_string())).await {
                println!("user:send_to(): failed: {}", e);
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
        s.serialize_field("status", &self.status)?;
        s.end()
    }
}

impl std::cmp::PartialEq for User {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
     && self.username == other.username
     && self.nickname == other.nickname && self.status == other.status
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
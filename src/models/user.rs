/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::server::Sender;
use crate::server::ServerCommandResponse;
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::time::Instant;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub enum OnlineStatus {
    Online,
    Away,
    Offline,
}

#[derive(Clone)]
pub struct User {
    pub id: i64,
    pub username: String,
    pub nickname: String,
    connections: HashMap<SocketAddr, Sender>,
    active_channels: HashMap<String, HashSet<SocketAddr>>,
    status: OnlineStatus,
}

impl User {
    pub fn new(id: i64, username: &str, nickname: &str) -> Self {
        User {
            id,
            username: username.to_string(),
            nickname: nickname.to_string(),
            connections: HashMap::new(),
            active_channels: HashMap::new(),
            status: OnlineStatus::Offline,
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

    // Updates the viewing status of the specified channel.
    pub fn set_viewing(&mut self, channel: &str, addr: &SocketAddr, is_viewing: bool) {
        if is_viewing {
            self.clear_viewing(addr);
            let viewers = self
                .active_channels
                .entry(channel.to_string())
                .or_insert_with(|| HashSet::new());
            //println!("User: {}: now viewing {}", self.username, channel);
            viewers.insert(*addr);
        } else {
            if let Some(viewers) = self.active_channels.get_mut(channel) {
                //println!("set_viewing(): {}: no longer viewing {}", self.username, channel);
                viewers.remove(addr);
            }
        }
    }

    pub fn clear_viewing(&mut self, addr: &SocketAddr) {
        let viewed_channel = self
            .active_channels
            .iter()
            .filter(|(_, v)| v.contains(addr))
            .map(|(channel, _)| channel.to_string())
            .next();

        if let Some(prev_channel) = viewed_channel {
            //println!("clear_viewing(): {}: no longer viewing {}", self.username, prev_channel);
            self.set_viewing(&prev_channel, addr, false);
        }
    }

    pub fn get_and_clear_no_viewers(&mut self) -> Vec<String> {
        let entries_to_remove: Vec<String> = self
            .active_channels
            .iter()
            .filter(|(_, viewers)| viewers.len() == 0)
            .map(|(name, _)| name.to_string())
            .collect();

        for name in entries_to_remove.iter() {
            //println!("get_and_clear_no_viewers(): {}: no more viewers for {}", self.username, name);
            self.active_channels.remove(name.as_str());
        }

        entries_to_remove
    }

    pub fn get_viewed_channels(&self) -> Vec<String> {
        self.active_channels.keys().map(|c| c.to_string()).collect()
    }

    // Sends a message to all connections this user has.
    pub async fn send_to_all(&self, data: &str) {
        for tx in self.connections.values() {
            if let Err(e) = tx.send(ServerCommandResponse::Text(data.to_string())).await {
                println!("user:send_to_all(): failed: {}", e);
            }
        }
    }

    // Sends a message to all connections viewing the specified channel.
    /*pub async fn send_to_all_viewing(&self, channel: &str, data: &str) {
        if let Some(addrs) = self.active_channels.get(channel) {
            for tx in addrs.iter().filter_map(|addr| self.connections.get(addr)) {
                if let Err(e) = tx.send(ServerCommandResponse::Text(data.to_string())).await {
                    println!("user:send_to_all_viewing(): failed: {}", e);
                }
            }
        }
    }*/

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
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
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
            && self.nickname == other.nickname
            && self.status == other.status
    }
}

#[derive(Clone)]
pub struct UnauthedUser {
    pub tx: Sender,
    pub time: Instant,
}

impl UnauthedUser {
    pub fn new(tx: Sender) -> Self {
        UnauthedUser {
            tx,
            time: Instant::now(),
        }
    }
}

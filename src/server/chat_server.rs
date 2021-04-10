/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use super::attachments::AttachmentInfo;
use super::{Sender, ServerCommandResponse};
use crate::channel::Channel;
use crate::commands::CommandError;
use crate::db::sqlite::SqliteChatDatabase;
use crate::db::{ChatDatabase, DatabaseError};
use crate::user::{UnauthedUser, User};
use multi_map::MultiMap;
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Instant;
use tokio::sync::mpsc;

pub struct ChatServer {
    pub db: Box<dyn ChatDatabase>,
    pub users: MultiMap<String, i64, User>,
    pub channels: HashMap<String, Channel>,
    unauth_connections: HashMap<SocketAddr, UnauthedUser>,
    connections: HashMap<SocketAddr, String>,
    url_sender: mpsc::Sender<AttachmentInfo>,
}

impl ChatServer {
    pub async fn new(db_path: &str, url_sender: mpsc::Sender<AttachmentInfo>) -> Self {
        // In the future we will of course want to generalize this so it doesn't just make a sqlite pool.
        let pool = sqlx::SqlitePool::connect(db_path).await.unwrap();
        let db = SqliteChatDatabase::new(pool);

        // These cannot fail.
        db.setup_database().await.expect("Database setup failed");
        let users = db
            .get_users()
            .await
            .expect("Could not read the users table");
        let channels = db
            .get_channels(&users)
            .await
            .expect("Could not read the channels table");

        ChatServer {
            db: Box::new(db),
            unauth_connections: HashMap::new(),
            connections: HashMap::new(),
            users,
            channels,
            url_sender,
        }
    }

    pub fn add_unauth_connection(&mut self, addr: SocketAddr, sender: Sender) {
        self.unauth_connections
            .insert(addr, UnauthedUser::new(sender));
    }

    pub fn remove_unauth_connection(&mut self, addr: SocketAddr) {
        self.unauth_connections.remove(&addr);
    }

    pub fn get_unauthed_connection(&self, addr: SocketAddr) -> Option<UnauthedUser> {
        if self.unauth_connections.contains_key(&addr) {
            Some(self.unauth_connections[&addr].clone())
        } else {
            None
        }
    }

    // Disconnects any unauthenticated users that haven't identified themselves in the right amount of time
    pub async fn disconnect_unauth_users(&self) {
        let now = Instant::now();
        let max_dur = std::time::Duration::from_secs(30);
        for client in self
            .unauth_connections
            .values()
            .filter(|c| now.duration_since(c.time) > max_dur)
        {
            if let Err(e) = client
                .tx
                .send(ServerCommandResponse::Disconnect(CommandError::DidNotAuth))
                .await
            {
                println!(
                    "Couldn't send error to client who didn't auth in time: {}",
                    e
                );
            } else {
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

    async fn send_user_status_update(
        channels: &HashMap<String, Channel>,
        users: &MultiMap<String, i64, User>,
        user: &User,
    ) {
        for channel in channels
            .values()
            .filter(|chan| chan.users.contains(&user.username))
        {
            let json = json!({
                "cmd": "STATUS",
                "user": user,
            });
            channel
                .broadcast_filter(
                    |username| users.get(&username.to_owned()).cloned(),
                    |u| u != user,
                    &json.to_string(),
                )
                .await;
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
                    if let Err(e) = self
                        .send_no_viewer_notifications(&no_viewer_channels, &user)
                        .await
                    {
                        println!(
                            "remove_connection(): failed to send no viewer notification: {}",
                            e
                        );
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
        } else {
            None
        }
    }
    pub fn get_user_mut(&mut self, addr: &SocketAddr) -> Option<&mut User> {
        if let Some(username) = self.connections.get(addr) {
            self.users.get_mut(username)
        } else {
            None
        }
    }

    pub async fn send_no_viewer_notifications(
        &self,
        channels: &Vec<String>,
        user: &User,
    ) -> Result<(), DatabaseError> {
        for channel in channels
            .iter()
            .filter_map(|name| self.channels.get(name.as_str()))
        {
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

    pub async fn add_attachment_to_message(
        &self,
        info: &AttachmentInfo,
    ) -> Result<(), DatabaseError> {
        self.db
            .add_message_attachment(info.message_id, &info.url, &info.mime)
            .await?;
        if let Some(channel) = self.channels.values().find(|c| c.id == info.channel_id) {
            let json = json!({
                "cmd" : "ADDATTACHMENT",
                "channel" : channel.name,
                "message_id" : info.message_id,
                "url" : info.url,
                "mime" : info.mime
            });

            channel
                .broadcast(
                    |username| self.users.get(&username.to_owned()).cloned(),
                    &json.to_string(),
                )
                .await;
        }
        Ok(())
    }

    pub async fn query_for_attachments(
        &self,
        channel_id: i64,
        message_id: i64,
        url: &str,
    ) -> Result<(), mpsc::error::SendError<AttachmentInfo>> {
        let info = AttachmentInfo::new(channel_id, message_id, url);
        self.url_sender.send(info).await?;
        Ok(())
    }
}

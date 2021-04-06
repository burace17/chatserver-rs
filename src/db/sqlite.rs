/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use async_trait::async_trait;
use futures::TryStreamExt;
use multi_map::MultiMap;
use sodiumoxide::crypto::pwhash::argon2i13::{pwhash_verify, HashedPassword};
use std::collections::HashMap;
use super::{ChatDatabase, DatabaseError, pwhash_interactive};
use crate::user::User;
use crate::channel::Channel;
use crate::message::{Message, MessageAttachment};
use sqlx::{Executor, Row};

pub struct SqliteChatDatabase {
    pool: sqlx::Pool<sqlx::Sqlite>
}

impl SqliteChatDatabase {
    pub fn new(pool: sqlx::Pool<sqlx::Sqlite>) -> Self {
        SqliteChatDatabase {
            pool
        }
    }
}

#[async_trait]
impl ChatDatabase for SqliteChatDatabase {
    async fn setup_database(&self) -> Result<(), DatabaseError> {
        let mut conn = self.pool.acquire().await?;
        (&mut conn).execute(include_str!("table_initialization.sql")).await?;
        Ok(())
    }
    async fn register_user(&self, username: &str, password: &str) -> Result<i64, DatabaseError> {
        sodiumoxide::init().ok().ok_or(DatabaseError::PasswordHashError)?;
        let mut conn = self.pool.acquire().await?;
        let pwh = pwhash_interactive(password.as_bytes()).ok().ok_or(DatabaseError::PasswordHashError)?;
        let pwh_ref = pwh.as_ref();
        let res = sqlx::query!("INSERT INTO users VALUES (null, ?, ?, ?);", username, username, pwh_ref)
            .execute(&mut conn).await?;

        Ok(res.last_insert_rowid())
    }
    async fn verify_login(&self, username: &str, password: &str) -> Result<bool, DatabaseError> {
        sodiumoxide::init().ok().ok_or(DatabaseError::PasswordHashError)?;
        let mut conn = self.pool.acquire().await?;
        let res = sqlx::query!("SELECT password FROM users WHERE username = ?;", username)
            .fetch_optional(&mut conn).await?;
        if let Some(record) = res {
            let hashed_password = HashedPassword::from_slice(record.password.as_ref()).ok_or(DatabaseError::PasswordHashError)?;
            Ok(pwhash_verify(&hashed_password, password.as_bytes()))
        }
        else {
            Ok(false)
        }
    }
    async fn get_users(&self) -> Result<MultiMap<String, i64, User>, DatabaseError> {
        let mut users = MultiMap::new();
        let mut conn = self.pool.acquire().await?;
        let user_records = sqlx::query!("SELECT id, username, nickname FROM users;").fetch_all(&mut conn).await?;

        for record in user_records {
            let user = User::new(record.id, &record.username, &record.nickname);
            users.insert(record.username.to_string(), record.id, user);
        }
        Ok(users)
    }
    async fn get_channels(&self, users: &MultiMap<String, i64, User>) -> Result<HashMap<String, Channel>, DatabaseError> {
        let mut channels: HashMap<String, Channel> = HashMap::new();
        let mut conn = self.pool.acquire().await?;
        let channel_records = sqlx::query!("SELECT channels.id,channels.name,users.username FROM channels
                                    LEFT JOIN user_channels ON channels.id = user_channels.channel_id
                                    LEFT JOIN users ON user_channels.user_id = users.id;").fetch_all(&mut conn).await?;

        for record in channel_records {
            let chan = channels.entry(record.name.to_string()).or_insert_with(|| Channel::new(record.id, &record.name));
            if let Some(username) = record.username {
                let user = users.get(&username).unwrap(); // if this fails, the database is messed up.
                chan.add_user(user);
            }
        }
        Ok(channels)
    }
    async fn make_channel(&self, name: &str) -> Result<i64, DatabaseError> {
        let mut conn = self.pool.acquire().await?;
        let res = sqlx::query!("INSERT INTO channels VALUES (null, ?);", name).execute(&mut conn).await?;
        Ok(res.last_insert_rowid())
    }
    async fn join_channel(&self, user_id: i64, channel_id: i64) -> Result<(), DatabaseError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query!("INSERT INTO user_channels VALUES (?, ?);", user_id, channel_id).execute(&mut conn).await?;
        Ok(())
    }
    async fn add_message(&self, user_id: i64, channel_id: i64, time: i64, nickname: &str, message: &str) -> Result<i64, DatabaseError> {
        let mut conn = self.pool.acquire().await?;
        let res = sqlx::query!("INSERT INTO messages VALUES (null, ?, ?, ?, ?, ?);", user_id, channel_id, time, nickname, message)
            .execute(&mut conn).await?;
        Ok(res.last_insert_rowid())
    }
    async fn get_channel_history(&self, channel_id: i64, limit: i64, users: &MultiMap<String, i64, User>) -> Result<Vec<Message>, DatabaseError> {
        let mut conn = self.pool.acquire().await?;
        // This should be expressible using query! but I kept getting compile errors.
        let mut res = sqlx::query("SELECT messages.id,messages.user_id,messages.time,messages.nickname,messages.content,message_attachments.url,message_attachments.mime
            FROM messages LEFT JOIN message_attachments ON messages.id = message_attachments.message_id WHERE channel_id = ? ORDER BY time DESC LIMIT ?;")
            .bind(channel_id)
            .bind(limit)
            .fetch(&mut conn);

        let mut messages: HashMap<i64, Message> = HashMap::new();
        while let Some(row) = res.try_next().await? {
            let user_id: i64 = row.try_get("user_id")?;
            let message_id: i64 = row.try_get("id")?;
            let time: i64 = row.try_get("time")?;
            let nickname: &str = row.try_get("nickname")?;
            let content: &str = row.try_get("content")?;
            let url: Option<&str> = row.try_get("url")?;
            let mime: Option<&str> = row.try_get("mime")?;

            if let Some(user) = users.get_alt(&user_id) {
                let message = messages.entry(message_id).or_insert_with(|| {
                    Message::new(message_id, user.clone(), time, &nickname, &content)
                });

                if url.is_some() && mime.is_some() {
                    message.attachments.push(MessageAttachment::new(&url.unwrap(), &mime.unwrap()));
                }
            }
        }

        // TODO: use into_values() once stable
        Ok(messages.values().map(|m| m.clone()).collect())
    }
    async fn get_last_message_read(&self, user_id: i64, channel_id: i64) -> Result<Option<i64>, DatabaseError> {
        let mut conn = self.pool.acquire().await?;
        let res = sqlx::query!("SELECT message_id FROM user_last_read_messages WHERE user_id = ? AND channel_id = ?;",
            user_id, channel_id).fetch_optional(&mut conn).await?;

        if let Some(record) = res {
            Ok(Some(record.message_id))
        }
        else {
            Ok(None)
        }
    }
    async fn set_last_message_read(&self, user_id: i64, channel_id: i64) -> Result<Option<i64>, DatabaseError> {
        let mut conn = self.pool.acquire().await?;
        // I'm going to change this soon so we accept a message_id rather than just getting the last id.
        // We'll need that for handling infinite scrolling properly.

        let message_id = {
            let res = sqlx::query!("SELECT MAX(id) as message_id FROM messages WHERE channel_id = ?;", channel_id)
                .fetch_optional(&mut conn).await?;
            if let Some(record) = res {
                Some(record.message_id)
            }
            else {
                None
            }
        };

        sqlx::query!("REPLACE INTO user_last_read_messages VALUES (?, ?, ?);", user_id, channel_id, message_id)
            .execute(&mut conn).await?;
        Ok(message_id)
    }
    async fn clear_last_message_read(&self, user_id: i64, channel_id: i64) -> Result<(), DatabaseError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query!("DELETE FROM user_last_read_messages WHERE user_id = ? AND channel_id = ?;", user_id, channel_id)
            .execute(&mut conn).await?;
        Ok(())
    }
    async fn add_message_attachment(&self, message_id: i64, url: &str, mime: &str) -> Result<(), DatabaseError> {
        let mut conn = self.pool.acquire().await?;
        sqlx::query!("INSERT INTO message_attachments VALUES (null, ?, ?, ?);", message_id, url, mime)
            .execute(&mut conn).await?;
        Ok(())
    }
}


/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use super::models::{Channel, Message, User};
use async_trait::async_trait;
use multi_map::MultiMap;
use sodiumoxide::crypto::pwhash::argon2i13::{
    pwhash, HashedPassword, MEMLIMIT_INTERACTIVE, OPSLIMIT_INTERACTIVE,
};
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("DBMS Error")]
    DBMSError(#[from] sqlx::Error),
    #[error("Password hashing error")]
    PasswordHashError,
}

#[async_trait]
pub trait ChatDatabase: Send + Sync {
    async fn setup_database(&self) -> Result<(), DatabaseError>;
    async fn register_user(&self, username: &str, password: &str) -> Result<i64, DatabaseError>;
    async fn verify_login(&self, username: &str, password: &str) -> Result<bool, DatabaseError>;
    async fn get_users(&self) -> Result<MultiMap<String, i64, User>, DatabaseError>;
    async fn get_channels(
        &self,
        users: &MultiMap<String, i64, User>,
    ) -> Result<HashMap<String, Channel>, DatabaseError>;
    async fn make_channel(&self, name: &str) -> Result<i64, DatabaseError>;
    async fn join_channel(&self, user_id: i64, channel_id: i64) -> Result<(), DatabaseError>;
    async fn add_message(
        &self,
        user_id: i64,
        channel_id: i64,
        time: i64,
        nickname: &str,
        message: &str,
    ) -> Result<i64, DatabaseError>;
    async fn get_channel_history(
        &self,
        channel_id: i64,
        limit: i64,
        users: &MultiMap<String, i64, User>,
    ) -> Result<Vec<Message>, DatabaseError>;
    async fn get_last_message_read(
        &self,
        user_id: i64,
        channel_id: i64,
    ) -> Result<Option<i64>, DatabaseError>;
    async fn set_last_message_read(
        &self,
        user_id: i64,
        channel_id: i64,
    ) -> Result<Option<i64>, DatabaseError>;
    async fn clear_last_message_read(
        &self,
        user_id: i64,
        channel_id: i64,
    ) -> Result<(), DatabaseError>;
    async fn add_message_attachment(
        &self,
        message_id: i64,
        url: &str,
        mime: &str,
    ) -> Result<(), DatabaseError>;
}

pub fn pwhash_interactive(bytes: &[u8]) -> Result<HashedPassword, ()> {
    pwhash(bytes, OPSLIMIT_INTERACTIVE, MEMLIMIT_INTERACTIVE)
}

pub mod sqlite;

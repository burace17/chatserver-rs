/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use multi_map::MultiMap;
use rusqlite::{Connection, Result, NO_PARAMS, params};
use sodiumoxide::crypto::pwhash::argon2i13::{pwhash, pwhash_verify, HashedPassword, OPSLIMIT_INTERACTIVE, MEMLIMIT_INTERACTIVE};
use std::collections::HashMap;
use super::user::User;
use super::channel::Channel;
use thiserror::Error;
use rusqlite::config::DbConfig;

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("DBMS Error")]
    DBMSError(#[from] rusqlite::Error),
    #[error("Password hashing error")]
    PasswordHashError,
    #[error("The data model is no longer valid")]
    BadDataModel,
}

fn pwhash_interactive(bytes: &[u8]) -> Result<HashedPassword, ()> {
    pwhash(bytes, OPSLIMIT_INTERACTIVE, MEMLIMIT_INTERACTIVE)
}

fn open(db_path: &str) -> Result<Connection> {
    let conn = Connection::open(db_path)?;
    conn.set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, true)?;
    Ok(conn)
}

pub fn setup_database(db_path: &str) -> Result<()> {
    let conn = open(db_path)?;
    conn.execute("CREATE TABLE IF NOT EXISTS users(id INTEGER PRIMARY KEY AUTOINCREMENT, username TEXT UNIQUE NOT NULL,
                                                   nickname TEXT UNIQUE NOT NULL, password TEXT NOT NULL);", NO_PARAMS)?;
    conn.execute("CREATE TABLE IF NOT EXISTS channels(id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT UNIQUE NOT NULL);", NO_PARAMS)?;
    conn.execute("CREATE TABLE IF NOT EXISTS user_channels(user_id INTEGER NOT NULL,
                                             channel_id INTEGER NOT NULL,
                                             UNIQUE(user_id, channel_id),
                                             FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
                                             FOREIGN KEY(channel_id) REFERENCES channels(id) ON DELETE CASCADE);", NO_PARAMS)?;
    conn.execute("CREATE TABLE IF NOT EXISTS messages(id INTEGER PRIMARY KEY AUTOINCREMENT,
                                             user_id INTEGER NOT NULL,
                                             channel_id INTEGER NOT NULL,
                                             time INTEGER NOT NULL,
                                             nickname TEXT NOT NULL,
                                             content TEXT NOT NULL,
                                             FOREIGN KEY(user_id) REFERENCES users(id) ON DELETE CASCADE,
                                             FOREIGN KEY(channel_id) REFERENCES channels(id) ON DELETE CASCADE);", NO_PARAMS)?;
    conn.execute("CREATE TABLE IF NOT EXISTS user_last_read_messages(user_id INTEGER NOT NULL,
                                             channel_id INTEGER NOT NULL,
                                             message_id INTEGER NOT NULL,
                                             UNIQUE(user_id, channel_id, message_id),
                                             FOREIGN KEY(message_id) REFERENCES messages(id) ON DELETE CASCADE,
                                             FOREIGN KEY(user_id, channel_id) REFERENCES user_channels(user_id, channel_id) ON DELETE CASCADE);", NO_PARAMS)?;
    Ok(())
}

pub fn register_user(db_path: &str, username: &str, password: &str) -> Result<i64, DatabaseError> {
    sodiumoxide::init().ok().ok_or(DatabaseError::PasswordHashError)?;
    let conn = open(db_path)?;
    let pwh = pwhash_interactive(password.as_bytes()).ok().ok_or(DatabaseError::PasswordHashError)?;
    conn.execute("INSERT INTO users VALUES (null, ?, ?, ?);", params![username, username, pwh.as_ref()])?;
    Ok(conn.last_insert_rowid())
}

pub fn verify_login(db_path: &str, username: &str, password: &str) -> Result<bool, DatabaseError> {
    sodiumoxide::init().ok().ok_or(DatabaseError::PasswordHashError)?;
    let conn = open(db_path)?;
    match conn.query_row::<Vec<u8>, _, _>("SELECT password FROM users WHERE username = ?;", params![username], |row| row.get(0)) {
        Ok(stored_password) => {
            let hashed_password = HashedPassword::from_slice(stored_password.as_ref()).ok_or(DatabaseError::PasswordHashError)?;
            Ok(pwhash_verify(&hashed_password, password.as_bytes()))
        },
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(e) => Err(DatabaseError::DBMSError(e))
    }
}

pub fn get_users(db_path: &str) -> Result<MultiMap<String, i64, User>, DatabaseError> {
    let mut users = MultiMap::new();
    let conn = open(db_path)?;
    let mut stmt = conn.prepare("SELECT id, username, nickname FROM users;")?;
    let user_iter = stmt.query_map(NO_PARAMS, |row| {
        let id = row.get(0)?;
        let username = &row.get::<_, String>(1)?;
        let nickname = &row.get::<_, String>(2)?;
        Ok(User::new(id, username, nickname))
    })?;

    for user in user_iter.filter_map(|u| u.ok()) {
        users.insert(user.username.to_string(), user.id, user);
    }
    Ok(users)
}

pub fn get_channels(db_path: &str, users: &MultiMap<String, i64, User>) -> Result<HashMap<String, Channel>, DatabaseError> {
    let mut channels: HashMap<String, Channel> = HashMap::new();
    let conn = open(db_path)?;
    let mut stmt = conn.prepare("SELECT channels.id,channels.name,users.username FROM channels
                                    LEFT JOIN user_channels ON channels.id = user_channels.channel_id
                                    LEFT JOIN users ON user_channels.user_id = users.id;")?;
    let channel_iter = stmt.query_map(NO_PARAMS, |row| {
        let channel_id = row.get(0)?;
        let channel_name = row.get::<_, String>(1)?;
        let username = row.get::<_, Option<String>>(2)?;
        Ok((channel_id, channel_name, username))
    })?;

    for (channel_id, channel_name, username_opt) in channel_iter.filter_map(|u| u.ok()) {
        let chan = channels.entry(channel_name.to_string()).or_insert_with(|| Channel::new(channel_id, &channel_name));
        if let Some(username) = username_opt {
            let user = users.get(&username).ok_or(DatabaseError::BadDataModel)?;
            chan.add_user(user);
        }
    }

    Ok(channels)
}

pub fn make_channel(db_path: &str, name: &str) -> Result<i64, DatabaseError> {
    let conn = open(db_path)?;
    conn.execute("INSERT INTO channels VALUES (null, ?);", params![name])?;
    Ok(conn.last_insert_rowid())
}

pub fn join_channel(db_path: &str, user_id: i64, channel_id: i64) -> Result<(), DatabaseError> {
    let conn = open(db_path)?;
    conn.execute("INSERT INTO user_channels VALUES (?, ?);", params![user_id, channel_id])?;
    Ok(())
}

pub fn add_message(db_path: &str, user_id: i64, channel_id: i64, time: i64, nickname: &str, message: &str) -> Result<i64, DatabaseError> {
    let conn = open(db_path)?;
    conn.execute("INSERT INTO messages VALUES (null, ?, ?, ?, ?, ?);", params![user_id, channel_id, time, nickname, message])?;
    Ok(conn.last_insert_rowid())
}

pub type HistoryResult = (i64, i64, i64, String, String);
pub fn get_channel_history(db_path: &str, channel_id: i64, limit: i64) -> Result<Vec<HistoryResult>, DatabaseError> {
    let conn = open(db_path)?;
    let mut stmt = conn.prepare("SELECT id,user_id,time,nickname,content FROM messages WHERE channel_id = ? ORDER BY time DESC LIMIT ?;")?;
    let message_iter = stmt.query_map(params![channel_id, limit], |row| {
        let message_id = row.get(0)?;
        let user_id = row.get(1)?;
        let time = row.get(2)?;
        let nickname = row.get::<_, String>(3)?;
        let content = row.get::<_, String>(4)?;
        Ok((message_id, user_id, time, nickname, content))
    })?;

    Ok(message_iter.filter_map(|m| m.ok()).collect())
}

pub fn get_last_message_read(db_path: &str, user_id: i64, channel_id: i64) -> Result<Option<i64>, DatabaseError> {
    let conn = open(db_path)?;
    match conn.query_row::<Option<i64>, _, _>("SELECT message_id FROM user_last_read_messages WHERE user_id = ? AND channel_id = ?;",
                                              params![user_id, channel_id], |row| row.get(0)) {
        Ok(message_id) => Ok(message_id),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(DatabaseError::DBMSError(e))
    }
}

pub fn set_last_message_read(db_path: &str, user_id: i64, channel_id: i64) -> Result<Option<i64>, DatabaseError> {
    let conn = open(db_path)?;
    let message_id = match conn.query_row::<Option<i64>, _, _>("SELECT MAX(id) from messages WHERE channel_id = ?;",
                                                               params![channel_id], |row| row.get(0)) {
        Ok(message_id) => Ok(message_id),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(DatabaseError::DBMSError(e))
    }?;

    if let Some(id) = message_id {
        println!("set_last_message_read: {}", id);
    }
    else {
        println!("set_last_message_read: None");
    }

    conn.execute("REPLACE INTO user_last_read_messages VALUES (?, ?, ?);", params![user_id, channel_id, message_id])?;
    Ok(message_id)
}

pub fn clear_last_message_read(db_path: &str, user_id: i64, channel_id: i64) -> Result<(), DatabaseError> {
    let conn = open(db_path)?;
    conn.execute("DELETE FROM user_last_read_messages WHERE user_id = ? AND channel_id = ?;", params![user_id, channel_id])?;
    Ok(())
}
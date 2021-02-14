/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */
 
use rusqlite::{Connection, Result, NO_PARAMS, params};
use sodiumoxide::crypto::pwhash::argon2i13::{pwhash, pwhash_verify, HashedPassword, OPSLIMIT_INTERACTIVE, MEMLIMIT_INTERACTIVE};
use std::collections::HashMap;
use super::user::User;
use super::channel::Channel;
use thiserror::Error;

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

pub fn setup_database(db_path: &str) -> Result<()> {
    let conn = Connection::open(db_path)?;
    conn.execute("CREATE TABLE IF NOT EXISTS users(id INTEGER PRIMARY KEY AUTOINCREMENT, username TEXT UNIQUE NOT NULL, 
                                                   nickname TEXT UNIQUE NOT NULL, password TEXT NOT NULL);", NO_PARAMS)?;
    conn.execute("CREATE TABLE IF NOT EXISTS channels(id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT UNIQUE NOT NULL);", NO_PARAMS)?;
    conn.execute("CREATE TABLE IF NOT EXISTS user_channels(user_id INTEGER NOT NULL, 
                                             channel_id INTEGER NOT NULL,
                                             UNIQUE(user_id, channel_id),
                                             FOREIGN KEY(user_id) REFERENCES users(id), 
                                             FOREIGN KEY(channel_id) REFERENCES channels(id));", NO_PARAMS)?;
    conn.execute("CREATE TABLE IF NOT EXISTS messages(id INTEGER PRIMARY KEY AUTOINCREMENT, 
                                             user_id INTEGER NOT NULL, 
                                             channel_id INTEGER NOT NULL,
                                             time INTEGER NOT NULL,
                                             nickname TEXT NOT NULL,
                                             content TEXT NOT NULL,
                                             FOREIGN KEY(user_id) REFERENCES users(id),
                                             FOREIGN KEY(channel_id) REFERENCES channels(id));", NO_PARAMS)?;
    
    Ok(())
}

pub fn register_user(db_path: &str, username: &str, password: &str) -> Result<i64, DatabaseError> {
    sodiumoxide::init().ok().ok_or(DatabaseError::PasswordHashError)?;
    let conn = Connection::open(db_path)?;
    let pwh = pwhash_interactive(password.as_bytes()).ok().ok_or(DatabaseError::PasswordHashError)?;
    conn.execute("INSERT INTO users VALUES (null, ?, ?, ?);", params![username, username, pwh.as_ref()])?;
    Ok(conn.last_insert_rowid())
}

pub fn verify_login(db_path: &str, username: &str, password: &str) -> Result<bool, DatabaseError> {
    sodiumoxide::init().ok().ok_or(DatabaseError::PasswordHashError)?;
    let conn = Connection::open(db_path)?;
    match conn.query_row::<Vec<u8>, _, _>("SELECT password FROM users WHERE username = ?;", params![username], |row| row.get(0)) {
        Ok(stored_password) => {
            let hashed_password = HashedPassword::from_slice(stored_password.as_ref()).ok_or(DatabaseError::PasswordHashError)?;
            Ok(pwhash_verify(&hashed_password, password.as_bytes()))
        },
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
        Err(e) => Err(DatabaseError::DBMSError(e))
    }
}

pub fn get_users(db_path: &str) -> Result<HashMap<String, User>, DatabaseError> {
    let mut users: HashMap<String, User> = HashMap::new();
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare("SELECT id, username, nickname FROM users;")?;
    let user_iter = stmt.query_map(NO_PARAMS, |row| {
        let id = row.get(0)?;
        let username = &row.get::<_, String>(1)?;
        let password = &row.get::<_, String>(2)?;
        Ok(User::new(id, username, password))
    })?;

    for user in user_iter.filter_map(|u| u.ok()) {
        users.insert(user.username.to_string(), user);
    }
    Ok(users)
}

pub fn get_channels(db_path: &str, users: &HashMap<String, User>) -> Result<HashMap<String, Channel>, DatabaseError> {
    let mut channels: HashMap<String, Channel> = HashMap::new();
    let conn = Connection::open(db_path)?;
    let mut stmt = conn.prepare("SELECT channels.id,channels.name,users.username FROM channels
                                    LEFT JOIN user_channels ON channels.id = user_channels.channel_id
                                    LEFT JOIN users ON user_channels.user_id = users.id;")?;
    let channel_iter = stmt.query_map(NO_PARAMS, |row| {
        let channel_id = row.get(0)?;
        let channel_name = row.get::<_, String>(1)?;
        let username = row.get::<_, String>(2)?;
        Ok((channel_id, channel_name, username))
    })?;

    for (channel_id, channel_name, username) in channel_iter.filter_map(|u| u.ok()) {
        let chan = channels.entry(channel_name.to_string()).or_insert_with(|| Channel::new(channel_id, &channel_name));
        let user = users.get(&username).ok_or(DatabaseError::BadDataModel)?;
        chan.add_user(user);
    }

    Ok(channels)
}

pub fn make_channel(db_path: &str, name: &str) -> Result<i64, DatabaseError> {
    let conn = Connection::open(db_path)?;
    conn.execute("INSERT INTO channels VALUES (null, ?);", params![name])?;
    Ok(conn.last_insert_rowid())
}

pub fn join_channel(db_path: &str, user_id: i64, channel_id: i64) -> Result<(), DatabaseError> {
    let conn = Connection::open(db_path)?;
    conn.execute("INSERT INTO user_channels VALUES (?, ?);", params![user_id, channel_id])?;
    Ok(())
}

pub fn add_message(db_path: &str, user_id: i64, channel_id: i64, time: i64, nickname: &str, message: &str) -> Result<i64, DatabaseError> {
    let conn = Connection::open(db_path)?;
    conn.execute("INSERT INTO messages VALUES (null, ?, ?, ?, ?, ?);", params![user_id, channel_id, time, nickname, message])?;
    Ok(conn.last_insert_rowid())
}
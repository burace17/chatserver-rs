use rusqlite::{Connection, Result, NO_PARAMS, params};
use sodiumoxide::crypto::pwhash::argon2i13::{pwhash, pwhash_verify, HashedPassword, OPSLIMIT_INTERACTIVE, MEMLIMIT_INTERACTIVE};
use std::collections::HashMap;
use super::server::User;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("DBMS Error")]
    DBMSError(#[from] rusqlite::Error),
    #[error("Password hashing error")]
    PasswordHashError
}

fn pwhash_interactive(bytes: &[u8]) -> Result<HashedPassword, ()> {
    pwhash(bytes, OPSLIMIT_INTERACTIVE, MEMLIMIT_INTERACTIVE)
}

pub fn setup_database(db_path: &str) -> Result<()> {
    let conn = Connection::open(db_path)?;
    conn.execute("
        CREATE TABLE IF NOT EXISTS users(id INTEGER PRIMARY KEY AUTOINCREMENT, username TEXT UNIQUE, nickname TEXT UNIQUE, password TEXT);
        CREATE TABLE IF NOT EXISTS channels(id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT UNIQUE);
        CREATE TABLE IF NOT EXISTS user_channels(user_id INTEGER, 
                                                 channel_id INTEGER,
                                                 FOREIGN KEY(user_id) REFERENCES users(id), 
                                                 FOREIGN KEY(channel_id) REFERENCES channels(id));
        CREATE TABLE IF NOT EXISTS messages(id INTEGER PRIMARY KEY AUTOINCREMENT, 
                                            user_id INTEGER, 
                                            channel_id INTEGER,
                                            time INTEGER,
                                            nickname TEXT,
                                            content TEXT,
                                            FOREIGN KEY(user_id) REFERENCES users(id),
                                            FOREIGN KEY(channel_id) REFERENCES channels(id));)", NO_PARAMS)?;
    
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
        Ok(User::new(row.get(0)?, &row.get::<_, String>(1)?, &row.get::<_, String>(2)?))
    })?;

    for user in user_iter.filter(|u| u.is_ok()).map(|u| u.unwrap()) {
        users.insert(user.username.to_string(), user);
    }
    Ok(users)
}
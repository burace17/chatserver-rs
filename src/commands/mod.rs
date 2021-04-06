/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use lazy_static::lazy_static;
use regex::Regex;
use std::net::SocketAddr;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;
use super::server::{ChatServer, ServerCommandResponse};
use super::db::DatabaseError;

mod history;
mod ident;
mod join;
mod msg;
mod not_viewing;
mod register;
mod viewing;

#[derive(Error, Debug)]
pub enum CommandError {
    #[error("Missing command")]
    MissingCommand,
    #[error("Invalid JSON")]
    InvalidJSON(#[from] serde_json::Error),
    #[error("Invalid command arguments")]
    InvalidArguments,
    #[error("Invalid username or password")]
    InvalidUsername,
    #[error("Internal server error")]
    SendFailed(Box<SendError<ServerCommandResponse>>),
    #[error("Internal server error")]
    LoginDBMSError(#[from] DatabaseError),
    #[error("Invalid username or password")]
    LoginFailed,
    #[error("Need to login to use this command")]
    NeedAuth,
    #[error("Need to be in a channel to use this command")]
    NotInChannel,
    #[error("Internal server error")]
    TimeError(#[from] std::time::SystemTimeError),
    #[error("Did not authenticate in time")]
    DidNotAuth,
    #[error("User is already registered")]
    AlreadyRegistered
}

impl std::convert::From<SendError<ServerCommandResponse>> for CommandError {
    fn from(error: SendError<ServerCommandResponse>) -> Self {
        CommandError::SendFailed(Box::new(error))
    }
}

lazy_static! {
    pub static ref USERNAME_PATTERN: Regex = Regex::new("^[A-Za-z0-9_-]*$").unwrap();
}

pub async fn process_command(server: &mut ChatServer, client: SocketAddr, data: &str) -> Result<(), CommandError> {
    // Parses the JSON and extracts the incoming command name.
    fn parse_incoming_json(data: &str) -> Result<(Value, String), CommandError> {
        let json = serde_json::from_str::<Value>(data)?;
        let cmd = json["cmd"].as_str().ok_or(CommandError::MissingCommand)?;
        Ok((json.clone(), cmd.to_string()))
    }

    let (json, cmd) = parse_incoming_json(data)?;
    match cmd.as_str() {
        "MSG" => msg::handle(server, client, &json).await,
        "IDENT" => ident::handle(server, client, &json).await,
        "REGISTER" => register::handle(server, client, &json).await,
        "JOIN" => join::handle(server, client, &json).await,
        "HISTORY" => history::handle(server, client, &json).await,
        "VIEWING" => viewing::handle(server, client, &json).await,
        "NOTVIEWING" => not_viewing::handle(server, client).await,
        _ => Err(CommandError::MissingCommand)
    }
}

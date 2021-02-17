/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use boolinator::Boolinator;
use lazy_static::lazy_static;
use regex::Regex;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::net::SocketAddr;
use std::time::SystemTime;
use serde_json::Value;
use serde_json::json;
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;
use super::channel::Channel;
use super::message::Message;
use super::server::ChatServer;
use super::server::ServerCommandResponse;
use super::user::User;
use super::db_interaction;

#[derive(Error, Debug)]
pub enum CommandError {
    #[error("Missing command")]
    MissingCommand,
    #[error("Invalid JSON")]
    InvalidJSON(#[from] serde_json::Error),
    #[error("Invalid arguments")]
    InvalidArguments,
    #[error("Invalid username")]
    InvalidUsername,
    #[error("Could not send command response")]
    SendFailed(Box<SendError<ServerCommandResponse>>),
    #[error("Could not login (dbms error)")]
    LoginDBMSError(#[from] db_interaction::DatabaseError),
    #[error("Could not login")]
    LoginFailed,
    #[error("Need to authenticate first")]
    NeedAuth,
    #[error("Not in specified channel")]
    NotInChannel,
    #[error("Bad time")]
    TimeError(#[from] std::time::SystemTimeError),
    #[error("Didn't auth in time")]
    DidNotAuth
}

impl std::convert::From<SendError<ServerCommandResponse>> for CommandError {
    fn from(error: SendError<ServerCommandResponse>) -> Self {
        CommandError::SendFailed(Box::new(error))
    }
}

lazy_static! {
    static ref USERNAME_PATTERN: Regex = Regex::new("^[A-Za-z0-9_-]*$").unwrap();
}

async fn handle_ident(state: &mut ChatServer, client: SocketAddr, json: &Value) -> Result<(), CommandError> {
    let username = json["username"].as_str().ok_or(CommandError::InvalidArguments)?.to_lowercase();
    let password = json["password"].as_str().ok_or(CommandError::InvalidArguments)?.to_lowercase();

    // check username for invalid characters
    USERNAME_PATTERN.is_match(&username).ok_or(CommandError::InvalidUsername)?;

    // check password
    let login_succeeded = db_interaction::verify_login(&state.db_path, &username, &password)?;
    login_succeeded.ok_or(CommandError::LoginFailed)?;

    let nickname = state.users.get(&username)
                              .ok_or(CommandError::InvalidUsername)? // wouldn't expect this to fail.
                              .nickname
                              .to_string();

    let info = state.get_unauthed_connection(client).ok_or(CommandError::InvalidArguments)?;

    state.remove_unauth_connection(client);
    state.add_connection(client, username.to_string(), info.tx.clone()).await;

    let channels: Vec<serde_json::Value> = state.channels.values()
                                              .filter(|chan| chan.users.contains(&username))
                                              .map(|chan| chan.serialize(|u| state.users.get(&u.to_owned()).cloned()))
                                              .collect();

    let response = json!({
        "cmd" : "WELCOME",
        "name" : "test",
        "channels" : channels,
        "nickname" : nickname,
    });

    info.tx.send(ServerCommandResponse::Text(response.to_string())).await?;
    Ok(())
}

async fn handle_register(state: &mut ChatServer, client: SocketAddr, json: &Value) -> Result<(), CommandError> {
    let username = json["username"].as_str().ok_or(CommandError::InvalidArguments)?.to_lowercase();
    let password = json["password"].as_str().ok_or(CommandError::InvalidArguments)?.to_lowercase();

    // check username for invalid characters
    USERNAME_PATTERN.is_match(&username).ok_or(CommandError::InvalidUsername)?;

    let user_id = db_interaction::register_user(&state.db_path, &username, &password)?;
    let info = state.get_unauthed_connection(client).ok_or(CommandError::InvalidArguments)?;

    state.remove_unauth_connection(client);
    state.add_connection(client, username.to_string(), info.tx.clone()).await;

    let mut user = User::new(user_id, &username, &username);
    user.add_connection(client, info.tx.clone());
    state.add_user(user);

    let response = json!({
        "cmd" : "WELCOME",
        "name" : "test",
        "channels" : [],
        "nickname" : username,
    });

    info.tx.send(ServerCommandResponse::Text(response.to_string())).await?;
    Ok(())
}

async fn handle_join(state: &mut ChatServer, client: SocketAddr, json: &Value) -> Result<(), CommandError> {
    // User must be authenticated for this command to work
    let user = {
        let u = state.get_user(&client).ok_or(CommandError::NeedAuth)?;
        u.clone()
    };

    let channel_name = json["name"].as_str().ok_or(CommandError::InvalidArguments)?.to_lowercase();
    channel_name.starts_with("#").ok_or(CommandError::InvalidArguments)?;

    {
        // Mutate the channel
        let channel = match state.channels.entry(channel_name.to_string()) {
            Entry::Occupied(o) => o.into_mut(),
            Entry::Vacant(v) => {
                let channel_id = db_interaction::make_channel(&state.db_path, &channel_name)?;
                v.insert(Channel::new(channel_id, &channel_name))
            }
        };

        db_interaction::join_channel(&state.db_path, user.id, channel.id)?;
        channel.add_user(&user);
    }

    // Getting the channel again but as an immutable reference..
    let channel = state.channels.get(&channel_name).unwrap();

    // Broadcast that a new user has joined to everyone in this channel.
    let json = json!({
        "cmd" : "JOIN",
        "channel" : channel.name,
        "user" : user
    });

    // The lookup_user closure is the same for both of these calls, but it doesn't compile if I store it in a local.
    channel.broadcast(|username| state.users.get(&username.to_owned()).cloned(), &json.to_string()).await;

    // Let this user know some information about the channel they joined.
    let json = json!({
        "cmd" : "CHANNELINFO",
        "channel" : channel.serialize(|username| state.users.get(&username.to_owned()).cloned())
    });

    user.send_to(&client, &json.to_string()).await;
    Ok(())
}

async fn handle_msg(state: &ChatServer, client: SocketAddr, json: &Value) -> Result<(), CommandError> {
    let user = state.get_user(&client).ok_or(CommandError::NeedAuth)?;
    let channel_name = json["channel"].as_str().ok_or(CommandError::InvalidArguments)?.to_lowercase();
    let channel = state.channels.get(&channel_name).ok_or(CommandError::InvalidArguments)?;
    channel.users.contains(&user.username).ok_or(CommandError::NotInChannel)?;

    let msg_text = json["content"].as_str().ok_or(CommandError::InvalidArguments)?;
    (msg_text.len() > 0).ok_or(CommandError::InvalidArguments)?;

    let time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs() as i64;
    let msg_id = db_interaction::add_message(&state.db_path, user.id, channel.id, time, &user.nickname, &msg_text)?;

    let json = json!({
        "cmd" : "MSG",
        "user" : user,
        "channel" : channel.name,
        "content" : msg_text,
        "message_id" : msg_id,
        "time" : time
    });

    channel.broadcast(|username| state.users.get(&username.to_owned()).cloned(), &json.to_string()).await;
    Ok(())
}

async fn handle_history(state: &ChatServer, client: SocketAddr, json: &Value) -> Result<(), CommandError> {
    let user = state.get_user(&client).ok_or(CommandError::NeedAuth)?;
    let mut all_messages = HashMap::new();

    // here we aren't disconnecting the user for invalid arguments... but maybe we should?
    let channels = json["channels"].as_array()
        .ok_or(CommandError::InvalidArguments)?.iter()
        .filter_map(|value| value.as_str())
        .filter_map(|str_value| state.channels.get(&str_value.to_lowercase()))
        .filter(|channel| channel.users.contains(&user.username));

    for channel in channels {
        let tmp = db_interaction::get_channel_history(&state.db_path, channel.id, 50)?;
        let messages: Vec<Message> = tmp.iter().filter_map(|(message_id, user_id, time, nickname, content)| {
            if let Some(user) = state.users.get_alt(user_id) {
                Some(Message::new(*message_id, user.clone(), *time, nickname, content))
            }
            else {
                None
            }
        }).collect();

        all_messages.insert(&channel.name, messages);
    }

    if !all_messages.is_empty() {
        let json = json!({
            "cmd" : "HISTORY",
            "messages" : all_messages
        });

        user.send_to(&client, &json.to_string()).await;
    }
    Ok(())
}

pub async fn process_command(state: &mut ChatServer, client: SocketAddr, data: &str) -> Result<(), CommandError> {
    // Parses the JSON and extracts the incoming command name.
    fn parse_incoming_json(data: &str) -> Result<(Value, String), CommandError> {
        let json = serde_json::from_str::<Value>(data)?;
        let cmd = json["cmd"].as_str().ok_or(CommandError::MissingCommand)?;
        Ok((json.clone(), cmd.to_string()))
    }

    let (json, cmd) = parse_incoming_json(data)?;
    match cmd.as_str() {
        "MSG" => handle_msg(state, client, &json).await,
        "IDENT" => handle_ident(state, client, &json).await,
        "REGISTER" => handle_register(state, client, &json).await,
        "JOIN" => handle_join(state, client, &json).await,
        "HISTORY" => handle_history(state, client, &json).await,
        _ => Err(CommandError::MissingCommand)
    }
}
/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use boolinator::Boolinator;
use lazy_static::lazy_static;
use linkify::LinkFinder;
use regex::Regex;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::net::SocketAddr;
use std::time::SystemTime;
use serde_json::{Value, json};
use serde::Serialize;
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;
use super::channel::Channel;
use super::message::Message;
use super::server::ChatServer;
use super::server::ServerCommandResponse;
use super::user::User;
use super::db;

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
    LoginDBMSError(#[from] db::DatabaseError),
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
    static ref USERNAME_PATTERN: Regex = Regex::new("^[A-Za-z0-9_-]*$").unwrap();
}

async fn handle_ident(state: &mut ChatServer, client: SocketAddr, json: &Value) -> Result<(), CommandError> {
    let username = json["username"].as_str().ok_or(CommandError::InvalidArguments)?.to_lowercase();
    let password = json["password"].as_str().ok_or(CommandError::InvalidArguments)?.to_lowercase();

    // check username for invalid characters
    USERNAME_PATTERN.is_match(&username).ok_or(CommandError::InvalidUsername)?;

    // check password
    let login_succeeded = state.db.verify_login(&username, &password).await?;
    login_succeeded.ok_or(CommandError::LoginFailed)?;

    let (nickname, channels_being_viewed) = {
        let user = state.users.get(&username).ok_or(CommandError::InvalidUsername)?;
        (user.nickname.to_string(), user.get_viewed_channels())
    };

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
        "viewing" : channels_being_viewed
    });

    info.tx.send(ServerCommandResponse::Text(response.to_string())).await?;
    Ok(())
}

async fn handle_register(state: &mut ChatServer, client: SocketAddr, json: &Value) -> Result<(), CommandError> {
    let username = json["username"].as_str().ok_or(CommandError::InvalidArguments)?.to_lowercase();
    let password = json["password"].as_str().ok_or(CommandError::InvalidArguments)?.to_lowercase();

    // check username for invalid characters
    USERNAME_PATTERN.is_match(&username).ok_or(CommandError::InvalidUsername)?;

    // check if already registered
    state.users.get(&username).is_none().ok_or(CommandError::AlreadyRegistered)?;

    let user_id = state.db.register_user(&username, &password).await?;
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
        "viewing" : []
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
                let channel_id = state.db.make_channel(&channel_name).await?;
                v.insert(Channel::new(channel_id, &channel_name))
            }
        };

        state.db.join_channel(user.id, channel.id).await?;
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
    let msg_id = state.db.add_message(user.id, channel.id, time, &user.nickname, &msg_text).await?;

    let json = json!({
        "cmd" : "MSG",
        "user" : user,
        "channel" : channel.name,
        "content" : msg_text,
        "message_id" : msg_id,
        "time" : time
    });

    channel.broadcast(|username| state.users.get(&username.to_owned()).cloned(), &json.to_string()).await;

    let links: Vec<String> = {
        let mut link_finder = LinkFinder::new();
        let links = link_finder.kinds(&[linkify::LinkKind::Url]).links(msg_text);
        links.map(|link| link.as_str().to_string()).collect()
    };

    for link in links {
        if let Err(e) = state.query_for_attachments(channel.id, msg_id, &link).await {
            println!("Could not send link to attachment manager task: {}", e);
        }
    }

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

    #[derive(Serialize)]
    struct HistoryValue {
        messages: Vec<Message>,
        last_read_message: Option<i64>
    }

    for channel in channels {
        let messages = state.db.get_channel_history(channel.id, 50, &state.users).await?;
        let value = HistoryValue {
            messages,
            last_read_message: state.db.get_last_message_read(user.id, channel.id).await?
        };
        all_messages.insert(&channel.name, value);
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

async fn handle_viewing(state: &mut ChatServer, client: SocketAddr, json: &Value) -> Result<(), CommandError> {
    let new_channel_name = json["channel"].as_str().ok_or(CommandError::InvalidArguments)?.to_lowercase();
    let new_channel_id = {
        let user = state.get_user(&client).ok_or(CommandError::NeedAuth)?;
        let new_channel = state.channels.get(&new_channel_name).ok_or(CommandError::InvalidArguments)?;
        new_channel.users.contains(&user.username).ok_or(CommandError::NotInChannel)?;
        new_channel.id
    };

    let (user_id, no_viewers) = {
        let user = state.get_user_mut(&client).ok_or(CommandError::NeedAuth)?;
        user.set_viewing(&new_channel_name, &client, true);
        let no_viewers = user.get_and_clear_no_viewers();
        (user.id, no_viewers)
    };

    state.db.clear_last_message_read(user_id, new_channel_id).await?;
    let user = state.get_user(&client).ok_or(CommandError::NeedAuth)?;
    state.send_no_viewer_notifications(&no_viewers, &user).await?;

    let json = json!({
        "cmd": "HASVIEWERS",
        "channel": new_channel_name
    });
    user.send_to_all(&json.to_string()).await;
    Ok(())
}

async fn handle_not_viewing(state: &mut ChatServer, client: SocketAddr, _json: &Value) -> Result<(), CommandError> {
    let (no_viewers, user) = {
        let user = state.get_user_mut(&client).ok_or(CommandError::NeedAuth)?;
        user.clear_viewing(&client);
        (user.get_and_clear_no_viewers(), user.clone())
    };

    state.send_no_viewer_notifications(&no_viewers, &user).await?;
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
        "VIEWING" => handle_viewing(state, client, &json).await,
        "NOTVIEWING" => handle_not_viewing(state, client, &json).await,
        _ => Err(CommandError::MissingCommand)
    }
}
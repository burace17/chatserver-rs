use boolinator::Boolinator;
use lazy_static::lazy_static;
use regex::Regex;
use std::net::SocketAddr;
use serde_json::Value;
use serde_json::json;
use thiserror::Error;
use super::server::ChatServer;
use super::server::ServerCommandResponse;

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
    SendFailed(#[from] tokio::sync::mpsc::error::SendError<ServerCommandResponse>)
}

lazy_static! {
    static ref UP: Regex = Regex::new("^[A-Za-z0-9_-]*$").unwrap();
}

async fn handle_ident(state: &mut ChatServer, client: SocketAddr, json: &Value) -> Result<(), CommandError> {
    println!("Got ident");
    let username = json["username"].as_str().ok_or(CommandError::InvalidArguments)?.to_lowercase();
    // TODO: password

    // check username for invalid characters
    UP.is_match(&username).ok_or(CommandError::InvalidUsername)?;
    let info = state.get_unauthed_connection(client).ok_or(CommandError::InvalidArguments)?;
    state.remove_unauth_connection(client);
    state.add_connection(client, username.to_string());

    // res 200
    let response = json!({
        "res" : 200,
        "name" : "test",
        "motd" : "None yet",
        "channels" : [],
        "nickname" : username,
        "is_registered" : false
    });

    info.tx.send(ServerCommandResponse::Text(response.to_string())).await?;
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
        "IDENT" => handle_ident(state, client, &json).await,
        _ => Err(CommandError::MissingCommand)
    }
}
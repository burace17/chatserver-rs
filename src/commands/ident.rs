/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use super::{CommandError, USERNAME_PATTERN};
use crate::server::{ChatServer, ServerCommandResponse};
use boolinator::Boolinator;
use serde_json::{json, Value};
use std::net::SocketAddr;

pub async fn handle(
    server: &mut ChatServer,
    client: SocketAddr,
    json: &Value,
) -> Result<(), CommandError> {
    let username = json["username"]
        .as_str()
        .ok_or(CommandError::InvalidArguments)?
        .to_lowercase();
    let password = json["password"]
        .as_str()
        .ok_or(CommandError::InvalidArguments)?
        .to_lowercase();

    // check username for invalid characters
    USERNAME_PATTERN
        .is_match(&username)
        .ok_or(CommandError::InvalidUsername)?;

    // check password
    let login_succeeded = server.db.verify_login(&username, &password).await?;
    login_succeeded.ok_or(CommandError::LoginFailed)?;

    let (nickname, channels_being_viewed) = {
        let user = server
            .users
            .get(&username)
            .ok_or(CommandError::InvalidUsername)?;
        (user.nickname.to_string(), user.get_viewed_channels())
    };

    let info = server
        .get_unauthed_connection(client)
        .ok_or(CommandError::InvalidArguments)?;

    server.remove_unauth_connection(client);
    server
        .add_connection(client, username.to_string(), info.tx.clone())
        .await;

    let channels: Vec<serde_json::Value> = server
        .channels
        .values()
        .filter(|chan| chan.users.contains(&username))
        .map(|chan| chan.serialize(|u| server.users.get(&u.to_owned()).cloned()))
        .collect();

    let response = json!({
        "cmd" : "WELCOME",
        "name" : "test",
        "channels" : channels,
        "nickname" : nickname,
        "viewing" : channels_being_viewed
    });

    info.tx
        .send(ServerCommandResponse::Text(response.to_string()))
        .await?;
    Ok(())
}

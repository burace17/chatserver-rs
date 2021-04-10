/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use super::{CommandError, USERNAME_PATTERN};
use crate::server::{ChatServer, ServerCommandResponse};
use crate::user::User;
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

    // check if already registered
    server
        .users
        .get(&username)
        .is_none()
        .ok_or(CommandError::AlreadyRegistered)?;

    let user_id = server.db.register_user(&username, &password).await?;
    let info = server
        .get_unauthed_connection(client)
        .ok_or(CommandError::InvalidArguments)?;

    server.remove_unauth_connection(client);
    server
        .add_connection(client, username.to_string(), info.tx.clone())
        .await;

    let mut user = User::new(user_id, &username, &username);
    user.add_connection(client, info.tx.clone());
    server.add_user(user);

    let response = json!({
        "cmd" : "WELCOME",
        "name" : "test",
        "channels" : [],
        "nickname" : username,
        "viewing" : []
    });

    info.tx
        .send(ServerCommandResponse::Text(response.to_string()))
        .await?;
    Ok(())
}

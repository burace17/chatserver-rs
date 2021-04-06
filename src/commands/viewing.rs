/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::server::ChatServer;
use super::CommandError;
use boolinator::Boolinator;
use std::net::SocketAddr;
use serde_json::{Value, json};

pub async fn handle(server: &mut ChatServer, client: SocketAddr, json: &Value) -> Result<(), CommandError> {
    let new_channel_name = json["channel"].as_str().ok_or(CommandError::InvalidArguments)?.to_lowercase();
    let new_channel_id = {
        let user = server.get_user(&client).ok_or(CommandError::NeedAuth)?;
        let new_channel = server.channels.get(&new_channel_name).ok_or(CommandError::InvalidArguments)?;
        new_channel.users.contains(&user.username).ok_or(CommandError::NotInChannel)?;
        new_channel.id
    };

    let (user_id, no_viewers) = {
        let user = server.get_user_mut(&client).ok_or(CommandError::NeedAuth)?;
        user.set_viewing(&new_channel_name, &client, true);
        let no_viewers = user.get_and_clear_no_viewers();
        (user.id, no_viewers)
    };

    server.db.clear_last_message_read(user_id, new_channel_id).await?;
    let user = server.get_user(&client).ok_or(CommandError::NeedAuth)?;
    server.send_no_viewer_notifications(&no_viewers, &user).await?;

    let json = json!({
        "cmd": "HASVIEWERS",
        "channel": new_channel_name
    });
    user.send_to_all(&json.to_string()).await;
    Ok(())
}

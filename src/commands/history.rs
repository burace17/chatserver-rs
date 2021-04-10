/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use super::CommandError;
use crate::message::Message;
use crate::server::ChatServer;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::SocketAddr;

pub async fn handle(
    server: &mut ChatServer,
    client: SocketAddr,
    json: &Value,
) -> Result<(), CommandError> {
    let user = server.get_user(&client).ok_or(CommandError::NeedAuth)?;
    let mut all_messages = HashMap::new();

    // here we aren't disconnecting the user for invalid arguments... but maybe we should?
    let channels = json["channels"]
        .as_array()
        .ok_or(CommandError::InvalidArguments)?
        .iter()
        .filter_map(|value| value.as_str())
        .filter_map(|str_value| server.channels.get(&str_value.to_lowercase()))
        .filter(|channel| channel.users.contains(&user.username));

    #[derive(Serialize)]
    struct HistoryValue {
        messages: Vec<Message>,
        last_read_message: Option<i64>,
    }

    for channel in channels {
        let messages = server
            .db
            .get_channel_history(channel.id, 50, &server.users)
            .await?;
        let value = HistoryValue {
            messages,
            last_read_message: server.db.get_last_message_read(user.id, channel.id).await?,
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

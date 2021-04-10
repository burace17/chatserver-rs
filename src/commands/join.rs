/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use super::CommandError;
use crate::models::Channel;
use crate::server::ChatServer;
use boolinator::Boolinator;
use serde_json::{json, Value};
use std::collections::hash_map::Entry;
use std::net::SocketAddr;

pub async fn handle(
    server: &mut ChatServer,
    client: SocketAddr,
    json: &Value,
) -> Result<(), CommandError> {
    // User must be authenticated for this command to work
    let user = {
        let u = server.get_user(&client).ok_or(CommandError::NeedAuth)?;
        u.clone()
    };

    let channel_name = json["name"]
        .as_str()
        .ok_or(CommandError::InvalidArguments)?
        .to_lowercase();
    channel_name
        .starts_with("#")
        .ok_or(CommandError::InvalidArguments)?;

    {
        // Mutate the channel
        let channel = match server.channels.entry(channel_name.to_string()) {
            Entry::Occupied(o) => o.into_mut(),
            Entry::Vacant(v) => {
                let channel_id = server.db.make_channel(&channel_name).await?;
                v.insert(Channel::new(channel_id, &channel_name))
            }
        };

        server.db.join_channel(user.id, channel.id).await?;
        channel.add_user(&user);
    }

    // Getting the channel again but as an immutable reference..
    let channel = server.channels.get(&channel_name).unwrap();

    // Broadcast that a new user has joined to everyone in this channel.
    let json = json!({
        "cmd" : "JOIN",
        "channel" : channel.name,
        "user" : user
    });

    // The lookup_user closure is the same for both of these calls, but it doesn't compile if I store it in a local.
    channel
        .broadcast(
            |username| server.users.get(&username.to_owned()).cloned(),
            &json.to_string(),
        )
        .await;

    // Let this user know some information about the channel they joined.
    let json = json!({
        "cmd" : "CHANNELINFO",
        "channel" : channel.serialize(|username| server.users.get(&username.to_owned()).cloned())
    });

    user.send_to(&client, &json.to_string()).await;
    Ok(())
}

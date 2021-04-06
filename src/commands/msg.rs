/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use crate::server::ChatServer;
use super::CommandError;
use boolinator::Boolinator;
use linkify::LinkFinder;
use std::net::SocketAddr;
use std::time::SystemTime;
use serde_json::{Value, json};

pub async fn handle(server: &mut ChatServer, client: SocketAddr, json: &Value) -> Result<(), CommandError> {
    let user = server.get_user(&client).ok_or(CommandError::NeedAuth)?;
    let channel_name = json["channel"].as_str().ok_or(CommandError::InvalidArguments)?.to_lowercase();
    let channel = server.channels.get(&channel_name).ok_or(CommandError::InvalidArguments)?;
    channel.users.contains(&user.username).ok_or(CommandError::NotInChannel)?;

    let msg_text = json["content"].as_str().ok_or(CommandError::InvalidArguments)?;
    (msg_text.len() > 0).ok_or(CommandError::InvalidArguments)?;

    let time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH)?.as_secs() as i64;
    let msg_id = server.db.add_message(user.id, channel.id, time, &user.nickname, &msg_text).await?;

    let json = json!({
        "cmd" : "MSG",
        "user" : user,
        "channel" : channel.name,
        "content" : msg_text,
        "message_id" : msg_id,
        "time" : time
    });

    channel.broadcast(|username| server.users.get(&username.to_owned()).cloned(), &json.to_string()).await;

    let links: Vec<String> = {
        let mut link_finder = LinkFinder::new();
        let links = link_finder.kinds(&[linkify::LinkKind::Url]).links(msg_text);
        links.map(|link| link.as_str().to_string()).collect()
    };

    for link in links {
        if let Err(e) = server.query_for_attachments(channel.id, msg_id, &link).await {
            println!("Could not send link to attachment manager task: {}", e);
        }
    }

    Ok(())
}
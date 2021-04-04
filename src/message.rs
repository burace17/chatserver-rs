/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use super::user::User;
use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct Message {
    message_id: i64,
    user: User,
    time: i64,
    nickname: String,
    content: String,
    pub attachments: Vec<MessageAttachment>
}

impl Message {
    pub fn new(message_id: i64, user: User, time: i64, nickname: &str, content: &str) -> Self {
        Message{
            message_id,
            user,
            time,
            nickname: nickname.to_string(),
            content: content.to_string(),
            attachments: Vec::new()
        }
    }
}

#[derive(Serialize, Clone)]
pub struct MessageAttachment {
    url: String,
    mime: String
}

impl MessageAttachment {
    pub fn new(url: &str, mime: &str) -> Self {
        MessageAttachment{ url: url.to_string(), mime: mime.to_string() }
    }
}
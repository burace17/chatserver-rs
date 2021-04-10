/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use super::user::User;
use serde_json::json;
use std::collections::HashSet;

#[derive(Clone)]
pub struct Channel {
    pub id: i64,
    pub name: String,
    pub users: HashSet<String>,
}

impl Channel {
    pub fn new(id: i64, name: &str) -> Self {
        Channel {
            id,
            name: name.to_string(),
            users: HashSet::new(),
        }
    }

    pub fn add_user(&mut self, user: &User) {
        self.users.insert(user.username.to_string());
    }

    // Sends a message to all users in this channel
    pub async fn broadcast<F: Fn(&str) -> Option<User>>(&self, lookup_user: F, data: &str) {
        for user in self
            .users
            .iter()
            .filter_map(|username| lookup_user(username))
        {
            user.send_to_all(data).await;
        }
    }

    pub async fn broadcast_filter<F: Fn(&str) -> Option<User>, G: Fn(&User) -> bool>(
        &self,
        lookup_user: F,
        filter: G,
        data: &str,
    ) {
        for user in self
            .users
            .iter()
            .filter_map(|username| lookup_user(username).filter(|u| filter(u)))
        {
            user.send_to_all(data).await;
        }
    }

    pub fn serialize<F: Fn(&str) -> Option<User>>(&self, lookup_user: F) -> serde_json::Value {
        let users: Vec<User> = self
            .users
            .iter()
            .filter_map(|username| lookup_user(username))
            .map(|u| u.clone())
            .collect();

        json!({
            "id" : self.id,
            "name" : self.name,
            "users" : users
        })
    }
}

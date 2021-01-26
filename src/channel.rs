use super::user::User;
use serde_json::json;

#[derive(Clone)]
pub struct Channel {
    pub id: i64,
    pub name: String,
    pub users: Vec<String>
}

impl Channel {
    pub fn new(id: i64, name: &str) -> Self {
        Channel{ id, name: name.to_string(), users: Vec::new() }
    }

    pub fn add_user(&mut self, user: &User) {
        self.users.push(user.username.to_string());
    }

    // Sends a message to all users in this channel
    pub async fn broadcast<F: Fn(&str) -> Option<User>>(&self, lookup_user: F, data: &str)  {
        for user in self.users.iter().filter_map(|username| lookup_user(username)) {
            user.send_to_all(data).await;
        }
    }

    pub fn serialize<F: Fn(&str) -> Option<User>>(&self, lookup_user: F) -> String {
        let users: Vec<String> = self.users.iter()
                                           .filter_map(|username| lookup_user(username))
                                           .filter_map(|u| serde_json::to_string(&u).ok())
                                           .collect();

        let json = json!({
            "id" : self.id,
            "name" : self.name,
            "users" : users
        });

        json.to_string()
    }
}
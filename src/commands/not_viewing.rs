/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use super::CommandError;
use crate::server::ChatServer;
use std::net::SocketAddr;

pub async fn handle(server: &mut ChatServer, client: SocketAddr) -> Result<(), CommandError> {
    let (no_viewers, user) = {
        let user = server.get_user_mut(&client).ok_or(CommandError::NeedAuth)?;
        user.clear_viewing(&client);
        (user.get_and_clear_no_viewers(), user.clone())
    };

    server
        .send_no_viewer_notifications(&no_viewers, &user)
        .await?;
    Ok(())
}

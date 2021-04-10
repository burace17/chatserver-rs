/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use futures::stream::StreamExt;
use signal_hook::consts::signal::*;
use signal_hook_tokio::Signals;
use tokio::sync::watch;

pub async fn start(signals: Signals, shutdown_notifier: watch::Sender<bool>) {
    let mut signals = signals.fuse();
    if let Some(s) = signals.next().await {
        match s {
            SIGINT | SIGTERM | SIGQUIT => {
                println!("Shutting down due to signal {}", s);
                shutdown_notifier
                    .send(true)
                    .expect("Failed to send shutdown notification");
            }
            _ => unreachable!(),
        }
    }
}

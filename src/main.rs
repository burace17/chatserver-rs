/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use signal_hook::consts::signal::*;
use signal_hook_tokio::Signals;
use std::env;
use tokio::sync::watch;

mod commands;
mod config_parser;
mod db;
mod models;
mod server;
mod signal_handler;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        println!("usage: {} [path to config file]", args[0]);
        return;
    }

    let signals = Signals::new(&[SIGINT, SIGTERM, SIGQUIT]).unwrap();
    let handle = signals.handle();
    let (shutdown_sender, shutdown_receiver) = watch::channel(false);
    let signals_task = tokio::spawn(signal_handler::start(signals, shutdown_sender));

    let config_path = &args[1];
    match config_parser::parse_config(config_path) {
        Ok(config) => {
            server::start(&config, shutdown_receiver).await;
        }
        Err(e) => {
            // TODO: For certain error types there's more info we can give here.
            println!("Error parsing config file: {}", e);
        }
    }

    handle.close();
    signals_task.await.unwrap();
}

use std::env;
mod client_connection;
mod commands;
mod config_parser;
mod db_interaction;
mod server;
mod user;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        println!("usage: {} [path to config file]", args[0]);
        return;
    }

    let config_path = &args[1];
    match config_parser::parse_config(config_path) {
        Ok(config) => {
            server::start_server(&config).await;
        },
        Err(e) => {
            // TODO: For certain error types there's more info we can give here.
            println!("Error parsing config file: {}", e);
        }
    }
}

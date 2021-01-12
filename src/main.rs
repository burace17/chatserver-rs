use std::env;
mod config_parser;
mod server;


fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        println!("usage: {} [path to config file]", args[0]);
        return;
    }

    let config_path = &args[1];
    match config_parser::parse_config(config_path) {
        Ok(config) => {
            let server = server::ChatServer::new();
            server.start(&config);
        },
        Err(e) => {
            // TODO: For certain error types there's more info we can give here.
            println!("Error parsing config file: {}", e);
        }
    }
}

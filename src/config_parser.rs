/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/. */

use std::fs::File;
use std::io::BufReader;
use std::io::Read;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigReadError {
    #[error("IO Error")]
    IOError(std::io::ErrorKind),
    #[error("The configuration file is not valid JSON.")]
    JSONParseError(#[from] serde_json::error::Error),
    #[error("cert_path is missing or invalid.")]
    MissingCertPath,
    #[error("cert_password is missing or invalid.")]
    MissingCertPassword,
    #[error("bind_ip is missing or invalid.")]
    MissingBindIP,
    #[error("bind_port is missing or invalid.")]
    MissingBindPort,
    #[error("database path is missing")]
    MissingDatabasePath,
}

impl From<std::io::Error> for ConfigReadError {
    fn from(error: std::io::Error) -> Self {
        ConfigReadError::IOError(error.kind())
    }
}

pub struct ServerConfig {
    pub cert: Vec<u8>,
    pub cert_password: String,
    pub bind_ip: String,
    pub port: u64,
    pub db_path: String,
}

pub fn parse_config(config_path: &str) -> Result<ServerConfig, ConfigReadError> {
    let file = File::open(config_path)?;
    let reader = BufReader::new(file);
    let config: serde_json::Value = serde_json::from_reader(reader)?;

    let cert_path = config["cert_path"]
        .as_str()
        .ok_or(ConfigReadError::MissingCertPath)?;
    let cert_password = config["cert_password"]
        .as_str()
        .ok_or(ConfigReadError::MissingCertPassword)?;

    let mut file = File::open(cert_path)?;
    let mut pkcs12 = vec![];
    file.read_to_end(&mut pkcs12)?;

    let bind_ip = config["bind_ip"]
        .as_str()
        .ok_or(ConfigReadError::MissingBindIP)?;
    let bind_port = config["bind_port"]
        .as_u64()
        .ok_or(ConfigReadError::MissingBindPort)?;

    let db_path = config["db_path"]
        .as_str()
        .ok_or(ConfigReadError::MissingDatabasePath)?;

    let server_config = ServerConfig {
        cert: pkcs12,
        cert_password: cert_password.to_string(),
        bind_ip: bind_ip.to_string(),
        port: bind_port,
        db_path: db_path.to_string(),
    };

    Ok(server_config)
}

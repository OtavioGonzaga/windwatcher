use std::net::IpAddr;

use crate::{
    cli::{Cli, http::HttpCli},
    config::{
        ConfigError,
        http::ports::{HttpConfig, HttpConfigProvider},
    },
};
use clap::Parser;

pub struct CliHttpConfig();

impl HttpConfigProvider for CliHttpConfig {
    fn load() -> Result<HttpConfig, ConfigError> {
        let args: HttpCli = Cli::parse_from(std::env::args_os()).http;

        let host: String = args.http_host.ok_or(ConfigError::Missing("http-host"))?;
        let port: u16 = args.http_port.ok_or(ConfigError::Missing("http-port"))?;
        let jwt_secret: String = args.jwt_secret.ok_or(ConfigError::Missing("jwt-secret"))?;

        if 1024 > port {
            return Err(ConfigError::Invalid("http-port"));
        }

        if !is_valid_host(&host) {
            return Err(ConfigError::Invalid("http-host"));
        }

        Ok(HttpConfig {
            host,
            port,
            jwt_secret,
        })
    }
}

fn is_valid_host(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok()
}

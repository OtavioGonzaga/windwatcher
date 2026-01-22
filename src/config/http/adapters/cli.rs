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
        let token_secret: String = args
            .token_secret
            .ok_or(ConfigError::Missing("token-secret"))?;
        let token_ttl: u64 = args
            .token_ttl
            .ok_or(ConfigError::Missing("token-ttl"))?
            .parse()
            .map_err(|_| ConfigError::Invalid("token-ttl"))?;
        let refresh_token_ttl: u64 = args
            .refresh_token_ttl
            .ok_or(ConfigError::Missing("refresh-token-ttl"))?
            .parse()
            .map_err(|_| ConfigError::Invalid("refresh-token-ttl"))?;

        if 1024 > port {
            return Err(ConfigError::Invalid("http-port"));
        }

        if !is_valid_host(&host) {
            return Err(ConfigError::Invalid("http-host"));
        }

        Ok(HttpConfig {
            host,
            port,
            token_secret,
            token_ttl,
            refresh_token_ttl,
        })
    }
}

fn is_valid_host(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok()
}

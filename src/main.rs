mod cli;
mod config;
mod domain;

use crate::config::Config;
use actix_web::{Error, main};
use log::info;

#[main]
async fn main() -> Result<(), Error> {
    let Config { http, logging } = Config::load().expect("Failed to load configuration");

    env_logger::Builder::from_env(env_logger::Env::default().filter_or("RUST_LOG", logging.level))
        .format_timestamp_secs()
        .init();

    info!("Starting application");

    Ok(())
}

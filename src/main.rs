mod cli;
mod config;
mod adapters;
mod application;
mod domain;

use crate::{
    adapters::{http::actix::server::build_app, persistence::postgres::connection::connect_to_db},
    config::Config,
};
use actix_web::main;
use log::info;
use sea_orm::DatabaseConnection;
use std::io::Error;

#[main]
async fn main() -> Result<(), Error> {
    let Config {
        http: http_config,
        logging: logging_config,
        database: database_config,
    } = Config::load().expect("Failed to load configuration");

    env_logger::Builder::from_env(
        env_logger::Env::default().filter_or("RUST_LOG", &logging_config.level),
    )
    .format_timestamp_secs()
    .init();

    let db: DatabaseConnection = connect_to_db(&database_config, &logging_config).await;

    info!("Starting application");

    build_app(http_config, db).await
}

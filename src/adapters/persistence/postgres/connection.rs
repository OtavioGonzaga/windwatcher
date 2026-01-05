use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use std::time::Duration;

use crate::config::database::ports::DatabaseConfig;

pub async fn connect_to_db(database_config: &DatabaseConfig) -> DatabaseConnection {
    let database_url: &String = &database_config.database_url;

    let mut opt = ConnectOptions::new(database_url);
    opt.max_connections(100)
        .min_connections(5)
        .connect_timeout(Duration::from_secs(8))
        .acquire_timeout(Duration::from_secs(8))
        .idle_timeout(Duration::from_secs(8))
        .max_lifetime(Duration::from_secs(8))
        .sqlx_logging(false) // disable SQLx logging
        .sqlx_logging_level(log::LevelFilter::Info)
        .set_schema_search_path("public"); // set default Postgres schema

    let db: DatabaseConnection = Database::connect(opt).await.unwrap();

    db
}

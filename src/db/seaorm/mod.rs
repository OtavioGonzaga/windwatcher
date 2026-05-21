//! SeaORM adapter - entities, migrations, and repository implementations.
//!
//! This module is the top-level entry-point for the SQL persistence layer.
//! It exposes:
//!
//! * [`setup_database`] - connects to the database and runs pending migrations.
//! * [`SeaOrmUserRepository`] - [`crate::domain::ports::UserRepository`] impl.
//! * [`SeaOrmChatRepository`] - [`crate::domain::ports::ChatRepository`] impl.
//!
//! Supported databases (via SeaORM / SQLx feature flags):
//! `sqlite`, `postgres`, `mysql`.  The default feature set enables SQLite,
//! which allows zero-configuration local development.

pub mod chat_repo;
pub mod entities;
pub mod migrations;
pub mod user_repo;

pub use chat_repo::SeaOrmChatRepository;
pub use user_repo::SeaOrmUserRepository;

use std::time::Duration;

use sea_orm::{ConnectOptions, Database, DatabaseConnection};
use sea_orm_migration::MigratorTrait;

use crate::{config::AppConfig, error::AppError};

use migrations::Migrator;

/// Connect to the database described by `config` and run all pending migrations.
///
/// This is the single entry-point used by `main` to bootstrap the persistence
/// layer.  Call it once at startup and share the returned [`DatabaseConnection`]
/// via `Arc` (or an Axum `State`).
///
/// # Parameters
///
/// * `config` - application configuration.  `config.database_url` is used as
///   the connection string; accepted URL schemes are `sqlite://`,
///   `postgres://`, and `mysql://`.
///
/// # What it does
///
/// 1. Builds a [`ConnectOptions`] from `config.database_url` with sensible
///    timeouts (connect / acquire / idle / max-lifetime all set to 8s).
/// 2. Calls [`Database::connect`] to open the connection pool.
/// 3. Runs all pending migrations in order via [`Migrator::up`], creating or
///    evolving the schema automatically on every startup.
///
/// # Errors
///
/// Returns [`AppError::Database`] if the connection cannot be established or
/// if any migration step fails.
///
/// > **Note on `max_connections(10)`**: the pool is intentionally capped at
/// > 10 concurrent connections, which is appropriate for SQLite and
/// > low-traffic deployments.  For high-concurrency Postgres deployments
/// > consider exposing this value via `AppConfig`.
pub async fn setup_database(config: &AppConfig) -> Result<DatabaseConnection, AppError> {
    let mut opts = ConnectOptions::new(&config.database_url);
    opts.max_connections(10)
        .min_connections(1)
        .connect_timeout(Duration::from_secs(8))
        .acquire_timeout(Duration::from_secs(8))
        .idle_timeout(Duration::from_secs(8))
        .max_lifetime(Duration::from_secs(8))
        .sqlx_logging(false);

    tracing::info!(url = %config.database_url, "connecting to database");

    let db = Database::connect(opts).await?;

    tracing::info!("running pending migrations");
    Migrator::up(&db, None).await?;
    tracing::info!("migrations complete");

    Ok(db)
}

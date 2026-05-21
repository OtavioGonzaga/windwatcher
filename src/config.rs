//! Application configuration.
//!
//! Configuration is assembled at startup by [`AppConfig::load`] using the
//! figment layered-configuration library.  Settings can be provided via a
//! `windwatcher.toml` file or `WINDWATCHER_*` environment variables - with
//! environment variables taking highest priority.
//!
//! When no database URL is supplied the application falls back to a local
//! SQLite file (`windwatcher_local_data.db`), making it runnable with zero
//! external dependencies.

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use serde::{Deserialize, Serialize};

/// Which database backend to use at runtime.
///
/// The variant is normally detected automatically from the `database_url`
/// prefix (see [`detect_provider`]), but it can also be set explicitly via
/// the `WINDWATCHER_DATABASE_PROVIDER` environment variable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseProvider {
    /// PostgreSQL (also matches `postgresql://` URLs).
    Postgres,
    /// MySQL / MariaDB (also matches `mariadb://` URLs).
    Mysql,
    /// SQLite - the default when no URL is configured.
    #[default]
    Sqlite,
    /// MongoDB - requires the `mongodb` Cargo feature to be enabled.
    MongoDb,
}

/// Which Apalis queue backend to use at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum QueueProvider {
    /// In-process queue backed by Apalis memory storage.
    Memory,
    /// SQLite-backed persistent queue.
    #[default]
    Sqlite,
    /// PostgreSQL-backed persistent queue.
    Postgres,
    /// MySQL-backed persistent queue.
    Mysql,
    /// Redis-backed queue.
    Redis,
}

/// Application-wide configuration, assembled at startup by [`AppConfig::load`].
///
/// Every field maps to an `WINDWATCHER_<FIELD>` environment variable (using `__`
/// as a nested-key separator) or its equivalent key inside `windwatcher.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// Full database connection URL.
    ///
    /// Supported schemes: `sqlite://`, `postgres://`, `postgresql://`,
    /// `mysql://`, `mariadb://`, `mongodb://`.
    /// Defaults to `sqlite://windwatcher_local_data.db?mode=rwc` (local file,
    /// created automatically if it does not exist).
    pub database_url: String,

    /// Database backend inferred from [`database_url`][Self::database_url].
    ///
    /// Detected automatically by [`detect_provider`]; can be overridden with
    /// `WINDWATCHER_DATABASE_PROVIDER` for edge-cases where the URL prefix is
    /// non-standard.
    pub database_provider: DatabaseProvider,
    /// Queue backend provider used by the background worker.
    pub queue_provider: QueueProvider,
    /// Connection URL used by the queue backend.
    ///
    /// This is intentionally independent from `database_url`.
    pub queue_url: String,
    /// Logical queue name.
    pub queue_name: String,
    /// Maximum number of concurrent job handlers per worker.
    pub queue_concurrency: usize,

    /// IP address the HTTP server binds to.
    ///
    /// Use `0.0.0.0` to listen on all interfaces (the default) or `127.0.0.1`
    /// to restrict to localhost.
    pub server_host: String,

    /// TCP port the HTTP server listens on.  Defaults to `3000`.
    pub server_port: u16,

    /// HMAC-SHA256 secret used to sign and verify JWT tokens.
    ///
    /// **Must** be changed to a strong random value before deploying to
    /// production.  Leaking this secret allows anyone to forge valid tokens.
    pub jwt_secret: String,

    /// How long (in seconds) a JWT remains valid after it is issued.
    ///
    /// Defaults to `86 400` (24 hours).  Lower values improve security at the
    /// cost of more frequent re-authentication.
    pub jwt_expiry_secs: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            database_url: "sqlite://windwatcher_local_data.db?mode=rwc".into(),
            database_provider: DatabaseProvider::Sqlite,
            queue_provider: QueueProvider::Sqlite,
            queue_url: "sqlite://windwatcher_jobs.db?mode=rwc".into(),
            queue_name: "chat_messages".into(),
            queue_concurrency: 4,
            server_host: "0.0.0.0".into(),
            server_port: 3000,
            jwt_secret: "change-this-secret-in-production".into(),
            jwt_expiry_secs: 86_400,
        }
    }
}

impl AppConfig {
    /// Load and assemble the application configuration.
    ///
    /// Sources are merged in the following priority order (highest wins):
    ///
    /// 1. `WINDWATCHER_*` environment variables (e.g. `WINDWATCHER_SERVER_PORT`)
    /// 2. `windwatcher.toml` in the current working directory (optional)
    /// 3. Built-in compile-time defaults (SQLite, port 3000, 24 h JWT expiry)
    ///
    /// After merging, the `database_provider` field is automatically inferred
    /// from the `database_url` prefix via [`detect_provider`] unless it was
    /// explicitly overridden.
    ///
    /// # Errors
    ///
    /// Returns a [`figment::Error`] when:
    /// - `windwatcher.toml` exists but contains invalid TOML.
    /// - An environment variable has an unexpected type (e.g. non-numeric port).
    /// - A required key cannot be deserialised into [`AppConfig`].
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use windwatcher::config::AppConfig;
    ///
    /// let config = AppConfig::load().expect("failed to load configuration");
    /// println!("Listening on {}:{}", config.server_host, config.server_port);
    /// ```
    pub fn load() -> Result<Self, anyhow::Error> {
        let mut config: AppConfig = Figment::new()
            .merge(Serialized::defaults(AppConfig::default()))
            .merge(Toml::file("windwatcher.toml"))
            .merge(Env::prefixed("WINDWATCHER_").split("__"))
            .extract()?;

        // Auto-detect provider from URL prefix when not explicitly set.
        if config.database_provider == DatabaseProvider::Sqlite {
            config.database_provider = detect_provider(&config.database_url);
        }

        Ok(config)
    }
}

/// Infer the [`DatabaseProvider`] variant from a connection URL prefix.
///
/// | URL prefix                       | Detected provider              |
/// | -------------------------------- | ------------------------------ |
/// | `postgres://` \| `postgresql://` | [`DatabaseProvider::Postgres`] |
/// | `mysql://` \| `mariadb://`       | [`DatabaseProvider::Mysql`]    |
/// | `mongodb://`                     | [`DatabaseProvider::MongoDb`]  |
/// | anything else                    | [`DatabaseProvider::Sqlite`]   |
///
/// This function is called automatically by [`AppConfig::load`] and should
/// rarely need to be invoked directly.
pub fn detect_provider(url: &str) -> DatabaseProvider {
    if url.starts_with("postgres") || url.starts_with("postgresql") {
        DatabaseProvider::Postgres
    } else if url.starts_with("mysql") || url.starts_with("mariadb") {
        DatabaseProvider::Mysql
    } else if url.starts_with("mongodb") {
        DatabaseProvider::MongoDb
    } else {
        DatabaseProvider::Sqlite
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── detect_provider ─────────────────────────────────────────────────────────

    #[test]
    fn detects_postgres() {
        assert_eq!(
            detect_provider("postgres://user:pass@localhost/db"),
            DatabaseProvider::Postgres
        );
        assert_eq!(
            detect_provider("postgresql://user:pass@localhost/db"),
            DatabaseProvider::Postgres
        );
    }

    #[test]
    fn detects_mysql() {
        assert_eq!(
            detect_provider("mysql://user:pass@localhost/db"),
            DatabaseProvider::Mysql
        );
        assert_eq!(
            detect_provider("mariadb://user:pass@localhost/db"),
            DatabaseProvider::Mysql
        );
    }

    #[test]
    fn detects_mongodb() {
        assert_eq!(
            detect_provider("mongodb://user:pass@localhost/db"),
            DatabaseProvider::MongoDb
        );
    }

    #[test]
    fn detects_sqlite_fallback() {
        assert_eq!(
            detect_provider("sqlite://path/to/db"),
            DatabaseProvider::Sqlite
        );
        assert_eq!(
            detect_provider("unknown://something"),
            DatabaseProvider::Sqlite
        );
    }
}

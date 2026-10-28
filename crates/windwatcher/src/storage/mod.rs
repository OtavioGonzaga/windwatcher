mod migration;
#[cfg(all(test, any(feature = "postgres", feature = "mysql")))]
#[macro_use]
mod snapshot_tests;
#[cfg(feature = "mysql")]
mod mysql;
#[cfg(feature = "postgres")]
mod postgres;
#[cfg(feature = "sqlite")]
mod sqlite;

pub use migration::MigrationStatus;
#[cfg(feature = "mysql")]
pub use mysql::{MySqlStorage, MySqlStorageError, MySqlStorageOptions};
#[cfg(feature = "postgres")]
pub use postgres::{PostgresStorage, PostgresStorageError, PostgresStorageOptions};
#[cfg(feature = "sqlite")]
pub use sqlite::{SqliteStorage, SqliteStorageError, SqliteStorageOptions};

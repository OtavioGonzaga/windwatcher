use std::{error::Error, path::PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("user {0} was not found")]
    UserNotFound(windwatcher::UserId),
    #[error("user management failed")]
    UserManagement(#[source] windwatcher::ManageUserError),
    #[error("initialization failed for {path}: {source}")]
    Init {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("installation is unhealthy")]
    DoctorUnhealthy,
    #[error("failed to bind server listener at {bind}")]
    ListenerBind {
        bind: std::net::SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error("server runtime failed")]
    ServerRuntime(#[source] std::io::Error),
    #[error("server terminated unexpectedly")]
    ServerTerminated,
    #[error("configuration error: {0}")]
    Config(String),
    #[error("path resolution failed: {0}")]
    Paths(String),
    #[error("failed to create data directory {path}")]
    DataDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to open {backend} database")]
    Storage {
        backend: &'static str,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    #[error("failed to inspect database migrations")]
    MigrationStatus(#[source] Box<dyn Error + Send + Sync>),
    #[error("failed to apply database migrations")]
    Migration(#[source] Box<dyn Error + Send + Sync>),
    #[error("database schema is not current")]
    SchemaPending,
    #[error("queue schema is not current")]
    QueueSchemaPending,
    #[error("failed to connect to {backend} queue")]
    QueueConnection {
        backend: &'static str,
        #[source]
        source: Box<dyn Error + Send + Sync>,
    },
    #[error("queue operation failed")]
    QueueOperation(#[source] Box<dyn Error + Send + Sync>),
    #[error("worker task terminated unexpectedly; restart the worker process")]
    WorkerTaskTerminated,
    #[error("failed to listen for shutdown signal")]
    ShutdownSignal(#[source] std::io::Error),
    #[error("username {username:?} is already taken")]
    UsernameTaken { username: String },
    #[error("password input error: {0}")]
    PasswordInput(String),
    #[error("user creation failed: {0}")]
    UserCreation(#[source] windwatcher::CreateLocalUserError),
}

impl AppError {
    pub fn hint(&self) -> Option<&'static str> {
        match self {
            Self::SchemaPending => Some("run `windwatcher migrate --storage`"),
            Self::QueueSchemaPending => Some("run `windwatcher migrate --queue`"),
            _ => None,
        }
    }

    pub fn operator_message(&self) -> String {
        match self {
            Self::DataDirectory { path, source } => {
                format!(
                    "failed to create data directory {}: {source}",
                    path.display()
                )
            }
            Self::Storage { source, .. } => source
                .downcast_ref::<windwatcher::MongoDbStorageError>()
                .and_then(|error| match error {
                    windwatcher::MongoDbStorageError::UnsupportedTopology(message) => {
                        Some(format!("unsupported MongoDB topology: {message}"))
                    }
                    windwatcher::MongoDbStorageError::Configuration(message) => {
                        Some(format!("invalid MongoDB storage configuration: {message}"))
                    }
                    _ => None,
                })
                .unwrap_or_else(|| self.to_string()),
            Self::MigrationStatus(source) | Self::Migration(source) => source
                .downcast_ref::<windwatcher::SqliteStorageError>()
                .and_then(|error| match error {
                    windwatcher::SqliteStorageError::InvalidMigrationHistory(message) => {
                        Some(format!("invalid database migration history: {message}"))
                    }
                    _ => None,
                })
                .or_else(|| {
                    source.downcast_ref::<crate::queue::QueueLifecycleError>().and_then(|error| {
                        use crate::queue::QueueLifecycleError as E;
                        match error {
                            E::Sqlite(windwatcher::SqliteQueueError::InvalidMigrationHistory(_))
                            | E::Postgres(windwatcher::PostgresQueueError::InvalidMigrationHistory)
                            | E::MySql(windwatcher::MySqlQueueError::InvalidMigrationHistory) => Some("invalid queue migration history; inspect the migration ledger before retrying".into()),
                            _ => None,
                        }
                    })
                })
                .or_else(|| {
                    source
                        .downcast_ref::<windwatcher::PostgresStorageError>()
                        .and_then(|error| match error {
                            windwatcher::PostgresStorageError::InvalidMigrationHistory(message) => {
                                Some(format!("invalid database migration history: {message}"))
                            }
                            _ => None,
                        })
                })
                .or_else(|| {
                    source
                        .downcast_ref::<windwatcher::MySqlStorageError>()
                        .and_then(|error| match error {
                            windwatcher::MySqlStorageError::InvalidMigrationHistory(message) => {
                                Some(format!("invalid database migration history: {message}"))
                            }
                            _ => None,
                        })
                })
                .or_else(|| {
                    source
                        .downcast_ref::<windwatcher::MongoDbStorageError>()
                        .and_then(|error| match error {
                            windwatcher::MongoDbStorageError::InvalidMigrationHistory(message) => {
                                Some(format!("invalid database migration history: {message}"))
                            }
                            _ => None,
                        })
                })
                .unwrap_or_else(|| self.to_string()),
            Self::UserCreation(windwatcher::CreateLocalUserError::InvalidUsername(error)) => {
                format!("invalid username: {error}")
            }
            Self::UserCreation(windwatcher::CreateLocalUserError::InvalidPassword(error)) => {
                format!("invalid password: {error}")
            }
            _ => self.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AppError;

    #[test]
    fn operator_messages_expose_safe_actionable_causes() {
        let password = AppError::UserCreation(windwatcher::CreateLocalUserError::InvalidPassword(
            windwatcher::PasswordPolicyError::TooShort,
        ));
        assert_eq!(
            password.operator_message(),
            "invalid password: password must contain at least 8 characters"
        );

        let migration = AppError::MigrationStatus(Box::new(
            windwatcher::SqliteStorageError::InvalidMigrationHistory(
                "migration 1 checksum differs".into(),
            ),
        ));
        assert_eq!(
            migration.operator_message(),
            "invalid database migration history: migration 1 checksum differs"
        );
        for source in [
            Box::new(windwatcher::PostgresStorageError::InvalidMigrationHistory(
                "migration 1 checksum differs".into(),
            )) as Box<dyn std::error::Error + Send + Sync>,
            Box::new(windwatcher::MySqlStorageError::InvalidMigrationHistory(
                "migration 1 checksum differs".into(),
            )),
            Box::new(windwatcher::MongoDbStorageError::InvalidMigrationHistory(
                "migration 1 checksum differs".into(),
            )),
        ] {
            assert_eq!(
                AppError::MigrationStatus(source).operator_message(),
                migration.operator_message()
            );
        }
        let connection = AppError::Storage {
            backend: "PostgreSQL",
            source: Box::new(std::io::Error::other("secret")),
        };
        assert_eq!(
            connection.operator_message(),
            "failed to open PostgreSQL database"
        );
        let topology = AppError::Storage {
            backend: "MongoDB",
            source: Box::new(windwatcher::MongoDbStorageError::UnsupportedTopology(
                "a transaction-capable replica set is required",
            )),
        };
        assert_eq!(
            topology.operator_message(),
            "unsupported MongoDB topology: a transaction-capable replica set is required"
        );
        let secret = AppError::Storage {
            backend: "MongoDB",
            source: Box::new(windwatcher::MongoDbStorageError::Connection(Box::new(
                std::io::Error::other("private://user:secret@host"),
            ))),
        };
        assert_eq!(secret.operator_message(), "failed to open MongoDB database");
        let invalid = AppError::Migration(Box::new(
            windwatcher::MongoDbStorageError::InvalidMigrationHistory(
                "duplicate migration version 1".into(),
            ),
        ));
        assert_eq!(
            invalid.operator_message(),
            "invalid database migration history: duplicate migration version 1"
        );
    }
}

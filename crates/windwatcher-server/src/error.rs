use std::{error::Error, path::PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
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
    #[error("failed to open SQLite database")]
    Storage(#[source] Box<dyn Error + Send + Sync>),
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
    #[error("failed to listen for worker shutdown signal")]
    WorkerSignal(#[source] std::io::Error),
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
            Self::MigrationStatus(source) => source
                .downcast_ref::<windwatcher::SqliteStorageError>()
                .and_then(|error| match error {
                    windwatcher::SqliteStorageError::InvalidMigrationHistory(message) => {
                        Some(format!("invalid database migration history: {message}"))
                    }
                    _ => None,
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
    }
}

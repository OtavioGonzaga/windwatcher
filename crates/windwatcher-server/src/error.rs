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
    #[error("username {username:?} is already taken")]
    UsernameTaken { username: String },
    #[error("password input error: {0}")]
    PasswordInput(String),
    #[error("user creation failed: {0}")]
    UserCreation(#[source] windwatcher::CreateLocalUserError),
}

impl AppError {
    pub fn hint(&self) -> Option<&'static str> {
        matches!(self, Self::SchemaPending).then_some("run `windwatcher migrate`")
    }
}

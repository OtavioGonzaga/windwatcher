//! MongoDB adapter - connection setup, index management, and repository
//! implementations.
//!
//! All items in this module (and its children) are compiled only when the
//! **`mongodb`** Cargo feature is enabled.
//!
//! ## Quick-start
//!
//! ```rust,ignore
//! // In your startup code:
//! let db = db::mongodb::setup_mongodb(&config).await?;
//!
//! let user_repo = MongoUserRepository { col: db.collection("users") };
//! let chat_repo = MongoChatRepository { db: db.clone() };
//! ```

#![cfg(feature = "mongodb")]

pub mod chat_repo;
mod setup;
pub mod user_repo;

pub use chat_repo::MongoChatRepository;
pub use user_repo::MongoUserRepository;

use crate::{config::AppConfig, error::AppError};

/// Connect to MongoDB, ensure all indexes exist, and return the
/// `windwatcher` [`mongodb::Database`] handle.
///
/// # Errors
/// Returns [`AppError::Database`] if the URI cannot be parsed or if index
/// creation fails.
pub async fn setup_mongodb(config: &AppConfig) -> Result<mongodb::Database, AppError> {
    let client = mongodb::Client::with_uri_str(&config.database_url)
        .await
        .map_err(|e| AppError::Database(format!("mongodb URI parse error: {e}")))?;

    let db = client.database("windwatcher");

    setup::setup_indexes(&db).await?;

    tracing::info!(url = %config.database_url, "connected to mongodb");

    Ok(db)
}

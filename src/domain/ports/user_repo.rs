use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    domain::models::User,
    error::AppError,
};

/// Storage port for user accounts.
///
/// The application layer calls this trait; concrete SQL and MongoDB
/// implementations are in `db/seaorm/user_repo.rs` and
/// `db/mongodb/user_repo.rs` respectively.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait UserRepository: Send + Sync {
    /// Look up a user by their primary key.
    ///
    /// Returns `Ok(None)` when no user with the given ID exists.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Database`] if the underlying storage query fails.
    async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, AppError>;

    /// Look up a user by their email address.
    ///
    /// Returns `Ok(None)` when no user with the given email exists.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Database`] if the underlying storage query fails.
    async fn find_by_email(&self, email: &str) -> Result<Option<User>, AppError>;

    /// Persist a new user and return the saved record.
    ///
    /// The caller is responsible for constructing the [`User`] with a
    /// pre-hashed password and a freshly generated UUIDv7.
    ///
    /// # Errors
    ///
    /// - [`AppError::Conflict`] - if a user with the same email already exists
    ///   (adapter implementations should translate unique-constraint violations).
    /// - [`AppError::Database`] - for any other storage failure.
    async fn create(&self, user: User) -> Result<User, AppError>;
}

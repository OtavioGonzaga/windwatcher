//! SeaORM implementation of [`UserRepository`].

use std::str::FromStr;

use async_trait::async_trait;
use chrono::Utc;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use uuid::Uuid;

use crate::{
    domain::{
        models::{User, UserRole},
        ports::UserRepository,
    },
    error::AppError,
};

use super::entities::user;

// ── Public struct ─────────────────────────────────────────────────────────────

/// SeaORM-backed implementation of [`UserRepository`].
///
/// Wraps a shared [`DatabaseConnection`] pool and fulfils the user-persistence
/// contract defined in [`crate::domain::ports::UserRepository`].  Compatible
/// with any SQL database supported by SeaORM: SQLite, PostgreSQL, and MySQL.
///
/// Construct this type directly and wrap it in an `Arc` (or store it inside
/// [`crate::state::AppState`]) before passing it to the service layer.
pub struct SeaOrmUserRepository {
    /// Shared SeaORM connection pool, injected at construction time.
    ///
    /// [`DatabaseConnection`] is `Clone + Send + Sync`, so it can be shared
    /// freely across threads without additional locking.
    pub db: DatabaseConnection,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convert a DB model to the domain type.
fn model_to_domain(m: user::Model) -> Result<User, AppError> {
    let role = UserRole::from_str(&m.role)
        .map_err(|e| AppError::Internal(format!("invalid role stored in database: {e}")))?;

    Ok(User {
        id: m.id,
        username: m.username,
        email: m.email,
        password_hash: m.password_hash,
        role,
        created_at: m.created_at.with_timezone(&Utc),
        updated_at: m.updated_at.with_timezone(&Utc),
    })
}

// ── Trait implementation ──────────────────────────────────────────────────────

#[async_trait]
impl UserRepository for SeaOrmUserRepository {
    /// Look up a user by their UUID primary key.
    ///
    /// Returns `Ok(None)` when no row with the given `id` exists.
    ///
    /// # Errors
    ///
    /// * [`AppError::Database`] - if the underlying SQL query fails.
    /// * [`AppError::Internal`] - if the `role` column contains an
    ///   unrecognised string that cannot be parsed into [`UserRole`].
    async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, AppError> {
        let model = user::Entity::find_by_id(id).one(&self.db).await?;
        model.map(model_to_domain).transpose()
    }

    /// Look up a user by their unique email address.
    ///
    /// Returns `Ok(None)` when no user with the given `email` exists.
    ///
    /// # Errors
    ///
    /// * [`AppError::Database`] - if the underlying SQL query fails.
    /// * [`AppError::Internal`] - if the `role` column contains an
    ///   unrecognised string that cannot be parsed into [`UserRole`].
    async fn find_by_email(&self, email: &str) -> Result<Option<User>, AppError> {
        let model = user::Entity::find()
            .filter(user::Column::Email.eq(email))
            .one(&self.db)
            .await?;
        model.map(model_to_domain).transpose()
    }

    /// Persist a new user row and return the inserted domain model.
    ///
    /// All fields of `new_user` (ID, password hash, timestamps, role) must be
    /// fully populated by the caller before invoking this method.  The `role`
    /// field is stored as its string representation (`"user"` / `"admin"`).
    ///
    /// The `DateTime<Utc>` timestamps are converted to `DateTimeWithTimeZone`
    /// via `fixed_offset()` before being handed to SeaORM's [`Set`] helper,
    /// and then converted back on the returned row.
    ///
    /// # Errors
    ///
    /// * [`AppError::Database`] - if the `INSERT` fails (e.g. a duplicate
    ///   email address violates the unique constraint).
    /// * [`AppError::Internal`] - if the round-tripped row cannot be
    ///   converted back to the domain type (should not happen in practice).
    async fn create(&self, new_user: User) -> Result<User, AppError> {
        let active = user::ActiveModel {
            id: Set(new_user.id),
            username: Set(new_user.username),
            email: Set(new_user.email),
            password_hash: Set(new_user.password_hash),
            role: Set(new_user.role.to_string()),
            created_at: Set(new_user.created_at.fixed_offset()),
            updated_at: Set(new_user.updated_at.fixed_offset()),
        };

        let inserted = active.insert(&self.db).await?;
        model_to_domain(inserted)
    }
}

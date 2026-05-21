//! MongoDB implementation of [`UserRepository`].
//!
//! ## Document schema - `users` collection
//!
//! | Field           | BSON type | Notes                                  |
//! | --------------- | --------- | -------------------------------------- |
//! | `_id`           | String    | UUID v4/v7 as hyphenated string        |
//! | `username`      | String    |                                        |
//! | `email`         | String    | unique (enforced by index)             |
//! | `password_hash` | String    | Argon2 hash                            |
//! | `role`          | String    | `"user"` or `"admin"`                  |
//! | `created_at`    | String    | RFC 3339                               |
//! | `updated_at`    | String    | RFC 3339                               |

#![cfg(feature = "mongodb")]

use async_trait::async_trait;
use chrono::Utc;
use mongodb::{
    Collection,
    bson::{Document, doc},
};
use uuid::Uuid;

use crate::{
    domain::{
        models::{User, UserRole},
        ports::UserRepository,
    },
    error::AppError,
};

// ── Public struct ─────────────────────────────────────────────────────────────

/// MongoDB-backed user repository.
///
/// Construct it from a typed `Collection<Document>` handle obtained via
/// `db.collection::<Document>("users")`.
pub struct MongoUserRepository {
    /// Typed handle to the `users` MongoDB collection.
    ///
    /// Every method on this repository obtains its data exclusively through
    /// this collection reference - no other collections are accessed.
    pub col: Collection<Document>,
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Serialize a domain [`User`] into a BSON document ready for insertion.
///
/// `_id` is set to the UUID string so MongoDB uses it as the primary key.
/// `password_hash` **is** included here - the `#[serde(skip_serializing)]`
/// attribute only affects JSON serialization, not our manual mapping.
fn user_to_doc(u: &User) -> Document {
    doc! {
        "_id":           u.id.to_string(),
        "username":      &u.username,
        "email":         &u.email,
        "password_hash": &u.password_hash,
        "role":          u.role.to_string(),
        "created_at":    u.created_at.to_rfc3339(),
        "updated_at":    u.updated_at.to_rfc3339(),
    }
}

/// Deserialize a BSON document retrieved from MongoDB into a domain [`User`].
fn doc_to_user(d: Document) -> Result<User, AppError> {
    macro_rules! str_field {
        ($key:literal) => {
            d.get_str($key)
                .map_err(|e| AppError::Internal(format!("users.{}: {e}", $key)))?
        };
    }

    let id = str_field!("_id")
        .parse::<Uuid>()
        .map_err(|e| AppError::Internal(format!("users._id parse: {e}")))?;

    let role = str_field!("role")
        .parse::<UserRole>()
        .map_err(|e| AppError::Internal(format!("users.role parse: {e}")))?;

    let created_at = chrono::DateTime::parse_from_rfc3339(str_field!("created_at"))
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| AppError::Internal(format!("users.created_at parse: {e}")))?;

    let updated_at = chrono::DateTime::parse_from_rfc3339(str_field!("updated_at"))
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| AppError::Internal(format!("users.updated_at parse: {e}")))?;

    Ok(User {
        id,
        username: str_field!("username").to_owned(),
        email: str_field!("email").to_owned(),
        password_hash: str_field!("password_hash").to_owned(),
        role,
        created_at,
        updated_at,
    })
}

// ── Trait implementation ──────────────────────────────────────────────────────

#[async_trait]
impl UserRepository for MongoUserRepository {
    /// Look up a user by their primary key (UUID).
    ///
    /// The `_id` field in MongoDB stores the UUID as a hyphenated string;
    /// this method converts `id` to a string before querying.
    ///
    /// Returns `Ok(None)` when no document with a matching `_id` exists.
    ///
    /// # Errors
    /// Returns [`AppError::Database`] on MongoDB driver errors, or
    /// [`AppError::Internal`] if the stored document cannot be deserialized
    /// (e.g. a required field is missing or has an unexpected type).
    async fn find_by_id(&self, id: Uuid) -> Result<Option<User>, AppError> {
        self.col
            .find_one(doc! { "_id": id.to_string() })
            .await
            .map_err(|e| AppError::Database(e.to_string()))?
            .map(doc_to_user)
            .transpose()
    }

    /// Look up a user by their e-mail address.
    ///
    /// Leverages the unique index on the `email` field for an efficient point
    /// lookup.
    ///
    /// Returns `Ok(None)` when no document with a matching `email` field
    /// exists.
    ///
    /// # Errors
    /// Returns [`AppError::Database`] on MongoDB driver errors, or
    /// [`AppError::Internal`] if the stored document cannot be deserialized.
    async fn find_by_email(&self, email: &str) -> Result<Option<User>, AppError> {
        self.col
            .find_one(doc! { "email": email })
            .await
            .map_err(|e| AppError::Database(e.to_string()))?
            .map(doc_to_user)
            .transpose()
    }

    /// Persist a new user document and return it unchanged.
    ///
    /// The document is built via [`user_to_doc`], which sets `_id` to the
    /// UUID string so MongoDB uses it as the primary key instead of
    /// generating an `ObjectId`.
    ///
    /// # Errors
    /// Returns [`AppError::Database`] on driver errors - most commonly a
    /// duplicate-key violation (error code 11000) when another user with the
    /// same `email` already exists, which is enforced by the unique index.
    async fn create(&self, user: User) -> Result<User, AppError> {
        self.col
            .insert_one(user_to_doc(&user))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(user)
    }
}

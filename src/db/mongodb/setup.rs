//! Index setup for the MongoDB adapter.
//!
//! Called once at startup by [`super::setup_mongodb`] before the database
//! handle is handed to the rest of the application.

#![cfg(feature = "mongodb")]

use mongodb::{Database, IndexModel, bson::doc, options::IndexOptions};

use crate::error::AppError;

/// Create all required indexes in the `windwatcher` database.
///
/// The function is idempotent: running it against a database that already has
/// the indexes is a no-op.
pub async fn setup_indexes(db: &Database) -> Result<(), AppError> {
    // ── users ──────────────────────────────────────────────────────────────────
    // Unique index on email so `find_by_email` is fast and duplicates are
    // rejected at the storage layer.
    db.collection::<mongodb::bson::Document>("users")
        .create_index(
            IndexModel::builder()
                .keys(doc! { "email": 1_i32 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await
        .map_err(|e| AppError::Database(format!("users.email index: {e}")))?;

    // ── messages ───────────────────────────────────────────────────────────────
    // Compound index that supports cursor-based pagination:
    //   filter on `room_id`, sort/seek on `id` (UUIDv7 - lexicographically
    //   sortable by creation time).
    db.collection::<mongodb::bson::Document>("messages")
        .create_index(
            IndexModel::builder()
                .keys(doc! { "room_id": 1_i32, "id": -1_i32 })
                .build(),
        )
        .await
        .map_err(|e| AppError::Database(format!("messages compound index: {e}")))?;

    // ── rooms ──────────────────────────────────────────────────────────────────
    // Unique sparse index on `direct_room_key` so that only one direct room can
    // exist for a given pair of users.  Sparse means group rooms (where the
    // field is absent/null) are not affected by the uniqueness constraint.
    db.collection::<mongodb::bson::Document>("rooms")
        .create_index(
            IndexModel::builder()
                .keys(doc! { "direct_room_key": 1_i32 })
                .options(IndexOptions::builder().unique(true).sparse(true).build())
                .build(),
        )
        .await
        .map_err(|e| AppError::Database(format!("rooms.direct_room_key index: {e}")))?;

    // ── room_users ─────────────────────────────────────────────────────────────
    // Unique compound index so a user can only appear once per room.
    // Also accelerates member look-ups and unread-count updates.
    db.collection::<mongodb::bson::Document>("room_users")
        .create_index(
            IndexModel::builder()
                .keys(doc! { "room_id": 1_i32, "user_id": 1_i32 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await
        .map_err(|e| AppError::Database(format!("room_users compound index: {e}")))?;

    tracing::info!("mongodb indexes ensured");
    Ok(())
}

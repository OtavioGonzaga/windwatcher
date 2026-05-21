//! MongoDB implementation of [`ChatRepository`].
//!
//! ## Document schemas
//!
//! ### `rooms`
//! | Field              | BSON type | Notes                                     |
//! | ------------------ | --------- | ----------------------------------------- |
//! | `_id`              | String    | UUID as hyphenated string (primary key)   |
//! | `room_type`        | String    | `"direct"` or `"group"`                   |
//! | `title`            | String?   | `null` for direct rooms                   |
//! | `direct_room_key`  | String?   | `"<uuid_a>:<uuid_b>"` sorted; null for groups |
//! | `created_at`       | String    | RFC 3339                                  |
//!
//! ### `messages`
//! | Field        | BSON type | Notes                                           |
//! | ------------ | --------- | ----------------------------------------------- |
//! | `_id`        | String    | Same as `id`; MongoDB primary key               |
//! | `id`         | String    | UUIDv7 string - used in compound index + cursor |
//! | `room_id`    | String    | UUID                                            |
//! | `sender_id`  | String    | UUID                                            |
//! | `content`    | String    |                                                 |
//! | `created_at` | String    | RFC 3339                                        |
//!
//! ### `room_users`
//! | Field                  | BSON type | Notes                       |
//! | ---------------------- | --------- | --------------------------- |
//! | `room_id`              | String    | UUID                        |
//! | `user_id`              | String    | UUID                        |
//! | `unread_count`         | Int64     |                             |
//! | `joined_at`            | String    | RFC 3339                    |
//! | `last_read_message_id` | String?   | UUID or null                |

#![cfg(feature = "mongodb")]

use async_trait::async_trait;
use chrono::Utc;
use futures::stream::TryStreamExt;
use mongodb::{
    Database,
    bson::{Bson, Document, doc},
};
use uuid::Uuid;

use crate::{
    domain::{
        models::{Message, Room, RoomType},
        ports::ChatRepository,
    },
    error::AppError,
};

// ── Public struct ─────────────────────────────────────────────────────────────

/// MongoDB-backed chat repository.
///
/// Obtain collection references on demand via `self.db.collection(...)`,
/// keeping the struct lightweight - a single cloneable [`Database`] handle.
pub struct MongoChatRepository {
    /// Handle to the `windwatcher` MongoDB database.
    ///
    /// Used to obtain typed collection references for `rooms`, `messages`,
    /// and `room_users` on each repository call.
    pub db: Database,
}

// ── Error helpers ─────────────────────────────────────────────────────────────

/// Return `true` when `e` is a MongoDB duplicate-key error (code 11000).
///
/// We use string matching rather than deep pattern matching on the private
/// error variants so that minor SDK changes do not break us.
fn is_duplicate_key(e: &mongodb::error::Error) -> bool {
    let msg = e.to_string();
    msg.contains("11000") || msg.contains("E11000") || msg.contains("duplicate key")
}

// ── Document converters ───────────────────────────────────────────────────────

/// Serialize a [`Room`] into a BSON document ready for insertion into the
/// `rooms` collection.
///
/// Optional fields (`title`, `direct_room_key`) are mapped to [`Bson::Null`]
/// when absent so that MongoDB stores an explicit null rather than omitting
/// the field entirely (which would break sparse-index semantics).
fn room_to_doc(r: &Room) -> Document {
    let title: Bson = r.title.as_deref().map(Into::into).unwrap_or(Bson::Null);
    let drk: Bson = r
        .direct_room_key
        .as_deref()
        .map(Into::into)
        .unwrap_or(Bson::Null);
    doc! {
        "_id":             r.id.to_string(),
        "room_type":       r.room_type.to_string(),
        "title":           title,
        "direct_room_key": drk,
        "created_at":      r.created_at.to_rfc3339(),
    }
}

/// Deserialize a BSON document retrieved from the `rooms` collection into a
/// domain [`Room`].
///
/// # Errors
/// Returns [`AppError::Internal`] if a required string field is missing or
/// if the `_id` / `room_type` values cannot be parsed into their respective
/// Rust types.
fn doc_to_room(d: Document) -> Result<Room, AppError> {
    macro_rules! str_field {
        ($key:literal) => {
            d.get_str($key)
                .map_err(|e| AppError::Internal(format!("rooms.{}: {e}", $key)))?
        };
    }

    let id = str_field!("_id")
        .parse::<Uuid>()
        .map_err(|e| AppError::Internal(format!("rooms._id parse: {e}")))?;

    let room_type = str_field!("room_type")
        .parse::<RoomType>()
        .map_err(|e| AppError::Internal(format!("rooms.room_type parse: {e}")))?;

    let title = match d.get("title") {
        Some(Bson::String(s)) => Some(s.clone()),
        _ => None,
    };

    let direct_room_key = match d.get("direct_room_key") {
        Some(Bson::String(s)) => Some(s.clone()),
        _ => None,
    };

    let created_at = chrono::DateTime::parse_from_rfc3339(str_field!("created_at"))
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| AppError::Internal(format!("rooms.created_at parse: {e}")))?;

    Ok(Room {
        id,
        room_type,
        title,
        direct_room_key,
        created_at,
    })
}

/// Serialize a [`Message`] into a BSON document ready for insertion into
/// the `messages` collection.
///
/// The `id` (UUIDv7) is stored in **two** fields:
/// - `_id` - MongoDB primary key, used for uniqueness and default `_id` index.
/// - `id`  - referenced by the compound index `{ room_id: 1, id: -1 }` that
///   powers cursor-based pagination in [`MongoChatRepository::list_messages`].
fn message_to_doc(m: &Message) -> Document {
    // `id` is stored both as `_id` (MongoDB primary key) and as the plain `id`
    // field used by the compound index `{ room_id: 1, id: -1 }`.
    doc! {
        "_id":        m.id.to_string(),
        "id":         m.id.to_string(),
        "room_id":    m.room_id.to_string(),
        "sender_id":  m.sender_id.to_string(),
        "content":    &m.content,
        "created_at": m.created_at.to_rfc3339(),
    }
}

/// Deserialize a BSON document retrieved from the `messages` collection into
/// a domain [`Message`].
///
/// Reads from the plain `id` field (not `_id`) so the same logic works
/// regardless of whether the document came from an index-covered projection.
///
/// # Errors
/// Returns [`AppError::Internal`] if a required field is absent, has an
/// unexpected BSON type, or cannot be parsed into its Rust type.
fn doc_to_message(d: Document) -> Result<Message, AppError> {
    macro_rules! str_field {
        ($key:literal) => {
            d.get_str($key)
                .map_err(|e| AppError::Internal(format!("messages.{}: {e}", $key)))?
        };
    }

    let id = str_field!("id")
        .parse::<Uuid>()
        .map_err(|e| AppError::Internal(format!("messages.id parse: {e}")))?;

    let room_id = str_field!("room_id")
        .parse::<Uuid>()
        .map_err(|e| AppError::Internal(format!("messages.room_id parse: {e}")))?;

    let sender_id = str_field!("sender_id")
        .parse::<Uuid>()
        .map_err(|e| AppError::Internal(format!("messages.sender_id parse: {e}")))?;

    let created_at = chrono::DateTime::parse_from_rfc3339(str_field!("created_at"))
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| AppError::Internal(format!("messages.created_at parse: {e}")))?;

    Ok(Message {
        id,
        room_id,
        sender_id,
        content: str_field!("content").to_owned(),
        created_at,
    })
}

// ── Trait implementation ──────────────────────────────────────────────────────

#[async_trait]
impl ChatRepository for MongoChatRepository {
    // ── Rooms ──────────────────────────────────────────────────────────────────

    /// Return the existing direct room for the pair (`user_a`, `user_b`), or
    /// atomically create one if it does not yet exist.
    ///
    /// ## Optimistic-concurrency strategy
    ///
    /// 1. **Fast path** - look up the room by `direct_room_key`; return it if
    ///    found.
    /// 2. **Slow path** - build a new [`Room`] document and attempt to insert
    ///    it.
    /// 3. If the insert returns **error code 11000** (duplicate key), a
    ///    concurrent request already created the room; fetch and return *that*
    ///    room instead.
    /// 4. On success, register both users in `room_users` (duplicate-key
    ///    errors there are silently ignored for the same concurrency reason).
    ///
    /// # Errors
    /// Returns [`AppError::Database`] on unrelated driver errors, or
    /// [`AppError::Internal`] if a document cannot be deserialized or if the
    /// room document disappears between the duplicate-key error and the
    /// subsequent fetch.
    async fn find_or_create_direct_room(
        &self,
        user_a: Uuid,
        user_b: Uuid,
    ) -> Result<Room, AppError> {
        let key = Room::direct_key(user_a, user_b);
        let rooms = self.db.collection::<Document>("rooms");
        let room_users = self.db.collection::<Document>("room_users");

        // Fast path: room already exists.
        if let Some(d) = rooms
            .find_one(doc! { "direct_room_key": &key })
            .await
            .map_err(|e| AppError::Database(e.to_string()))?
        {
            return doc_to_room(d);
        }

        // Slow path: create the room, then register both members.
        let now = Utc::now();
        let room = Room {
            id: Uuid::new_v4(),
            room_type: RoomType::Direct,
            title: None,
            direct_room_key: Some(key.clone()),
            created_at: now,
        };

        match rooms.insert_one(room_to_doc(&room)).await {
            Ok(_) => {}
            Err(e) if is_duplicate_key(&e) => {
                // Lost a race with another request - return the winner's room.
                let d = rooms
                    .find_one(doc! { "direct_room_key": &key })
                    .await
                    .map_err(|e2| AppError::Database(e2.to_string()))?
                    .ok_or_else(|| {
                        AppError::Internal("room disappeared after duplicate-key conflict".into())
                    })?;
                return doc_to_room(d);
            }
            Err(e) => return Err(AppError::Database(e.to_string())),
        }

        // Insert room_users for both participants (ignore duplicate-key errors
        // that could occur in a concurrent scenario).
        let now_str = now.to_rfc3339();
        for uid in [user_a, user_b] {
            let member_doc = doc! {
                "room_id":               room.id.to_string(),
                "user_id":               uid.to_string(),
                "unread_count":          0_i64,
                "joined_at":             &now_str,
                "last_read_message_id":  Bson::Null,
            };
            match room_users.insert_one(member_doc).await {
                Ok(_) => {}
                Err(e) if is_duplicate_key(&e) => {} // concurrent insert, fine
                Err(e) => return Err(AppError::Database(e.to_string())),
            }
        }

        Ok(room)
    }

    /// Create a new group room with the given `title` and register all
    /// `member_ids` as participants, each with an initial `unread_count` of 0.
    ///
    /// Unlike direct rooms, group rooms have no `direct_room_key` and are not
    /// subject to a uniqueness constraint, so no optimistic-concurrency
    /// handling is needed.
    ///
    /// # Errors
    /// Returns [`AppError::Database`] on driver errors during room or
    /// room-membership insertion.
    async fn create_group_room(
        &self,
        title: String,
        member_ids: Vec<Uuid>,
    ) -> Result<Room, AppError> {
        let rooms = self.db.collection::<Document>("rooms");
        let room_users = self.db.collection::<Document>("room_users");

        let now = Utc::now();
        let room = Room {
            id: Uuid::new_v4(),
            room_type: RoomType::Group,
            title: Some(title),
            direct_room_key: None,
            created_at: now,
        };

        rooms
            .insert_one(room_to_doc(&room))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let now_str = now.to_rfc3339();
        for uid in &member_ids {
            let member_doc = doc! {
                "room_id":               room.id.to_string(),
                "user_id":               uid.to_string(),
                "unread_count":          0_i64,
                "joined_at":             &now_str,
                "last_read_message_id":  Bson::Null,
            };
            room_users
                .insert_one(member_doc)
                .await
                .map_err(|e| AppError::Database(e.to_string()))?;
        }

        Ok(room)
    }

    // ── Messages ───────────────────────────────────────────────────────────────

    /// Persist a new chat message to the `messages` collection and return it
    /// unchanged.
    ///
    /// Returning the saved value avoids an extra database round-trip for
    /// callers (e.g. the WebSocket broadcast in [`process_chat_message`]).
    ///
    /// # Errors
    /// Returns [`AppError::Database`] on driver errors.
    ///
    /// [`process_chat_message`]: crate::jobs::chat_processor::process_chat_message
    async fn add_message(&self, message: Message) -> Result<Message, AppError> {
        self.db
            .collection::<Document>("messages")
            .insert_one(message_to_doc(&message))
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(message)
    }

    /// Returns messages in **reverse-chronological order** (newest first).
    ///
    /// When `before_id` is `Some(cursor)`, only messages whose `id` is
    /// lexicographically less than the cursor are returned.  Because the `id`
    /// field is a UUIDv7 (timestamp-prefixed), lexicographic < equals
    /// chronological <, so the cursor correctly selects "older" messages.
    async fn list_messages(
        &self,
        room_id: Uuid,
        before_id: Option<Uuid>,
        limit: u64,
    ) -> Result<Vec<Message>, AppError> {
        let col = self.db.collection::<Document>("messages");

        let mut filter = doc! { "room_id": room_id.to_string() };
        if let Some(bid) = before_id {
            filter.insert("id", doc! { "$lt": bid.to_string() });
        }

        let mut cursor = col
            .find(filter)
            .sort(doc! { "id": -1_i32 })
            .limit(limit as i64)
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut messages = Vec::new();
        while let Some(d) = cursor
            .try_next()
            .await
            .map_err(|e| AppError::Database(e.to_string()))?
        {
            messages.push(doc_to_message(d)?);
        }

        Ok(messages)
    }

    // ── Room membership ────────────────────────────────────────────────────────

    /// Return the UUIDs of every user that belongs to `room_id`.
    ///
    /// Scans the `room_users` collection with a filter on `room_id` (covered
    /// by the compound index `{ room_id: 1, user_id: 1 }`).
    ///
    /// # Errors
    /// Returns [`AppError::Database`] on driver errors, or
    /// [`AppError::Internal`] if a stored `user_id` value cannot be parsed
    /// as a UUID.
    async fn get_room_members(&self, room_id: Uuid) -> Result<Vec<Uuid>, AppError> {
        let col = self.db.collection::<Document>("room_users");

        let mut cursor = col
            .find(doc! { "room_id": room_id.to_string() })
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;

        let mut members = Vec::new();
        while let Some(d) = cursor
            .try_next()
            .await
            .map_err(|e| AppError::Database(e.to_string()))?
        {
            let uid = d
                .get_str("user_id")
                .map_err(|e| AppError::Internal(format!("room_users.user_id: {e}")))?
                .parse::<Uuid>()
                .map_err(|e| AppError::Internal(format!("room_users.user_id parse: {e}")))?;
            members.push(uid);
        }

        Ok(members)
    }

    /// Increment the `unread_count` field by 1 for every member of `room_id`
    /// **except** `exclude_user` (the message sender).
    ///
    /// Executes a single `update_many` with:
    /// - **filter**: `{ room_id: ..., user_id: { $ne: exclude_user } }`
    /// - **update**: `{ $inc: { unread_count: 1 } }`
    ///
    /// This approach avoids fetching member lists and performs the increment
    /// atomically on the server side.
    ///
    /// # Errors
    /// Returns [`AppError::Database`] on driver errors.
    async fn increment_unread(&self, room_id: Uuid, exclude_user: Uuid) -> Result<(), AppError> {
        self.db
            .collection::<Document>("room_users")
            .update_many(
                doc! {
                    "room_id": room_id.to_string(),
                    "user_id": { "$ne": exclude_user.to_string() },
                },
                doc! { "$inc": { "unread_count": 1_i64 } },
            )
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }

    /// Reset the unread counter and record the latest read position for
    /// `user_id` in `room_id`.
    ///
    /// Executes a single `update_one` with:
    /// - **filter**: `{ room_id: ..., user_id: ... }`
    /// - **update**: `{ $set: { unread_count: 0, last_read_message_id: message_id } }`
    ///
    /// # Errors
    /// Returns [`AppError::Database`] on driver errors.
    async fn mark_as_read(
        &self,
        room_id: Uuid,
        user_id: Uuid,
        message_id: Uuid,
    ) -> Result<(), AppError> {
        self.db
            .collection::<Document>("room_users")
            .update_one(
                doc! {
                    "room_id": room_id.to_string(),
                    "user_id": user_id.to_string(),
                },
                doc! {
                    "$set": {
                        "unread_count":         0_i64,
                        "last_read_message_id": message_id.to_string(),
                    }
                },
            )
            .await
            .map_err(|e| AppError::Database(e.to_string()))?;
        Ok(())
    }
}

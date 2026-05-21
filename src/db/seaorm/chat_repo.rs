//! SeaORM implementation of [`ChatRepository`].

use std::str::FromStr;

use async_trait::async_trait;
use chrono::Utc;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
};
use uuid::Uuid;

use crate::{
    domain::{
        models::{Message, Room, RoomType},
        ports::ChatRepository,
    },
    error::AppError,
};

use super::entities::{message, room, room_user};

// ── Public struct ─────────────────────────────────────────────────────────────

/// SeaORM-backed implementation of [`ChatRepository`].
///
/// Handles all room and message persistence for the chat subsystem using any
/// SQL database supported by SeaORM (SQLite, PostgreSQL, MySQL).
///
/// Construct this type directly and wrap it in an `Arc` (or store it inside
/// [`crate::state::AppState`]) before passing it to the service layer.
pub struct SeaOrmChatRepository {
    /// Shared SeaORM connection pool, injected at construction time.
    ///
    /// [`DatabaseConnection`] is `Clone + Send + Sync`, so it can be shared
    /// freely across threads without additional locking.
    pub db: DatabaseConnection,
}

// ── Conversion helpers ────────────────────────────────────────────────────────────

/// Convert a [`room::Model`] row to the domain [`Room`] type.
///
/// Parses `room_type` via [`std::str::FromStr`] and re-zones `created_at`
/// from `DateTimeWithTimeZone` to `DateTime<Utc>`.
fn room_model_to_domain(m: room::Model) -> Result<Room, AppError> {
    let room_type = RoomType::from_str(&m.room_type)
        .map_err(|e| AppError::Internal(format!("invalid room_type stored in database: {e}")))?;

    Ok(Room {
        id: m.id,
        room_type,
        title: m.title,
        direct_room_key: m.direct_room_key,
        created_at: m.created_at.with_timezone(&Utc),
    })
}

/// Convert a [`message::Model`] row to the domain [`Message`] type.
///
/// Re-zones `created_at` from `DateTimeWithTimeZone` to `DateTime<Utc>`.
/// This conversion is infallible because all fields map directly.
fn message_model_to_domain(m: message::Model) -> Message {
    Message {
        id: m.id,
        room_id: m.room_id,
        sender_id: m.sender_id,
        content: m.content,
        created_at: m.created_at.with_timezone(&Utc),
    }
}

// ── Trait implementation ──────────────────────────────────────────────────────

#[async_trait]
impl ChatRepository for SeaOrmChatRepository {
    // ── Rooms ─────────────────────────────────────────────────────────────────

    /// Find an existing direct (1-to-1) room for the two users, or create one.
    ///
    /// A **direct room key** is computed via [`Room::direct_key`] by sorting
    /// both UUIDs and joining them with `:`.  This deterministic key guarantees
    /// that the pair `(user_a, user_b)` and `(user_b, user_a)` always resolve
    /// to the same room.
    ///
    /// The entire check-then-insert sequence runs inside a **database
    /// transaction** to prevent duplicate rooms under concurrent requests.
    /// Both users are added as members (with `unread_count = 0`) in the same
    /// transaction.
    ///
    /// # Errors
    ///
    /// * [`AppError::Database`] - if any SQL statement or the transaction
    ///   commit fails.
    /// * [`AppError::Internal`] - if the `room_type` column of an existing
    ///   row contains an unrecognised value.
    async fn find_or_create_direct_room(
        &self,
        user_a: Uuid,
        user_b: Uuid,
    ) -> Result<Room, AppError> {
        let key = Room::direct_key(user_a, user_b);

        // Open a transaction so the check-then-insert is atomic.
        let txn = self.db.begin().await?;

        // Fast path: room already exists.
        if let Some(existing) = room::Entity::find()
            .filter(room::Column::DirectRoomKey.eq(&key))
            .one(&txn)
            .await?
        {
            txn.commit().await?;
            return room_model_to_domain(existing);
        }

        let now = Utc::now().fixed_offset();
        let room_id = Uuid::now_v7();

        // Create the room.
        let new_room = room::ActiveModel {
            id: Set(room_id),
            room_type: Set("direct".to_owned()),
            title: Set(None),
            direct_room_key: Set(Some(key)),
            created_at: Set(now),
        };
        let room_model = new_room.insert(&txn).await?;

        // Register both participants.
        let ru_a = room_user::ActiveModel {
            room_id: Set(room_id),
            user_id: Set(user_a),
            unread_count: Set(0),
            joined_at: Set(now),
            last_read_message_id: Set(None),
        };
        let ru_b = room_user::ActiveModel {
            room_id: Set(room_id),
            user_id: Set(user_b),
            unread_count: Set(0),
            joined_at: Set(now),
            last_read_message_id: Set(None),
        };
        room_user::Entity::insert_many([ru_a, ru_b])
            .exec(&txn)
            .await?;

        txn.commit().await?;
        room_model_to_domain(room_model)
    }

    /// Create a new group room with the given `title` and initial `member_ids`.
    ///
    /// All provided members are inserted into `room_users` within the same
    /// transaction as the room creation, ensuring the room is never visible
    /// without its members.
    ///
    /// If `member_ids` is empty no `room_users` rows are inserted (the room
    /// will have zero members).
    ///
    /// # Errors
    ///
    /// * [`AppError::Database`] - if any SQL statement or the transaction
    ///   commit fails.
    /// * [`AppError::Internal`] - if the returned row cannot be converted
    ///   back to the domain type.
    async fn create_group_room(
        &self,
        title: String,
        member_ids: Vec<Uuid>,
    ) -> Result<Room, AppError> {
        let txn = self.db.begin().await?;
        let now = Utc::now().fixed_offset();
        let room_id = Uuid::now_v7();

        let new_room = room::ActiveModel {
            id: Set(room_id),
            room_type: Set("group".to_owned()),
            title: Set(Some(title)),
            direct_room_key: Set(None),
            created_at: Set(now),
        };
        let room_model = new_room.insert(&txn).await?;

        let members: Vec<room_user::ActiveModel> = member_ids
            .into_iter()
            .map(|uid| room_user::ActiveModel {
                room_id: Set(room_id),
                user_id: Set(uid),
                unread_count: Set(0),
                joined_at: Set(now),
                last_read_message_id: Set(None),
            })
            .collect();

        if !members.is_empty() {
            room_user::Entity::insert_many(members).exec(&txn).await?;
        }

        txn.commit().await?;
        room_model_to_domain(room_model)
    }

    // ── Messages ──────────────────────────────────────────────────────────────

    /// Persist a chat message and return the inserted domain model.
    ///
    /// All fields of `msg` (ID, room_id, sender_id, content, created_at) must
    /// be populated by the caller.  The `created_at` timestamp is stored as
    /// `DateTimeWithTimeZone` via `fixed_offset()`.
    ///
    /// # Errors
    ///
    /// * [`AppError::Database`] - if the `INSERT` fails (e.g. unknown
    ///   `room_id` or `sender_id` FK violation).
    async fn add_message(&self, msg: Message) -> Result<Message, AppError> {
        let active = message::ActiveModel {
            id: Set(msg.id),
            room_id: Set(msg.room_id),
            sender_id: Set(msg.sender_id),
            content: Set(msg.content),
            created_at: Set(msg.created_at.fixed_offset()),
        };
        let inserted = active.insert(&self.db).await?;
        Ok(message_model_to_domain(inserted))
    }

    /// List messages in a room with keyset (cursor) pagination.
    ///
    /// Messages are returned in **reverse-chronological order** (`ORDER BY id
    /// DESC`).  Because message IDs are UUIDv7 values they are monotonically
    /// increasing, so `id < before_id` is a semantically correct time-based
    /// cursor: "give me messages older than `before_id`".
    ///
    /// * If `before_id` is `None` the most recent `limit` messages are
    ///   returned.
    /// * If `before_id` is `Some(cursor)` only messages with `id < cursor`
    ///   are considered.
    ///
    /// # Errors
    ///
    /// * [`AppError::Database`] - if the SQL query fails.
    async fn list_messages(
        &self,
        room_id: Uuid,
        before_id: Option<Uuid>,
        limit: u64,
    ) -> Result<Vec<Message>, AppError> {
        // UUIDv7 is monotonically increasing, so `id < before_id` is a valid
        // time-based cursor for reverse-chronological pagination.
        let mut query = message::Entity::find()
            .filter(message::Column::RoomId.eq(room_id))
            .order_by_desc(message::Column::Id);

        if let Some(cursor) = before_id {
            query = query.filter(message::Column::Id.lt(cursor));
        }

        let models = query.limit(limit).all(&self.db).await?;
        Ok(models.into_iter().map(message_model_to_domain).collect())
    }

    // ── Membership ────────────────────────────────────────────────────────────

    /// Return the list of user IDs that are members of the given room.
    ///
    /// # Errors
    ///
    /// * [`AppError::Database`] - if the SQL query fails.
    async fn get_room_members(&self, room_id: Uuid) -> Result<Vec<Uuid>, AppError> {
        let rows = room_user::Entity::find()
            .filter(room_user::Column::RoomId.eq(room_id))
            .all(&self.db)
            .await?;
        Ok(rows.into_iter().map(|r| r.user_id).collect())
    }

    /// Atomically increment the `unread_count` for all room members except the sender.
    ///
    /// Issues a single `UPDATE` statement using a column expression so that no
    /// rows need to be loaded into memory:
    ///
    /// ```sql
    /// UPDATE room_users
    /// SET    unread_count = unread_count + 1
    /// WHERE  room_id = ? AND user_id != ?
    /// ```
    ///
    /// # Errors
    ///
    /// * [`AppError::Database`] - if the SQL statement fails.
    async fn increment_unread(&self, room_id: Uuid, exclude_user: Uuid) -> Result<(), AppError> {
        // Generates: UPDATE room_users SET unread_count = unread_count + 1
        //            WHERE room_id = ? AND user_id != ?
        room_user::Entity::update_many()
            .col_expr(
                room_user::Column::UnreadCount,
                Expr::col(room_user::Column::UnreadCount).add(1i64),
            )
            .filter(room_user::Column::RoomId.eq(room_id))
            .filter(room_user::Column::UserId.ne(exclude_user))
            .exec(&self.db)
            .await?;
        Ok(())
    }

    /// Mark all messages up to `message_id` as read for the given user.
    ///
    /// Loads the `room_users` row identified by the composite PK
    /// `(room_id, user_id)`, then performs a targeted `UPDATE` via
    /// `ActiveModel` so SeaORM generates the correct composite-PK `WHERE`
    /// clause automatically.  Sets `unread_count = 0` and
    /// `last_read_message_id = message_id`.
    ///
    /// If no membership row is found (the user is not a member of the room)
    /// the method returns `Ok(())` without error.
    ///
    /// # Errors
    ///
    /// * [`AppError::Database`] - if the `SELECT` or `UPDATE` SQL statement
    ///   fails.
    async fn mark_as_read(
        &self,
        room_id: Uuid,
        user_id: Uuid,
        message_id: Uuid,
    ) -> Result<(), AppError> {
        // Load the single row, then perform a targeted UPDATE via ActiveModel so
        // SeaORM generates the correct composite-PK WHERE clause automatically.
        let model = room_user::Entity::find()
            .filter(room_user::Column::RoomId.eq(room_id))
            .filter(room_user::Column::UserId.eq(user_id))
            .one(&self.db)
            .await?;

        if let Some(m) = model {
            let mut active: room_user::ActiveModel = m.into();
            active.unread_count = Set(0);
            active.last_read_message_id = Set(Some(message_id));
            active.update(&self.db).await?;
        }
        Ok(())
    }
}

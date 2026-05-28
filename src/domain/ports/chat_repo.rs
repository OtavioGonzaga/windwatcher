use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    domain::models::{Message, Room},
    error::AppError,
};

/// Storage port for chat rooms, memberships, and messages.
///
/// Concrete SQL and MongoDB implementations live in `db/seaorm/chat_repo.rs`
/// and `db/mongodb/chat_repo.rs`.  The background job worker calls the
/// write methods; HTTP handlers call the read methods.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ChatRepository: Send + Sync {
    /// Find the existing direct room for the two users, or atomically create
    /// one if it does not yet exist.
    ///
    /// The lookup is performed on the deterministic
    /// [`Room::direct_room_key`][crate::domain::models::Room::direct_room_key]
    /// (`sorted_id_a:sorted_id_b`), so calling this method with the same pair
    /// of users in any order always returns the same room.
    ///
    /// When a new room is created both `user_a` and `user_b` are automatically
    /// added as members.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Database`] if the storage operation fails.
    async fn find_or_create_direct_room(
        &self,
        user_a: Uuid,
        user_b: Uuid,
    ) -> Result<Room, AppError>;

    /// Create a new group room with the given title and add all `member_ids`
    /// as initial members.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Database`] if the storage operation fails.
    async fn create_group_room(
        &self,
        title: String,
        member_ids: Vec<Uuid>,
    ) -> Result<Room, AppError>;

    /// Persist a new message to storage.
    ///
    /// This method is called exclusively by the background job worker after
    /// the message has been dequeued from the [`JobQueue`][crate::domain::ports::JobQueue].
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Database`] if the storage operation fails.
    async fn add_message(&self, message: Message) -> Result<Message, AppError>;

    /// Retrieve a page of messages from a room in **reverse-chronological**
    /// order (newest first).
    ///
    /// ## Cursor-based pagination
    ///
    /// `before_id` is an exclusive cursor: when supplied, only messages whose
    /// UUIDv7 is strictly less than `before_id` are returned.  Because UUIDv7
    /// is time-ordered, this effectively returns messages sent *before* the
    /// message with that ID.  Pass `None` to fetch the most-recent page.
    ///
    /// The `limit` parameter caps how many messages are returned per call.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Database`] if the storage query fails.
    async fn list_messages(
        &self,
        room_id: Uuid,
        before_id: Option<Uuid>,
        limit: u64,
    ) -> Result<Vec<Message>, AppError>;

    /// Return the IDs of every member in the given room.
    ///
    /// Used by the background worker to determine which users should receive
    /// a WebSocket notification when a new message arrives.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Database`] if the storage query fails.
    async fn get_room_members(&self, room_id: Uuid) -> Result<Vec<Uuid>, AppError>;

    /// Atomically increment `unread_count` for all members of `room_id`
    /// except `exclude_user` (the sender).
    ///
    /// Called by the background worker immediately after [`add_message`][Self::add_message].
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Database`] if the storage operation fails.
    async fn increment_unread(&self, room_id: Uuid, exclude_user: Uuid) -> Result<(), AppError>;

    /// Reset the unread counter and advance the read cursor for a member.
    ///
    /// Sets `unread_count = 0` and `last_read_message_id = message_id` for
    /// the `(room_id, user_id)` membership record.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Database`] if the storage operation fails.
    async fn mark_as_read(
        &self,
        room_id: Uuid,
        user_id: Uuid,
        message_id: Uuid,
    ) -> Result<(), AppError>;
}

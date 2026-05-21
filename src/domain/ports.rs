//! Port traits (interfaces) for the Hexagonal Architecture.
//!
//! In Ports & Adapters terminology, **ports** are the abstract boundaries
//! between the application core and the outside world.  This module defines
//! all port traits that the `application/` layer depends on; concrete
//! implementations live in `db/seaorm/`, `db/mongodb/`, and `jobs/`.
//!
//! ## Ports defined here
//!
//! | Trait               | Adapters                                          |
//! | ------------------- | ------------------------------------------------- |
//! | [`UserRepository`]  | `SeaOrmUserRepository`, `MongoUserRepository`     |
//! | [`ChatRepository`]  | `SeaOrmChatRepository`, `MongoChatRepository`     |
//! | [`JobQueue`]        | `InMemoryJobQueue`                                |
//!
//! All traits are `Send + Sync` and object-safe so they can be boxed or
//! wrapped in [`std::sync::Arc`] and injected through [`crate::state::AppState`].
//!
//! In test builds, mockall generates `MockUserRepository`, `MockChatRepository`,
//! and `MockJobQueue` automatically via the `#[cfg_attr(test, mockall::automock)]`
//! attribute.

use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    domain::models::{Message, Room, User},
    error::AppError,
};

// ── User Repository ─────────────────────────────────────────────────────────────────

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

// ── Chat Repository ──────────────────────────────────────────────────────────────────

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
    /// the message has been dequeued from the [`JobQueue`].
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

// ── Job Queue Port ────────────────────────────────────────────────────────────────────

/// Port for submitting background work items.
///
/// The current implementation is [`InMemoryJobQueue`][crate::jobs::chat_processor::InMemoryJobQueue],
/// which uses a Tokio `mpsc` channel and a single spawned worker task.  For
/// production durability (survive process restarts) this should be replaced
/// with an [Apalis](https://github.com/geofmureithi/apalis)-backed
/// implementation that persists jobs to a database.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait JobQueue: Send + Sync {
    /// Submit a chat-message job to the queue for asynchronous processing.
    ///
    /// The worker will dequeue the job, persist the message via
    /// [`ChatRepository::add_message`], update unread counters, and broadcast
    /// the message to online WebSocket clients.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::Internal`] if the underlying channel is closed or
    /// the send fails (e.g. the worker task has panicked).
    async fn enqueue_chat_message(&self, job: ChatMessageJob) -> Result<(), AppError>;
}

/// Payload carried by a chat-message background job.
///
/// All fields needed to persist a message and deliver it to connected clients
/// are bundled here so the worker is self-contained and does not need to
/// re-query the database for basic message metadata.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ChatMessageJob {
    /// Pre-generated UUIDv7 for the message, created by the HTTP handler
    /// *before* enqueueing so the 202 response can include it.
    pub message_id: Uuid,

    /// The room the message was sent to.
    pub room_id: Uuid,

    /// The user who sent the message.
    pub sender_id: Uuid,

    /// Plain-text message body.
    pub content: String,
}

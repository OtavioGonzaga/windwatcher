use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single chat message sent inside a [`crate::domain::models::Room`].
///
/// Because [`id`][Message::id] is a **UUIDv7** (time-ordered), it encodes the
/// insertion timestamp and can be used directly as a pagination cursor: to
/// retrieve the page before a known message, pass its ID to the `before_id`
/// parameter of [`ChatRepository::list_messages`][crate::domain::ports::ChatRepository::list_messages].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Unique message identifier (UUIDv7).
    ///
    /// The time-ordered nature of UUIDv7 makes this field double as a
    /// pagination cursor: messages with a lower `id` were sent earlier.
    pub id: Uuid,
    /// The room this message belongs to.
    pub room_id: Uuid,
    /// The user who sent the message.
    pub sender_id: Uuid,
    /// Plain-text message body.
    pub content: String,
    /// Timestamp when the message was persisted (UTC).
    pub created_at: DateTime<Utc>,
}

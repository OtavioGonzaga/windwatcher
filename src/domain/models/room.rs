use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The topology of a chat room.
///
/// Serialises as lowercase (`"direct"`, `"group"`) in JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomType {
    /// A private 1:1 conversation between exactly two users.
    ///
    /// Identified by a deterministic [`Room::direct_room_key`] so that the
    /// same two users are always routed to the same room.
    Direct,
    /// A named room with any number of members.
    Group,
}

impl std::fmt::Display for RoomType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RoomType::Direct => write!(f, "direct"),
            RoomType::Group => write!(f, "group"),
        }
    }
}

impl std::str::FromStr for RoomType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "direct" => Ok(RoomType::Direct),
            "group" => Ok(RoomType::Group),
            other => Err(format!("unknown room type: {other}")),
        }
    }
}

/// A chat room, either a direct (1:1) conversation or a named group.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    /// Unique room identifier (UUIDv7).
    pub id: Uuid,
    /// Whether this is a direct or group room.
    pub room_type: RoomType,
    /// Human-readable name.  Present only for [`RoomType::Group`] rooms;
    /// `None` for direct rooms.
    pub title: Option<String>,
    /// Deterministic deduplication key for [`RoomType::Direct`] rooms,
    /// computed as the two participant UUIDs sorted lexicographically and
    /// joined with `:`.
    ///
    /// This key ensures that at most one direct room exists between any pair
    /// of users and enables O(1) lookup without scanning all rooms.  `None`
    /// for group rooms.
    ///
    /// See [`Room::direct_key`] for the construction algorithm.
    pub direct_room_key: Option<String>,
    /// Timestamp when the room was created (UTC).
    pub created_at: DateTime<Utc>,
}

impl Room {
    /// Build the canonical deduplication key for a direct room.
    ///
    /// The algorithm is:
    /// 1. Convert each [`Uuid`] to its lowercase hyphenated string form.
    /// 2. Sort the two strings lexicographically (ensuring symmetry).
    /// 3. Join them with a `:` separator.
    ///
    /// The result is **symmetric**: `direct_key(a, b) == direct_key(b, a)`,
    /// which guarantees that calling `find_or_create_direct_room(a, b)` and
    /// `find_or_create_direct_room(b, a)` always resolve to the same room.
    ///
    /// The generated string is stored in [`Room::direct_room_key`] and indexed
    /// in the database for fast lookup.
    pub fn direct_key(a: Uuid, b: Uuid) -> String {
        let mut ids = [a.to_string(), b.to_string()];
        ids.sort();
        ids.join(":")
    }
}

/// Membership record that links a [`crate::domain::models::User`] to a [`Room`].
///
/// Tracks per-user read state so that the API can report how many unread
/// messages each member has and which message they last read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomUser {
    /// The room this membership belongs to.
    pub room_id: Uuid,
    /// The user who is a member of the room.
    pub user_id: Uuid,
    /// Number of messages received in this room since the user last called
    /// the mark-as-read endpoint.  Incremented by the background worker;
    /// reset to `0` by [`ChatRepository::mark_as_read`][crate::domain::ports::ChatRepository::mark_as_read].
    pub unread_count: i64,
    /// Timestamp when the user joined or was added to the room (UTC).
    pub joined_at: DateTime<Utc>,
    /// ID of the most recent message the user has explicitly marked as read.
    /// `None` until the user calls the mark-as-read endpoint at least once.
    pub last_read_message_id: Option<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn direct_key_is_symmetric() {
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        assert_eq!(Room::direct_key(a, b), Room::direct_key(b, a));
    }

    #[test]
    fn direct_key_contains_both_ids() {
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        let key = Room::direct_key(a, b);
        assert!(key.contains(&a.to_string()));
        assert!(key.contains(&b.to_string()));
    }

    #[test]
    fn direct_key_uses_separator() {
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        let key = Room::direct_key(a, b);
        assert!(key.contains(':'));
    }

    #[test]
    fn room_type_roundtrip() {
        assert_eq!(RoomType::Direct.to_string(), "direct");
        assert_eq!("direct".parse::<RoomType>().unwrap(), RoomType::Direct);
        assert_eq!(RoomType::Group.to_string(), "group");
        assert_eq!("group".parse::<RoomType>().unwrap(), RoomType::Group);
    }

    #[test]
    fn room_type_unknown_fails() {
        assert!("xyz".parse::<RoomType>().is_err());
    }
}

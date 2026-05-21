//! In-memory WebSocket connection manager.
//!
//! [`WsManager`] maintains a live registry of every authenticated WebSocket
//! connection.  It is the single point through which the rest of the
//! application (primarily the background job worker) pushes real-time events
//! to connected clients.
//!
//! ## Data model
//!
//! ```text
//! DashMap<UserId, mpsc::Sender<SocketMessage>>
//!        │
//!        └── one entry per online user
//!               │
//!               └── Sender <-──(async channel)──► Receiver (owned by ws handler task)
//! ```
//!
//! ## Concurrency
//!
//! [`DashMap`] provides fine-grained locking, so concurrent inserts, removals,
//! and reads from multiple Tokio tasks are safe without additional
//! synchronisation.
//!
//! ## Message delivery guarantee
//!
//! Delivery is **best-effort**: if a user is offline or their channel buffer is
//! full, the message is silently dropped.  Clients that reconnect must fetch
//! missed messages via `GET /rooms/:id/messages`.

use dashmap::DashMap;
use serde::Serialize;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::domain::models::Message;

// ── Messages carried over the WebSocket ───────────────────────────────────────

/// Messages that the server can push to a connected WebSocket client.
///
/// Serialised as a JSON object with a `"type"` discriminant field
/// (`#[serde(tag = "type", rename_all = "snake_case")]`).
///
/// ## Wire format example
///
/// ```json
/// {
///   "type": "new_message",
///   "id": "<uuid>",
///   "room_id": "<uuid>",
///   "sender_id": "<uuid>",
///   "content": "Hello!",
///   "created_at": "2024-01-01T00:00:00Z"
/// }
/// ```
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SocketMessage {
    /// A new chat message has been written to a room the client belongs to.
    NewMessage(Message),
}

// ── Manager ────────────────────────────────────────────────────────────────────

/// In-memory registry of active WebSocket connections.
///
/// The registry maps each connected user's [`Uuid`] to the [`mpsc::Sender`]
/// half of a bounded channel (capacity 64).  The matching [`mpsc::Receiver`]
/// is handed to the WebSocket handler task via [`WsManager::connect`] and
/// drives the per-socket write loop.
///
/// `WsManager` is wrapped in [`Arc`](std::sync::Arc) inside [`AppState`] so
/// that it is shared across all Axum handler tasks without cloning.
///
/// [`AppState`]: crate::state::AppState
pub struct WsManager {
    connections: DashMap<Uuid, mpsc::Sender<SocketMessage>>,
}

impl WsManager {
    /// Create a new, empty manager with no active connections.
    pub fn new() -> Self {
        Self {
            connections: DashMap::new(),
        }
    }

    /// Register a new WebSocket connection for `user_id` and return the
    /// receive end of the server-to-client message channel.
    ///
    /// If the same user opens a second connection (e.g. multiple browser tabs),
    /// the previous sender is replaced, effectively hijacking delivery to the
    /// newer connection.
    ///
    /// The returned [`mpsc::Receiver`] must be polled by the WebSocket handler
    /// task.  Messages sent via [`send_to_users`](Self::send_to_users) will
    /// appear on this receiver.
    pub fn connect(&self, user_id: Uuid) -> mpsc::Receiver<SocketMessage> {
        let (tx, rx) = mpsc::channel(64);
        self.connections.insert(user_id, tx);
        rx
    }

    /// Deregister the connection for `user_id`.
    ///
    /// Should be called by the WebSocket handler task when the socket is
    /// closed (either by the client or due to a network error).
    pub fn disconnect(&self, user_id: Uuid) {
        self.connections.remove(&user_id);
    }

    /// Return the number of users that are currently connected.
    pub fn online_count(&self) -> usize {
        self.connections.len()
    }

    /// Push `msg` to each user in `user_ids` that has an active connection.
    ///
    /// Users that are offline (not present in the registry) are silently
    /// skipped.  Send errors - e.g. a full channel buffer or a closing socket
    /// - are also silently ignored, making this a **best-effort** broadcast.
    pub async fn send_to_users(&self, user_ids: &[Uuid], msg: SocketMessage) {
        for user_id in user_ids {
            if let Some(tx) = self.connections.get(user_id) {
                // Best-effort: ignore send errors (connection may be closing).
                let _ = tx.send(msg.clone()).await;
            }
        }
    }
}

impl Default for WsManager {
    fn default() -> Self {
        Self::new()
    }
}

// ── Unit tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use crate::domain::models::Message;

    use super::{SocketMessage, WsManager};

    fn make_message() -> Message {
        Message {
            id: Uuid::now_v7(),
            room_id: Uuid::now_v7(),
            sender_id: Uuid::now_v7(),
            content: "hello".into(),
            created_at: Utc::now(),
        }
    }

    // Test 1: connect registers user, disconnect removes them
    #[tokio::test]
    async fn connect_and_disconnect() {
        let mgr = WsManager::new();
        let id = Uuid::now_v7();

        let _rx = mgr.connect(id);
        assert_eq!(
            mgr.online_count(),
            1,
            "user should be registered after connect"
        );

        mgr.disconnect(id);
        assert_eq!(
            mgr.online_count(),
            0,
            "user should be removed after disconnect"
        );
    }

    // Test 2: online_count reflects the exact number of active connections
    #[tokio::test]
    async fn online_count_tracks_connections() {
        let mgr = WsManager::new();
        assert_eq!(mgr.online_count(), 0);

        let id1 = Uuid::now_v7();
        let id2 = Uuid::now_v7();

        let _rx1 = mgr.connect(id1);
        assert_eq!(mgr.online_count(), 1);

        let _rx2 = mgr.connect(id2);
        assert_eq!(mgr.online_count(), 2);

        mgr.disconnect(id1);
        assert_eq!(mgr.online_count(), 1);

        mgr.disconnect(id2);
        assert_eq!(mgr.online_count(), 0);
    }

    // Test 3: message sent to connected user is received on their channel
    #[tokio::test]
    async fn send_to_connected_user_is_received() {
        let mgr = WsManager::new();
        let id = Uuid::now_v7();
        let mut rx = mgr.connect(id);

        mgr.send_to_users(&[id], SocketMessage::NewMessage(make_message()))
            .await;

        // send_to_users awaits the send, so the message is already buffered.
        let received = rx.try_recv();
        assert!(
            received.is_ok(),
            "connected user should receive the message"
        );
    }

    // Test 4: sending to a user who is not connected must not panic
    #[tokio::test]
    async fn send_to_offline_user_is_noop() {
        let mgr = WsManager::new();
        let id = Uuid::now_v7(); // never connected
        // If this call panics the test fails; no assertion needed beyond that.
        mgr.send_to_users(&[id], SocketMessage::NewMessage(make_message()))
            .await;
    }

    // Test 5: when sending to multiple users, only the connected one receives
    #[tokio::test]
    async fn send_to_multiple_users_partial() {
        let mgr = WsManager::new();
        let online_id = Uuid::now_v7();
        let offline_id = Uuid::now_v7();
        let mut rx = mgr.connect(online_id);
        // offline_id is intentionally never connected.

        mgr.send_to_users(
            &[online_id, offline_id],
            SocketMessage::NewMessage(make_message()),
        )
        .await;

        assert!(
            rx.try_recv().is_ok(),
            "online user should receive the message"
        );
        // No entry for offline_id - no panic is the only invariant here.
    }

    // Test 6: SocketMessage serialises with the correct "type" discriminant tag
    #[test]
    fn socket_message_serializes_correctly() {
        let msg = SocketMessage::NewMessage(make_message());
        let json = serde_json::to_value(&msg).unwrap();

        assert_eq!(
            json["type"], "new_message",
            "serde tag must be \"new_message\""
        );
        // The Message fields must be present at the top level (flattened by #[serde(tag)]).
        assert!(
            json.get("content").is_some(),
            "serialised value must include the 'content' field from Message"
        );
    }
}

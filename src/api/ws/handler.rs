//! WebSocket upgrade handler.
//!
//! Exposes a single endpoint:
//!
//! | Method | Path  | Auth                         | Description          |
//! | ------ | ----- | ---------------------------- | -------------------- |
//! | `GET`  | `/ws` | `?token=<jwt>` (query param) | Upgrade to WebSocket |
//!
//! ## Why a query parameter instead of a header?
//!
//! The browser-native [`WebSocket`](https://developer.mozilla.org/en-US/docs/Web/API/WebSocket)
//! constructor does not allow setting custom HTTP headers at upgrade time.
//! Passing the JWT as a URL query parameter (`?token=<jwt>`) is the standard
//! workaround for browser-based WebSocket clients.
//!
//! ## Socket I/O loop
//!
//! After a successful upgrade the private `handle_socket` function drives a
//! `tokio::select!` loop that concurrently waits on two branches:
//!
//! * **Outgoing** (`rx.recv()`) - messages pushed by [`WsManager::send_to_users`](crate::api::ws::manager::WsManager::send_to_users)
//!   are serialised to JSON and forwarded to the client as `Text` frames.
//! * **Incoming** (`ws_receiver.next()`) - handles `Ping` -> `Pong` reflexes
//!   and detects `Close` frames or errors to terminate the loop cleanly.

use axum::extract::ws::{Message as WsMessage, WebSocket};
use axum::{
    extract::{Query, State, WebSocketUpgrade},
    response::IntoResponse,
};
use futures::{sink::SinkExt, stream::StreamExt};
use serde::Deserialize;
use uuid::Uuid;

use crate::state::AppState;

// ── Query params ──────────────────────────────────────────────────────────

/// Query parameters for the WebSocket upgrade endpoint.
#[derive(Debug, Deserialize)]
pub struct WsParams {
    /// Signed JWT that identifies the connecting user.
    ///
    /// The token is validated **before** the HTTP connection is upgraded to a
    /// WebSocket.  If validation fails the connection is immediately closed.
    pub token: String,
}

// ── Upgrade handler ────────────────────────────────────────────────────────────

/// `GET /ws?token=<jwt>` - authenticate and upgrade to a WebSocket connection.
///
/// # Authentication
///
/// The JWT is taken from the `token` query parameter.  Validation happens
/// synchronously inside the HTTP handler (before the upgrade handshake)
/// so that invalid tokens receive a standard HTTP error response rather
/// than a WebSocket close frame:
///
/// * Valid token -> `101 Switching Protocols`, connection enters the I/O loop.
/// * Invalid / expired token -> the upgrade still proceeds at the protocol
///   level (Axum requires this), but the socket is closed immediately with
///   no data sent.
///
/// # After upgrade
///
/// The private `handle_socket` coroutine takes over.  It registers the
/// connection with [`WsManager`](crate::api::ws::manager::WsManager) and
/// drives the bidirectional I/O loop until the client disconnects.
///
/// # Responses
///
/// | Status                    | Description                                  |
/// | ------------------------- | -------------------------------------------- |
/// | `101 Switching Protocols` | Upgrade successful; real-time session begins |
/// | `400 Bad Request`         | `token` query parameter absent               |
pub async fn ws_handler(
    State(state): State<AppState>,
    Query(params): Query<WsParams>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    match state.user_service.decode_token(&params.token) {
        Ok(claims) => {
            let user_id = Uuid::parse_str(&claims.sub).unwrap_or_default();
            ws.on_upgrade(move |socket| handle_socket(socket, state, user_id))
        }
        Err(_) => {
            // Reject by closing immediately after upgrade.
            ws.on_upgrade(|mut socket| async move {
                let _ = socket.close().await;
            })
        }
    }
}

// ── Socket loop ──────────────────────────────────────────────────────────

/// # Per-connection I/O loop.
///
/// Splits the WebSocket into sender/receiver halves, registers the user in
/// [`WsManager`](crate::api::ws::manager::WsManager), then enters the
/// `tokio::select!` loop described below.  When the loop exits (for any
/// reason), the user is deregistered via `WsManager::disconnect`.
///
/// # The select! loop
///
/// The select! loop races two async branches on every iteration:
///
/// • Branch A (outgoing): waits for the next SocketMessage pushed by
///   WsManager::send_to_users.  Serialises it to JSON and sends it as a
///   WebSocket Text frame.  Breaks the loop if the send fails (client gone).
///
/// • Branch B (incoming): waits for the next frame from the client.
///   - Close frame or stream end  -> break (clean disconnect).
///   - Ping frame                 -> reply with Pong (keepalive).
///   - Any other error            -> log and break.
///   - Text / Binary frames       -> ignored (server-to-client only for now).
async fn handle_socket(socket: WebSocket, state: AppState, user_id: Uuid) {
    let (mut ws_sender, mut ws_receiver) = socket.split();

    // Register and obtain the inbound server->client channel.
    let mut rx = state.ws_manager.connect(user_id);
    tracing::info!(%user_id, online = state.ws_manager.online_count(), "ws connected");

    loop {
        tokio::select! {
            // ── Outgoing: server pushes a message to this client ──────────────
            Some(msg) = rx.recv() => {
                let text = match serde_json::to_string(&msg) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(%user_id, "ws serialize error: {e}");
                        continue;
                    }
                };
                if ws_sender.send(WsMessage::Text(text.into())).await.is_err() {
                    break; // Client disconnected.
                }
            }

            // ── Incoming: client sends something ─────────────────────────────
            msg = ws_receiver.next() => {
                match msg {
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    Some(Ok(WsMessage::Ping(data))) => {
                        let _ = ws_sender.send(WsMessage::Pong(data)).await;
                    }
                    Some(Err(e)) => {
                        tracing::warn!(%user_id, "ws recv error: {e}");
                        break;
                    }
                    Some(Ok(_)) => {} // Ignore Text/Binary from client for now.
                }
            }
        }
    }

    state.ws_manager.disconnect(user_id);
    tracing::info!(%user_id, "ws disconnected");
}

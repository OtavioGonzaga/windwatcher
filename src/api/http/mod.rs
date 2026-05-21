//! HTTP transport sub-module.
//!
//! Builds the Axum [`Router`] that exposes all REST endpoints,
//! applies shared middleware layers, and declares the handler sub-modules.
//!
//! ## Route map
//!
//! | Method | Path                       | Auth       | Handler                      |
//! | ------ | ----------------           | ---------- | ---------------------------- |
//! | `POST` | `/auth/register`           | -          | [`auth::register`]           |
//! | `POST` | `/auth/login`              | -          | [`auth::login`]              |
//! | `GET`  | `/users/me`                | Bearer JWT | [`users::me`]                |
//! | `POST` | `/rooms/direct`            | Bearer JWT | [`chat::create_direct_room`] |
//! | `POST` | `/rooms/group`             | Bearer JWT | [`chat::create_group_room`]  |
//! | `POST` | `/rooms/{room_id}/messages` | Bearer JWT | [`chat::send_message`]       |
//! | `GET`  | `/rooms/{room_id}/messages` | Bearer JWT | [`chat::list_messages`]      |
//! | `PUT`  | `/rooms/{room_id}/read`     | Bearer JWT | [`chat::mark_as_read`]       |
//!
//! ## Middleware stack (applied outermost-first)
//!
//! 1. **[`TraceLayer`]** - emits structured
//!    `tracing` spans for every HTTP request, including latency and status code.
//! 2. **[`CorsLayer::permissive`](tower_http::cors::CorsLayer::permissive)** -
//!    allows all origins, methods and headers.  Tighten this in production by
//!    replacing with an explicit [`CorsLayer`]
//!    configuration.

pub mod auth;
pub mod chat;
pub mod extractors;
pub mod users;

use axum::{
    Router,
    routing::{get, post, put},
};
use tower_http::{cors::CorsLayer, trace::TraceLayer};

use crate::state::AppState;

/// Assemble the top-level HTTP [`Router`] with all routes and middleware.
///
/// The router is stateful: [`AppState`] is passed as Axum's shared state and
/// made available to every handler via [`axum::extract::State`].
///
/// ## Routes mounted
///
/// | Method | Path                       | Handler                      |
/// | ------ | -------------------------- | ---------------------------- |
/// | `POST` | `/auth/register`           | [`auth::register`]           |
/// | `POST` | `/auth/login`              | [`auth::login`]              |
/// | `GET`  | `/users/me`                | [`users::me`]                |
/// | `POST` | `/rooms/direct`            | [`chat::create_direct_room`] |
/// | `POST` | `/rooms/group`             | [`chat::create_group_room`]  |
/// | `POST` | `/rooms/{room_id}/messages` | [`chat::send_message`]       |
/// | `GET`  | `/rooms/{room_id}/messages` | [`chat::list_messages`]      |
/// | `PUT`  | `/rooms/{room_id}/read`     | [`chat::mark_as_read`]       |
///
/// ## Middleware (applied in declaration order, outermost last)
///
/// * [`TraceLayer::new_for_http`](tower_http::trace::TraceLayer::new_for_http) -
///   wraps every request in a `tracing` span that records HTTP method, URI,
///   status code, and latency.
/// * [`CorsLayer::permissive`](tower_http::cors::CorsLayer::permissive) -
///   accepts cross-origin requests from any origin.  Replace with a
///   restrictive policy before deploying to production.
pub fn router() -> Router<AppState> {
    Router::new()
        // ── Auth ──────────────────────────────────────────────────────────────
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        // ── Users ─────────────────────────────────────────────────────────────
        .route("/users/me", get(users::me))
        // ── Rooms ─────────────────────────────────────────────────────────────
        .route("/rooms/direct", post(chat::create_direct_room))
        .route("/rooms/group", post(chat::create_group_room))
        // ── Messages ──────────────────────────────────────────────────────────
        .route(
            "/rooms/{room_id}/messages",
            post(chat::send_message).get(chat::list_messages),
        )
        .route("/rooms/{room_id}/read", put(chat::mark_as_read))
        // ── Middleware ────────────────────────────────────────────────────────
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
}

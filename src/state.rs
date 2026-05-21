//! Shared application state.
//!
//! [`AppState`] is the single value injected by Axum via
//! [`axum::extract::State`] into every handler and extractor.  It is
//! constructed once during server startup and then cloned cheaply (all fields
//! are [`Arc`]-wrapped) for each request.
//!
//! The state acts as the **composition root**: it holds references to the
//! fully-wired services and infrastructure objects, enabling handlers to remain
//! thin delegation layers with no direct dependency on storage or business logic.

use std::sync::Arc;

use crate::{
    api::ws::manager::WsManager,
    application::{chat_service::ChatService, user_service::UserService},
    config::AppConfig,
};

/// Shared application state injected by Axum into every handler and extractor.
///
/// Constructed once in `main` and then passed to
/// [`axum::Router::with_state`].  Every field is wrapped in [`Arc`] so that
/// cloning the state (which Axum does per request) is O(1) and does not copy
/// any heap data.
///
/// # Usage in handlers
///
/// ```rust,ignore
/// async fn my_handler(State(state): State<AppState>) -> impl IntoResponse {
///     // state.user_service, state.chat_service, etc.
/// }
/// ```
#[derive(Clone)]
pub struct AppState {
    /// Loaded application configuration (database URL, port, JWT secret, …).
    pub config: Arc<AppConfig>,

    /// Service responsible for user registration, authentication, and profile
    /// retrieval.  Backed by a [`UserRepository`][crate::domain::ports::UserRepository]
    /// implementation chosen at startup.
    pub user_service: Arc<UserService>,

    /// Service responsible for room management, message enqueueing, and
    /// read-receipt tracking.  Backed by a
    /// [`ChatRepository`][crate::domain::ports::ChatRepository] implementation.
    pub chat_service: Arc<ChatService>,

    /// In-memory WebSocket connection manager.  Maintains a map of
    /// `user_id -> sender channel` so that background workers can push
    /// messages to connected clients without holding locks.
    pub ws_manager: Arc<WsManager>,
}

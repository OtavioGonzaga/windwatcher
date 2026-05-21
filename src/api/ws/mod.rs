//! WebSocket transport sub-module.
//!
//! Provides real-time, bi-directional communication between the server and
//! authenticated clients.  The sub-module is split into two concerns:
//!
//! | Module      | Responsibility                                                 |
//! | ----------- | -------------------------------------------------------------- |
//! | [`handler`] | HTTP -> WebSocket upgrade, JWT validation, per-socket I/O loop  |
//! | [`manager`] | In-memory registry of active connections; server -> client push |
//!
//! ## Connection lifecycle
//!
//! ```text
//! Client                           Server
//!   |  GET /ws?token=<jwt>            |
//!   | ------------------------------> |
//!   |                   validate JWT  |
//!   |          101 Switching Protocols|
//!   | <------------------------------ |
//!   |                  WsManager::connect() -> Receiver
//!   |                                 |
//!   |  <-- NewMessage (JSON text)     | <- WsManager::send_to_users()
//!   |  --> Ping                       |
//!   |  <-- Pong                       |
//!   |  --> Close / drop               |
//!   |                  WsManager::disconnect()
//! ```
//!
//! ## Authentication
//!
//! Unlike HTTP endpoints that use the `Authorization` header, the WebSocket
//! upgrade carries the JWT as a `token` query parameter because browsers'
//! native `WebSocket` API does not support custom headers.

pub mod handler;
pub mod manager;

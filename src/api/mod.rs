//! Transport layer - wires together the HTTP and WebSocket sub-systems.
//!
//! This crate module acts as the public entry point for all network-facing
//! code.  It is intentionally thin: it re-exports the two sub-modules and
//! leaves all routing and protocol concerns to them.
//!
//! ## Sub-modules
//!
//! | Module   | Description                                                |
//! | -------- | ---------------------------------------------------------- |
//! | [`http`] | Axum HTTP router, middleware, handlers and JWT extractors  |
//! | [`ws`]   | WebSocket upgrade handler and in-memory connection manager |
//!
//! ## Dependency rule
//!
//! Nothing inside `api/` may import from `db/` directly.  All data access
//! must go through the `application/` service layer so that the transport
//! layer remains decoupled from persistence concerns.

pub mod http;
pub mod ws;

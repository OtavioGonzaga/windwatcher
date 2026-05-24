//! # Windwatcher
//!
//! Windwatcher is a self-hosted real-time chat API written in Rust, built around
//! the **Ports & Adapters (Hexagonal Architecture)** pattern.  Each layer has a
//! strictly defined boundary so that transport, business logic, and storage
//! concerns never bleed into one another.
//!
//! ## Architecture
//!
//! ```text
//! Transport (api/)  ->  Application (application/)  ->  Domain (domain/)  <-  Adapters (db/)
//! ```
//!
//! | Module          | Responsibility                                           |
//! | --------------- | -------------------------------------------------------- |
//! | [`config`]      | Configuration loading via figment + env vars             |
//! | [`error`]       | Centralised [`error::AppError`] + HTTP response mapping  |
//! | [`state`]       | [`state::AppState`] shared across all Axum handlers      |
//! | [`domain`]      | Pure domain models and repository / queue port traits    |
//! | [`application`] | Use-case orchestration (user & chat services)            |
//! | [`db`]          | SeaORM and MongoDB adapter implementations               |
//! | [`api`]         | HTTP handlers, JWT extractors, WebSocket upgrade         |
//! | [`jobs`]        | Background job runtime (Apalis-backed)                    |
//!
//! ## Technology stack
//!
//! - **Axum** - HTTP framework and router
//! - **Tokio** - async runtime
//! - **SeaORM** - SQL adapter (SQLite · PostgreSQL · MySQL)
//! - **Argon2** - password hashing (Argon2id)
//! - **jsonwebtoken** - stateless HS256 authentication
//! - **WebSockets** - real-time message delivery to connected clients
//! - **DashMap** - concurrent in-memory map for open WebSocket sessions
//!
//! ## Chat message flow
//!
//! ```text
//! POST /rooms/:id/messages
//!   -> JWT extractor (AuthenticatedUser)
//!   -> ChatService::enqueue_message()        <- generates UUIDv7, returns 202
//!       -> JobQueue::enqueue_chat_message()
//!           -> background job runtime (Apalis worker)
//!               -> ChatRepository::add_message()
//!               -> ChatRepository::increment_unread()
//!               -> WsManager::send_to_users()  <- broadcasts to online members
//! ```
//!
//! ## Configuration
//!
//! The server reads configuration from (highest to lowest priority):
//! 1. `WINDWATCHER_*` environment variables
//! 2. `windwatcher.toml` (optional, in the working directory)
//! 3. Built-in defaults (SQLite local file, port 3000)
//!
//! See [`config::AppConfig`] for the full list of available settings.

pub mod api;
pub mod application;
pub mod config;
pub mod db;
pub mod domain;
pub mod error;
pub mod jobs;
pub mod state;

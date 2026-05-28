//! Pure domain models for the Windwatcher application.
//!
//! This module defines the **aggregate root structs and enumerations** that
//! represent the core business entities.  All types here are
//! infrastructure-agnostic: they carry no SeaORM, Axum, or MongoDB imports.
//!
//! ## Entities
//!
//! | Type         | Module          | Description                                          |
//! | ------------ | --------------- | ---------------------------------------------------- |
//! | [`User`]     | [`user`]        | Registered account with a role and hashed password   |
//! | [`Room`]     | [`room`]        | Chat room, either direct (1:1) or group              |
//! | [`RoomUser`] | [`room`]        | Membership record linking a user to a room           |
//! | [`Message`]  | [`message`]     | A single chat message inside a room                  |
//!
//! ## Conventions
//!
//! - All primary keys are [`uuid::Uuid`] values generated with
//!   [`uuid::Uuid::now_v7`] (time-ordered UUIDv7).
//! - Timestamps are [`chrono::DateTime<Utc>`].
//! - Serialisation uses `serde` with `rename_all = "lowercase"` for enums.

pub mod message;
pub mod room;
pub mod user;

pub use message::Message;
pub use room::{Room, RoomType, RoomUser};
pub use user::{User, UserRole};

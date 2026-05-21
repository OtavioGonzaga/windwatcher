//! SeaORM entity definitions for the Windwatcher schema.
//!
//! Each sub-module contains a single `DeriveEntityModel` that maps a
//! database table to a Rust struct.  The four entities covered are:
//!
//! | Module        | Table         | Domain model |
//! |:--------------|:--------------|:-------------|
//! | [`user`]      | `users`       | [`crate::domain::models::User`]    |
//! | [`room`]      | `rooms`       | [`crate::domain::models::Room`]    |
//! | [`room_user`] | `room_users`  | *(join table - no direct domain equivalent)* |
//! | [`message`]   | `messages`    | [`crate::domain::models::Message`] |
//!
//! Entities are internal to the `db` crate; the rest of the application
//! always works with the domain types from `domain::models`.

pub mod message;
pub mod room;
pub mod room_user;
pub mod user;

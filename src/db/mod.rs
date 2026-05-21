//! Database adapter layer for Windwatcher.
//!
//! This module contains the concrete implementations of the
//! [`crate::domain::ports::UserRepository`] and [`crate::domain::ports::ChatRepository`]
//! ports that are defined in `domain::ports`.
//!
//! ## Available backends
//!
//! | Feature flag | Backend                                         | Sub-module       |
//! | ------------ | ----------------------------------------------- | ---------------- |
//! | *(default)*  | SQL via SeaORM (SQLite · PostgreSQL · MySQL)    | [`seaorm`]       |
//! | `mongodb`    | MongoDB (feature-gated, opt-in at compile time) | `mongodb`        |
//!
//! ## Design rule
//!
//! Adapters are **pure infrastructure**: they translate between the database
//! representation and the domain types defined in `domain::models`.  They must
//! not contain any business logic - that responsibility belongs exclusively to
//! `application/`.

pub mod seaorm;

#[cfg(feature = "mongodb")]
pub mod mongodb;

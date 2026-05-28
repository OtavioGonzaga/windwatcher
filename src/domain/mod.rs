//! Domain layer — enterprise business logic.
//!
//! This crate contains the **pure domain** of Windwatcher: types, traits,
//! and business rules with zero infrastructure dependencies.
//!
//! ## Modules
//!
//! | Module    | Contents                                                  |
//! | --------- | --------------------------------------------------------- |
//! | [`models`]| Domain structs and enums (`User`, `Room`, `Message`, ...) |
//! | [`ports`] | Abstract repository and job-queue traits                  |
//!
//! The [`models`] module defines the **aggregate root structs** that represent
//! core business entities.  All types are infrastructure-agnostic: they carry
//! no SeaORM, Axum, or MongoDB imports.
//!
//! The [`ports`] module defines the **trait interfaces** (ports) that the
//! application layer depends on.  Concrete adapters live in `db/` and `jobs/`.

pub mod models;
pub mod ports;

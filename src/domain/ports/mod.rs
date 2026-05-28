//! Port traits (interfaces) for the Hexagonal Architecture.
//!
//! In Ports & Adapters terminology, **ports** are the abstract boundaries
//! between the application core and the outside world.  This module defines
//! all port traits that the `application/` layer depends on; concrete
//! implementations live in `db/seaorm/`, `db/mongodb/`, and `jobs/`.
//!
//! ## Ports defined here
//!
//! | Trait               | Module          | Adapters                                          |
//! | ------------------- | --------------- | ------------------------------------------------- |
//! | [`UserRepository`]  | [`user_repo`]   | `SeaOrmUserRepository`, `MongoUserRepository`     |
//! | [`ChatRepository`]  | [`chat_repo`]   | `SeaOrmChatRepository`, `MongoChatRepository`     |
//! | [`JobQueue`]        | [`job_queue`]   | Apalis-backed implementations (see `crate::jobs`) |
//!
//! All traits are `Send + Sync` and object-safe so they can be boxed or
//! wrapped in [`std::sync::Arc`] and injected through [`crate::state::AppState`].
//!
//! In test builds, mockall generates `MockUserRepository`, `MockChatRepository`,
//! and `MockJobQueue` automatically via the `#[cfg_attr(test, mockall::automock)]`
//! attribute.

pub mod chat_repo;
pub mod job_queue;
pub mod user_repo;

pub use chat_repo::ChatRepository;
pub use job_queue::{ChatMessageJob, JobQueue};
pub use user_repo::UserRepository;

#[cfg(test)]
pub use chat_repo::MockChatRepository;
#[cfg(test)]
pub use job_queue::MockJobQueue;
#[cfg(test)]
pub use user_repo::MockUserRepository;

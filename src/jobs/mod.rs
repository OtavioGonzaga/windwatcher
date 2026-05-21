//! Background job infrastructure based on Apalis.
//!
//! `build_job_runtime` is the entry-point used in `main.rs` to:
//! 1. Build an adapter that implements [`crate::domain::ports::JobQueue`].
//! 2. Start an Apalis worker for the selected queue provider.
//! 3. Keep business logic in [`processor::process_chat_message`].

#[cfg(feature = "queue-memory")]
mod memory;
mod processor;
#[cfg(feature = "queue-redis")]
mod redis;
mod runtime;
#[cfg(any(
    feature = "queue-sqlite",
    feature = "queue-postgres",
    feature = "queue-mysql"
))]
mod sql;

pub use processor::process_chat_message;
pub use runtime::{JobRuntime, build_job_runtime};

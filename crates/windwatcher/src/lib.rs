#![warn(missing_docs)]

//! Embeddable messaging engine for Rust applications.

mod id;
mod identity;

pub use id::{ConversationId, IdentifierError, MessageId, SessionId, UserId};
pub use identity::{User, Username, UsernameError};

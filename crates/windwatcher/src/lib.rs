#![warn(missing_docs)]

//! Embeddable messaging engine for Rust applications.

mod authentication;
mod id;
mod identity;
mod identity_store;
mod local_users;
mod password;

pub use authentication::{AuthenticateLocalUser, AuthenticateLocalUserError};
pub use identity_store::{IdentityStore, IdentityStoreError, LocalAuthentication};
pub use local_users::{
    DisableUser, EnableUser, ManageUserError, RegisterLocalUser, RegisterLocalUserError,
    RegistrationPolicy,
};
pub use password::{
    Argon2idPasswordHasher, PasswordHash, PasswordHashError, PasswordHasher, PasswordPolicyError,
    SecretPassword,
};

pub use id::{ConversationId, IdentifierError, MessageId, SessionId, UserId};
pub use identity::{User, Username, UsernameError};

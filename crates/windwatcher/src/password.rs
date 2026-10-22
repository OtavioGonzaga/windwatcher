use std::{fmt, future::Future};

use argon2::{
    Algorithm, Argon2, Params, Version,
    password_hash::{PasswordHasher as _, PasswordVerifier as _, SaltString},
};
use rand_core::{OsRng, RngCore};
use zeroize::Zeroizing;

/// An owned, unmodified password, zeroized on drop. Not serializable or cloneable.
pub struct SecretPassword(Zeroizing<String>);

/// New-password policy or password input safety violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum PasswordPolicyError {
    /// New passwords require at least eight Unicode scalar values.
    #[error("password must contain at least 8 characters")]
    TooShort,
    /// All password inputs are limited to 1024 UTF-8 bytes.
    #[error("password must contain at most 1024 UTF-8 bytes")]
    TooLong,
}

impl SecretPassword {
    /// Takes ownership without normalization, enforcing only the 1024-byte safety bound.
    /// The owned input is also zeroized when validation fails.
    pub fn new(value: String) -> Result<Self, PasswordPolicyError> {
        let value = Zeroizing::new(value);
        if value.len() > 1024 {
            return Err(PasswordPolicyError::TooLong);
        }
        Ok(Self(value))
    }

    /// Checks the current policy for a newly registered password.
    /// Authentication deliberately does not call this method.
    pub fn validate_new_password(&self) -> Result<(), PasswordPolicyError> {
        if self.0.chars().count() < 8 {
            return Err(PasswordPolicyError::TooShort);
        }
        Ok(())
    }

    /// Explicit secret access for hashing adapters. Do not log or retain this value.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretPassword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretPassword([REDACTED])")
    }
}

/// Opaque, complete PHC-encoded credential, separate from the user identity.
#[derive(Clone)]
pub struct PasswordHash(String);

impl PasswordHash {
    /// Reconstructs a persisted PHC value, requiring a valid encoding, salt and output.
    /// Algorithm support and algorithm-specific parameters are checked by the adapter.
    pub fn parse(encoded: &str) -> Result<Self, PasswordHashError> {
        let parsed = argon2::password_hash::PasswordHash::new(encoded)
            .map_err(|e| PasswordHashError::InvalidEncodedHash(Box::new(e)))?;
        if parsed.salt.is_none() || parsed.hash.is_none() {
            return Err(PasswordHashError::InvalidEncodedHash(Box::new(
                argon2::password_hash::Error::PhcStringField,
            )));
        }
        Ok(Self(encoded.to_owned()))
    }

    /// Explicit persistence access. This authentication material must not be logged.
    pub fn as_encoded(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for PasswordHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PasswordHash([REDACTED])")
    }
}

/// Normalized hashing failures; sources are available for diagnostics only.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PasswordHashError {
    /// Persisted data is not a complete valid encoded hash.
    #[error("invalid encoded password hash")]
    InvalidEncodedHash(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Hash generation or salt generation failed.
    #[error("password hashing failed")]
    HashingFailed(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Verification failed operationally, rather than a password mismatch.
    #[error("password verification failed")]
    VerificationFailed(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Runtime-independent password operations with sendable futures.
/// Runtime-aware wrappers may offload CPU-bound work onto a blocking pool.
pub trait PasswordHasher: Sync {
    /// Hashes the exact password using a fresh cryptographically secure salt.
    fn hash(
        &self,
        password: &SecretPassword,
    ) -> impl Future<Output = Result<PasswordHash, PasswordHashError>> + Send;
    /// Verifies a password. `None` MUST perform comparable cryptographic work and
    /// MUST return false, regardless of the candidate. A mismatch is not an error.
    fn verify(
        &self,
        password: &SecretPassword,
        expected: Option<&PasswordHash>,
    ) -> impl Future<Output = Result<bool, PasswordHashError>> + Send;
}

/// Argon2id v19 adapter: 19456 KiB, two passes, one lane, 32-byte output.
///
/// Work runs synchronously when its future is polled. Hosts should use a
/// runtime-aware blocking-pool wrapper for latency-sensitive executors.
#[derive(Debug, Clone, Copy, Default)]
pub struct Argon2idPasswordHasher;

impl Argon2idPasswordHasher {
    fn engine() -> Argon2<'static> {
        Argon2::new(
            Algorithm::Argon2id,
            Version::V0x13,
            Params::new(19456, 2, 1, Some(32)).expect("fixed valid Argon2 parameters"),
        )
    }
}

// Complete, fixed-cost dummy credential. Never exposed through the public API.
const DUMMY: &str = "$argon2id$v=19$m=19456,t=2,p=1$d2luZHdhdGNoZXItZHVtbXk$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

impl PasswordHasher for Argon2idPasswordHasher {
    async fn hash(&self, password: &SecretPassword) -> Result<PasswordHash, PasswordHashError> {
        let mut salt = [0u8; 16];
        OsRng
            .try_fill_bytes(&mut salt)
            .map_err(|e| PasswordHashError::HashingFailed(Box::new(e)))?;
        let salt = SaltString::encode_b64(&salt)
            .map_err(|e| PasswordHashError::HashingFailed(Box::new(e)))?;
        let hash = Self::engine()
            .hash_password(password.expose_secret().as_bytes(), &salt)
            .map_err(|e| PasswordHashError::HashingFailed(Box::new(e)))?
            .to_string();
        Ok(PasswordHash(hash))
    }

    async fn verify(
        &self,
        password: &SecretPassword,
        expected: Option<&PasswordHash>,
    ) -> Result<bool, PasswordHashError> {
        let encoded = expected.map_or(DUMMY, PasswordHash::as_encoded);
        let parsed = argon2::password_hash::PasswordHash::new(encoded)
            .map_err(|e| PasswordHashError::InvalidEncodedHash(Box::new(e)))?;
        match Self::engine().verify_password(password.expose_secret().as_bytes(), &parsed) {
            Ok(()) => Ok(expected.is_some()),
            Err(argon2::password_hash::Error::Password) => Ok(false),
            Err(e) => Err(PasswordHashError::VerificationFailed(Box::new(e))),
        }
    }
}

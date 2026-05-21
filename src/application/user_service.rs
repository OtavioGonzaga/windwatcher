//! User service - registration, authentication, and profile management.
//!
//! This module owns the complete lifecycle of a [`User`] account:
//!
//! * **Registration** - validates input, enforces e-mail uniqueness, hashes
//!   the password with Argon2id, and persists the record via
//!   [`UserRepository`].
//! * **Authentication** - verifies the submitted password against the stored
//!   Argon2 hash and issues a signed HS256 JWT on success.
//! * **Profile lookup** - fetches an existing user by their UUID primary key.
//! * **Token decoding** - validates a JWT and returns its [`Claims`] payload;
//!   consumed by the HTTP extractors in `api/http/extractors.rs`.
//!
//! # Security model
//!
//! * Passwords are **never** stored in plain text; only the Argon2id hash
//!   (including the per-user salt) is persisted.
//! * Authentication failures always return the same generic message
//!   (`"invalid credentials"`) regardless of whether the e-mail exists,
//!   preventing user-enumeration attacks.
//! * JWTs are signed with HS256.  The secret must be kept confidential and
//!   rotated if compromised.
//!
//! # Dependencies
//!
//! * [`crate::domain::ports::UserRepository`] - data-access port (injected).
//! * [`argon2`] - Argon2id password hashing.
//! * [`jsonwebtoken`] - HS256 JWT signing and verification.

use std::sync::Arc;

use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use chrono::Utc;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    domain::{
        models::{User, UserRole},
        ports::UserRepository,
    },
    error::AppError,
};

// ── DTOs ───────────────────────────────────────────────────────────────────────

/// Data transfer object for the user-registration endpoint (`POST /auth/register`).
///
/// All fields are required.  Before the record is persisted, the service
/// validates that:
/// * `username` is non-blank.
/// * `email` contains an `@` character.
/// * `password` is at least 8 characters long.
#[derive(Debug, Deserialize)]
pub struct RegisterUserDto {
    /// Desired display name for the new account.  Must not be blank or
    /// contain only whitespace.
    pub username: String,
    /// E-mail address used for login.  Must be unique across all accounts.
    pub email: String,
    /// Plain-text password chosen by the user.
    ///
    /// This value is **never** stored; it is hashed with Argon2id (random
    /// salt) before being written to the database.
    pub password: String,
}

/// Data transfer object for the login / authentication endpoint
/// (`POST /auth/login`).
#[derive(Debug, Deserialize)]
pub struct LoginDto {
    /// E-mail address of the account to authenticate.
    pub email: String,
    /// Plain-text password to verify against the stored Argon2 hash.
    pub password: String,
}

/// Successful authentication response returned to the caller.
///
/// Contains a freshly signed JWT and the authenticated user's profile.
///
/// > **Note**: the embedded [`User`] value includes a `password_hash` field.
/// > HTTP handlers must **not** forward it to untrusted clients; they should
/// > serialise only the safe subset of fields (e.g. id, username, email, role).
#[derive(Debug, Serialize)]
pub struct AuthTokenResponse {
    /// Signed HS256 JSON Web Token.  Include it in the
    /// `Authorization: Bearer <token>` header on subsequent requests.
    pub token: String,
    /// Full profile of the authenticated user.
    pub user: User,
}

// ── JWT Claims ─────────────────────────────────────────────────────────────────

/// JWT claims embedded in every access token issued by [`UserService`].
///
/// The token is signed with HS256 using the `jwt_secret` from
/// [`crate::config::AppConfig`].  Standard registered claims (`exp`, `iat`,
/// `sub`) follow [RFC 7519](https://www.rfc-editor.org/rfc/rfc7519).
/// The `email` and `role` private claims are added for handler convenience so
/// that most requests can be authorised without an extra database round-trip.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    /// Subject - the authenticated user's UUID serialised as a hyphenated string.
    pub sub: String,
    /// E-mail address of the authenticated user, carried for convenience.
    pub email: String,
    /// Role of the authenticated user - `"user"` or `"admin"`.
    pub role: String,
    /// Expiration time as a Unix timestamp (seconds since the UNIX epoch).
    pub exp: usize,
    /// Issued-at time as a Unix timestamp (seconds since the UNIX epoch).
    pub iat: usize,
}

// ── Service ────────────────────────────────────────────────────────────────────

/// Application service that manages user accounts and JWT-based authentication.
///
/// Constructed once at startup and shared across all request handlers through
/// [`crate::state::AppState`].  All persistence is delegated to the injected
/// [`UserRepository`]; cryptographic operations use [`argon2`] and
/// [`jsonwebtoken`].
pub struct UserService {
    /// Repository adapter for user persistence (Postgres, SQLite, MongoDB, …).
    pub user_repo: Arc<dyn UserRepository>,
    /// Secret key used to sign and verify HS256 JWTs.
    ///
    /// Must be kept confidential.  If leaked, all outstanding tokens should be
    /// considered compromised and the secret rotated immediately.
    pub jwt_secret: String,
    /// Validity window for newly issued tokens, in seconds (e.g. `86400` = 24 h).
    pub jwt_expiry_secs: u64,
}

impl UserService {
    /// Create a new [`UserService`].
    ///
    /// # Parameters
    ///
    /// * `user_repo` - concrete repository implementation (e.g. the SeaORM
    ///   adapter [`crate::db::seaorm::SeaOrmUserRepository`]).
    /// * `jwt_secret` - HMAC secret for HS256 signing; must be kept
    ///   confidential and be sufficiently random (≥ 32 bytes recommended).
    /// * `jwt_expiry_secs` - lifetime of issued tokens in seconds.  The
    ///   default configured value is `86400` (24 hours).
    pub fn new(
        user_repo: Arc<dyn UserRepository>,
        jwt_secret: String,
        jwt_expiry_secs: u64,
    ) -> Self {
        Self {
            user_repo,
            jwt_secret,
            jwt_expiry_secs,
        }
    }

    // ── Register ───────────────────────────────────────────────────────────────

    /// Create a new user account.
    ///
    /// # Flow
    ///
    /// 1. **Validation** - rejects blank usernames, malformed e-mails, and
    ///    passwords shorter than 8 characters.
    /// 2. **Uniqueness check** - queries the repository; aborts with
    ///    [`AppError::Conflict`] if the e-mail is already registered.
    /// 3. **Password hashing** - derives a new Argon2id PHC string with a
    ///    freshly generated random salt ([`argon2::password_hash::SaltString`]).
    /// 4. **Persistence** - builds a [`User`] record with a UUIDv7 primary key
    ///    and [`chrono::Utc::now`] timestamps, then delegates to
    ///    [`UserRepository::create`].
    ///
    /// # Errors
    ///
    /// * [`AppError::Validation`] - one of the input constraints was violated
    ///   (blank username, missing `@` in email, or password < 8 chars).
    /// * [`AppError::Conflict`] - the supplied e-mail address is already in use.
    /// * [`AppError::Internal`] - Argon2 failed to hash the password (extremely
    ///   rare; indicates an OS-level RNG failure).
    /// * [`AppError::Database`] - the repository returned a database error.
    pub async fn register_user(&self, dto: RegisterUserDto) -> Result<User, AppError> {
        // ── Validation ─────────────────────────────────────────────────────────
        if dto.username.trim().is_empty() {
            return Err(AppError::Validation("username must not be empty".into()));
        }
        if !dto.email.contains('@') {
            return Err(AppError::Validation(
                "email must contain an '@' character".into(),
            ));
        }
        if dto.password.len() < 8 {
            return Err(AppError::Validation(
                "password must be at least 8 characters long".into(),
            ));
        }

        // ── Duplicate check ────────────────────────────────────────────────────
        if self.user_repo.find_by_email(&dto.email).await?.is_some() {
            return Err(AppError::Conflict(format!(
                "email '{}' is already registered",
                dto.email
            )));
        }

        // ── Password hashing ───────────────────────────────────────────────────
        let salt = SaltString::generate(&mut OsRng);
        let password_hash = Argon2::default()
            .hash_password(dto.password.as_bytes(), &salt)?
            .to_string();

        // ── Build and persist the user ─────────────────────────────────────────
        let now = Utc::now();
        let user = User {
            id: Uuid::now_v7(),
            username: dto.username,
            email: dto.email,
            password_hash,
            role: UserRole::User,
            created_at: now,
            updated_at: now,
        };

        self.user_repo.create(user).await
    }

    // ── Authenticate ───────────────────────────────────────────────────────────

    /// Verify credentials and return a signed JWT together with the user record.
    ///
    /// # Security note
    ///
    /// Whether the e-mail address is unknown **or** the password is wrong, the
    /// error is always [`AppError::Unauthorized`] with the message
    /// `"invalid credentials"`.  This makes both failure modes
    /// indistinguishable to the caller, preventing user-enumeration attacks.
    ///
    /// # Flow
    ///
    /// 1. Look up the user by e-mail; return [`AppError::Unauthorized`] if not
    ///    found (same error as wrong password - see security note above).
    /// 2. Parse the stored Argon2 PHC string and verify the submitted password.
    /// 3. Build [`Claims`] with the current time and the configured expiry, then
    ///    sign them with HS256 using `jwt_secret`.
    ///
    /// # Errors
    ///
    /// * [`AppError::Unauthorized`] - e-mail not found, or the password does
    ///   not match the stored Argon2 hash.
    /// * [`AppError::Internal`] - the stored hash string is malformed and could
    ///   not be parsed (indicates a corrupt database record).
    /// * [`AppError::Database`] - the repository returned a database error.
    pub async fn authenticate(&self, dto: LoginDto) -> Result<AuthTokenResponse, AppError> {
        // ── Look up the user ───────────────────────────────────────────────────
        let user = self
            .user_repo
            .find_by_email(&dto.email)
            .await?
            .ok_or_else(|| AppError::Unauthorized("invalid credentials".into()))?;

        // ── Verify the password ────────────────────────────────────────────────
        let parsed_hash = PasswordHash::new(&user.password_hash)?;
        Argon2::default()
            .verify_password(dto.password.as_bytes(), &parsed_hash)
            .map_err(|_| AppError::Unauthorized("invalid credentials".into()))?;

        // ── Issue a JWT ────────────────────────────────────────────────────────
        let now = Utc::now().timestamp() as usize;
        let claims = Claims {
            sub: user.id.to_string(),
            email: user.email.clone(),
            role: user.role.to_string(),
            exp: now + self.jwt_expiry_secs as usize,
            iat: now,
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )?;

        Ok(AuthTokenResponse { token, user })
    }

    // ── Get by ID ──────────────────────────────────────────────────────────────

    /// Fetch a single user by their UUID primary key.
    ///
    /// # Errors
    ///
    /// * [`AppError::NotFound`] - no user with the given `id` exists in the
    ///   repository.
    /// * [`AppError::Database`] - the repository returned a database error.
    pub async fn get_by_id(&self, id: Uuid) -> Result<User, AppError> {
        self.user_repo
            .find_by_id(id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("user {id} not found")))
    }

    // ── Decode JWT ─────────────────────────────────────────────────────────────

    /// Decode and validate a JWT, returning its [`Claims`] payload on success.
    ///
    /// The token is verified against `jwt_secret` using the HS256 algorithm.
    /// Token expiry (`exp`) is checked automatically by the [`jsonwebtoken`]
    /// crate; expired tokens are rejected with [`AppError::Unauthorized`].
    ///
    /// # Errors
    ///
    /// * [`AppError::Unauthorized`] - the token is malformed, the signature is
    ///   invalid, the token has expired, or any other validation check fails.
    ///   The underlying [`jsonwebtoken::errors::Error`] is converted via the
    ///   `From` impl in [`crate::error`].
    pub fn decode_token(&self, token: &str) -> Result<Claims, AppError> {
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &Validation::default(),
        )?;
        Ok(token_data.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{models::UserRole, ports::MockUserRepository};
    use chrono::Utc;
    use mockall::predicate::*;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn svc(repo: MockUserRepository) -> UserService {
        UserService::new(Arc::new(repo), "test-secret".into(), 3600)
    }

    /// Build a `User` whose password is properly hashed with Argon2.
    async fn hashed_user(email: &str, password: &str) -> User {
        use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
        let salt = SaltString::generate(&mut OsRng);
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .unwrap()
            .to_string();
        User {
            id: Uuid::now_v7(),
            username: "testuser".into(),
            email: email.into(),
            password_hash: hash,
            role: UserRole::User,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn bare_user(email: &str) -> User {
        User {
            id: Uuid::now_v7(),
            username: "testuser".into(),
            email: email.into(),
            password_hash: "irrelevant".into(),
            role: UserRole::User,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    // ── register_user ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn register_ok() {
        let mut repo = MockUserRepository::new();
        repo.expect_find_by_email().returning(|_| Ok(None));
        repo.expect_create().returning(|u| Ok(u));

        let result = svc(repo)
            .register_user(RegisterUserDto {
                username: "alice".into(),
                email: "alice@example.com".into(),
                password: "password123".into(),
            })
            .await;

        let user = result.expect("register should succeed");
        assert_eq!(user.username, "alice");
        assert_eq!(user.role, UserRole::User);
        assert_eq!(user.email, "alice@example.com");
    }

    #[tokio::test]
    async fn register_empty_username() {
        let repo = MockUserRepository::new(); // no calls expected
        let err = svc(repo)
            .register_user(RegisterUserDto {
                username: "".into(),
                email: "a@b.com".into(),
                password: "password123".into(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn register_invalid_email() {
        let repo = MockUserRepository::new();
        let err = svc(repo)
            .register_user(RegisterUserDto {
                username: "alice".into(),
                email: "not-an-email".into(),
                password: "password123".into(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn register_short_password() {
        let repo = MockUserRepository::new();
        let err = svc(repo)
            .register_user(RegisterUserDto {
                username: "alice".into(),
                email: "a@b.com".into(),
                password: "short".into(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[tokio::test]
    async fn register_duplicate_email() {
        let existing = bare_user("dup@example.com");
        let mut repo = MockUserRepository::new();
        repo.expect_find_by_email()
            .returning(move |_| Ok(Some(existing.clone())));

        let err = svc(repo)
            .register_user(RegisterUserDto {
                username: "alice".into(),
                email: "dup@example.com".into(),
                password: "password123".into(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));
    }

    // ── authenticate ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn authenticate_ok() {
        let password = "correct-password";
        let user = hashed_user("alice@example.com", password).await;
        let user_clone = user.clone();
        let mut repo = MockUserRepository::new();
        repo.expect_find_by_email()
            .returning(move |_| Ok(Some(user_clone.clone())));

        let resp = svc(repo)
            .authenticate(LoginDto {
                email: "alice@example.com".into(),
                password: password.into(),
            })
            .await
            .expect("authenticate should succeed");

        assert!(!resp.token.is_empty());
        assert_eq!(resp.user.email, "alice@example.com");
    }

    #[tokio::test]
    async fn authenticate_user_not_found() {
        let mut repo = MockUserRepository::new();
        repo.expect_find_by_email().returning(|_| Ok(None));

        let err = svc(repo)
            .authenticate(LoginDto {
                email: "nobody@example.com".into(),
                password: "password".into(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Unauthorized(_)));
    }

    #[tokio::test]
    async fn authenticate_wrong_password() {
        let user = hashed_user("alice@example.com", "correct").await;
        let mut repo = MockUserRepository::new();
        repo.expect_find_by_email()
            .returning(move |_| Ok(Some(user.clone())));

        let err = svc(repo)
            .authenticate(LoginDto {
                email: "alice@example.com".into(),
                password: "wrong-password".into(),
            })
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Unauthorized(_)));
    }

    // ── get_by_id ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn get_by_id_found() {
        let user = bare_user("u@u.com");
        let id = user.id;
        let user_clone = user.clone();
        let mut repo = MockUserRepository::new();
        repo.expect_find_by_id()
            .returning(move |_| Ok(Some(user_clone.clone())));

        let result = svc(repo).get_by_id(id).await.unwrap();
        assert_eq!(result.id, id);
    }

    #[tokio::test]
    async fn get_by_id_not_found() {
        let mut repo = MockUserRepository::new();
        repo.expect_find_by_id().returning(|_| Ok(None));

        let err = svc(repo).get_by_id(Uuid::now_v7()).await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

    // ── decode_token ──────────────────────────────────────────────────────────

    #[test]
    fn decode_valid_token() {
        let svc = UserService::new(
            Arc::new(MockUserRepository::new()),
            "my-secret".into(),
            3600,
        );
        let now = Utc::now().timestamp() as usize;
        let claims = Claims {
            sub: "user-id".into(),
            email: "e@e.com".into(),
            role: "user".into(),
            exp: now + 3600,
            iat: now,
        };
        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(b"my-secret"),
        )
        .unwrap();

        let decoded = svc.decode_token(&token).unwrap();
        assert_eq!(decoded.sub, "user-id");
        assert_eq!(decoded.email, "e@e.com");
    }

    #[test]
    fn decode_tampered_token_fails() {
        let svc = UserService::new(Arc::new(MockUserRepository::new()), "secret".into(), 3600);
        let err = svc.decode_token("not.a.valid.jwt").unwrap_err();
        assert!(matches!(err, AppError::Unauthorized(_)));
    }
}

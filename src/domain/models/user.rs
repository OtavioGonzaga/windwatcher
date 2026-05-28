use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Role assigned to a user account, controlling access to privileged endpoints.
///
/// Serialises as lowercase (`"user"`, `"admin"`) in JSON and TOML.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UserRole {
    /// Standard user - can send messages and manage their own profile.
    User,
    /// Administrator - has access to privileged management endpoints.
    Admin,
}

impl std::fmt::Display for UserRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserRole::User => write!(f, "user"),
            UserRole::Admin => write!(f, "admin"),
        }
    }
}

impl std::str::FromStr for UserRole {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(UserRole::User),
            "admin" => Ok(UserRole::Admin),
            other => Err(format!("unknown role: {other}")),
        }
    }
}

/// A registered user account.
///
/// This struct is the canonical representation of a user throughout the
/// application.  When serialised to JSON (e.g. in API responses) the
/// `password_hash` field is **always omitted** via `#[serde(skip_serializing)]`
/// to prevent accidental exposure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    /// Unique user identifier (UUIDv7).
    pub id: Uuid,
    /// Human-readable display name, unique across all users.
    pub username: String,
    /// Email address used for login, unique across all users.
    pub email: String,
    /// Argon2id password hash.  Never included in JSON output.
    #[serde(skip_serializing)]
    pub password_hash: String,
    /// Access role controlling which endpoints the user may call.
    pub role: UserRole,
    /// Timestamp when the account was created (UTC).
    pub created_at: DateTime<Utc>,
    /// Timestamp of the last profile update (UTC).
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_role_roundtrip() {
        assert_eq!(UserRole::User.to_string(), "user");
        assert_eq!("user".parse::<UserRole>().unwrap(), UserRole::User);
        assert_eq!(UserRole::Admin.to_string(), "admin");
        assert_eq!("admin".parse::<UserRole>().unwrap(), UserRole::Admin);
    }

    #[test]
    fn user_role_unknown_fails() {
        assert!("unknown".parse::<UserRole>().is_err());
    }
}

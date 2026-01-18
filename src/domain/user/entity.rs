use super::value_objects::{name::Name, password_hash::PasswordHash, username::Username};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum UserRole {
    #[serde(rename = "administrator")]
    Administrator,
    #[default]
    #[serde(rename = "user")]
    User,
}

pub struct User {
    pub id: Uuid,
    pub name: Name,
    pub username: Username,
    pub password_hash: PasswordHash,
    pub role: UserRole,
}

impl User {
    pub fn new(
        id: Uuid,
        name: Name,
        username: Username,
        password_hash: PasswordHash,
        role: Option<UserRole>,
    ) -> Self {
        let role: UserRole = role.unwrap_or_default();

        Self {
            id,
            name,
            username,
            password_hash,
            role,
        }
    }
}

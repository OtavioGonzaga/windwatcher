use super::value_objects::{password_hash::PasswordHash, username::Username};
use uuid::Uuid;

pub struct User {
    pub id: Uuid,
    pub name: String,
    pub username: Username,
    pub password_hash: PasswordHash,
}

impl User {
    pub fn new(id: Uuid, name: String, username: Username, password_hash: PasswordHash) -> Self {
        Self {
            id,
            name,
            username,
            password_hash,
        }
    }
}

use super::value_objects::{name::Name, password_hash::PasswordHash, username::Username};
use uuid::Uuid;

pub struct User {
    pub id: Uuid,
    pub name: Name,
    pub username: Username,
    pub password_hash: PasswordHash,
}

impl User {
    pub fn new(id: Uuid, name: Name, username: Username, password_hash: PasswordHash) -> Self {
        Self {
            id,
            name,
            username,
            password_hash,
        }
    }
}

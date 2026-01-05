use super::value_objects::password_hash::PasswordHash;
use super::value_objects::username::Username;
use uuid::Uuid;

pub struct User {
    pub id: Uuid,
    pub name: String,
    pub username: Username,
    pub password_hash: PasswordHash,
}

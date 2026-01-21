use crate::domain::{
    errors::domain::DomainError,
    user::entity::{User, UserRole},
};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub id: Uuid,
    pub username: String,
    pub roles: Vec<UserRole>,
}

impl AuthenticatedUser {
    pub fn new(id: Uuid, username: String, roles: Vec<UserRole>) -> Self {
        Self {
            id,
            username,
            roles,
        }
    }

    pub fn must_be_admin(&self) -> Result<(), DomainError> {
        if self.roles.contains(&UserRole::Administrator) {
            Ok(())
        } else {
            Err(DomainError::Forbidden)
        }
    }

    pub fn must_be_admin_or_owner(&self, id: &Uuid) -> Result<(), DomainError> {
        if id == &self.id {
            Ok(())
        } else {
            self.must_be_admin()
        }
    }
}

impl From<User> for AuthenticatedUser {
    fn from(value: User) -> Self {
        AuthenticatedUser {
            id: value.id,
            username: value.username.as_str().into(),
            roles: vec![value.role],
        }
    }
}

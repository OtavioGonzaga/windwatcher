use crate::domain::{errors::domain::DomainError, user::entity::UserRole};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub id: Uuid,
    pub username: String,
    pub roles: Vec<UserRole>,
}

impl AuthenticatedUser {
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

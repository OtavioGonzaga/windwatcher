use crate::{application::auth::authenticated_user::AuthenticatedUser, domain::{
    errors::{domain::DomainError, repository::RepositoryError},
    user::{entity::User, repository::UserRepository},
}};
use uuid::Uuid;

#[derive(Clone)]
pub struct DeleteUserService<R>
where
    R: UserRepository,
{
    user_repository: R,
}

impl<R> DeleteUserService<R>
where
    R: UserRepository,
{
    pub fn new(user_repository: R) -> Self {
        Self { user_repository }
    }

    pub async fn execute(
        &self,
        id: &Uuid,
        actor: &AuthenticatedUser,
    ) -> Result<(), DeleteUserError> {
        actor.must_be_admin_or_owner(&id)?;

        let user: User = self
            .user_repository
            .find_by_id(id)
            .await?
            .ok_or(DeleteUserError::NotFound)?;

        self.user_repository.delete(&user.id).await?;

        Ok(())
    }
}

pub enum DeleteUserError {
    NotFound,
    InfrastructureError,
    Forbidden,
}

impl From<RepositoryError> for DeleteUserError {
    fn from(_: RepositoryError) -> Self {
        DeleteUserError::InfrastructureError
    }
}

impl From<DomainError> for DeleteUserError {
    fn from(value: DomainError) -> Self {
        match value {
            DomainError::Forbidden => DeleteUserError::Forbidden,
        }
    }
}

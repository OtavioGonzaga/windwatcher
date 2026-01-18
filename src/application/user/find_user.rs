use crate::domain::{
    auth::authenticated_user::AuthenticatedUser,
    errors::{domain::DomainError, repository::RepositoryError},
    user::{entity::User, repository::UserRepository},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct FindUserService<R>
where
    R: UserRepository,
{
    repo: R,
}

impl<R> FindUserService<R>
where
    R: UserRepository,
{
    pub fn new(repo: R) -> Self {
        Self { repo }
    }

    pub async fn find_by_id(
        &self,
        id: &Uuid,
        authenticated_user: &AuthenticatedUser,
    ) -> Result<User, FindUserError> {
        authenticated_user.must_be_admin_or_owner(id)?;

        match self.repo.find_by_id(id).await? {
            Some(user) => Ok(user),
            None => Err(FindUserError::NotFound),
        }
    }
}

pub enum FindUserError {
    NotFound,
    RepositoryError,
    Forbidden,
}

impl From<RepositoryError> for FindUserError {
    fn from(_: RepositoryError) -> Self {
        FindUserError::RepositoryError
    }
}

impl From<DomainError> for FindUserError {
    fn from(value: DomainError) -> Self {
        match value {
            DomainError::Forbidden => FindUserError::Forbidden,
        }
    }
}

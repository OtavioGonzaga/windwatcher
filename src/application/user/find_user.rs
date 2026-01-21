use crate::domain::{
    errors::repository::RepositoryError,
    user::{entity::User, repository::UserRepository},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct FindUserService<R>
where
    R: UserRepository,
{
    user_repository: R,
}

impl<R> FindUserService<R>
where
    R: UserRepository,
{
    pub fn new(user_repository: R) -> Self {
        Self { user_repository }
    }

    pub async fn find_by_id(&self, id: &Uuid) -> Result<User, FindUserError> {
        match self.user_repository.find_by_id(id).await? {
            Some(user) => Ok(user),
            None => Err(FindUserError::NotFound),
        }
    }
}

pub enum FindUserError {
    NotFound,
    RepositoryError,
}

impl From<RepositoryError> for FindUserError {
    fn from(_: RepositoryError) -> Self {
        FindUserError::RepositoryError
    }
}

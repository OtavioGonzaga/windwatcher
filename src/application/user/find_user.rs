use uuid::Uuid;
use crate::domain::{
    errors::repository::RepositoryError,
    user::{entity::User, repository::UserRepository},
};

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

    pub async fn find_by_id(&self, id: Uuid) -> Result<User, FindUserError> {
        match self.repo.find_by_id(&id).await? {
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

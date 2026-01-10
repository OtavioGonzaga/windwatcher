use uuid::Uuid;
use crate::domain::{
    errors::repository::RepositoryError,
    user::{entity::User, repository::UserRepository},
};

#[derive(Clone)]
pub struct DeleteUserService<R>
where
    R: UserRepository,
{
    repo: R,
}

impl<R> DeleteUserService<R>
where
    R: UserRepository,
{
    pub fn new(repo: R) -> Self {
        Self { repo }
    }

    pub async fn execute(&self, user_id: Uuid) -> Result<(), DeleteUserError> {
        let user: User = self
            .repo
            .find_by_id(&user_id)
            .await?
            .ok_or(DeleteUserError::NotFound)?;

        self.repo.delete(&user.id).await?;

        Ok(())
    }
}

pub enum DeleteUserError {
    NotFound,
    InfrastructureError,
}

impl From<RepositoryError> for DeleteUserError {
    fn from(_: RepositoryError) -> Self {
        DeleteUserError::InfrastructureError
    }
}

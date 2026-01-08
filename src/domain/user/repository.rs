use super::{entity::User, value_objects::username::Username};
use crate::domain::errors::repository::RepositoryError;
use async_trait::async_trait;
use uuid::Uuid;

#[async_trait]
pub trait UserRepository {
    async fn find_by_id(&self, id: &Uuid) -> Result<Option<User>, RepositoryError>;
    async fn find_by_username(&self, username: &Username) -> Result<Option<User>, RepositoryError>;
    async fn create(&self, user: User) -> Result<User, RepositoryError>;
}

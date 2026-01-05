use super::value_objects::username::Username;
use super::{entity::User, error::UserError};
use async_trait::async_trait;
use uuid::Uuid;

#[async_trait]
pub trait UserRepository {
    async fn create(&self, user: User) -> Result<User, UserError>;
    async fn find_by_id(&self, id: &Uuid) -> Result<Option<User>, UserError>;
    async fn find_by_username(&self, username: &Username) -> Result<Option<User>, UserError>;
}

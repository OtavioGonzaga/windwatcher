use super::entity::{Entity as UserEntity, Model};
use crate::domain::{
    errors::repository::RepositoryError,
    user::{entity::User, repository::UserRepository, value_objects::username::Username},
};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

#[derive(Clone)]
pub struct PostgresUserRepository {
    db: DatabaseConnection,
}

impl PostgresUserRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

#[async_trait::async_trait]
impl UserRepository for PostgresUserRepository {
    async fn create(&self, user: User) -> Result<User, RepositoryError> {
        let active: super::entity::ActiveModel = user.into();

        let model: Model = active
            .insert(&self.db)
            .await
            .map_err(|_| RepositoryError::Unexpected)?;

        User::try_from(model)
    }

    async fn find_by_id(&self, id: &Uuid) -> Result<Option<User>, RepositoryError> {
        let model = UserEntity::find()
            .filter(super::entity::Column::Id.eq(id.to_owned()))
            .one(&self.db)
            .await
            .map_err(|_| RepositoryError::Unavailable)?;

        match model {
            Some(m) => Ok(Some(User::try_from(m)?)),
            None => Ok(None),
        }
    }

    async fn find_by_username(&self, username: &Username) -> Result<Option<User>, RepositoryError> {
        let model = UserEntity::find()
            .filter(super::entity::Column::Username.eq(username.as_str()))
            .one(&self.db)
            .await
            .map_err(|_| RepositoryError::Unavailable)?;

        match model {
            Some(m) => Ok(Some(User::try_from(m)?)),
            None => Ok(None),
        }
    }
}

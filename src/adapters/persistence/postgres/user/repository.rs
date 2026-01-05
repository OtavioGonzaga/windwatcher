use super::entity::Entity as UserEntity;
use super::entity::Model;
use crate::domain::user::{
    entity::User, error::UserError, repository::UserRepository, value_objects::username::Username,
};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use uuid::Uuid;

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
    async fn create(&self, user: User) -> Result<User, UserError> {
        let active: super::entity::ActiveModel = user.into();

        let model: Model = active
            .insert(&self.db)
            .await
            .map_err(|_| UserError::PersistenceError)?;

        User::try_from(model)
    }

    async fn find_by_id(&self, id: &Uuid) -> Result<Option<User>, UserError> {
        let model = UserEntity::find()
            .filter(super::entity::Column::Id.eq(id.to_owned()))
            .one(&self.db)
            .await
            .map_err(|_| UserError::PersistenceError)?;

        match model {
            Some(m) => Ok(Some(User::try_from(m)?)),
            None => Ok(None),
        }
    }

    async fn find_by_username(&self, username: &Username) -> Result<Option<User>, UserError> {
        let model = UserEntity::find()
            .filter(super::entity::Column::Username.eq(username.as_str()))
            .one(&self.db)
            .await
            .map_err(|_| UserError::PersistenceError)?;

        match model {
            Some(m) => Ok(Some(User::try_from(m)?)),
            None => Ok(None),
        }
    }
}

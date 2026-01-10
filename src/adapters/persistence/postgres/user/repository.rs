use super::entity::{ActiveModel, Column, Entity as UserEntity, Model};
use crate::domain::{
    errors::repository::RepositoryError,
    user::{
        entity::User, patch::UserPatch, repository::UserRepository,
        value_objects::username::Username,
    },
};
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter};
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
        let active: ActiveModel = user.into();

        let model: Model = active.insert(&self.db).await?;

        User::try_from(model)
    }

    async fn find_by_id(&self, id: &Uuid) -> Result<Option<User>, RepositoryError> {
        let model = UserEntity::find()
            .filter(Column::Id.eq(id.to_owned()))
            .one(&self.db)
            .await?;

        match model {
            Some(m) => Ok(Some(User::try_from(m)?)),
            None => Ok(None),
        }
    }

    async fn find_by_username(&self, username: &Username) -> Result<Option<User>, RepositoryError> {
        let model: Option<Model> = UserEntity::find()
            .filter(Column::Username.eq(username.as_str()))
            .one(&self.db)
            .await?;

        match model {
            Some(m) => Ok(Some(User::try_from(m)?)),
            None => Ok(None),
        }
    }

    async fn update(&self, id: &Uuid, user: UserPatch) -> Result<User, RepositoryError> {
        let model: Option<Model> = UserEntity::find()
            .filter(Column::Id.eq(id.to_owned()))
            .one(&self.db)
            .await?;

        if let Some(m) = model {
            let mut active_model: ActiveModel = m.into();

            if let Some(name) = user.name {
                active_model.name = sea_orm::ActiveValue::Set(name);
            }

            if let Some(username) = user.username {
                active_model.username = sea_orm::ActiveValue::Set(username.as_str().to_owned());
            }

            if let Some(password_hash) = user.password_hash {
                active_model.password_hash =
                    sea_orm::ActiveValue::Set(password_hash.as_str().to_owned());
            }

            let updated_model: Model = active_model.update(&self.db).await?;

            return User::try_from(updated_model);
        }

        Err(RepositoryError::Unavailable)
    }

    async fn delete(&self, id: &Uuid) -> Result<(), RepositoryError> {
        let model = UserEntity::find()
            .filter(Column::Id.eq(id.to_owned()))
            .one(&self.db)
            .await?;

        if let Some(m) = model {
            let active_model: ActiveModel = m.into();
            active_model.delete(&self.db).await?;
        }

        Ok(())
    }
}

impl From<DbErr> for RepositoryError {
    fn from(error: DbErr) -> Self {
        match error {
            DbErr::RecordNotFound(_) => RepositoryError::Unavailable,
            _ => RepositoryError::Unexpected,
        }
    }
}

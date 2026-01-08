use super::entity::{ActiveModel, Model};
use crate::domain::{
    errors::repository::RepositoryError,
    user::{
        entity::User,
        value_objects::{password_hash::PasswordHash, username::Username},
    },
};
use sea_orm::ActiveValue::Set;

impl TryFrom<Model> for User {
    type Error = RepositoryError;

    fn try_from(model: Model) -> Result<Self, RepositoryError> {
        let username: Username = Username::new(model.username).map_err(|_| RepositoryError::InvariantViolation)?;

        let password_hash: PasswordHash =
            PasswordHash::new(model.password_hash).map_err(|_| RepositoryError::InvariantViolation)?;

        Ok(User {
            id: model.id,
            username,
            name: model.name,
            password_hash,
        })
    }
}

impl From<User> for ActiveModel {
    fn from(user: User) -> Self {
        ActiveModel {
            id: Set(user.id),
            username: Set(user.username.as_str().to_owned()),
            name: Set(user.name),
            password_hash: Set(user.password_hash.as_str().to_owned()),
        }
    }
}

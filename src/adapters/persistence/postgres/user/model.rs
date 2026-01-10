use super::entity::{ActiveModel, Model};
use crate::domain::{
    errors::repository::RepositoryError,
    user::{
        entity::User,
        error::UserError,
        value_objects::{password_hash::PasswordHash, username::Username},
    },
};
use sea_orm::ActiveValue::Set;

impl TryFrom<Model> for User {
    type Error = RepositoryError;

    fn try_from(model: Model) -> Result<Self, RepositoryError> {
        let username: Username = Username::new(model.username)?;

        let password_hash: PasswordHash = PasswordHash::new(model.password_hash)?;

        Ok(User::new(model.id, model.name, username, password_hash))
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

impl From<UserError> for RepositoryError {
    fn from(error: UserError) -> Self {
        match error {
            _ => RepositoryError::InvariantViolation,
        }
    }
}

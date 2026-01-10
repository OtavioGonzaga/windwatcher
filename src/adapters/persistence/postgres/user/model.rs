use super::entity::{ActiveModel, Model};
use crate::domain::{
    errors::repository::RepositoryError,
    user::{
        entity::User,
        error::UserError,
        value_objects::{name::Name, password_hash::PasswordHash, username::Username},
    },
};
use sea_orm::ActiveValue::Set;

impl TryFrom<Model> for User {
    type Error = RepositoryError;

    fn try_from(model: Model) -> Result<Self, RepositoryError> {
        let username: Username = Username::new(model.username)?;
        let name: Name = Name::new(model.name)?;
        let password_hash: PasswordHash = PasswordHash::new(model.password_hash)?;

        Ok(User::new(model.id, name, username, password_hash))
    }
}

impl From<User> for ActiveModel {
    fn from(user: User) -> Self {
        ActiveModel {
            id: Set(user.id),
            username: Set(user.username.as_str().into()),
            name: Set(user.name.as_str().into()),
            password_hash: Set(user.password_hash.as_str().into()),
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

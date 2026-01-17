use crate::domain::{
    errors::repository::RepositoryError,
    user::{
        entity::User,
        error::UserError,
        password_hasher::PasswordHasher,
        patch::UserPatch,
        repository::UserRepository,
        value_objects::{password_hash::PasswordHash, username::Username},
    },
};
use uuid::Uuid;

#[derive(Clone)]
pub struct UpdateUserService<R, H>
where
    R: UserRepository,
    H: PasswordHasher,
{
    repo: R,
    hasher: H,
}

impl<R, H> UpdateUserService<R, H>
where
    R: UserRepository,
    H: PasswordHasher,
{
    pub fn new(repo: R, hasher: H) -> Self {
        Self { repo, hasher }
    }

    pub async fn execute(
        &self,
        id: Uuid,
        input: UpdateUserInput,
    ) -> Result<UpdateUserOutput, UpdateUserError> {
        let user: User = self
            .repo
            .find_by_id(&id)
            .await?
            .ok_or(UpdateUserError::NotFound)?;

        let username: Option<Username> = match input.username {
            Some(raw) => {
                let username: Username = Username::new(raw)?;

                if username != user.username {
                    if self.repo.find_by_username(&username).await?.is_some() {
                        return Err(UpdateUserError::AlreadyExists);
                    }
                }

                Some(username)
            }
            None => None,
        };

        let password_hash: Option<PasswordHash> = if let Some(password) = input.password {
            let hash: String = self.hasher.hash(&password);
            Some(PasswordHash::new(hash)?)
        } else {
            None
        };

        let patch = UserPatch::new(input.name, username, password_hash);

        let updated_user = self.repo.update(&id, patch).await?;

        Ok(UpdateUserOutput::from(updated_user))
    }
}

pub struct UpdateUserInput {
    pub username: Option<String>,
    pub name: Option<String>,
    pub password: Option<String>,
}

pub struct UpdateUserOutput {
    pub id: Uuid,
    pub username: String,
    pub name: String,
}

impl From<User> for UpdateUserOutput {
    fn from(user: User) -> Self {
        Self {
            id: user.id,
            username: user.username.as_str().into(),
            name: user.name.as_str().into(),
        }
    }
}

pub enum UpdateUserError {
    UserError(UserError),
    NotFound,
    AlreadyExists,
    InfrastructureError,
}

impl From<UserError> for UpdateUserError {
    fn from(e: UserError) -> Self {
        UpdateUserError::UserError(e)
    }
}

impl From<RepositoryError> for UpdateUserError {
    fn from(_: RepositoryError) -> Self {
        UpdateUserError::InfrastructureError
    }
}

impl From<std::io::Error> for UpdateUserError {
    fn from(_: std::io::Error) -> Self {
        UpdateUserError::InfrastructureError
    }
}

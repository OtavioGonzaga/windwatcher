use uuid::Uuid;

use crate::domain::{
    errors::repository::RepositoryError,
    user::{
        entity::User,
        error::UserError,
        password_hasher::PasswordHasher,
        repository::UserRepository,
        value_objects::{
            password_hash::PasswordHash, password_plain::PasswordPlain, username::Username,
        },
    },
};

#[derive(Clone)]
pub struct CreateUserService<R, H>
where
    R: UserRepository,
    H: PasswordHasher,
{
    repo: R,
    hasher: H,
}

impl<R, H> CreateUserService<R, H>
where
    R: UserRepository,
    H: PasswordHasher,
{
    pub fn new(repo: R, hasher: H) -> Self {
        Self { repo, hasher }
    }

    pub async fn execute(
        &self,
        input: CreateUserInput,
    ) -> Result<CreateUserOutput, CreateUserError> {
        let username: Username = Username::new(input.username)?;
        let password: PasswordPlain = PasswordPlain::new(input.password)?;

        if self.repo.find_by_username(&username).await?.is_some() {
            return Err(CreateUserError::AlreadyExists);
        }

        let password_hash_raw = self.hasher.hash(password.as_str()).await?;
        let password_hash = PasswordHash::new(password_hash_raw)?;

        let user: User = User::new(Uuid::now_v7(), input.name, username, password_hash);

        let user: User = self.repo.create(user).await?;

        Ok(CreateUserOutput {
            id: user.id,
            username: user.username.as_str().into(),
            name: user.name,
        })
    }
}

pub struct CreateUserInput {
    pub username: String,
    pub name: String,
    pub password: String,
}

pub struct CreateUserOutput {
    pub id: Uuid,
    pub username: String,
    pub name: String,
}

pub enum CreateUserError {
    UserError(UserError),
    AlreadyExists,
    InfrastructureError,
}

impl From<UserError> for CreateUserError {
    fn from(e: UserError) -> Self {
        CreateUserError::UserError(e)
    }
}

impl From<RepositoryError> for CreateUserError {
    fn from(_: RepositoryError) -> Self {
        CreateUserError::InfrastructureError
    }
}

impl From<std::io::Error> for CreateUserError {
    fn from(_: std::io::Error) -> Self {
        CreateUserError::InfrastructureError
    }
}

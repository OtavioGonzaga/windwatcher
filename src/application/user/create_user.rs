use crate::domain::{
    auth::authenticated_user::AuthenticatedUser,
    errors::{domain::DomainError, repository::RepositoryError},
    user::{
        entity::{User, UserRole, UserStatus},
        error::UserError,
        password_hasher::PasswordHasher,
        repository::UserRepository,
        value_objects::{
            name::Name, password_hash::PasswordHash, password_plain::PasswordPlain,
            username::Username,
        },
    },
};
use uuid::Uuid;

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
        actor: &AuthenticatedUser,
    ) -> Result<CreateUserOutput, CreateUserError> {
        actor.must_be_admin()?;

        let username: Username = Username::new(input.username)?;
        let password: PasswordPlain = PasswordPlain::new(input.password)?;
        let name: Name = Name::new(input.name)?;
        let role: Option<UserRole> = None;
        let status: Option<UserStatus> = None;

        if self.repo.find_by_username(&username).await?.is_some() {
            return Err(CreateUserError::AlreadyExists);
        }

        let password_hash_raw: String = self.hasher.hash(password.as_str());
        let password_hash: PasswordHash = PasswordHash::new(password_hash_raw)?;

        let user: User = User::new(Uuid::now_v7(), name, username, password_hash, role, status);

        let user: User = self.repo.create(user).await?;

        Ok(CreateUserOutput {
            id: user.id,
            username: user.username.as_str().into(),
            name: user.name.as_str().into(),
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
    Forbidden,
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

impl From<DomainError> for CreateUserError {
    fn from(value: DomainError) -> Self {
        match value {
            DomainError::Forbidden => CreateUserError::Forbidden,
        }
    }
}

impl From<std::io::Error> for CreateUserError {
    fn from(_: std::io::Error) -> Self {
        CreateUserError::InfrastructureError
    }
}

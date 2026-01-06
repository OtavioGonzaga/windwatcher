use super::error::UserError;
use super::password_hasher::PasswordHasher;
use super::value_objects::password_hash::PasswordHash;
use super::value_objects::username::Username;
use super::{entity::User, repository::UserRepository};
use uuid::Uuid;

#[derive(Clone)]
pub struct UserService<R, H>
where
    R: UserRepository,
    H: PasswordHasher,
{
    repo: R,
    hasher: H,
}

impl<R, H> UserService<R, H>
where
    R: UserRepository,
    H: PasswordHasher,
{
    pub fn new(repo: R, hasher: H) -> Self {
        Self { repo, hasher }
    }

    pub async fn find_by_id(&self, id: Uuid) -> Result<User, UserError> {
        match self.repo.find_by_id(&id).await? {
            Some(user) => Ok(user),
            None => Err(UserError::NotFound),
        }
    }

    pub async fn create_user(
        &self,
        CreateUserInput {
            name,
            password,
            username,
        }: CreateUserInput,
    ) -> Result<User, UserError> {
        if let Err(e) = self.validate_password(&password) {
            return Err(e);
        }

        if let Err(e) = self.validate_username(&username) {
            return Err(e);
        }

        let username: Username = Username::new(username)?;
        let user_already_exists = self.repo.find_by_username(&username).await?;

        if user_already_exists.is_some() {
            return Err(UserError::AlreadyExists);
        }

        let id: Uuid = Uuid::now_v7();
        let hash: String = self.hasher.hash(&password).await.unwrap();
        let password_hash: PasswordHash = PasswordHash::new(hash)?;

        let user: User = User {
            id,
            name,
            username,
            password_hash,
        };

        self.repo.create(user).await
    }

    fn validate_password(&self, password: &str) -> Result<(), UserError> {
        if password.is_empty() || password.chars().count() < 8 {
            return Err(UserError::InvalidPassword(
                "Password must be at least 8 characters long".into(),
            ));
        }

        if password.chars().count() > 64 {
            return Err(UserError::InvalidPassword(
                "Password must be at most 64 characters long".into(),
            ));
        }

        Ok(())
    }

    fn validate_username(&self, username: &str) -> Result<(), UserError> {
        if username.is_empty() || username.chars().count() < 3 || username.chars().count() > 32 {
            return Err(UserError::InvalidUsername(
                "Username must be between 3 and 32 characters long".into(),
            ));
        }

        if !username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(UserError::InvalidUsername(
                "Username can only contain alphanumeric characters, underscores, and hyphens"
                    .into(),
            ));
        }

        Ok(())
    }
}

// TODO: Move to a separate file
pub struct CreateUserInput {
    pub username: String,
    pub name: String,
    pub password: String,
}

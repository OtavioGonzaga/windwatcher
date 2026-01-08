use crate::domain::user::error::UserError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Username(String);

impl Username {
    pub fn new(value: String) -> Result<Self, UserError> {
        Self::validate_username(&value)?;

        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate_username(username: &str) -> Result<(), UserError> {
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

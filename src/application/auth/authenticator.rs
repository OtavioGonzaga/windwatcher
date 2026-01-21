use super::{credentials::Credentials, error::AuthenticationError};
use crate::application::auth::authenticated_user::AuthenticatedUser;

pub trait Authenticator {
    async fn authenticate(
        &self,
        credentials: Credentials,
    ) -> Result<AuthenticatedUser, AuthenticationError>;
}

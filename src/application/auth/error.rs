#[derive(Debug)]
pub enum AuthenticationError {
    InvalidCredentials,
    UserInactive,
    UserNotFound,
    ProviderUnavailable,
    UnsupportedCredentials,
}

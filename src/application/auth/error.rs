#[derive(Debug)]
pub enum AuthenticationError {
    InvalidCredentials,
    UserInactive,
    UserNotFound,
    ProviderUnavailable,
    _UnsupportedCredentials,
}

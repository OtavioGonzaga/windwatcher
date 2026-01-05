pub enum UserError {
    NotFound,
    AlreadyExists,
    InvalidUsername(String),
    InvalidPassword(String),
    PersistenceError,
}

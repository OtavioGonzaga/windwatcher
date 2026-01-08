#[derive(Debug)]
pub enum RepositoryError {
    Unavailable,
    Unexpected,
    InvariantViolation,
}

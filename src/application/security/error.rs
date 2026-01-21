#[derive(Debug)]
pub enum TokenError {
    Invalid,
    Expired,
    NotYetValid,
    InvalidSignature,
    InvalidIssuer,
    InvalidAudience,
    Malformed,
    Internal,
}

pub struct Token(String);

impl Token {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub struct RefreshToken(String);

impl RefreshToken {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub struct IssuedToken {
    pub token: Token,
    pub expires_in: u64,
    pub refresh_token: Option<RefreshToken>,
}

impl IssuedToken {
    pub fn new(token: Token, expires_in: u64, refresh_token: Option<RefreshToken>) -> Self {
        Self {
            token,
            expires_in,
            refresh_token,
        }
    }
}

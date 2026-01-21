use crate::{
    application::security::token::RefreshToken,
    domain::user::value_objects::{password_plain::PasswordPlain, username::Username},
};

pub enum Credentials {
    UsernamePassword {
        username: Username,
        password: PasswordPlain,
    },
    RefreshToken(RefreshToken),
}

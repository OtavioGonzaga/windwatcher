use super::value_objects::{password_hash::PasswordHash, username::Username};

pub struct UserPatch {
	pub name: Option<String>,
    pub username: Option<Username>,
    pub password_hash: Option<PasswordHash>,
}

impl UserPatch {
	pub fn new(
		name: Option<String>,
		username: Option<Username>,
		password_hash: Option<PasswordHash>,
	) -> Self {
		Self {
			name,
			username,
			password_hash,
		}
	}
}

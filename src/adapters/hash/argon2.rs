use std::io::Result;

use crate::domain::user::password_hasher::PasswordHasher as DomainPasswordHasher;
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};

pub struct Argon2Hasher;

#[async_trait::async_trait]
impl DomainPasswordHasher for Argon2Hasher {
    async fn hash(&self, plain: &str) -> Result<String> {
        let argon2 = Argon2::default();

        let salt: SaltString = SaltString::generate(&mut OsRng);

        let password_hash = argon2.hash_password(plain.as_bytes(), &salt).unwrap();

        Ok(password_hash.to_string())
    }

    async fn verify(&self, plain: &str, hash: &str) -> Result<bool> {
        let password_hash = PasswordHash::new(hash).unwrap();
        let argon2 = Argon2::default();

        Ok(argon2
            .verify_password(plain.as_bytes(), &password_hash)
            .is_ok())
    }
}

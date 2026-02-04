use crate::domain::user::password_hasher::PasswordHasher as DomainPasswordHasher;
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};

#[derive(Clone)]
pub struct Argon2Hasher;

impl DomainPasswordHasher for Argon2Hasher {
    fn hash(&self, plain: &str) -> String {
        let argon2: Argon2<'_> = Argon2::default();

        let salt: SaltString = SaltString::generate(&mut OsRng);

        let password_hash: PasswordHash<'_> =
            argon2.hash_password(plain.as_bytes(), &salt).unwrap();

        password_hash.to_string()
    }

    fn verify(&self, plain: &str, hash: &str) -> bool {
        let password_hash: PasswordHash<'_> = PasswordHash::new(hash).unwrap();
        let argon2: Argon2<'_> = Argon2::default();

        argon2
            .verify_password(plain.as_bytes(), &password_hash)
            .is_ok()
    }
}

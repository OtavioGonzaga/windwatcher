use std::io::Result;

use async_trait::async_trait;

#[async_trait]
pub trait PasswordHasher {
    async fn hash(&self, plain: &str) -> Result<String>;
    async fn verify(&self, plain: &str, hash: &str) -> Result<bool>;
}

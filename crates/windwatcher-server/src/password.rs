use std::{num::NonZeroUsize, sync::Arc};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use windwatcher::{
    Argon2idPasswordHasher, PasswordHash, PasswordHashError, PasswordHasher, SecretPassword,
};

/// Bounded Tokio blocking-pool wrapper around the engine's exact Argon2 adapter.
/// Clones share the concurrency budget. Requires a running Tokio runtime.
#[derive(Clone)]
pub struct TokioPasswordHasher {
    permits: Arc<Semaphore>,
}

impl Default for TokioPasswordHasher {
    fn default() -> Self {
        // Bound both CPU and Argon2 memory consumption, independently of peer quotas.
        Self::new(
            NonZeroUsize::new(
                std::thread::available_parallelism()
                    .map_or(1, NonZeroUsize::get)
                    .min(4),
            )
            .expect("positive worker count"),
        )
    }
}

impl TokioPasswordHasher {
    /// Constructs a shared budget for simultaneous Argon2 operations.
    pub fn new(workers: NonZeroUsize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(workers.get())),
        }
    }

    async fn permit(&self) -> OwnedSemaphorePermit {
        Arc::clone(&self.permits)
            .acquire_owned()
            .await
            .expect("private semaphore is never closed")
    }
}

impl PasswordHasher for TokioPasswordHasher {
    async fn hash(&self, password: &SecretPassword) -> Result<PasswordHash, PasswordHashError> {
        let permit = self.permit().await;
        let password = SecretPassword::new(password.expose_secret().to_owned())
            .expect("already validated secret bound");
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            // Owned by the worker, not the awaiting request: cancellation cannot
            // release the budget while expensive work is still running.
            let _permit = permit;
            runtime.block_on(Argon2idPasswordHasher.hash(&password))
        })
        .await
        .map_err(|error| PasswordHashError::HashingFailed(Box::new(error)))?
    }

    async fn verify(
        &self,
        password: &SecretPassword,
        expected: Option<&PasswordHash>,
    ) -> Result<bool, PasswordHashError> {
        let permit = self.permit().await;
        let password = SecretPassword::new(password.expose_secret().to_owned())
            .expect("already validated secret bound");
        let expected = expected.cloned();
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            runtime.block_on(Argon2idPasswordHasher.verify(&password, expected.as_ref()))
        })
        .await
        .map_err(|error| PasswordHashError::VerificationFailed(Box::new(error)))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn interoperates_with_existing_adapter_and_preserves_dummy_verification() {
        let adapter = TokioPasswordHasher::new(NonZeroUsize::new(1).unwrap());
        let secret = SecretPassword::new(" exact password\n".into()).unwrap();
        let hash = adapter.hash(&secret).await.unwrap();
        assert!(
            Argon2idPasswordHasher
                .verify(&secret, Some(&hash))
                .await
                .unwrap()
        );
        let existing = Argon2idPasswordHasher.hash(&secret).await.unwrap();
        assert!(adapter.verify(&secret, Some(&existing)).await.unwrap());
        let wrong = SecretPassword::new("wrong password".into()).unwrap();
        assert!(!adapter.verify(&wrong, Some(&hash)).await.unwrap());
        assert!(!adapter.verify(&secret, None).await.unwrap());
        let unsupported = PasswordHash::parse("$argon2id$v=42$m=19456,t=2,p=1$d2luZHdhdGNoZXItZHVtbXk$AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap();
        assert!(adapter.verify(&secret, Some(&unsupported)).await.is_err());
    }

    #[tokio::test]
    async fn clones_share_a_budget() {
        let adapter = TokioPasswordHasher::new(NonZeroUsize::new(1).unwrap());
        let permit = adapter.permit().await;
        assert!(adapter.clone().permits.try_acquire().is_err());
        drop(permit);
        assert!(adapter.permits.try_acquire().is_ok());
    }

    #[test]
    fn cancelling_a_request_keeps_queued_work_bounded_and_executor_responsive() {
        use std::{future::Future, task::Poll};
        let runtime = tokio::runtime::Builder::new_current_thread()
            .max_blocking_threads(1)
            .build()
            .unwrap();
        runtime.block_on(async {
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let blocker = tokio::task::spawn_blocking(move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            });
            started_rx.await.unwrap();
            let adapter = TokioPasswordHasher::new(NonZeroUsize::new(1).unwrap());
            let secret = SecretPassword::new("test password".into()).unwrap();
            let mut hashing = Box::pin(adapter.hash(&secret));
            std::future::poll_fn(|cx| {
                assert!(hashing.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            drop(hashing);
            assert!(adapter.permits.try_acquire().is_err());
            release_tx.send(()).unwrap();
            blocker.await.unwrap();
            let _permit = adapter.permit().await;
        });
    }
}

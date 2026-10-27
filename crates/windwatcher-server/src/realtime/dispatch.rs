use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

/// A bounded number of best-effort tasks, never an unbounded set of permit waiters.
pub(super) struct Dispatch {
    permits: Arc<Semaphore>,
    deadline: Duration,
}

impl Dispatch {
    pub(super) fn new(concurrency: usize, deadline: Duration) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(concurrency)),
            deadline,
        }
    }

    pub(super) fn submit(
        &self,
        event_type: &'static str,
        work: impl Future<Output = ()> + Send + 'static,
        expired: impl FnOnce() + Send + 'static,
    ) -> bool {
        let Ok(permit) = self.permits.clone().try_acquire_owned() else {
            tracing::warn!(event_type, "realtime_dispatch_saturated");
            return false;
        };
        let deadline = self.deadline;
        tokio::spawn(async move {
            let _permit = permit;
            if tokio::time::timeout(deadline, work).await.is_err() {
                tracing::warn!(event_type, "realtime_dispatch_timeout");
                expired();
            }
        });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test(start_paused = true)]
    async fn submission_never_waits_and_entire_job_times_out_releasing_capacity() {
        let dispatch = Dispatch::new(1, Duration::from_secs(2));
        let timed_out = Arc::new(AtomicBool::new(false));
        let flag = timed_out.clone();
        let (entered, waiting) = tokio::sync::oneshot::channel();
        assert!(dispatch.submit(
            "test.first",
            async {
                entered.send(()).unwrap();
                // Represents any phase, including projection hydration before audience lookup.
                std::future::pending::<()>().await;
            },
            move || {
                flag.store(true, Ordering::SeqCst);
            }
        ));
        // Capacity is reserved before spawn; no second task or waiter is created.
        assert!(!dispatch.submit(
            "test.dropped",
            async { panic!("must not be polled") },
            || {}
        ));
        waiting.await.unwrap();
        assert!(!timed_out.load(Ordering::SeqCst));
        tokio::time::advance(Duration::from_secs(2)).await;
        tokio::task::yield_now().await;
        assert!(timed_out.load(Ordering::SeqCst));
        assert_eq!(dispatch.permits.available_permits(), 1);
        let (sent, received) = tokio::sync::oneshot::channel();
        assert!(dispatch.submit(
            "test.next",
            async move {
                sent.send(()).unwrap();
            },
            || {}
        ));
        received.await.unwrap();
    }
}

use std::{future::Future, sync::Arc};

use time::OffsetDateTime;
use tokio::{
    sync::watch,
    task::JoinSet,
    time::{MissedTickBehavior, interval, sleep, timeout},
};
use windwatcher::{JobFailure, JobFailureCode, LeaseId, QueueStore, RetryOutcome, WorkerId};

use crate::{config::ResolvedQueueConfig, error::AppError, queue::ConfiguredQueue};

use super::{backoff, handler::JobExecutionError, registry::JobRegistry};

pub(crate) async fn run(
    queue: Arc<ConfiguredQueue>,
    config: ResolvedQueueConfig,
) -> Result<(), AppError> {
    let signal =
        crate::runtime::signal::ShutdownSignal::register().map_err(AppError::ShutdownSignal)?;
    run_until(
        queue,
        config,
        Arc::new(JobRegistry::default()),
        signal.wait(),
    )
    .await
}

async fn run_until(
    queue: Arc<ConfiguredQueue>,
    config: ResolvedQueueConfig,
    registry: Arc<JobRegistry>,
    shutdown: impl Future<Output = std::io::Result<()>>,
) -> Result<(), AppError> {
    let status = queue
        .migration_status()
        .await
        .map_err(|e| AppError::MigrationStatus(Box::new(e)))?;
    if !status.is_current() {
        return Err(AppError::QueueSchemaPending);
    }
    let worker = WorkerId::new();
    let (stop_tx, stop_rx) = watch::channel(false);
    let mut tasks = JoinSet::new();
    for slot in 0..config.concurrency {
        tasks.spawn(worker_slot(
            slot,
            worker,
            Arc::clone(&queue),
            Arc::clone(&registry),
            config.clone(),
            stop_rx.clone(),
        ));
    }
    tasks.spawn(maintenance_loop(
        Arc::clone(&queue),
        config.clone(),
        stop_rx,
    ));
    tracing::info!(worker_id = %worker, concurrency = config.concurrency,
        default_max_attempts = config.job_policy.default_max_attempts(),
        max_payload_bytes = config.job_policy.max_payload_bytes(), "worker_started");
    let trigger = tokio::select! {
        biased;
        _ = tasks.join_next() => Err(AppError::WorkerTaskTerminated),
        signal = shutdown => signal.map_err(AppError::ShutdownSignal),
    };
    let _ = stop_tx.send(true);
    tracing::info!(worker_id = %worker, "worker_stopping");
    let result = match trigger {
        Err(error) => Err(error),
        Ok(()) => match timeout(config.shutdown_grace_period, async {
            while let Some(result) = tasks.join_next().await {
                result.map_err(|_| AppError::WorkerTaskTerminated)?;
            }
            Ok(())
        })
        .await
        {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!(worker_id = %worker, "worker shutdown grace period expired; remaining leases will expire normally");
                Ok(())
            }
        },
    };
    // Unexpected termination is fatal, including during graceful drain. Do not
    // restart slots or release their leases; the process supervisor owns restart.
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    result
}

async fn worker_slot(
    slot: usize,
    worker: WorkerId,
    queue: Arc<ConfiguredQueue>,
    registry: Arc<JobRegistry>,
    config: ResolvedQueueConfig,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        if *stop.borrow() {
            return;
        }
        let now = OffsetDateTime::now_utc();
        let claim = tokio::select! {
            biased;
            _ = stop.changed() => return,
            result = queue.claim_next(worker, LeaseId::new(), config.lease_duration, now) => result,
        };
        match claim {
            Ok(Some(claimed)) => {
                tracing::info!(worker_id=%worker, job_id=%claimed.job().id(), lease_id=%claimed.lease().id(), kind=%claimed.job().kind(), payload_version=claimed.job().payload_version(), attempt=claimed.job().attempts(), max_attempts=claimed.job().max_attempts(), slot, "job_claimed");
                execute_claim(queue.as_ref(), registry.as_ref(), &config, claimed).await;
            }
            Ok(None) => {
                tokio::select! { _ = stop.changed() => return, _ = sleep(config.poll_interval) => {} }
            }
            Err(error) => {
                tracing::error!(worker_id=%worker, slot, error=%error, "queue claim failed");
                tokio::select! { _ = stop.changed() => return, _ = sleep(config.poll_interval) => {} }
            }
        }
    }
}

async fn execute_claim(
    queue: &ConfiguredQueue,
    registry: &JobRegistry,
    config: &ResolvedQueueConfig,
    claimed: windwatcher::ClaimedJob,
) {
    let job_id = claimed.job().id();
    let lease_id = claimed.lease().id();
    let worker_id = claimed.lease().worker_id();
    let kind = claimed.job().kind().clone();
    let version = claimed.job().payload_version();
    let attempt = claimed.job().attempts();
    let payload = claimed.job().payload().clone();
    let mut handler = Box::pin(if claimed.payload_invalid() {
        Box::pin(async {
            Err(JobExecutionError::Permanent {
                code: JobFailureCode::new("payload.invalid").expect("static failure code is valid"),
                message: "persisted job payload is invalid".into(),
            })
        }) as super::handler::HandlerFuture
    } else {
        registry.dispatch(&kind, version, payload)
    });
    let mut renewals = interval(config.lease_renew_interval);
    renewals.set_missed_tick_behavior(MissedTickBehavior::Delay);
    renewals.tick().await;
    let result = loop {
        tokio::select! {
            result = &mut handler => break Some(result),
            _ = renewals.tick() => {
                match queue.renew(job_id, lease_id, config.lease_duration, OffsetDateTime::now_utc()).await {
                    Ok(windwatcher::LeaseMutationOutcome::Applied) => tracing::debug!(worker_id=%worker_id, job_id=%job_id, lease_id=%lease_id, "lease_renewed"),
                    Ok(windwatcher::LeaseMutationOutcome::LeaseLost) => {
                        tracing::warn!(worker_id=%worker_id, job_id=%job_id, lease_id=%lease_id, "lease_lost");
                        break None;
                    }
                    Err(error) => {
                        tracing::error!(worker_id=%worker_id, job_id=%job_id, lease_id=%lease_id, error=%error, "lease renewal failed; cancelling handler");
                        break None;
                    }
                }
            }
        }
    };
    let Some(result) = result else {
        return;
    };
    let now = OffsetDateTime::now_utc();
    match result {
        Ok(()) => match queue.complete(job_id, lease_id, now).await {
            Ok(windwatcher::LeaseMutationOutcome::Applied) => {
                tracing::info!(worker_id=%worker_id, job_id=%job_id, lease_id=%lease_id, kind=%kind, attempt, "job_completed")
            }
            Ok(windwatcher::LeaseMutationOutcome::LeaseLost) => {
                tracing::warn!(worker_id=%worker_id, job_id=%job_id, lease_id=%lease_id, "lease_lost before completion")
            }
            Err(error) => {
                tracing::error!(worker_id=%worker_id, job_id=%job_id, lease_id=%lease_id, error=%error, "job completion persistence failed")
            }
        },
        Err(JobExecutionError::Permanent { code, message }) => {
            let failure = bounded_failure(code.clone(), message);
            match queue.fail(job_id, lease_id, &failure, now).await {
                Ok(windwatcher::LeaseMutationOutcome::Applied) => {
                    tracing::warn!(worker_id=%worker_id, job_id=%job_id, kind=%kind, attempt, failure_code=%code, "job_failed")
                }
                Ok(windwatcher::LeaseMutationOutcome::LeaseLost) => {
                    tracing::warn!(worker_id=%worker_id, job_id=%job_id, lease_id=%lease_id, "lease_lost before failure persistence")
                }
                Err(error) => {
                    tracing::error!(worker_id=%worker_id, job_id=%job_id, lease_id=%lease_id, error=%error, "job failure persistence failed")
                }
            }
        }
        Err(JobExecutionError::Retryable { code, message }) => {
            let failure = bounded_failure(code.clone(), message);
            let delay =
                backoff::retry_delay(attempt, config.retry_base_delay, config.retry_max_delay);
            let available_at = time::Duration::try_from(delay)
                .ok()
                .and_then(|delay| now.checked_add(delay))
                .unwrap_or(now);
            match queue
                .retry(job_id, lease_id, &failure, available_at, now)
                .await
            {
                Ok(RetryOutcome::Scheduled) => {
                    tracing::warn!(worker_id=%worker_id, job_id=%job_id, kind=%kind, attempt, failure_code=%code, delay_ms=delay.as_millis(), "job_retry_scheduled")
                }
                Ok(RetryOutcome::Exhausted) => {
                    tracing::warn!(worker_id=%worker_id, job_id=%job_id, kind=%kind, attempt, failure_code=%code, "job_failed_attempts_exhausted")
                }
                Ok(RetryOutcome::LeaseLost) => {
                    tracing::warn!(worker_id=%worker_id, job_id=%job_id, lease_id=%lease_id, "lease_lost before retry persistence")
                }
                Err(error) => {
                    tracing::error!(worker_id=%worker_id, job_id=%job_id, lease_id=%lease_id, error=%error, "job retry persistence failed")
                }
            }
        }
    }
}

fn bounded_failure(code: JobFailureCode, message: String) -> JobFailure {
    if let Ok(failure) = JobFailure::new(code.clone(), message.clone()) {
        return failure;
    }
    let safe_message = message
        .chars()
        .scan(0usize, |bytes, character| {
            let next = bytes.saturating_add(character.len_utf8());
            if next > 4096 {
                None
            } else {
                *bytes = next;
                Some(character)
            }
        })
        .collect::<String>();
    JobFailure::new(code, safe_message).unwrap_or_else(|_| {
        JobFailure::new(
            JobFailureCode::new("handler.invalid_failure").expect("static code is valid"),
            "handler returned an invalid diagnostic",
        )
        .expect("bounded static diagnostic")
    })
}

async fn maintenance_loop(
    queue: Arc<ConfiguredQueue>,
    config: ResolvedQueueConfig,
    mut stop: watch::Receiver<bool>,
) {
    let mut timer = interval(config.maintenance_interval);
    timer.set_missed_tick_behavior(MissedTickBehavior::Delay);
    timer.tick().await;
    loop {
        tokio::select! {
            _ = stop.changed() => return,
            _ = timer.tick() => match queue.maintain(OffsetDateTime::now_utc(), config.completed_retention).await {
                Ok(report) => tracing::info!(exhausted_jobs_failed=report.exhausted_jobs_failed(), completed_jobs_pruned=report.completed_jobs_pruned(), "maintenance_finished"),
                Err(error) => tracing::error!(error=%error, "queue maintenance failed"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use std::time::Duration;

    use serde::Deserialize;
    use tempfile::TempDir;
    use windwatcher::{
        EnqueueJob, EnqueueOptions, JobFailure, JobFailureCode, JobKind, JobPayload, JobPolicy,
        JobStatus, LeaseId, QueueListOptions, QueueStore, SqliteQueue, SqliteQueueOptions,
        WorkerId,
    };

    use super::*;
    use crate::config::ResolvedQueueConfig;

    #[derive(Deserialize)]
    struct TestPayload {
        value: String,
    }

    fn settings() -> ResolvedQueueConfig {
        ResolvedQueueConfig {
            job_policy: JobPolicy::default(),
            concurrency: 2,
            lease_duration: Duration::from_millis(80),
            lease_renew_interval: Duration::from_millis(10),
            poll_interval: Duration::from_millis(5),
            retry_base_delay: Duration::from_millis(10),
            retry_max_delay: Duration::from_millis(100),
            completed_retention: Duration::from_secs(3600),
            maintenance_interval: Duration::from_secs(3600),
            shutdown_grace_period: Duration::from_millis(100),
            provider: crate::config::ResolvedQueueProvider::Sqlite {
                options: windwatcher::SqliteQueueOptions::new(":memory:"),
                create_if_missing: true,
                derived_path: false,
            },
        }
    }

    async fn queue() -> (TempDir, Arc<ConfiguredQueue>) {
        let dir = TempDir::new().unwrap();
        let url = format!("sqlite://{}", dir.path().join("queue.db").display());
        let inner = SqliteQueue::connect(SqliteQueueOptions::new(url))
            .await
            .unwrap();
        inner.migrate().await.unwrap();
        (dir, Arc::new(ConfiguredQueue::Sqlite(inner)))
    }

    async fn enqueue(
        queue: &ConfiguredQueue,
        value: &str,
        now: OffsetDateTime,
    ) -> windwatcher::Job {
        EnqueueJob::new(queue, JobPolicy::default())
            .execute(
                JobKind::new("test.handle").unwrap(),
                1,
                JobPayload::try_from(serde_json::json!({"value": value})).unwrap(),
                EnqueueOptions::new(),
                now,
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn handler_renews_a_long_claim_and_completes() {
        let (_dir, queue) = queue().await;
        let now = OffsetDateTime::now_utc();
        let job = enqueue(&queue, "long", now).await;
        let claim = queue
            .claim_next(
                WorkerId::new(),
                LeaseId::new(),
                Duration::from_millis(80),
                now,
            )
            .await
            .unwrap()
            .unwrap();
        let mut registry = JobRegistry::default();
        registry.register::<TestPayload, _, _>(
            JobKind::new("test.handle").unwrap(),
            1,
            |payload| async move {
                tokio::time::sleep(Duration::from_millis(120)).await;
                assert_eq!(payload.value, "long");
                Ok(())
            },
        );
        execute_claim(&queue, &registry, &settings(), claim).await;
        let rows = queue
            .list(QueueListOptions::new(Some(JobStatus::Completed), 10).unwrap())
            .await
            .unwrap();
        assert_eq!(rows[0].id(), job.id());
    }

    #[tokio::test]
    async fn lease_loss_drops_the_handler_future() {
        let (_dir, queue) = queue().await;
        let now = OffsetDateTime::now_utc();
        let job = enqueue(&queue, "cancel", now).await;
        let claim = queue
            .claim_next(WorkerId::new(), LeaseId::new(), Duration::from_secs(2), now)
            .await
            .unwrap()
            .unwrap();
        let lease = claim.lease().id();
        let started = Arc::new(tokio::sync::Notify::new());
        let started_handler = Arc::clone(&started);
        let completed = Arc::new(AtomicBool::new(false));
        let completed_handler = Arc::clone(&completed);
        let mut registry = JobRegistry::default();
        registry.register::<TestPayload, _, _>(
            JobKind::new("test.handle").unwrap(),
            1,
            move |_| {
                let started = Arc::clone(&started_handler);
                let completed = Arc::clone(&completed_handler);
                async move {
                    started.notify_one();
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    completed.store(true, Ordering::SeqCst);
                    Ok(())
                }
            },
        );
        let run_queue = Arc::clone(&queue);
        let run = tokio::spawn(async move {
            execute_claim(&run_queue, &registry, &settings(), claim).await;
        });
        started.notified().await;
        let failure = JobFailure::new(
            JobFailureCode::new("test.external").unwrap(),
            "lease invalidated",
        )
        .unwrap();
        assert_eq!(
            queue
                .fail(job.id(), lease, &failure, OffsetDateTime::now_utc())
                .await
                .unwrap(),
            windwatcher::LeaseMutationOutcome::Applied
        );
        run.await.unwrap();
        assert!(!completed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn concurrent_claimed_handlers_run_without_exceeding_available_slots() {
        let (_dir, queue) = queue().await;
        let now = OffsetDateTime::now_utc();
        enqueue(&queue, "one", now).await;
        enqueue(&queue, "two", now).await;
        let first = queue
            .claim_next(WorkerId::new(), LeaseId::new(), Duration::from_secs(2), now)
            .await
            .unwrap()
            .unwrap();
        let second = queue
            .claim_next(WorkerId::new(), LeaseId::new(), Duration::from_secs(2), now)
            .await
            .unwrap()
            .unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let mut registry = JobRegistry::default();
        let active_handler = Arc::clone(&active);
        let maximum_handler = Arc::clone(&maximum);
        registry.register::<TestPayload, _, _>(
            JobKind::new("test.handle").unwrap(),
            1,
            move |_| {
                let active = Arc::clone(&active_handler);
                let maximum = Arc::clone(&maximum_handler);
                async move {
                    let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(count, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                }
            },
        );
        let settings = settings();
        tokio::join!(
            execute_claim(&queue, &registry, &settings, first),
            execute_claim(&queue, &registry, &settings, second)
        );
        assert_eq!(maximum.load(Ordering::SeqCst), 2);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn retryable_handler_failure_schedules_using_queue_state() {
        let (_dir, queue) = queue().await;
        let now = OffsetDateTime::now_utc();
        let job = enqueue(&queue, "retry", now).await;
        let claim = queue
            .claim_next(WorkerId::new(), LeaseId::new(), Duration::from_secs(2), now)
            .await
            .unwrap()
            .unwrap();
        let mut registry = JobRegistry::default();
        registry.register::<TestPayload, _, _>(
            JobKind::new("test.handle").unwrap(),
            1,
            |_| async {
                Err(JobExecutionError::Retryable {
                    code: JobFailureCode::new("provider.timeout").unwrap(),
                    message: "temporarily unavailable".into(),
                })
            },
        );
        execute_claim(&queue, &registry, &settings(), claim).await;
        let rows = queue
            .list(QueueListOptions::new(Some(JobStatus::Pending), 10).unwrap())
            .await
            .unwrap();
        assert_eq!(rows[0].id(), job.id());
        assert_eq!(rows[0].attempts(), 1);
        assert_eq!(
            rows[0].last_failure().unwrap().code().as_str(),
            "provider.timeout"
        );
    }

    #[tokio::test]
    async fn malformed_persisted_payload_fails_permanently_without_dispatch() {
        let (dir, queue) = queue().await;
        let now = OffsetDateTime::now_utc();
        let job = enqueue(&queue, "corrupt", now).await;
        let pool = sqlx::SqlitePool::connect(&format!(
            "sqlite://{}",
            dir.path().join("queue.db").display()
        ))
        .await
        .unwrap();
        sqlx::query("UPDATE queue_jobs SET payload_json='not-json' WHERE id=?")
            .bind(job.id().as_uuid().as_bytes().to_vec())
            .execute(&pool)
            .await
            .unwrap();
        let claim = queue
            .claim_next(WorkerId::new(), LeaseId::new(), Duration::from_secs(2), now)
            .await
            .unwrap()
            .unwrap();
        assert!(claim.payload_invalid());
        execute_claim(&queue, &JobRegistry::default(), &settings(), claim).await;
        let failed = queue
            .list(QueueListOptions::new(Some(JobStatus::Failed), 10).unwrap())
            .await
            .unwrap();
        assert_eq!(
            failed[0].last_failure().unwrap().code().as_str(),
            "payload.invalid"
        );
    }

    struct Dropped(Arc<AtomicBool>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn handler_panic_fails_runtime_and_cancels_other_slots_without_releasing_leases() {
        let (_dir, queue) = queue().await;
        let now = OffsetDateTime::now_utc();
        enqueue(&queue, "panic", now).await;
        enqueue(&queue, "hang", now).await;
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let dropped = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&dropped);
        let mut registry = JobRegistry::default();
        registry.register::<TestPayload, _, _>(
            JobKind::new("test.handle").unwrap(),
            1,
            move |payload| {
                let barrier = Arc::clone(&barrier);
                let dropped = Arc::clone(&dropped);
                async move {
                    let _guard = (payload.value == "hang").then(|| Dropped(dropped));
                    barrier.wait().await;
                    assert_ne!(payload.value, "panic", "test handler panic");
                    std::future::pending::<()>().await;
                    Ok(())
                }
            },
        );
        let mut config = settings();
        config.lease_duration = Duration::from_secs(30);
        config.shutdown_grace_period = Duration::from_secs(30);
        let result = timeout(
            Duration::from_secs(2),
            run_until(
                Arc::clone(&queue),
                config,
                Arc::new(registry),
                std::future::pending(),
            ),
        )
        .await
        .expect("must fail before any signal or grace timeout");
        assert!(matches!(result, Err(AppError::WorkerTaskTerminated)));
        assert!(observed.load(Ordering::SeqCst));
        let rows = queue
            .list(QueueListOptions::new(Some(JobStatus::Leased), 10).unwrap())
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|job| job.attempts() == 1));
        let after_expiry = rows
            .iter()
            .map(|job| job.lease().unwrap().expires_at())
            .max()
            .unwrap()
            + time::Duration::seconds(1);
        let recovered = queue
            .claim_next(
                WorkerId::new(),
                LeaseId::new(),
                Duration::from_secs(10),
                after_expiry,
            )
            .await
            .unwrap()
            .unwrap();
        let previous = rows
            .iter()
            .find(|job| job.id() == recovered.job().id())
            .unwrap();
        assert_eq!(recovered.job().attempts(), 2);
        assert_ne!(recovered.lease().id(), previous.lease().unwrap().id());
    }

    #[tokio::test]
    async fn injected_shutdown_drains_and_renews_without_claiming_more_work() {
        let (_dir, queue) = queue().await;
        let now = OffsetDateTime::now_utc();
        enqueue(&queue, "drain", now).await;
        enqueue(&queue, "next", now).await;
        let started = Arc::new(tokio::sync::Notify::new());
        let started_handler = Arc::clone(&started);
        let mut registry = JobRegistry::default();
        registry.register::<TestPayload, _, _>(
            JobKind::new("test.handle").unwrap(),
            1,
            move |_| {
                let started = Arc::clone(&started_handler);
                async move {
                    started.notify_one();
                    sleep(Duration::from_millis(150)).await;
                    Ok(())
                }
            },
        );
        let mut config = settings();
        config.concurrency = 1;
        config.shutdown_grace_period = Duration::from_secs(2);
        timeout(
            Duration::from_secs(3),
            run_until(Arc::clone(&queue), config, Arc::new(registry), async {
                started.notified().await;
                Ok(())
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            queue
                .list(QueueListOptions::new(Some(JobStatus::Completed), 10).unwrap())
                .await
                .unwrap()
                .len(),
            1
        );
        let pending = queue
            .list(QueueListOptions::new(Some(JobStatus::Pending), 10).unwrap())
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].attempts(), 0);
    }

    #[tokio::test]
    async fn grace_timeout_drops_handler_but_leaves_claim_for_expiry() {
        let (_dir, queue) = queue().await;
        enqueue(&queue, "hang", OffsetDateTime::now_utc()).await;
        let started = Arc::new(tokio::sync::Notify::new());
        let started_handler = Arc::clone(&started);
        let dropped = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&dropped);
        let mut registry = JobRegistry::default();
        registry.register::<TestPayload, _, _>(
            JobKind::new("test.handle").unwrap(),
            1,
            move |_| {
                let started = Arc::clone(&started_handler);
                let dropped = Arc::clone(&dropped);
                async move {
                    let _guard = Dropped(dropped);
                    started.notify_one();
                    std::future::pending::<()>().await;
                    Ok(())
                }
            },
        );
        timeout(
            Duration::from_secs(2),
            run_until(Arc::clone(&queue), settings(), Arc::new(registry), async {
                started.notified().await;
                Ok(())
            }),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(observed.load(Ordering::SeqCst));
        let leased = queue
            .list(QueueListOptions::new(Some(JobStatus::Leased), 10).unwrap())
            .await
            .unwrap();
        assert_eq!(leased.len(), 1);
        assert_eq!(leased[0].attempts(), 1);
    }

    #[tokio::test]
    async fn shutdown_listener_error_is_a_runtime_error() {
        let (_dir, queue) = queue().await;
        let result = run_until(queue, settings(), Arc::new(JobRegistry::default()), async {
            Err(std::io::Error::other("test signal failure"))
        })
        .await;
        assert!(matches!(result, Err(AppError::ShutdownSignal(_))));
    }

    #[tokio::test]
    async fn handler_panic_during_grace_is_not_reported_as_successful_shutdown() {
        let (_dir, queue) = queue().await;
        enqueue(&queue, "panic", OffsetDateTime::now_utc()).await;
        let started = Arc::new(tokio::sync::Notify::new());
        let handler_started = Arc::clone(&started);
        let draining = Arc::new(tokio::sync::Notify::new());
        let handler_draining = Arc::clone(&draining);
        let mut registry = JobRegistry::default();
        registry.register::<TestPayload, _, _>(
            JobKind::new("test.handle").unwrap(),
            1,
            move |_| {
                let started = Arc::clone(&handler_started);
                let draining = Arc::clone(&handler_draining);
                async move {
                    started.notify_one();
                    draining.notified().await;
                    panic!("test panic during graceful drain");
                }
            },
        );
        let result = timeout(
            Duration::from_secs(2),
            run_until(queue, settings(), Arc::new(registry), async {
                started.notified().await;
                draining.notify_one();
                Ok(())
            }),
        )
        .await
        .unwrap();
        assert!(matches!(result, Err(AppError::WorkerTaskTerminated)));
    }
}

use crate::{
    config,
    connection::{self, ConnectionPurpose},
    error::AppError,
    paths::ResolvedPaths,
    queue::QueueMigrationState,
};

struct DoctorCheck {
    name: &'static str,
    healthy: bool,
    detail: String,
    hint: Option<&'static str>,
}

impl DoctorCheck {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            healthy: true,
            detail: detail.into(),
            hint: None,
        }
    }

    fn fail(name: &'static str, detail: impl Into<String>, hint: Option<&'static str>) -> Self {
        Self {
            name,
            healthy: false,
            detail: detail.into(),
            hint,
        }
    }
}

fn report(checks: &[DoctorCheck]) -> Result<(), AppError> {
    for check in checks {
        println!(
            "[{}] {}: {}",
            if check.healthy { "ok" } else { "fail" },
            check.name,
            check.detail
        );
        if let Some(hint) = check.hint {
            println!("       hint: {hint}");
        }
    }
    let healthy = checks.iter().all(|check| check.healthy);
    println!("Doctor: {}", if healthy { "healthy" } else { "unhealthy" });
    if healthy {
        Ok(())
    } else {
        Err(AppError::DoctorUnhealthy)
    }
}

pub(crate) fn configuration_failure() -> AppError {
    // Parser diagnostics can quote input values, including credentials. Doctor
    // intentionally uses a fixed message even for deserialization failures.
    let _ = report(&[DoctorCheck::fail(
        "configuration",
        "invalid",
        Some("review configuration with `windwatcher config check`"),
    )]);
    AppError::DoctorUnhealthy
}

pub(crate) async fn run(raw: &config::RawConfig, paths: &ResolvedPaths) -> Result<(), AppError> {
    if config::validate_full(raw, paths).await.is_err() {
        return Err(configuration_failure());
    }
    let (storage, queue) = tokio::join!(storage_checks(raw, paths), queue_checks(raw, paths));
    let mut checks = vec![DoctorCheck::ok("configuration", "valid")];
    checks.extend(storage);
    checks.extend(queue);
    report(&checks)
}

async fn storage_checks(raw: &config::RawConfig, paths: &ResolvedPaths) -> Vec<DoctorCheck> {
    let storage = match connection::connect_storage(raw, paths, ConnectionPurpose::ExistingOnly)
        .await
    {
        Ok(storage) => storage,
        Err(error) => return vec![DoctorCheck::fail("storage", error.operator_message(), None)],
    };
    let mut checks = vec![DoctorCheck::ok("storage", storage.backend_name())];
    checks.push(match storage.migration_status().await {
        Ok(status) if status.is_current() => DoctorCheck::ok("storage migrations", "current"),
        Ok(_) => DoctorCheck::fail(
            "storage migrations",
            "pending",
            AppError::SchemaPending.hint(),
        ),
        Err(error) => DoctorCheck::fail(
            "storage migrations",
            AppError::MigrationStatus(error).operator_message(),
            None,
        ),
    });
    checks
}

async fn queue_checks(raw: &config::RawConfig, paths: &ResolvedPaths) -> Vec<DoctorCheck> {
    if raw.queue.is_none() {
        return vec![DoctorCheck::ok("queue", "disabled")];
    }
    let queue = match connection::connect_queue(raw, paths, ConnectionPurpose::ExistingOnly).await {
        Ok(queue) => queue,
        Err(error) => return vec![DoctorCheck::fail("queue", error.operator_message(), None)],
    };
    let mut checks = vec![DoctorCheck::ok("queue", queue.backend_name())];
    checks.push(match queue.migration_status().await {
        Ok(QueueMigrationState::NotApplicable) => {
            DoctorCheck::ok("queue migrations", "not applicable")
        }
        Ok(status) if status.is_current() => DoctorCheck::ok("queue migrations", "current"),
        Ok(_) => DoctorCheck::fail(
            "queue migrations",
            "pending",
            AppError::QueueSchemaPending.hint(),
        ),
        Err(error) => DoctorCheck::fail(
            "queue migrations",
            AppError::MigrationStatus(Box::new(error)).operator_message(),
            None,
        ),
    });
    checks
}

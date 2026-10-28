use std::{
    fs,
    io::{self, IsTerminal, Read},
    path::Path,
};

use time::OffsetDateTime;
use windwatcher::{
    Argon2idPasswordHasher, CreateLocalUser, CreateLocalUserError, JobId, JobStatus,
    QueueListOptions, QueueStore, RetryFailedOutcome, SecretPassword,
};
use zeroize::Zeroize;

use crate::{
    cli::{CreateUserArgs, SensitiveString},
    error::AppError,
    queue::ConfiguredQueue,
    storage::Storage,
};

pub async fn jobs_list(
    queue: &ConfiguredQueue,
    status: Option<JobStatus>,
    limit: u32,
) -> Result<(), AppError> {
    let options = QueueListOptions::new(status, limit)
        .map_err(|error| AppError::Config(error.to_string()))?;
    let jobs = queue
        .list(options)
        .await
        .map_err(|e| AppError::QueueOperation(Box::new(e)))?;
    println!(
        "ID\tKIND\tVERSION\tSTATUS\tATTEMPTS\tMAX\tAVAILABLE\tCREATED\tLEASE EXPIRES\tERROR CODE"
    );
    for job in jobs {
        println!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            job.id(),
            job.kind(),
            job.payload_version(),
            status_name(job.status()),
            job.attempts(),
            job.max_attempts(),
            job.available_at(),
            job.created_at(),
            job.lease()
                .map(|lease| lease.expires_at().to_string())
                .unwrap_or_default(),
            job.last_failure()
                .map(|failure| failure.code().as_str())
                .unwrap_or("")
        );
    }
    Ok(())
}

fn status_name(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Pending => "pending",
        JobStatus::Leased => "leased",
        JobStatus::Completed => "completed",
        JobStatus::Failed => "failed",
    }
}

pub async fn jobs_retry(queue: &ConfiguredQueue, raw_id: &str) -> Result<(), AppError> {
    let id = raw_id
        .parse::<JobId>()
        .map_err(|_| AppError::Config("invalid job ID; expected a UUIDv7".into()))?;
    match queue
        .retry_failed(id, OffsetDateTime::now_utc())
        .await
        .map_err(|e| AppError::QueueOperation(Box::new(e)))?
    {
        RetryFailedOutcome::Retried => println!("Job {id} retried."),
        RetryFailedOutcome::NotFound => println!("Job {id} was not found."),
        RetryFailedOutcome::NotFailed => {
            println!("Job {id} is not failed; only failed jobs can be retried.")
        }
    }
    Ok(())
}

pub async fn migrate(storage: &Storage) -> Result<(), AppError> {
    let before = storage
        .migration_status()
        .await
        .map_err(|error| AppError::MigrationStatus(Box::new(error)))?;
    storage
        .migrate()
        .await
        .map_err(|error| AppError::Migration(Box::new(error)))?;
    let after = storage
        .migration_status()
        .await
        .map_err(|error| AppError::MigrationStatus(Box::new(error)))?;
    if before.pending() == 0 {
        println!("No pending migrations.");
    } else {
        println!("Applied {} migration(s).", before.pending());
    }
    println!(
        "Schema is current at version {}.",
        after.latest_version().unwrap_or_default()
    );
    Ok(())
}

pub async fn migration_status(storage: &Storage) -> Result<(), AppError> {
    let status = storage
        .migration_status()
        .await
        .map_err(|error| AppError::MigrationStatus(Box::new(error)))?;
    println!("Storage: {}", storage.backend_name());
    println!(
        "Current version: {}",
        status
            .current_version()
            .map_or_else(|| "none".into(), |version| version.to_string())
    );
    println!(
        "Latest version: {}",
        status
            .latest_version()
            .map_or_else(|| "none".into(), |version| version.to_string())
    );
    println!("Pending migrations: {}", status.pending());
    println!(
        "Status: {}",
        if status.is_current() {
            "current"
        } else {
            "pending"
        }
    );
    Ok(())
}

pub async fn create_user(
    storage: &Storage,
    args: CreateUserArgs,
    argument_password: Option<SensitiveString>,
) -> Result<(), AppError> {
    let status = storage
        .migration_status()
        .await
        .map_err(|error| AppError::MigrationStatus(Box::new(error)))?;
    if !status.is_current() {
        return Err(AppError::SchemaPending);
    }
    let password = password(&args, argument_password)?;
    tracing::info!(username = %args.username, "user provisioning started");
    let hasher = Argon2idPasswordHasher;
    let user = CreateLocalUser::new(storage, &hasher)
        .execute(&args.username, &password, OffsetDateTime::now_utc())
        .await
        .map_err(|error| match error {
            CreateLocalUserError::UsernameTaken => AppError::UsernameTaken {
                username: args.username.clone(),
            },
            other => AppError::UserCreation(other),
        })?;
    println!(
        "Created user {:?} ({})",
        user.username().as_str(),
        user.id()
    );
    tracing::info!(user_id = %user.id(), "user provisioning succeeded");
    Ok(())
}

fn password(
    args: &CreateUserArgs,
    argument_password: Option<SensitiveString>,
) -> Result<SecretPassword, AppError> {
    let value = match (argument_password, args.password_stdin, &args.password_file) {
        (Some(password), false, None) => password,
        (None, true, None) => read_password(&mut io::stdin())?,
        (None, false, Some(path)) => read_password_file(path)?,
        (None, false, None) => {
            if !io::stdin().is_terminal() {
                return Err(AppError::PasswordInput("stdin is not interactive; use --password, --password-stdin, or --password-file".into()));
            }
            let first = SensitiveString::new(
                rpassword::prompt_password("Password: ")
                    .map_err(|error| AppError::PasswordInput(error.to_string()))?,
            );
            let confirmation = SensitiveString::new(
                rpassword::prompt_password("Confirm password: ")
                    .map_err(|error| AppError::PasswordInput(error.to_string()))?,
            );
            if first.as_str() != confirmation.as_str() {
                return Err(AppError::PasswordInput(
                    "password confirmation does not match".into(),
                ));
            }
            first
        }
        _ => {
            return Err(AppError::PasswordInput(
                "password sources are mutually exclusive".into(),
            ));
        }
    };
    SecretPassword::new(value.into_inner())
        .map_err(|error| AppError::PasswordInput(error.to_string()))
}

fn read_password_file(path: &Path) -> Result<SensitiveString, AppError> {
    fs::File::open(path)
        .map_err(|error| {
            AppError::PasswordInput(format!(
                "failed to read password file {}: {error}",
                path.display()
            ))
        })
        .and_then(|mut file| read_password(&mut file))
}
fn read_password(reader: &mut impl Read) -> Result<SensitiveString, AppError> {
    let mut bytes = Vec::with_capacity(1025);
    if let Err(error) = reader.take(1025).read_to_end(&mut bytes) {
        bytes.zeroize();
        return Err(AppError::PasswordInput(error.to_string()));
    }
    if bytes.len() > 1024 {
        bytes.zeroize();
        return Err(AppError::PasswordInput(
            "password must contain at most 1024 UTF-8 bytes".into(),
        ));
    }
    let value = String::from_utf8(bytes).map_err(|error| {
        error.into_bytes().zeroize();
        AppError::PasswordInput("password input must be valid UTF-8".into())
    })?;
    Ok(SensitiveString::new(value))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::read_password;

    #[test]
    fn password_input_preserves_exact_utf8_bytes() {
        assert_eq!(
            read_password(&mut Cursor::new(" leading\ntrailing "))
                .unwrap()
                .as_str(),
            " leading\ntrailing "
        );
        assert_eq!(
            read_password(&mut Cursor::new("å".repeat(512)))
                .unwrap()
                .as_str()
                .len(),
            1024
        );
    }

    #[test]
    fn password_input_rejects_invalid_utf8_and_oversized_values() {
        assert!(read_password(&mut Cursor::new(vec![0xff])).is_err());
        assert!(read_password(&mut Cursor::new(vec![b'x'; 1025])).is_err());
    }
}

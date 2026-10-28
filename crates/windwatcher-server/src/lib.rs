//! Composition for the official Windwatcher server, separate from the reusable engine.

mod cli;
mod commands;
mod config;
mod error;
pub mod http;
mod logging;
mod password;
mod paths;
mod queue;
mod realtime;
mod storage;
mod worker;

pub use config::http::{RawCors, RawHttp, RawRateLimit, RawRateLimitPolicy, ResolvedHttpConfig};
pub use config::realtime::{RawRealtime, ResolvedRealtimeConfig};
pub use password::TokioPasswordHasher;

use clap::Parser;
use cli::{Cli, Command, JobStatusArg, JobsCommand, MigrateCommand};
use error::AppError;
use paths::{ResolvedPaths, RuntimeMode};
pub use storage::Storage;

/// Runs the operator CLI, reporting safe errors and exiting unsuccessfully on failure.
pub async fn run_cli() {
    if let Err(error) = run().await {
        eprintln!("error: {}", error.operator_message());
        if let Some(hint) = error.hint() {
            eprintln!("hint: {hint}");
        }
        std::process::exit(1);
    }
}

async fn run() -> Result<(), AppError> {
    let mut cli = Cli::parse();
    let argument_password = match &mut cli.command {
        Command::User {
            command: cli::UserCommand::Create(args),
        } => args.password.take().map(cli::SensitiveString::new),
        _ => None,
    };
    if argument_password.is_some() {
        eprintln!(
            "warning: --password may expose the password through process arguments; prefer interactive input, --password-stdin, or --password-file"
        );
    }
    let mode = if cli.system {
        RuntimeMode::System
    } else {
        RuntimeMode::User
    };
    let paths = ResolvedPaths::resolve(mode)?;
    let raw = config::load(&paths, cli.config)?;
    logging::initialize(&raw.logging, cli.log_filter.as_deref(), cli.log_format)?;
    tracing::info!(mode = ?mode, "configuration loaded");
    match cli.command {
        Command::Config {
            command: cli::ConfigCommand::Check,
        } => {
            config::resolve_full(&raw, &paths)?;
            println!(
                "Configuration valid.\nMode: {}\nStorage: {}\nQueue: {}",
                if cli.system { "system" } else { "user" },
                match config::resolve_storage(&raw, &paths)? {
                    config::ResolvedStorageProvider::Sqlite { .. } => "SQLite",
                    config::ResolvedStorageProvider::Postgres(_) => "PostgreSQL",
                    config::ResolvedStorageProvider::MySql(_) => "MySQL",
                },
                if raw.queue.is_some() {
                    config::resolve_queue(&raw, &paths)?.provider.backend_name()
                } else {
                    "disabled"
                }
            );
            Ok(())
        }
        Command::Migrate(args) => {
            let explicit = args.storage || args.queue;
            let do_storage = args.storage || !explicit;
            let do_queue = args.queue || (!explicit && raw.queue.is_some());
            if do_storage {
                let storage = connect_storage(&raw, &paths).await?;
                match args.command {
                    Some(MigrateCommand::Status) => commands::migration_status(&storage).await?,
                    None => commands::migrate(&storage).await?,
                }
            }
            if do_queue {
                let queue = connect_queue(&raw, &paths).await?;
                let status = queue
                    .migration_status()
                    .await
                    .map_err(|e| AppError::MigrationStatus(Box::new(e)))?;
                if let queue::QueueMigrationState::Applicable(status) = status {
                    if matches!(args.command, Some(MigrateCommand::Status)) {
                        println!("Queue: {}", queue.backend_name());
                        println!(
                            "Current version: {}",
                            status
                                .current_version()
                                .map_or_else(|| "none".into(), |v| v.to_string())
                        );
                        println!(
                            "Latest version: {}",
                            status
                                .latest_version()
                                .map_or_else(|| "none".into(), |v| v.to_string())
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
                    } else {
                        let pending = status.pending();
                        queue
                            .migrate()
                            .await
                            .map_err(|e| AppError::Migration(Box::new(e)))?;
                        println!(
                            "Queue: {}",
                            if pending == 0 {
                                "No pending migrations.".to_owned()
                            } else {
                                format!("Applied {pending} migration(s).")
                            }
                        );
                    }
                } else {
                    println!(
                        "Queue ({}): migrations not applicable",
                        queue.backend_name()
                    );
                }
            }
            Ok(())
        }
        Command::Worker => {
            let resolved = config::resolve_queue(&raw, &paths)?;
            let queue = connect_queue(&raw, &paths).await?;
            worker::run(queue, resolved).await
        }
        Command::Jobs { command } => {
            let queue = connect_queue(&raw, &paths).await?;
            let status = queue
                .migration_status()
                .await
                .map_err(|e| AppError::MigrationStatus(Box::new(e)))?;
            if !status.is_current() {
                return Err(AppError::QueueSchemaPending);
            }
            match command {
                JobsCommand::List { status, limit } => {
                    let status = status.map(|value| match value {
                        JobStatusArg::Pending => windwatcher::JobStatus::Pending,
                        JobStatusArg::Leased => windwatcher::JobStatus::Leased,
                        JobStatusArg::Completed => windwatcher::JobStatus::Completed,
                        JobStatusArg::Failed => windwatcher::JobStatus::Failed,
                    });
                    commands::jobs_list(&queue, status, limit).await
                }
                JobsCommand::Retry { job_id } => commands::jobs_retry(&queue, &job_id).await,
            }
        }
        Command::User {
            command: cli::UserCommand::Create(args),
        } => {
            let storage = connect_storage(&raw, &paths).await?;
            commands::create_user(&storage, args, argument_password).await
        }
    }
}

async fn connect_queue(
    raw: &config::RawConfig,
    paths: &ResolvedPaths,
) -> Result<queue::ConfiguredQueue, AppError> {
    let resolved = config::resolve_queue(raw, paths)?;
    if resolved.provider.needs_data_directory() {
        std::fs::create_dir_all(&paths.data_dir).map_err(|source| AppError::DataDirectory {
            path: paths.data_dir.clone(),
            source,
        })?;
    }
    let backend = resolved.provider.backend_name();
    let queue = queue::ConfiguredQueue::connect(resolved.provider)
        .await
        .map_err(|error| AppError::QueueConnection {
            backend,
            source: Box::new(error),
        })?;
    tracing::info!(queue_backend = backend, "queue backend selected");
    Ok(queue)
}

async fn connect_storage(
    raw: &config::RawConfig,
    paths: &ResolvedPaths,
) -> Result<Storage, AppError> {
    let storage = match config::resolve_storage(raw, paths)? {
        config::ResolvedStorageProvider::Sqlite {
            options,
            derived_path,
            create_if_missing,
        } => {
            if derived_path && create_if_missing {
                std::fs::create_dir_all(&paths.data_dir).map_err(|source| {
                    AppError::DataDirectory {
                        path: paths.data_dir.clone(),
                        source,
                    }
                })?;
            }
            Storage::connect(options)
                .await
                .map_err(|error| AppError::Storage {
                    backend: "SQLite",
                    source: Box::new(error),
                })?
        }
        provider @ config::ResolvedStorageProvider::Postgres(_)
        | provider @ config::ResolvedStorageProvider::MySql(_) => {
            let backend = match &provider {
                config::ResolvedStorageProvider::Postgres(_) => "PostgreSQL",
                _ => "MySQL",
            };
            Storage::connect_provider(provider)
                .await
                .map_err(|source| AppError::Storage { backend, source })?
        }
    };
    tracing::info!(backend = storage.backend_name(), "storage backend selected");
    Ok(storage)
}

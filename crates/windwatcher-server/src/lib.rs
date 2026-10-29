//! Composition for the official Windwatcher server, separate from the reusable engine.

mod cli;
mod commands;
mod config;
mod connection;
mod error;
pub mod http;
mod logging;
mod password;
mod paths;
mod queue;
mod realtime;
mod runtime;
mod storage;
mod worker;

pub use config::http::{RawCors, RawHttp, RawRateLimit, RawRateLimitPolicy, ResolvedHttpConfig};
pub use config::realtime::{RawRealtime, ResolvedRealtimeConfig};
pub use password::TokioPasswordHasher;

use clap::Parser;
use cli::{Cli, Command, JobStatusArg, JobsCommand, MigrateCommand};
use connection::{ConnectionPurpose, connect_queue, connect_storage};
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
    let mut command = cli.command.take().unwrap_or(Command::Serve);
    let argument_password = match &mut command {
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
    match command {
        Command::Serve => runtime::serve::run(&raw, &paths).await,
        Command::Config {
            command: cli::ConfigCommand::Check,
        } => {
            config::validate_full(&raw, &paths).await?;
            let storage = config::resolve_storage(&raw, &paths)?;
            println!(
                "Configuration valid.\nMode: {}\nStorage: {}\nQueue: {}",
                if cli.system { "system" } else { "user" },
                match storage {
                    config::ResolvedStorageProvider::Sqlite { .. } => "SQLite",
                    config::ResolvedStorageProvider::Postgres(_) => "PostgreSQL",
                    config::ResolvedStorageProvider::MySql(_) => "MySQL",
                    config::ResolvedStorageProvider::MongoDb(_) => "MongoDB",
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
                let purpose = if args.command.is_some() {
                    ConnectionPurpose::ExistingOnly
                } else {
                    ConnectionPurpose::Migration
                };
                let storage = connect_storage(&raw, &paths, purpose).await?;
                match args.command {
                    Some(MigrateCommand::Status) => commands::migration_status(&storage).await?,
                    None => commands::migrate(&storage).await?,
                }
            }
            if do_queue {
                let purpose = if args.command.is_some() {
                    ConnectionPurpose::ExistingOnly
                } else {
                    ConnectionPurpose::Migration
                };
                let queue = connect_queue(&raw, &paths, purpose).await?;
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
            let queue = connect_queue(&raw, &paths, ConnectionPurpose::ExistingOnly).await?;
            worker::run(queue, resolved).await
        }
        Command::Jobs { command } => {
            let queue = connect_queue(&raw, &paths, ConnectionPurpose::ExistingOnly).await?;
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
            let storage = connect_storage(&raw, &paths, ConnectionPurpose::ExistingOnly).await?;
            commands::create_user(&storage, args, argument_password).await
        }
    }
}

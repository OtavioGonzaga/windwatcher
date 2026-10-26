//! Composition for the official Windwatcher server, separate from the reusable engine.

mod cli;
mod commands;
mod config;
mod error;
mod logging;
mod paths;
mod storage;

use clap::Parser;
use cli::{Cli, Command, MigrateCommand};
use error::AppError;
use paths::{ResolvedPaths, RuntimeMode};
use storage::Storage;

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
                "Configuration valid.\nMode: {}\nStorage: SQLite",
                if cli.system { "system" } else { "user" }
            );
            Ok(())
        }
        Command::Migrate { command } => {
            let storage = connect_storage(&raw, &paths).await?;
            match command {
                Some(MigrateCommand::Status) => commands::migration_status(&storage).await,
                None => commands::migrate(&storage).await,
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

async fn connect_storage(
    raw: &config::RawConfig,
    paths: &ResolvedPaths,
) -> Result<Storage, AppError> {
    let (options, derived_path, create_if_missing) = config::resolve_sqlite(raw, paths)?;
    if derived_path && create_if_missing {
        std::fs::create_dir_all(&paths.data_dir).map_err(|source| AppError::DataDirectory {
            path: paths.data_dir.clone(),
            source,
        })?;
    }
    let storage = Storage::connect(options)
        .await
        .map_err(|error| AppError::Storage(Box::new(error)))?;
    tracing::info!(backend = storage.backend_name(), "storage backend selected");
    Ok(storage)
}

use std::{fmt, path::PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};
use zeroize::Zeroize;

#[derive(Debug, Parser)]
#[command(name = "windwatcher", about = "Windwatcher operator commands")]
pub struct Cli {
    /// Use system configuration and data paths.
    #[arg(long)]
    pub system: bool,
    /// Select one TOML configuration file.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Override the configured tracing filter.
    #[arg(long)]
    pub log_filter: Option<String>,
    /// Override the configured tracing format.
    #[arg(long)]
    pub log_format: Option<LogFormat>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize a minimal configuration and signing key.
    Init,
    /// Display the standalone server version.
    Version,
    /// Run the standalone HTTP and realtime service (the default command).
    Serve,
    /// Diagnose configuration, storage and configured queue without changing them.
    Doctor,
    /// Apply pending database migrations.
    Migrate(MigrateArgs),
    /// Run the standalone background worker.
    Worker,
    /// Inspect and retry durable jobs.
    Jobs {
        #[command(subcommand)]
        command: JobsCommand,
    },
    /// Manage local users.
    User {
        #[command(subcommand)]
        command: UserCommand,
    },
    /// Inspect configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum JobsCommand {
    /// List jobs without displaying payloads.
    List {
        /// Filter to one persisted status.
        #[arg(long, value_enum)]
        status: Option<JobStatusArg>,
        /// Maximum number of jobs to display (1..=100).
        #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
    },
    /// Retry a failed job, preserving its JobId and attempt policy.
    Retry { job_id: String },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum JobStatusArg {
    Pending,
    Leased,
    Completed,
    Failed,
}

#[derive(Debug, Args)]
pub struct MigrateArgs {
    /// Target primary storage migrations.
    #[arg(long, global = true)]
    pub storage: bool,
    /// Target queue migrations.
    #[arg(long, global = true)]
    pub queue: bool,
    #[command(subcommand)]
    pub command: Option<MigrateCommand>,
}

#[derive(Debug, Subcommand)]
pub enum MigrateCommand {
    /// Display embedded and applied migration state.
    Status,
}

#[derive(Debug, Subcommand)]
pub enum UserCommand {
    /// Provision a local user as a trusted operator.
    Create(CreateUserArgs),
    /// Disable a user and revoke all sessions.
    Disable { user_id: windwatcher::UserId },
    /// Enable a user without restoring revoked sessions.
    Enable { user_id: windwatcher::UserId },
}

#[derive(Args)]
pub struct CreateUserArgs {
    #[arg(long)]
    pub username: String,
    /// Read a password from process arguments (less safe; emits a warning).
    #[arg(long, conflicts_with_all = ["password_stdin", "password_file"])]
    pub password: Option<String>,
    /// Read an exact UTF-8 password from standard input until EOF.
    #[arg(long, conflicts_with_all = ["password", "password_file"])]
    pub password_stdin: bool,
    /// Read an exact UTF-8 password from a file.
    #[arg(long, conflicts_with_all = ["password", "password_stdin"])]
    pub password_file: Option<PathBuf>,
}

/// Plaintext command input that is redacted in diagnostics and zeroized on drop.
pub struct SensitiveString(String);
impl SensitiveString {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        let mut value = self;
        std::mem::take(&mut value.0)
    }
}
impl Drop for SensitiveString {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}
impl fmt::Debug for SensitiveString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SensitiveString([REDACTED])")
    }
}
impl fmt::Debug for CreateUserArgs {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateUserArgs")
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("password_stdin", &self.password_stdin)
            .field("password_file", &self.password_file)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, ValueEnum, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Compact,
    Pretty,
    Json,
}

impl std::fmt::Display for LogFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Compact => "compact",
            Self::Pretty => "pretty",
            Self::Json => "json",
        })
    }
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Fully validate server configuration without changing state.
    Check,
    /// Display effective configuration with sensitive values redacted.
    Show,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_and_explicit_serve_normalize_identically_with_existing_flags() {
        for args in [vec!["windwatcher"], vec!["windwatcher", "serve"]] {
            assert!(matches!(
                Cli::try_parse_from(args)
                    .unwrap()
                    .command
                    .unwrap_or(Command::Serve),
                Command::Serve
            ));
        }
        let cli = Cli::try_parse_from([
            "windwatcher",
            "--system",
            "--config",
            "config.toml",
            "--log-filter",
            "warn",
            "--log-format",
            "json",
        ])
        .unwrap();
        assert!(cli.system);
        assert_eq!(cli.config.unwrap(), PathBuf::from("config.toml"));
        assert_eq!(cli.log_filter.as_deref(), Some("warn"));
        assert!(matches!(cli.log_format, Some(LogFormat::Json)));
    }
}

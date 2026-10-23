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
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Apply pending database migrations.
    Migrate {
        #[command(subcommand)]
        command: Option<MigrateCommand>,
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
pub enum MigrateCommand {
    /// Display embedded and applied migration state.
    Status,
}

#[derive(Debug, Subcommand)]
pub enum UserCommand {
    /// Provision a local user as a trusted operator.
    Create(CreateUserArgs),
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
}

use tracing_subscriber::{EnvFilter, fmt};

use crate::{cli::LogFormat, config::RawLogging, error::AppError};

pub fn initialize(
    config: &RawLogging,
    filter_override: Option<&str>,
    format_override: Option<LogFormat>,
) -> Result<(), AppError> {
    let filter = filter_override.unwrap_or(&config.filter);
    let filter = EnvFilter::try_new(filter)
        .map_err(|error| AppError::Config(format!("invalid logging filter: {error}")))?;
    let format = format_override.unwrap_or(config.format);
    let ansi = std::io::IsTerminal::is_terminal(&std::io::stderr());
    match format {
        LogFormat::Compact => fmt()
            .with_env_filter(filter)
            .with_ansi(ansi)
            .compact()
            .with_writer(std::io::stderr)
            .try_init(),
        LogFormat::Pretty => fmt()
            .with_env_filter(filter)
            .with_ansi(ansi)
            .pretty()
            .with_writer(std::io::stderr)
            .try_init(),
        LogFormat::Json => fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .json()
            .with_writer(std::io::stderr)
            .try_init(),
    }
    .map_err(|error| AppError::Config(format!("failed to initialize logging: {error}")))
}

use tracing_subscriber::{EnvFilter, fmt};
use tracing_subscriber::{filter::filter_fn, prelude::*};

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
        LogFormat::Compact => tracing_subscriber::registry()
            .with(filter)
            .with(
                fmt::layer()
                    .with_ansi(ansi)
                    .compact()
                    .with_writer(std::io::stderr)
                    .with_filter(filter_fn(protocol_log_allowed)),
            )
            .try_init(),
        LogFormat::Pretty => tracing_subscriber::registry()
            .with(filter)
            .with(
                fmt::layer()
                    .with_ansi(ansi)
                    .pretty()
                    .with_writer(std::io::stderr)
                    .with_filter(filter_fn(protocol_log_allowed)),
            )
            .try_init(),
        LogFormat::Json => tracing_subscriber::registry()
            .with(filter)
            .with(
                fmt::layer()
                    .with_ansi(false)
                    .json()
                    .with_writer(std::io::stderr)
                    .with_filter(filter_fn(protocol_log_allowed)),
            )
            .try_init(),
    }
    .map_err(|error| AppError::Config(format!("failed to initialize logging: {error}")))
}

// Tungstenite's TRACE/DEBUG diagnostics include raw frames and handshake data.
// Operator filters must not accidentally enable message/ticket disclosure.
fn protocol_log_allowed(metadata: &tracing::Metadata<'_>) -> bool {
    !["tungstenite", "tokio_tungstenite"].iter().any(|prefix| {
        metadata
            .target()
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with("::"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sensitive_protocol_targets_cannot_be_enabled_by_operator_filter() {
        use std::{
            io::Write,
            sync::{Arc, Mutex},
        };
        #[derive(Clone)]
        struct Buffer(Arc<Mutex<Vec<u8>>>);
        impl Write for Buffer {
            fn write(&mut self, value: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(value);
                Ok(value.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let output = Buffer(Arc::default());
        let capture = output.clone();
        tracing_log::LogTracer::init().unwrap();
        let subscriber = tracing_subscriber::registry()
            .with(EnvFilter::new("trace,tungstenite::protocol=trace"))
            .with(
                fmt::layer()
                    .with_writer(move || capture.clone())
                    .with_filter(filter_fn(protocol_log_allowed)),
            );
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "windwatcher_server::realtime", "safe_connection_metadata");
            tracing::trace!(target: "tungstenite::protocol", "secret_message_content");
            tracing::debug!(target: "tokio_tungstenite", "secret_ticket");
            log::trace!(target: "tungstenite::protocol::frame", "secret_bridged_frame");
            log::debug!(target: "tokio_tungstenite::handshake", "secret_bridged_ticket");
        });
        let logs = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
        assert!(logs.contains("safe_connection_metadata"));
        assert!(!logs.contains("secret_message_content"));
        assert!(!logs.contains("secret_ticket"));
        assert!(!logs.contains("secret_bridged_frame"));
        assert!(!logs.contains("secret_bridged_ticket"));
    }
}

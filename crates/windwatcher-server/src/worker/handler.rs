use std::{future::Future, pin::Pin};

use windwatcher::{JobFailureCode, JobPayload};

pub(crate) type HandlerFuture = Pin<Box<dyn Future<Output = Result<(), JobExecutionError>> + Send>>;

/// Retry classification returned by private server-owned job handlers.
#[derive(Debug)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the initial production registry has no handlers; test handlers exercise retryable classification"
    )
)]
pub(crate) enum JobExecutionError {
    Retryable {
        code: JobFailureCode,
        message: String,
    },
    Permanent {
        code: JobFailureCode,
        message: String,
    },
}

pub(crate) type Handler = Box<dyn Fn(JobPayload) -> HandlerFuture + Send + Sync>;

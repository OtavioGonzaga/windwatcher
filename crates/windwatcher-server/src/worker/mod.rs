mod backoff;
mod handler;
mod registry;
mod runtime;

use std::sync::Arc;

use crate::{config::ResolvedQueueConfig, error::AppError, queue::ConfiguredQueue};

pub async fn run(queue: ConfiguredQueue, config: ResolvedQueueConfig) -> Result<(), AppError> {
    runtime::run(Arc::new(queue), config).await
}

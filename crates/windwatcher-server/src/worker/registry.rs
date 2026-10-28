use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::Arc,
};

use serde::de::DeserializeOwned;
use windwatcher::{JobFailureCode, JobKind, JobPayload};

use super::handler::{Handler, HandlerFuture, JobExecutionError};

#[derive(Default)]
pub(crate) struct JobRegistry {
    kinds: HashSet<JobKind>,
    handlers: HashMap<(JobKind, u16), Arc<Handler>>,
}

impl JobRegistry {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the initial production registry has no handlers; test handlers exercise dispatch"
        )
    )]
    pub(crate) fn register<T, F, Fut>(&mut self, kind: JobKind, version: u16, handler: F)
    where
        T: DeserializeOwned + Send + 'static,
        F: Fn(T) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), JobExecutionError>> + Send + 'static,
    {
        self.kinds.insert(kind.clone());
        let handler = Arc::new(handler);
        let wrapped: Handler = Box::new(move |payload: JobPayload| -> HandlerFuture {
            let value = serde_json::Value::Object(payload.into_map());
            match serde_json::from_value::<T>(value) {
                Ok(decoded) => Box::pin(handler(decoded)),
                Err(_) => {
                    Box::pin(async { Err(permanent("payload.invalid", "job payload is invalid")) })
                }
            }
        });
        self.handlers.insert((kind, version), Arc::new(wrapped));
    }

    pub(crate) fn dispatch(
        &self,
        kind: &JobKind,
        version: u16,
        payload: JobPayload,
    ) -> HandlerFuture {
        if let Some(handler) = self.handlers.get(&(kind.clone(), version)) {
            return handler(payload);
        }
        let error = if self.kinds.contains(kind) {
            permanent(
                "handler.unsupported_version",
                "job payload version is unsupported",
            )
        } else {
            permanent("handler.unsupported_kind", "job kind is unsupported")
        };
        Box::pin(async move { Err(error) })
    }
}

fn permanent(code: &str, message: &str) -> JobExecutionError {
    JobExecutionError::Permanent {
        code: JobFailureCode::new(code).expect("static failure code is valid"),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn distinguishes_unknown_kind_unsupported_version_and_bad_payload() {
        let mut registry = JobRegistry::default();
        let kind = JobKind::new("test.kind").unwrap();
        registry.register::<u32, _, _>(kind.clone(), 1, |_| async { Ok(()) });
        let payload = JobPayload::try_from(serde_json::json!({})).unwrap();
        assert!(
            matches!(registry.dispatch(&kind, 2, payload.clone()).await, Err(JobExecutionError::Permanent { code, .. }) if code.as_str() == "handler.unsupported_version")
        );
        assert!(
            matches!(registry.dispatch(&JobKind::new("other.kind").unwrap(), 1, payload.clone()).await, Err(JobExecutionError::Permanent { code, .. }) if code.as_str() == "handler.unsupported_kind")
        );
        assert!(
            matches!(registry.dispatch(&kind, 1, payload).await, Err(JobExecutionError::Permanent { code, .. }) if code.as_str() == "payload.invalid")
        );
    }
}

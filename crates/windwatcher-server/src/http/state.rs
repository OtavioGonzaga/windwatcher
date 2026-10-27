use crate::{Storage, TokioPasswordHasher};
use std::sync::Arc;
use windwatcher::{
    Hs256JwtAccessTokenCodec, OsRefreshTokenProvider, RegistrationPolicy, SessionPolicy,
};

/// Concrete, cloneable dependencies for HTTP handlers; contains no raw config.
#[derive(Clone)]
pub struct HttpState {
    pub(super) storage: Storage,
    pub(super) passwords: TokioPasswordHasher,
    pub(super) refresh_tokens: Arc<OsRefreshTokenProvider>,
    pub(super) codec: Arc<Hs256JwtAccessTokenCodec>,
    pub(super) registration: RegistrationPolicy,
    pub(super) session: SessionPolicy,
    pub(super) messages: windwatcher::MessagePolicy,
    pub(super) realtime: crate::realtime::RealtimeService,
}

impl HttpState {
    /// Consumes already-resolved dependencies. Does not read secrets or open storage.
    pub fn new(
        storage: Storage,
        passwords: TokioPasswordHasher,
        codec: Hs256JwtAccessTokenCodec,
        registration: RegistrationPolicy,
        session: SessionPolicy,
        messages: windwatcher::MessagePolicy,
    ) -> Self {
        Self {
            realtime: crate::realtime::RealtimeService::new(
                storage.clone(),
                crate::RawRealtime::default()
                    .resolve()
                    .expect("valid built-in realtime policy"),
            ),
            storage,
            passwords,
            codec: Arc::new(codec),
            registration,
            session,
            messages,
            refresh_tokens: Arc::new(OsRefreshTokenProvider),
        }
    }

    /// Selects validated realtime policy before composing routers. Clones share its runtime.
    pub fn with_realtime_config(mut self, config: crate::ResolvedRealtimeConfig) -> Self {
        self.realtime = crate::realtime::RealtimeService::new(self.storage.clone(), config);
        self
    }
}

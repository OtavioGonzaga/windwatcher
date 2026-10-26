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
}

impl HttpState {
    /// Consumes already-resolved dependencies. Does not read secrets or open storage.
    pub fn new(
        storage: Storage,
        passwords: TokioPasswordHasher,
        codec: Hs256JwtAccessTokenCodec,
        registration: RegistrationPolicy,
        session: SessionPolicy,
    ) -> Self {
        Self {
            storage,
            passwords,
            codec: Arc::new(codec),
            registration,
            session,
            refresh_tokens: Arc::new(OsRefreshTokenProvider),
        }
    }
}

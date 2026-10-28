mod connection;
mod dispatch;
pub(crate) mod event;
mod hub;
mod notifier;
mod reaction;
mod ticket;

use crate::{ResolvedRealtimeConfig, Storage};
use hub::{Hub, Registration};
use std::sync::{Arc, Mutex};
use ticket::{Grant, Ticket, Tickets};
use time::OffsetDateTime;
use windwatcher::{Session, SessionId, UserId, ValidateSession, ValidateSessionError};

#[derive(Clone)]
pub(crate) struct RealtimeService(Arc<Inner>);
struct Inner {
    storage: Storage,
    config: ResolvedRealtimeConfig,
    tickets: Mutex<Tickets>,
    hub: Hub,
    lifecycle: tokio::sync::RwLock<()>,
    dispatch: dispatch::Dispatch,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("invalid ticket")]
    InvalidTicket,
    #[error("session inactive")]
    SessionInactive,
    #[error("session validation failed")]
    Storage(#[source] ValidateSessionError),
    #[error("ticket generation failed")]
    Random,
}

pub(crate) struct Prepared {
    registration: Registration,
    session: Session,
}

impl RealtimeService {
    pub(crate) fn new(storage: Storage, config: ResolvedRealtimeConfig) -> Self {
        Self(Arc::new(Inner {
            dispatch: dispatch::Dispatch::new(config.dispatch_concurrency, config.dispatch_timeout),
            storage,
            config,
            tickets: Mutex::new(Tickets::default()),
            hub: Hub::default(),
            lifecycle: tokio::sync::RwLock::new(()),
        }))
    }
    async fn validate(&self, user: UserId, session: SessionId) -> Result<Session, Error> {
        ValidateSession::new(&self.0.storage)
            .execute(user, session, OffsetDateTime::now_utc())
            .await
            .map_err(|error| match error {
                ValidateSessionError::Inactive => Error::SessionInactive,
                _ => Error::Storage(error),
            })
    }
    pub(crate) async fn issue(
        &self,
        user: UserId,
        session: SessionId,
    ) -> Result<(String, OffsetDateTime), Error> {
        let _lifecycle = self.0.lifecycle.read().await;
        let active = self.validate(user, session).await?;
        let now = OffsetDateTime::now_utc();
        self.issue_validated(&active, now)
    }
    fn issue_validated(
        &self,
        active: &Session,
        now: OffsetDateTime,
    ) -> Result<(String, OffsetDateTime), Error> {
        if !active.is_active(now) {
            return Err(Error::SessionInactive);
        }
        let user = active.user_id();
        let session = active.id();
        let expires_at = now
            .checked_add(
                time::Duration::try_from(self.0.config.ticket_ttl).map_err(|_| Error::Random)?,
            )
            .ok_or(Error::Random)?
            .min(active.expires_at());
        let ticket: Ticket = self
            .0
            .tickets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .issue(
                Grant {
                    user,
                    session,
                    expires_at,
                },
                now,
            )
            .map_err(|error| match error {
                ticket::IssueError::Expired => Error::SessionInactive,
                ticket::IssueError::Random => Error::Random,
            })?;
        // A revoke racing issuance still cannot authenticate: preparation revalidates persistence.
        tracing::info!(user_id = %user, session_id = %session, "ticket_issued");
        Ok((ticket.expose().to_owned(), expires_at))
    }
    pub(crate) async fn prepare(&self, raw: &str) -> Result<Prepared, Error> {
        let _lifecycle = self.0.lifecycle.read().await;
        let grant = self
            .0
            .tickets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .consume(raw, OffsetDateTime::now_utc())
            .ok_or_else(|| {
                tracing::info!("ticket_rejected");
                Error::InvalidTicket
            })?;
        tracing::info!(user_id = %grant.user, session_id = %grant.session, "ticket_consumed");
        let registration =
            self.0
                .hub
                .register(grant.user, grant.session, self.0.config.outgoing_buffer);
        // Register before reading: local logout can cancel this exact pending entry.
        self.validate(grant.user, grant.session).await?;
        self.complete_registration(registration, grant).await
    }
    async fn complete_registration(
        &self,
        registration: Registration,
        grant: Grant,
    ) -> Result<Prepared, Error> {
        let session = self.validate(grant.user, grant.session).await?;
        if session.is_expired(OffsetDateTime::now_utc())
            || !self.0.hub.activate(registration.guard.id)
        {
            return Err(Error::SessionInactive);
        }
        Ok(Prepared {
            registration,
            session,
        })
    }
    pub(crate) fn invalidate(&self, session: SessionId) {
        self.0
            .tickets
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .invalidate(session);
        self.0.hub.close_session(session);
    }
    pub(crate) async fn revocation_guard(&self) -> tokio::sync::RwLockWriteGuard<'_, ()> {
        self.0.lifecycle.write().await
    }
    pub(crate) async fn connect(&self, socket: axum::extract::ws::WebSocket, prepared: Prepared) {
        connection::run(self.clone(), socket, prepared).await;
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use windwatcher::{IdentityStore, RefreshTokenProvider, SessionStore};

    pub(super) async fn fixture() -> (tempfile::TempDir, RealtimeService, Session) {
        let directory = tempfile::TempDir::new().unwrap();
        let storage = Storage::connect(windwatcher::SqliteStorageOptions::new(
            directory.path().join("race.db").display().to_string(),
        ))
        .await
        .unwrap();
        storage.migrate().await.unwrap();
        let now = OffsetDateTime::now_utc();
        let user = windwatcher::User::new(UserId::new(), "alice".parse().unwrap(), now);
        let hash = windwatcher::PasswordHash::parse(
            "$argon2id$v=19$m=32,t=1,p=1$dGVzdC1vbmx5LXNhbHQ$AAAAAAAAAAAAAAAAAAAAAA",
        )
        .unwrap();
        storage.create_local_user(&user, &hash).await.unwrap();
        let session = Session::new(
            SessionId::new(),
            user.id(),
            now,
            now + time::Duration::hours(1),
        )
        .unwrap();
        let provider = windwatcher::OsRefreshTokenProvider;
        storage
            .create_session(&session, &provider.hash(&provider.generate().unwrap()))
            .await
            .unwrap();
        let service =
            RealtimeService::new(storage, crate::RawRealtime::default().resolve().unwrap());
        (directory, service, session)
    }

    #[tokio::test]
    async fn revocation_at_pending_boundary_prevents_activation() {
        let (_directory, service, session) = fixture().await;
        let grant = Grant {
            user: session.user_id(),
            session: session.id(),
            expires_at: session.expires_at(),
        };
        let pending = service.0.hub.register(grant.user, grant.session, 2);
        let id = pending.guard.id;
        service.validate(grant.user, grant.session).await.unwrap();
        // Exercise the exact boundary between initial validation and activation.
        windwatcher::RevokeSession::new(&service.0.storage)
            .execute(grant.user, grant.session, OffsetDateTime::now_utc())
            .await
            .unwrap();
        service.invalidate(grant.session);
        assert!(matches!(
            service.complete_registration(pending, grant).await,
            Err(Error::SessionInactive)
        ));
        assert!(!service.0.hub.activate(id));
        assert_eq!(
            service.0.hub.publish(&[grant.user], Arc::from("forbidden")),
            0
        );
    }

    #[tokio::test]
    async fn local_revocation_holds_preparation_until_commit_and_cleanup() {
        let (_directory, service, session) = fixture().await;
        let (raw, _) = service
            .issue(session.user_id(), session.id())
            .await
            .unwrap();
        let lock = service.revocation_guard().await;
        let preparation = service.prepare(&raw);
        tokio::pin!(preparation);
        assert!(
            std::future::Future::poll(
                preparation.as_mut(),
                &mut std::task::Context::from_waker(std::task::Waker::noop())
            )
            .is_pending()
        );
        windwatcher::RevokeSession::new(&service.0.storage)
            .execute(session.user_id(), session.id(), OffsetDateTime::now_utc())
            .await
            .unwrap();
        service.invalidate(session.id());
        drop(lock);
        assert!(matches!(preparation.await, Err(Error::InvalidTicket)));
    }

    #[tokio::test]
    async fn concurrent_preparation_consumes_ticket_once_and_drop_unregisters() {
        let (_directory, service, session) = fixture().await;
        let (raw, _) = service
            .issue(session.user_id(), session.id())
            .await
            .unwrap();
        let (a, b) = tokio::join!(service.prepare(&raw), service.prepare(&raw));
        assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
        drop(a);
        drop(b);
        assert_eq!(
            service
                .0
                .hub
                .publish(&[session.user_id()], Arc::from("gone")),
            0
        );
    }

    #[tokio::test]
    async fn ticket_issuance_rechecks_expiration_after_validation() {
        let (_directory, service, session) = fixture().await;
        let active = service
            .validate(session.user_id(), session.id())
            .await
            .unwrap();
        for now in [
            active.expires_at(),
            active.expires_at() + time::Duration::NANOSECOND,
        ] {
            assert!(matches!(
                service.issue_validated(&active, now),
                Err(Error::SessionInactive)
            ));
        }
        let now = active.expires_at() - time::Duration::NANOSECOND;
        let (raw, expires_at) = service.issue_validated(&active, now).unwrap();
        assert_eq!(expires_at, active.expires_at());
        assert!(
            service
                .0
                .tickets
                .lock()
                .unwrap()
                .consume(&raw, now)
                .is_some()
        );
    }
}

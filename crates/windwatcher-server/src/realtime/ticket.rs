use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use getrandom::SysRng;
use rand_core::TryRng;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use time::OffsetDateTime;
use windwatcher::{SessionId, UserId};
use zeroize::Zeroizing;

pub(super) struct Ticket(Zeroizing<String>);
impl std::fmt::Debug for Ticket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Ticket([REDACTED])")
    }
}
impl Ticket {
    pub(super) fn generate() -> Result<Self, getrandom::Error> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        SysRng.try_fill_bytes(bytes.as_mut())?;
        Ok(Self(Zeroizing::new(URL_SAFE_NO_PAD.encode(bytes.as_ref()))))
    }
    pub(super) fn expose(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy)]
pub(super) struct Grant {
    pub user: UserId,
    pub session: SessionId,
    pub expires_at: OffsetDateTime,
}

#[derive(Default)]
pub(super) struct Tickets(HashMap<[u8; 32], Grant>);

#[derive(Debug, thiserror::Error)]
pub(super) enum IssueError {
    #[error("ticket grant expired")]
    Expired,
    #[error("ticket generation failed")]
    Random,
}
impl Tickets {
    pub(super) fn clear(&mut self) {
        self.0.clear();
    }
    fn hash(raw: &str) -> [u8; 32] {
        Sha256::digest(raw.as_bytes()).into()
    }
    pub(super) fn issue(
        &mut self,
        grant: Grant,
        now: OffsetDateTime,
    ) -> Result<Ticket, IssueError> {
        self.0.retain(|_, grant| grant.expires_at > now);
        if grant.expires_at <= now {
            return Err(IssueError::Expired);
        }
        let ticket = Ticket::generate().map_err(|_| IssueError::Random)?;
        self.0.insert(Self::hash(ticket.expose()), grant);
        Ok(ticket)
    }
    pub(super) fn consume(&mut self, raw: &str, now: OffsetDateTime) -> Option<Grant> {
        // Bound work before touching the registry. The decoder rejects padding and
        // nonzero trailing bits, so only the canonical 256-bit representation passes.
        let mut decoded = Zeroizing::new([0u8; 32]);
        if raw.len() != 43 || URL_SAFE_NO_PAD.decode_slice(raw, decoded.as_mut()).ok() != Some(32) {
            return None;
        }
        self.0.retain(|_, grant| grant.expires_at > now);
        self.0.remove(&Self::hash(raw))
    }
    pub(super) fn invalidate(&mut self, session: SessionId) {
        self.0.retain(|_, grant| grant.session != session);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_tickets_do_not_even_touch_the_registry() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut tickets = Tickets::default();
        let valid = tickets
            .issue(
                Grant {
                    user: UserId::new(),
                    session: SessionId::new(),
                    expires_at: now + time::Duration::seconds(30),
                },
                now,
            )
            .unwrap();
        for raw in [
            String::new(),
            "a".repeat(42),
            "a".repeat(44),
            "雪".repeat(43),
            format!("{}=", "A".repeat(42)),
            format!("{}+", "A".repeat(42)),
            format!("{}B", "A".repeat(42)),
            "x".repeat(100_000),
        ] {
            assert!(
                tickets
                    .consume(&raw, now + time::Duration::seconds(31))
                    .is_none()
            );
            // Invalid input must not hash, look up, consume or prune any grant.
            assert_eq!(tickets.0.len(), 1);
        }
        assert!(tickets.consume(valid.expose(), now).is_some());
    }
    #[test]
    fn secrets_expiration_and_one_use() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let grant = Grant {
            user: UserId::new(),
            session: SessionId::new(),
            expires_at: now + time::Duration::seconds(30),
        };
        let mut registry = Tickets::default();
        let a = registry.issue(grant, now).unwrap();
        let b = registry.issue(grant, now).unwrap();
        assert_ne!(a.expose(), b.expose());
        assert_eq!(a.expose().len(), 43);
        assert_eq!(URL_SAFE_NO_PAD.decode(a.expose()).unwrap().len(), 32);
        assert!(!format!("{a:?}").contains(a.expose()));
        assert!(registry.0.contains_key(&Tickets::hash(a.expose())));
        let consumed = registry.consume(a.expose(), now).unwrap();
        assert_eq!(consumed.user, grant.user);
        assert_eq!(consumed.session, grant.session);
        assert!(registry.consume(a.expose(), now).is_none());
        assert!(registry.consume(b.expose(), grant.expires_at).is_none());
        assert!(registry.0.is_empty());
        let c = registry.issue(grant, now).unwrap();
        registry.invalidate(grant.session);
        assert!(registry.consume(c.expose(), now).is_none());
    }
    #[test]
    fn concurrent_consumption_has_exactly_one_winner() {
        use std::sync::{Arc, Barrier, Mutex};
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut tickets = Tickets::default();
        let ticket = tickets
            .issue(
                Grant {
                    user: UserId::new(),
                    session: SessionId::new(),
                    expires_at: now + time::Duration::seconds(30),
                },
                now,
            )
            .unwrap();
        let tickets = Arc::new(Mutex::new(tickets));
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let tickets = tickets.clone();
                let barrier = barrier.clone();
                let raw = ticket.expose().to_owned();
                std::thread::spawn(move || {
                    barrier.wait();
                    tickets.lock().unwrap().consume(&raw, now).is_some()
                })
            })
            .collect();
        assert_eq!(
            handles
                .into_iter()
                .map(|handle| usize::from(handle.join().unwrap()))
                .sum::<usize>(),
            1
        );
    }
    #[test]
    fn expired_grants_are_never_inserted() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut tickets = Tickets::default();
        for expires_at in [now - time::Duration::NANOSECOND, now] {
            assert!(matches!(
                tickets.issue(
                    Grant {
                        user: UserId::new(),
                        session: SessionId::new(),
                        expires_at
                    },
                    now
                ),
                Err(IssueError::Expired)
            ));
            assert!(tickets.0.is_empty());
        }
    }
}

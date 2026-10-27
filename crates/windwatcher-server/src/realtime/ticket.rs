use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand_core::{OsRng, RngCore};
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
    pub(super) fn generate() -> Result<Self, rand_core::Error> {
        let mut bytes = Zeroizing::new([0u8; 32]);
        OsRng.try_fill_bytes(bytes.as_mut())?;
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
impl Tickets {
    fn hash(raw: &str) -> [u8; 32] {
        Sha256::digest(raw.as_bytes()).into()
    }
    pub(super) fn issue(
        &mut self,
        grant: Grant,
        now: OffsetDateTime,
    ) -> Result<Ticket, rand_core::Error> {
        self.0.retain(|_, grant| grant.expires_at > now);
        let ticket = Ticket::generate()?;
        self.0.insert(Self::hash(ticket.expose()), grant);
        Ok(ticket)
    }
    pub(super) fn consume(&mut self, raw: &str, now: OffsetDateTime) -> Option<Grant> {
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
}

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex, MutexGuard},
};
use tokio::sync::{mpsc, watch};
use windwatcher::{SessionId, UserId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct ConnectionId(pub uuid::Uuid);

#[derive(Clone, Copy, Debug)]
pub(super) struct Close {
    pub code: u16,
    pub reason: &'static str,
}
pub(super) const POLICY: Close = Close {
    code: 1008,
    reason: "session inactive",
};
pub(super) const INTERNAL: Close = Close {
    code: 1011,
    reason: "internal failure",
};
pub(super) const SLOW: Close = Close {
    code: 1013,
    reason: "slow consumer",
};

struct Entry {
    user: UserId,
    session: SessionId,
    active: bool,
    events: mpsc::Sender<Arc<str>>,
    close: watch::Sender<Option<Close>>,
}

#[derive(Clone, Default)]
pub(super) struct Hub(Arc<Mutex<HashMap<ConnectionId, Entry>>>);
impl Hub {
    fn entries(&self) -> MutexGuard<'_, HashMap<ConnectionId, Entry>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    pub(super) fn register(
        &self,
        user: UserId,
        session: SessionId,
        capacity: usize,
    ) -> Registration {
        let id = ConnectionId(uuid::Uuid::now_v7());
        let (events, receiver) = mpsc::channel(capacity);
        let (close, control) = watch::channel(None);
        self.entries().insert(
            id,
            Entry {
                user,
                session,
                active: false,
                events,
                close,
            },
        );
        Registration {
            guard: Guard {
                hub: self.clone(),
                id,
                user,
                session,
            },
            events: receiver,
            control,
        }
    }
    pub(super) fn activate(&self, id: ConnectionId) -> bool {
        let mut entries = self.entries();
        if let Some(entry) = entries.get_mut(&id) {
            entry.active = true;
            return true;
        }
        false
    }
    pub(super) fn close_session(&self, session: SessionId) {
        self.entries().retain(|id, entry| {
            if entry.session != session {
                return true;
            }
            entry.close.send_replace(Some(POLICY));
            tracing::info!(connection_id = %id.0, session_id = %session, "session_revoked_close");
            false
        });
    }
    pub(super) fn publish(&self, recipients: &[UserId], frame: Arc<str>) -> usize {
        let recipients: HashSet<_> = recipients.iter().copied().collect();
        let mut delivered = 0;
        self.entries().retain(|id, entry| {
            if !entry.active || !recipients.contains(&entry.user) {
                return true;
            }
            match entry.events.try_send(frame.clone()) {
                Ok(()) => {
                    delivered += 1;
                    true
                }
                Err(mpsc::error::TrySendError::Full(_)) => {
                    entry.close.send_replace(Some(SLOW));
                    tracing::warn!(connection_id = %id.0, "slow_consumer_close");
                    false
                }
                Err(mpsc::error::TrySendError::Closed(_)) => false,
            }
        });
        delivered
    }
}

pub(super) struct Registration {
    pub guard: Guard,
    pub events: mpsc::Receiver<Arc<str>>,
    pub control: watch::Receiver<Option<Close>>,
}
pub(super) struct Guard {
    hub: Hub,
    pub id: ConnectionId,
    pub user: UserId,
    pub session: SessionId,
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.hub.entries().remove(&self.id);
        tracing::info!(connection_id = %self.id.0, user_id = %self.user, session_id = %self.session, "connection_closed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiple_connections_sessions_pending_and_guards() {
        let hub = Hub::default();
        let user = UserId::new();
        let session = SessionId::new();
        let a = hub.register(user, session, 2);
        let b = hub.register(user, SessionId::new(), 2);
        let pending = hub.register(user, session, 2);
        assert!(hub.activate(a.guard.id));
        assert!(hub.activate(b.guard.id));
        assert_eq!(hub.publish(&[user, user], Arc::from("same")), 2);
        hub.close_session(session);
        assert!(!hub.activate(pending.guard.id));
        assert_eq!(a.control.borrow().unwrap().code, 1008);
        assert_eq!(hub.publish(&[user], Arc::from("next")), 1);
        drop(a);
        drop(pending);
        assert_eq!(hub.entries().len(), 1);
        drop(b);
        assert!(hub.entries().is_empty());
    }
    #[test]
    fn slow_consumers_are_isolated_and_frame_is_shared() {
        let hub = Hub::default();
        let user = UserId::new();
        let slow = hub.register(user, SessionId::new(), 1);
        let mut healthy = hub.register(user, SessionId::new(), 2);
        hub.activate(slow.guard.id);
        hub.activate(healthy.guard.id);
        let frame: Arc<str> = Arc::from("json");
        assert_eq!(hub.publish(&[user, UserId::new()], frame.clone()), 2);
        assert!(Arc::ptr_eq(&healthy.events.try_recv().unwrap(), &frame));
        assert_eq!(hub.publish(&[user], frame), 1);
        assert_eq!(slow.control.borrow().unwrap().code, 1013);
        assert!(healthy.events.try_recv().is_ok());
        assert_eq!(hub.entries().len(), 1);
    }
}

//! The session manager of one local identity
//! (`docs/adr/0009-session-manager.md`).
//!
//! It knows, for every contact record of the identity (by its
//! `ContactId`), the confirmed sessions that stand for it and which one is
//! preferred: only that one sends the queue. A session is registered by
//! the task that runs its link, when the session is confirmed and the
//! contact store says it stands for the record, and leaves when its task
//! ends. When a second session of a record registers, the duplicate rule
//! of `docs/PROTOCOL.md` section 14 (`monolith_protocol::duplicate`)
//! decides which stays; the loser is told to close.
//!
//! The manager holds no link. It reaches the task of a session through a
//! [`SessionControl`]: a flag that asks it to close, a probe to send, and
//! a wakeup. Everything it decides is decided under one lock, in one step,
//! so two registrations of one record are resolved one after the other.

use core::fmt;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use monolith_identity::IdentityPublicKey;
use monolith_protocol::duplicate::{self, Contender, Initiator, ProbeOutcome, Resolution};
use monolith_protocol::limits::PING_NONCE_LEN;
use monolith_storage::record::ContactId;
use tokio::sync::{Notify, watch};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What the manager asks of the task that runs one session.
#[derive(Default)]
pub(crate) struct SessionControl {
    close: AtomicBool,
    probe: Mutex<Option<[u8; PING_NONCE_LEN]>>,
    wake: Notify,
}

impl fmt::Debug for SessionControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionControl")
            .field("close", &self.close.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl SessionControl {
    /// Asks the session to end with a Close.
    pub(crate) fn close(&self) {
        self.close.store(true, Ordering::SeqCst);
        self.wake.notify_one();
    }

    /// Returns true once the session was asked to close.
    pub(crate) fn closing(&self) -> bool {
        self.close.load(Ordering::SeqCst)
    }

    /// Asks the session to send a Ping with `nonce`, the probe of a
    /// duplicate session.
    fn probe(&self, nonce: [u8; PING_NONCE_LEN]) {
        *lock(&self.probe) = Some(nonce);
        self.wake.notify_one();
    }

    /// The probe to send, once.
    pub(crate) fn take_probe(&self) -> Option<[u8; PING_NONCE_LEN]> {
        lock(&self.probe).take()
    }

    /// Wakes the task, so that it looks at its queue again.
    pub(crate) fn wake(&self) {
        self.wake.notify_one();
    }

    /// Waits for a wakeup.
    pub(crate) async fn woken(&self) {
        self.wake.notified().await;
    }
}

/// A confirmed session of a contact record, as the manager holds it.
#[derive(Debug)]
struct Registered {
    serial: u64,
    initiator: Initiator,
    control: Arc<SessionControl>,
}

/// The sessions of one contact record.
#[derive(Debug, Default)]
struct Contenders {
    /// The session that sends the queue.
    preferred: Option<Registered>,
    /// A newer session of the other side, while the preferred one is
    /// probed.
    pending: Option<Registered>,
    /// The nonce of the probe that runs, if one runs.
    probe: Option<[u8; PING_NONCE_LEN]>,
}

/// What registering a session decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Registration {
    /// The session is the preferred one of its record.
    Preferred,
    /// The preferred session of the record is probed; this one waits for
    /// the outcome.
    Waiting,
    /// The session lost to the one the record has, and was told to close.
    Closed,
}

/// The sessions of every contact record of one local identity.
pub(crate) struct SessionManager {
    local: IdentityPublicKey,
    records: Mutex<HashMap<ContactId, Contenders>>,
    serials: AtomicU64,
    /// Turns over whenever the sessions of a record change.
    changes: watch::Sender<u64>,
}

impl fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionManager").finish_non_exhaustive()
    }
}

/// A random nonce for a probe. A failing random source gives a fixed one:
/// the probe only tells a live session from a dead one, and a Pong for an
/// older probe is refused by the state, not by the nonce.
fn nonce() -> [u8; PING_NONCE_LEN] {
    let mut bytes = [0_u8; PING_NONCE_LEN];
    if getrandom::fill(&mut bytes).is_err() {
        bytes = [0x5A; PING_NONCE_LEN];
    }
    bytes
}

impl SessionManager {
    pub(crate) fn new(local: IdentityPublicKey) -> Self {
        Self {
            local,
            records: Mutex::new(HashMap::new()),
            serials: AtomicU64::new(1),
            changes: watch::Sender::new(0),
        }
    }

    /// A number for a new session of this identity.
    pub(crate) fn next_serial(&self) -> u64 {
        self.serials.fetch_add(1, Ordering::SeqCst)
    }

    fn changed(&self) {
        self.changes
            .send_modify(|count| *count = count.wrapping_add(1));
    }

    /// Follows the changes of the sessions.
    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Registers the confirmed session `serial` of the contact record
    /// `contact`, whose remote identity is `remote`. `current` tells for a
    /// session whether its transport key is still the active credential
    /// of the contact, for the session registering and for the one the
    /// record has. It is asked under the lock of the manager: it may take
    /// the lock of a contact, which never waits for the manager, and must
    /// not call the manager.
    pub(crate) fn register(
        &self,
        contact: ContactId,
        remote: &IdentityPublicKey,
        serial: u64,
        initiator: Initiator,
        control: Arc<SessionControl>,
        current: impl Fn(u64) -> bool,
    ) -> Registration {
        let newer = Registered {
            serial,
            initiator,
            control,
        };
        let registration = {
            let mut records = lock(&self.records);
            let contenders = records.entry(contact).or_default();
            match &contenders.preferred {
                None => {
                    contenders.preferred = Some(newer);
                    Registration::Preferred
                }
                Some(older) => {
                    let resolution = duplicate::resolve(
                        &self.local,
                        remote,
                        Contender {
                            initiator: older.initiator,
                            current: current(older.serial),
                        },
                        Contender {
                            initiator: newer.initiator,
                            current: current(newer.serial),
                        },
                    );
                    match resolution {
                        Ok(Resolution::CloseOlder) => {
                            if let Some(older) = contenders.preferred.replace(newer) {
                                older.control.close();
                            }
                            // A probe that ran is moot: the session it
                            // probed is closed.
                            if let Some(pending) = contenders.pending.take() {
                                pending.control.close();
                            }
                            contenders.probe = None;
                            Registration::Preferred
                        }
                        Ok(Resolution::ProbeOlder) => {
                            // A newer session that waited for an earlier
                            // probe gives way to this one.
                            if let Some(pending) = contenders.pending.replace(newer) {
                                pending.control.close();
                            }
                            if contenders.probe.is_none() {
                                let nonce = nonce();
                                contenders.probe = Some(nonce);
                                older.control.probe(nonce);
                            }
                            Registration::Waiting
                        }
                        Ok(Resolution::CloseBoth) => {
                            if let Some(older) = contenders.preferred.take() {
                                older.control.close();
                            }
                            if let Some(pending) = contenders.pending.take() {
                                pending.control.close();
                            }
                            contenders.probe = None;
                            newer.control.close();
                            Registration::Closed
                        }
                        // The newer session was made with a retired key, or
                        // the two keys are one (which a session with oneself
                        // would be, refused long before this).
                        Ok(Resolution::CloseNewer) | Err(_) => {
                            newer.control.close();
                            Registration::Closed
                        }
                    }
                }
            }
        };
        self.changed();
        registration
    }

    /// The probe `nonce` of the preferred session `serial` of `contact`
    /// ended: answered, or not in time. Applies the outcome of the
    /// duplicate rule.
    pub(crate) fn probe_ended(
        &self,
        contact: &ContactId,
        serial: u64,
        nonce: [u8; PING_NONCE_LEN],
        answered: bool,
    ) {
        {
            let mut records = lock(&self.records);
            let Some(contenders) = records.get_mut(contact) else {
                return;
            };
            let probed = contenders
                .preferred
                .as_ref()
                .is_some_and(|preferred| preferred.serial == serial);
            if !probed || contenders.probe != Some(nonce) {
                return;
            }
            contenders.probe = None;
            match duplicate::after_probe(answered) {
                ProbeOutcome::CloseNewer => {
                    if let Some(pending) = contenders.pending.take() {
                        pending.control.close();
                    }
                }
                ProbeOutcome::CloseOlder => {
                    if let Some(pending) = contenders.pending.take() {
                        // The newer session carries the queue now; its
                        // task is told.
                        pending.control.wake();
                        if let Some(older) = contenders.preferred.replace(pending) {
                            older.control.close();
                        }
                    }
                }
            }
        }
        self.changed();
    }

    /// The session `serial` of `contact` ended. A session that waited for
    /// a probe takes the place of the preferred one if that one ended.
    pub(crate) fn unregister(&self, contact: &ContactId, serial: u64) {
        {
            let mut records = lock(&self.records);
            let Some(contenders) = records.get_mut(contact) else {
                return;
            };
            if contenders
                .preferred
                .as_ref()
                .is_some_and(|preferred| preferred.serial == serial)
            {
                contenders.preferred = contenders.pending.take();
                contenders.probe = None;
                if let Some(preferred) = &contenders.preferred {
                    preferred.control.wake();
                }
            } else if contenders
                .pending
                .as_ref()
                .is_some_and(|pending| pending.serial == serial)
            {
                contenders.pending = None;
            } else {
                // Not registered (closed when it registered): nothing
                // changed, and nobody is woken.
                return;
            }
            if contenders.preferred.is_none() && contenders.pending.is_none() {
                records.remove(contact);
            }
        }
        self.changed();
    }

    /// Returns true if the session `serial` is the preferred one of
    /// `contact`.
    pub(crate) fn is_preferred(&self, contact: &ContactId, serial: u64) -> bool {
        lock(&self.records)
            .get(contact)
            .and_then(|contenders| contenders.preferred.as_ref())
            .is_some_and(|preferred| preferred.serial == serial)
    }

    /// Returns true if a session of `contact` is registered.
    pub(crate) fn has_session(&self, contact: &ContactId) -> bool {
        lock(&self.records)
            .get(contact)
            .is_some_and(|contenders| contenders.preferred.is_some())
    }

    /// The number of registered sessions of `contact`: the preferred one
    /// and one that waits for a probe.
    pub(crate) fn sessions(&self, contact: &ContactId) -> usize {
        lock(&self.records).get(contact).map_or(0, |contenders| {
            usize::from(contenders.preferred.is_some())
                .saturating_add(usize::from(contenders.pending.is_some()))
        })
    }

    /// Asks every session of `contact` to close, and wakes whoever follows
    /// the sessions: a scheduler of a record that ended looks at it again
    /// and stops, also in the middle of a delay or a dial.
    pub(crate) fn close(&self, contact: &ContactId) {
        if let Some(contenders) = lock(&self.records).get(contact) {
            for registered in contenders.preferred.iter().chain(contenders.pending.iter()) {
                registered.control.close();
            }
        }
        self.changed();
    }

    /// Asks every session to close.
    pub(crate) fn close_all(&self) {
        for contenders in lock(&self.records).values() {
            for registered in contenders.preferred.iter().chain(contenders.pending.iter()) {
                registered.control.close();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use monolith_identity::IdentitySecretKey;
    use monolith_storage::record::RECORD_ID_LEN;

    fn key(seed: u8) -> IdentityPublicKey {
        IdentitySecretKey::from_seed(&[seed; 32]).public_key()
    }

    fn record() -> ContactId {
        ContactId::from_bytes([1; RECORD_ID_LEN]).unwrap()
    }

    /// Two keys and which one is smaller.
    fn pair() -> (IdentityPublicKey, IdentityPublicKey) {
        let (one, other) = (key(1), key(2));
        if one < other {
            (one, other)
        } else {
            (other, one)
        }
    }

    struct Side {
        manager: SessionManager,
        remote: IdentityPublicKey,
    }

    impl Side {
        fn new(local: IdentityPublicKey, remote: IdentityPublicKey) -> Self {
            Self {
                manager: SessionManager::new(local),
                remote,
            }
        }

        fn register(
            &self,
            serial: u64,
            initiator: Initiator,
        ) -> (Registration, Arc<SessionControl>) {
            let control = Arc::new(SessionControl::default());
            let registration = self.manager.register(
                record(),
                &self.remote,
                serial,
                initiator,
                control.clone(),
                |_| true,
            );
            (registration, control)
        }
    }

    #[test]
    fn the_first_session_is_preferred_and_a_later_one_of_the_same_side_replaces_it() {
        let (small, large) = pair();
        let side = Side::new(small, large);
        let (first, one) = side.register(1, Initiator::Local);
        assert_eq!(first, Registration::Preferred);
        let (second, two) = side.register(2, Initiator::Local);
        assert_eq!(second, Registration::Preferred);
        assert!(one.closing());
        assert!(!two.closing());
        assert!(side.manager.is_preferred(&record(), 2));
        side.manager.unregister(&record(), 1);
        assert!(side.manager.is_preferred(&record(), 2));
        side.manager.unregister(&record(), 2);
        assert!(!side.manager.has_session(&record()));
    }

    /// Both sides dial at once: each sees the two sessions in some order,
    /// and each keeps the one initiated by the side with the smaller key.
    #[test]
    fn simultaneous_sessions_converge_on_both_sides_in_every_order() {
        let (small, large) = pair();
        // The session the smaller side initiated is "s", the other "l".
        for small_order in [["s", "l"], ["l", "s"]] {
            for large_order in [["s", "l"], ["l", "s"]] {
                let at_small = Side::new(small, large);
                let at_large = Side::new(large, small);
                let mut kept = Vec::new();
                for (side, order, local_initiated) in
                    [(&at_small, small_order, "s"), (&at_large, large_order, "l")]
                {
                    let mut controls = HashMap::new();
                    for (serial, name) in (1_u64..).zip(order) {
                        let initiator = if name == local_initiated {
                            Initiator::Local
                        } else {
                            Initiator::Remote
                        };
                        let (_, control) = side.register(serial, initiator);
                        controls.insert(name, (serial, control));
                    }
                    // A probe asked of the older session is answered: both
                    // are alive.
                    for (serial, control) in controls.values() {
                        if let Some(nonce) = control.take_probe() {
                            side.manager.probe_ended(&record(), *serial, nonce, true);
                        }
                    }
                    let open: Vec<&str> = controls
                        .iter()
                        .filter(|(_, (_, control))| !control.closing())
                        .map(|(name, _)| *name)
                        .collect();
                    assert_eq!(open.len(), 1, "{small_order:?} {large_order:?}");
                    let (serial, _) = controls[open[0]];
                    assert!(side.manager.is_preferred(&record(), serial));
                    kept.push(open[0]);
                }
                assert_eq!(kept, vec!["s", "s"], "{small_order:?} {large_order:?}");
            }
        }
    }

    #[test]
    fn a_probe_that_is_not_answered_closes_the_older_session() {
        let (small, large) = pair();
        // At the larger side, the session the smaller one initiated came
        // first; a newer one of its own arrives and the older is probed.
        let side = Side::new(large, small);
        let (_, older) = side.register(1, Initiator::Remote);
        let (registration, newer) = side.register(2, Initiator::Local);
        assert_eq!(registration, Registration::Waiting);
        let nonce = older.take_probe().unwrap();
        // While the probe runs, the older session keeps the queue.
        assert!(side.manager.is_preferred(&record(), 1));
        // A wrong nonce, or another session, changes nothing.
        side.manager
            .probe_ended(&record(), 1, [0; PING_NONCE_LEN], false);
        side.manager.probe_ended(&record(), 2, nonce, false);
        assert!(side.manager.is_preferred(&record(), 1));
        side.manager.probe_ended(&record(), 1, nonce, false);
        assert!(older.closing());
        assert!(!newer.closing());
        assert!(side.manager.is_preferred(&record(), 2));
    }

    #[test]
    fn a_session_of_a_retired_key_loses() {
        let (small, large) = pair();
        let side = Side::new(small, large);
        let (_, older) = side.register(1, Initiator::Local);
        let control = Arc::new(SessionControl::default());
        let registration = side.manager.register(
            record(),
            &large,
            2,
            Initiator::Local,
            control.clone(),
            |serial| serial == 1,
        );
        assert_eq!(registration, Registration::Closed);
        assert!(control.closing());
        assert!(!older.closing());
    }

    #[test]
    fn records_are_kept_apart() {
        let (small, large) = pair();
        let side = Side::new(small, large);
        let other = ContactId::from_bytes([2; RECORD_ID_LEN]).unwrap();
        let (_, first) = side.register(1, Initiator::Local);
        let control = Arc::new(SessionControl::default());
        assert_eq!(
            side.manager
                .register(other, &large, 2, Initiator::Local, control.clone(), |_| {
                    true
                }),
            Registration::Preferred
        );
        assert!(!first.closing());
        assert!(side.manager.is_preferred(&record(), 1));
        assert!(side.manager.is_preferred(&other, 2));
    }

    #[test]
    fn ending_a_session_that_was_not_registered_wakes_nobody() {
        let (small, large) = pair();
        let side = Side::new(small, large);
        let (registration, _control) = side.register(1, Initiator::Local);
        assert_eq!(registration, Registration::Preferred);
        let changes = side.manager.subscribe();
        // A session the record never held (one closed as it registered)
        // ends: nothing changed.
        side.manager.unregister(&record(), 7);
        assert!(!changes.has_changed().unwrap());
        assert!(side.manager.is_preferred(&record(), 1));
        // The one it holds ends: that is a change.
        side.manager.unregister(&record(), 1);
        assert!(changes.has_changed().unwrap());
    }
}

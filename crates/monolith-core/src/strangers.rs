//! The budget for authenticated peers that are not contacts, of one local
//! identity (`docs/RESOURCE_LIMITS.md` section 5, `MAX_UNKNOWN_SESSIONS`).
//!
//! Definitions:
//!
//! - Unknown: an inbound session whose standing, decided at admission, is
//!   not that of a requested or accepted contact: no record, declined,
//!   blocked, a stale or conflicting card, a pending key. Outbound sessions
//!   are never unknown: a dial sends message 3 only to a contact.
//! - Silent: an unknown session from which no complete message has been
//!   taken. Its first message ends it anyway, with the Close of
//!   `docs/PROTOCOL.md` section 12, so a silent stranger is one that holds
//!   a slot without having done anything with it yet.
//! - Oldest: the slot taken first. Slots are numbered in the order they
//!   are taken, under one lock, so the order does not depend on timing
//!   between tasks.
//!
//! When every slot is held and another stranger is admitted, the oldest
//! silent one is evicted: its link ends with a Close, as if its first
//! message deadline had passed, and the newcomer takes its slot. A
//! stranger that has sent its message is not evicted; its link ends on its
//! own within `FRAME_WRITE_TIMEOUT`. If no slot can be freed the newcomer
//! is closed without a reply, as before. A contact is never in this budget
//! and is never evicted for a stranger.
//!
//! The first message of a stranger and its eviction race on one atomic
//! state: whichever changes it from silent first wins. A stranger whose
//! message won is not evicted; one that was evicted first has its message
//! dropped and its link ends with the Close of the eviction.
//!
//! An evicted link still exists until it has written its Close. Such links
//! are counted apart, and no more than `MAX_UNKNOWN_SESSIONS` of them at a
//! time: a newcomer that would need another eviction beyond that is
//! closed. So at most twice `MAX_UNKNOWN_SESSIONS` stranger links exist at
//! any moment, however strangers arrive.

use core::fmt;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

const SILENT: u8 = 0;
const HEARD: u8 = 1;
const EVICTED: u8 = 2;

/// The state of one stranger's slot, shared with its link.
#[derive(Debug)]
pub(crate) struct StrangerState {
    state: AtomicU8,
    wake: Notify,
}

impl StrangerState {
    /// The link took the first message of the stranger. Returns false if
    /// the stranger was evicted first: the message is then dropped.
    pub(crate) fn heard(&self) -> bool {
        match self
            .state
            .compare_exchange(SILENT, HEARD, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => true,
            Err(current) => current == HEARD,
        }
    }

    /// Returns true once the stranger was evicted.
    pub(crate) fn is_evicted(&self) -> bool {
        self.state.load(Ordering::SeqCst) == EVICTED
    }

    /// Waits until the stranger is evicted.
    pub(crate) async fn evicted(&self) {
        loop {
            let notified = self.wake.notified();
            if self.is_evicted() {
                return;
            }
            notified.await;
        }
    }

    fn evict(&self) -> bool {
        let evicted = self
            .state
            .compare_exchange(SILENT, EVICTED, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok();
        if evicted {
            self.wake.notify_waiters();
        }
        evicted
    }
}

#[derive(Debug)]
struct Held {
    number: u64,
    state: Arc<StrangerState>,
}

#[derive(Debug, Default)]
struct Slots {
    next: u64,
    held: Vec<Held>,
    /// Evicted links that have not ended yet.
    draining: Vec<Arc<StrangerState>>,
}

/// The slots for strangers of one local identity.
#[derive(Clone)]
pub struct Strangers {
    slots: Arc<Mutex<Slots>>,
    capacity: usize,
}

impl fmt::Debug for Strangers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Strangers")
            .field("capacity", &self.capacity)
            .finish_non_exhaustive()
    }
}

/// A slot for one stranger, held by its link and given back when dropped.
#[derive(Debug)]
pub struct StrangerSlot {
    slots: Arc<Mutex<Slots>>,
    state: Arc<StrangerState>,
}

impl StrangerSlot {
    pub(crate) fn state(&self) -> &Arc<StrangerState> {
        &self.state
    }
}

impl Drop for StrangerSlot {
    fn drop(&mut self) {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        slots
            .held
            .retain(|held| !Arc::ptr_eq(&held.state, &self.state));
        slots
            .draining
            .retain(|state| !Arc::ptr_eq(state, &self.state));
    }
}

impl Strangers {
    /// A budget of `capacity` slots.
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            slots: Arc::new(Mutex::new(Slots::default())),
            capacity,
        }
    }

    /// A slot for a stranger that was just admitted: a free one, or the
    /// slot of the oldest silent stranger, which is evicted. `None` if
    /// neither exists; the newcomer is then closed without a reply.
    pub(crate) fn take(&self) -> Option<StrangerSlot> {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slots.held.len() >= self.capacity {
            if slots.draining.len() >= self.capacity {
                return None;
            }
            // Oldest first: held slots are kept in the order they were
            // taken.
            let position = slots.held.iter().position(|held| held.state.evict())?;
            let evicted = slots.held.remove(position);
            slots.draining.push(evicted.state);
        }
        let number = slots.next;
        slots.next = slots.next.saturating_add(1);
        let state = Arc::new(StrangerState {
            state: AtomicU8::new(SILENT),
            wake: Notify::new(),
        });
        slots.held.push(Held {
            number,
            state: state.clone(),
        });
        debug_assert!(slots.held.windows(2).all(|pair| match pair {
            [a, b] => a.number < b.number,
            _ => true,
        }));
        Some(StrangerSlot {
            slots: self.slots.clone(),
            state,
        })
    }

    /// Slots held by strangers that have not been evicted.
    pub fn held(&self) -> usize {
        self.slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .held
            .len()
    }

    /// Links of evicted strangers that have not ended yet.
    pub fn draining(&self) -> usize {
        self.slots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .draining
            .len()
    }

    /// Free slots.
    pub fn free(&self) -> usize {
        self.capacity.saturating_sub(self.held())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_oldest_silent_stranger_makes_room() {
        let strangers = Strangers::new(3);
        let first = strangers.take().unwrap();
        let second = strangers.take().unwrap();
        let third = strangers.take().unwrap();
        assert_eq!(strangers.free(), 0);
        // The first one has sent its message: it is not evicted.
        assert!(first.state().heard());
        let fourth = strangers.take().unwrap();
        assert!(!first.state().is_evicted());
        assert!(second.state().is_evicted());
        assert!(!third.state().is_evicted());
        assert_eq!(strangers.held(), 3);
        assert_eq!(strangers.draining(), 1);
        // An evicted stranger's message is dropped.
        assert!(!second.state().heard());
        drop(second);
        assert_eq!(strangers.draining(), 0);
        // Next in age: the third.
        let _fifth = strangers.take().unwrap();
        assert!(third.state().is_evicted());
        assert!(!fourth.state().is_evicted());
        drop(fourth);
        assert_eq!(strangers.held(), 2);
    }

    #[test]
    fn nobody_is_evicted_when_every_stranger_has_spoken() {
        let strangers = Strangers::new(2);
        let a = strangers.take().unwrap();
        let b = strangers.take().unwrap();
        assert!(a.state().heard() && b.state().heard());
        assert!(strangers.take().is_none());
        assert_eq!(strangers.held(), 2);
        drop(a);
        assert!(strangers.take().is_some());
        drop(b);
    }

    #[test]
    fn evicted_links_are_bounded_too() {
        // Strangers arrive faster than evicted links end: at most the
        // capacity of each, held and draining.
        let strangers = Strangers::new(2);
        let mut kept = Vec::new();
        let mut refused = 0;
        for _ in 0..20 {
            match strangers.take() {
                Some(slot) => kept.push(slot),
                None => refused += 1,
            }
            assert!(strangers.held() <= 2);
            assert!(strangers.draining() <= 2);
        }
        assert_eq!(kept.len(), 4);
        assert_eq!(refused, 16);
    }

    #[test]
    fn the_first_message_and_the_eviction_race_on_one_state() {
        for _ in 0..200 {
            let strangers = Strangers::new(1);
            let slot = strangers.take().unwrap();
            let state = slot.state().clone();
            let reader = std::thread::spawn(move || state.heard());
            let newcomer = strangers.take();
            let heard = reader.join().unwrap();
            // Exactly one of the two won.
            assert_eq!(heard, newcomer.is_none());
            assert_eq!(slot.state().is_evicted(), !heard);
        }
    }
}

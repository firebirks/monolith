//! The rates of `docs/RESOURCE_LIMITS.md` section 6 that the runtime
//! enforces (`docs/DESIGN_QUESTIONS.md` P7-8).
//!
//! Each rate is a token bucket of [`RateLimit::burst`] tokens refilled at
//! [`RateLimit::per_minute`], held as one instant: the moment the bucket
//! would be full again (the generic cell rate algorithm). A bucket that is
//! full holds nothing and is dropped. Every bucket is per contact, keyed
//! by what the session authenticated, so that what one contact sends never
//! slows another, and lives in memory: a restart starts every bucket full.
//!
//! - The messages of a contact (`CHAT_MESSAGE_RATE`, `CONTROL_MESSAGE_RATE`,
//!   `FILE_OFFER_RATE`, `PROFILE_UPDATE_RATE`) are counted per contact
//!   record across its sessions, once a session is confirmed. Above the
//!   rate the session waits for its token before it handles the message
//!   and reads nothing meanwhile; the sender is slowed, not cut off. A
//!   FileChunk is not counted, and a Close ends the session.
//! - The sessions a contact opens (`CONTACT_SESSION_RATE`) are counted per
//!   remote identity, for inbound sessions of a contact record. One above
//!   the rate is closed once admitted.
//!
//! The rates of streams and of strangers before and after authentication
//! (`INBOUND_CONNECTION_RATE`, `UNKNOWN_SESSION_RATE`) are not enforced:
//! under Tor nothing tells one of those peers from another, so a bucket
//! for them all would let one peer shut out every other, contacts
//! included; the budgets of section 5 bound what they hold.

use core::time::Duration;
use std::collections::HashMap;
use std::hash::Hash;

use monolith_identity::IdentityPublicKey;
use monolith_protocol::body::Message;
use monolith_protocol::limits::{
    CHAT_MESSAGE_RATE, CONTACT_SESSION_RATE, CONTROL_MESSAGE_RATE, FILE_OFFER_RATE, MAX_CONTACTS,
    PROFILE_UPDATE_RATE, RateLimit,
};
use tokio::time::Instant;

use crate::api::ContactId;

/// One token bucket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Bucket {
    /// When it is full again if no token is taken before.
    full_at: Instant,
}

impl Bucket {
    const fn new(now: Instant) -> Self {
        Self { full_at: now }
    }

    /// Takes a token of `limit` at `now`, or returns when there is one.
    fn take(&mut self, limit: RateLimit, now: Instant) -> Result<(), Instant> {
        let interval = interval(limit);
        let base = self.full_at.max(now);
        let after = base.checked_add(interval).unwrap_or(base);
        let depth = interval.saturating_mul(limit.burst);
        if after.saturating_duration_since(now) > depth {
            return Err(after.checked_sub(depth).unwrap_or(now));
        }
        self.full_at = after;
        Ok(())
    }

    fn is_full(&self, now: Instant) -> bool {
        self.full_at <= now
    }
}

/// The time one token of `limit` takes to come back.
fn interval(limit: RateLimit) -> Duration {
    Duration::from_secs(60)
        .checked_div(limit.per_minute)
        .unwrap_or(Duration::MAX)
}

/// The rate a received message counts against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Chat,
    Control,
    FileOffer,
    Profile,
}

impl Kind {
    /// The rate `message` counts against on a confirmed session, if any.
    pub(crate) const fn of(message: &Message) -> Option<Self> {
        match message {
            Message::ChatMessage { .. } => Some(Self::Chat),
            Message::Ping(_)
            | Message::Pong(_)
            | Message::MessageAck(_)
            | Message::EndpointUpdate(_)
            | Message::FileAccept(_)
            | Message::FileReject(_)
            | Message::FileComplete { .. }
            | Message::FileAbort(_)
            | Message::ContactRequest(_)
            | Message::ContactAccept => Some(Self::Control),
            Message::FileOffer { .. } => Some(Self::FileOffer),
            Message::Profile { .. } => Some(Self::Profile),
            Message::FileChunk(_) | Message::Close => None,
        }
    }

    const fn limit(self) -> RateLimit {
        match self {
            Self::Chat => CHAT_MESSAGE_RATE,
            Self::Control => CONTROL_MESSAGE_RATE,
            Self::FileOffer => FILE_OFFER_RATE,
            Self::Profile => PROFILE_UPDATE_RATE,
        }
    }
}

/// The buckets of one rate, one per key. Only buckets that are not full
/// are held; when `MAX_CONTACTS` are, the full ones are dropped before
/// another is added.
#[derive(Debug)]
struct Buckets<K> {
    held: HashMap<K, Bucket>,
}

impl<K> Default for Buckets<K> {
    fn default() -> Self {
        Self {
            held: HashMap::new(),
        }
    }
}

impl<K: Hash + Eq> Buckets<K> {
    fn take(&mut self, key: K, limit: RateLimit, now: Instant) -> Result<(), Instant> {
        if !self.held.contains_key(&key) && self.held.len() >= MAX_CONTACTS {
            self.held.retain(|_, bucket| !bucket.is_full(now));
        }
        self.held
            .entry(key)
            .or_insert_with(|| Bucket::new(now))
            .take(limit, now)
    }

    fn remove(&mut self, key: &K) {
        self.held.remove(key);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.held.len()
    }
}

/// The rates of the contacts of one local identity.
#[derive(Debug, Default)]
pub(crate) struct ContactRates {
    chat: Buckets<ContactId>,
    control: Buckets<ContactId>,
    file_offer: Buckets<ContactId>,
    profile: Buckets<ContactId>,
    sessions: Buckets<IdentityPublicKey>,
}

impl ContactRates {
    /// Takes a token of `kind` for a message of the contact record
    /// `contact` at `now`, or returns when there is one.
    pub(crate) fn message(
        &mut self,
        contact: ContactId,
        kind: Kind,
        now: Instant,
    ) -> Result<(), Instant> {
        let buckets = match kind {
            Kind::Chat => &mut self.chat,
            Kind::Control => &mut self.control,
            Kind::FileOffer => &mut self.file_offer,
            Kind::Profile => &mut self.profile,
        };
        buckets.take(contact, kind.limit(), now)
    }

    /// Takes a token for a session that `remote` opened at `now`. False if
    /// there is none: the session is above the rate.
    pub(crate) fn session(&mut self, remote: IdentityPublicKey, now: Instant) -> bool {
        self.sessions
            .take(remote, CONTACT_SESSION_RATE, now)
            .is_ok()
    }

    /// The record `contact` ended: what it took is forgotten, since no
    /// session stands for it any more.
    pub(crate) fn ended(&mut self, contact: &ContactId) {
        self.chat.remove(contact);
        self.control.remove(contact);
        self.file_offer.remove(contact);
        self.profile.remove(contact);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::arithmetic_side_effects)]

    use super::*;
    use monolith_identity::IdentitySecretKey;
    use monolith_storage::record::RECORD_ID_LEN;

    fn record(byte: u8) -> ContactId {
        ContactId::from_bytes([byte; RECORD_ID_LEN]).unwrap()
    }

    #[test]
    fn a_bucket_gives_its_burst_at_once_and_then_one_token_per_interval() {
        for limit in [
            CHAT_MESSAGE_RATE,
            CONTROL_MESSAGE_RATE,
            FILE_OFFER_RATE,
            PROFILE_UPDATE_RATE,
            CONTACT_SESSION_RATE,
        ] {
            let start = Instant::now();
            let mut bucket = Bucket::new(start);
            for _ in 0..limit.burst {
                assert_eq!(bucket.take(limit, start), Ok(()));
            }
            let step = interval(limit);
            assert_eq!(bucket.take(limit, start), Err(start + step));
            // Not a moment before.
            assert!(
                bucket
                    .take(limit, start + step - Duration::from_nanos(1))
                    .is_err()
            );
            assert_eq!(bucket.take(limit, start + step), Ok(()));
            assert_eq!(bucket.take(limit, start + step), Err(start + step * 2));
            // Left alone, it fills up to its burst, and no further.
            let later = start + step * (limit.burst + 10);
            assert!(bucket.is_full(later));
            for _ in 0..limit.burst {
                assert_eq!(bucket.take(limit, later), Ok(()));
            }
            assert!(bucket.take(limit, later).is_err());
        }
    }

    #[test]
    fn a_refused_take_takes_nothing() {
        let start = Instant::now();
        let mut bucket = Bucket::new(start);
        for _ in 0..FILE_OFFER_RATE.burst {
            bucket.take(FILE_OFFER_RATE, start).unwrap();
        }
        let before = bucket;
        for _ in 0..100 {
            assert!(bucket.take(FILE_OFFER_RATE, start).is_err());
        }
        assert_eq!(bucket, before);
    }

    #[test]
    fn contacts_and_kinds_are_kept_apart() {
        let now = Instant::now();
        let mut rates = ContactRates::default();
        for _ in 0..FILE_OFFER_RATE.burst {
            rates.message(record(1), Kind::FileOffer, now).unwrap();
        }
        assert!(rates.message(record(1), Kind::FileOffer, now).is_err());
        // Another kind of the same contact, the same kind of another.
        assert_eq!(rates.message(record(1), Kind::Chat, now), Ok(()));
        assert_eq!(rates.message(record(2), Kind::FileOffer, now), Ok(()));
        // A record that ended starts over.
        rates.ended(&record(1));
        assert_eq!(rates.message(record(1), Kind::FileOffer, now), Ok(()));
    }

    #[test]
    fn sessions_are_counted_per_remote_identity() {
        let now = Instant::now();
        let mut rates = ContactRates::default();
        let (one, other) = (
            IdentitySecretKey::from_seed(&[1; 32]).public_key(),
            IdentitySecretKey::from_seed(&[2; 32]).public_key(),
        );
        for _ in 0..CONTACT_SESSION_RATE.burst {
            assert!(rates.session(one, now));
        }
        assert!(!rates.session(one, now));
        assert!(rates.session(other, now));
        assert!(rates.session(one, now + interval(CONTACT_SESSION_RATE)));
    }

    #[test]
    fn full_buckets_are_dropped_before_the_held_ones_grow_past_the_contacts() {
        let start = Instant::now();
        let mut buckets = Buckets::default();
        for n in 0..MAX_CONTACTS {
            buckets.take(n, CHAT_MESSAGE_RATE, start).unwrap();
        }
        assert_eq!(buckets.len(), MAX_CONTACTS);
        // Once every one of them is full again, a new one takes their place.
        let later = start + interval(CHAT_MESSAGE_RATE);
        buckets
            .take(MAX_CONTACTS, CHAT_MESSAGE_RATE, later)
            .unwrap();
        assert_eq!(buckets.len(), 1);
        // One that is not full is kept.
        let mut buckets = Buckets::default();
        for n in 0..MAX_CONTACTS {
            buckets.take(n, CHAT_MESSAGE_RATE, start).unwrap();
        }
        buckets.take(0, CHAT_MESSAGE_RATE, start).unwrap();
        buckets
            .take(MAX_CONTACTS, CHAT_MESSAGE_RATE, later)
            .unwrap();
        assert_eq!(buckets.len(), 2);
    }

    /// A token bucket written apart from [`Bucket`]: a credit of time in
    /// nanoseconds, refilled by the time that passes up to the burst, and
    /// one interval spent for each token.
    #[derive(Clone, Copy, Debug)]
    struct Credit {
        credit: u128,
        at: Instant,
    }

    impl Credit {
        fn step(limit: RateLimit) -> u128 {
            60_000_000_000 / u128::from(limit.per_minute)
        }

        fn new(limit: RateLimit, now: Instant) -> Self {
            Self {
                credit: Self::step(limit) * u128::from(limit.burst),
                at: now,
            }
        }

        fn take(&mut self, limit: RateLimit, now: Instant) -> bool {
            let depth = Self::step(limit) * u128::from(limit.burst);
            self.credit = (self.credit + (now - self.at).as_nanos()).min(depth);
            self.at = now;
            if self.credit >= Self::step(limit) {
                self.credit -= Self::step(limit);
                true
            } else {
                false
            }
        }
    }

    const KINDS: [Kind; 4] = [Kind::Chat, Kind::Control, Kind::FileOffer, Kind::Profile];

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

        /// Any sequence of messages of three contacts, sessions they open
        /// and records that end, in bursts at any times: each take is
        /// allowed exactly when the model of its own contact and rate
        /// allows it, and a refusal names the first instant the model
        /// gives a token.
        #[test]
        fn every_take_is_that_of_a_bucket_of_its_own_contact_and_rate(
            steps in proptest::collection::vec(
                (
                    proptest::prop_oneof![
                        proptest::strategy::Just(0_u64),
                        0_u64..2_000,
                        0_u64..120_000,
                    ],
                    0_usize..3,
                    0_usize..6,
                    1_usize..150,
                ),
                1..60,
            ),
        ) {
            let start = Instant::now();
            let mut now = start;
            let mut rates = ContactRates::default();
            let mut models: HashMap<(usize, usize), Credit> = HashMap::new();
            let remotes: Vec<IdentityPublicKey> = (1..=3_u8)
                .map(|n| IdentitySecretKey::from_seed(&[n; 32]).public_key())
                .collect();
            for (millis, who, what, count) in steps {
                now += Duration::from_millis(millis);
                if what == 5 {
                    rates.ended(&record(u8::try_from(who).unwrap() + 1));
                    for kind in 0..4 {
                        models.remove(&(who, kind));
                    }
                    continue;
                }
                let limit = KINDS.get(what).map_or(CONTACT_SESSION_RATE, |kind| kind.limit());
                for _ in 0..count {
                    let model = models
                        .entry((who, what))
                        .or_insert_with(|| Credit::new(limit, now));
                    let mut probe = *model;
                    let allowed = model.take(limit, now);
                    if let Some(kind) = KINDS.get(what) {
                        match rates.message(record(u8::try_from(who).unwrap() + 1), *kind, now) {
                            Ok(()) => proptest::prop_assert!(allowed),
                            Err(when) => {
                                proptest::prop_assert!(!allowed);
                                // A token at that instant, and none before.
                                let mut before = probe;
                                proptest::prop_assert!(
                                    !before.take(limit, when - Duration::from_nanos(1))
                                );
                                proptest::prop_assert!(probe.take(limit, when));
                            }
                        }
                    } else {
                        proptest::prop_assert_eq!(rates.session(remotes[who], now), allowed);
                    }
                }
            }
        }
    }

    #[test]
    fn every_message_but_a_chunk_and_a_close_is_counted() {
        use monolith_protocol::body::{FileChunk, TransferId};
        let transfer = TransferId::from_bytes([1; 16]);
        assert_eq!(
            Kind::of(&Message::FileChunk(
                FileChunk::new(transfer, vec![0; 4]).unwrap()
            )),
            None
        );
        assert_eq!(Kind::of(&Message::Close), None);
        assert_eq!(Kind::of(&Message::Ping([0; 8])), Some(Kind::Control));
        assert_eq!(Kind::of(&Message::ContactAccept), Some(Kind::Control));
        assert_eq!(Kind::of(&Message::FileAbort(transfer)), Some(Kind::Control));
    }
}

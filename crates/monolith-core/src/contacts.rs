//! The contact store of one local identity: the record of every remote
//! identity it knows, and the sessions admitted for each.
//!
//! The store is the authority for everything a remote identity is to the
//! local one: no record, declined, blocked, requested or accepted; for a
//! contact its credentials (`monolith_protocol::credential::Credentials`),
//! the card the user confirmed for dialing, the verification mark and the
//! local alias; and the sessions that were admitted as the contact's.
//! Every card of a known identity is judged by `contact::decide`, whatever
//! path it took: an admission, an announcement, an import, a confirmation.
//!
//! Serialization. Each remote identity has one slot with its own lock.
//! Every operation on that identity (admission, import, confirmation,
//! announcement, acceptance, block, unblock, deletion, decline, the
//! confirmation of a dial card) takes that lock, does its work on memory,
//! and lets go; nothing waits while it holds it. So the operations on one
//! identity happen in one order, the order in which they took the lock,
//! and operations on different identities do not wait for each other.
//!
//! A second lock guards the map from identities to slots. It is taken
//! before the lock of a slot, held until that lock is taken, and never
//! taken while a slot is locked, so the two cannot deadlock, and a slot
//! that is taken out of the map, which needs both, cannot be found and
//! locked afterwards. An identity without a slot is decided on while the
//! map lock is held, so that a slot made for it comes before or after
//! that decision, never during it.
//!
//! Withdrawal. After every change of a slot, every session that was
//! admitted as the contact's and no longer stands for it is withdrawn in
//! the same step: its key was retired, or the record is no longer a
//! contact's (blocked, deleted). That is one rule for every transition
//! (S49). A session admitted with another standing is on the path of a
//! stranger and is left to it (`docs/PROTOCOL.md` section 12.1).
//!
//! Durability. A change is stamped with a generation of the installation
//! in the same step ([`crate::persist`]). Functions that change something
//! return the generation that what they decided depends on; the caller
//! waits for it to be durable before it uses the result.

use core::fmt;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use monolith_identity::IdentityPublicKey;
use monolith_protocol::ProtocolError;
use monolith_protocol::card::ContactCard;
use monolith_protocol::contact::{self, Context, Decision, RecordKind, Refusal};
use monolith_protocol::credential::{CardRelation, CredentialChange, Credentials};
use monolith_protocol::limits::{MAX_BLOCKED_IDENTITIES, MAX_CONTACTS, MAX_DECLINED_IDENTITIES};
use monolith_protocol::session::PeerRecord;
use monolith_protocol::text::DisplayName;
use monolith_session::{Admitted, InboundPeer, OutboundAdmission, OutboundPeer, SessionError};
use monolith_storage::record::StoredContact;

use crate::identity::RotationId;
use crate::link::{Owner, Withdrawal};
use crate::persist::Durability;

/// Why an operation on the contact store was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreError {
    /// No such contact, request or invitation.
    NotFound,
    /// The identity is blocked. Unblock it first.
    Blocked,
    /// The card is one of the local identity.
    OwnIdentity,
    /// A limit of `docs/RESOURCE_LIMITS.md` section 5 is reached.
    Full,
    /// The card is not the one the operation needs: not the pending
    /// successor for a confirmation, not the active card for a dial
    /// confirmation.
    NotThatCard,
    /// The session the operation came from no longer stands for the
    /// contact.
    Withdrawn,
    /// A card or a stored record is invalid.
    Protocol(ProtocolError),
    /// The change could not be made durable.
    Commit(crate::persist::CommitError),
    /// The installation failed earlier and refuses every change.
    Failed,
    /// The session was admitted by another local identity, or by an
    /// earlier identity of the same keys.
    OtherIdentity,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("not found"),
            Self::Blocked => f.write_str("identity is blocked"),
            Self::OwnIdentity => f.write_str("card of the local identity"),
            Self::Full => f.write_str("limit reached"),
            Self::NotThatCard => f.write_str("not the card this needs"),
            Self::Withdrawn => f.write_str("session withdrawn"),
            Self::Protocol(error) => write!(f, "{error}"),
            Self::Commit(error) => write!(f, "{error}"),
            Self::Failed => f.write_str("storage failed; restart needed"),
            Self::OtherIdentity => f.write_str("session of another local identity"),
        }
    }
}

impl core::error::Error for StoreError {}

impl From<ProtocolError> for StoreError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

impl From<crate::persist::CommitError> for StoreError {
    fn from(error: crate::persist::CommitError) -> Self {
        Self::Commit(error)
    }
}

/// A requested or accepted contact.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ContactRecord {
    /// Which contact this record is, of the contacts this store has made
    /// for any identity: a contact deleted and made again for the same
    /// identity is another one. What was started for one (an announcement)
    /// completes for it only.
    instance: u64,
    kind: RecordKind,
    credentials: Credentials,
    /// The card the user confirmed for dialing (`docs/PROTOCOL.md` section
    /// 11.4, dialing). Kept apart from the credentials: confirming where to
    /// connect never changes which key stands for the contact.
    dial: ContactCard,
    verified: bool,
    alias: Option<DisplayName>,
    /// The local rotation whose successor was sent to this contact on a
    /// confirmed session of the old key. It counts only for that rotation.
    announced: Option<RotationId>,
    /// The local rotation whose new key this contact confirmed a session
    /// with. It counts only for that rotation.
    promoted: Option<RotationId>,
}

/// Which step of a local rotation a contact made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Progress {
    /// It was sent the successor.
    Announced,
    /// It confirmed a session with the new key.
    Promoted,
}

/// What the local identity holds about one remote identity.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Record {
    None,
    Declined,
    Blocked,
    Contact(Box<ContactRecord>),
}

impl Record {
    fn kind(&self) -> RecordKind {
        match self {
            Self::None => RecordKind::None,
            Self::Declined => RecordKind::Declined,
            Self::Blocked => RecordKind::Blocked,
            Self::Contact(contact) => contact.kind,
        }
    }

    fn credentials(&self) -> Option<&Credentials> {
        match self {
            Self::Contact(contact) => Some(&contact.credentials),
            Self::None | Self::Declined | Self::Blocked => None,
        }
    }

    /// The record as the session logic takes it.
    fn peer_record(&mut self) -> PeerRecord<'_> {
        match self {
            Self::None => PeerRecord::None,
            Self::Declined => PeerRecord::Declined,
            Self::Blocked => PeerRecord::Blocked,
            Self::Contact(contact) => match contact.kind {
                RecordKind::Accepted => PeerRecord::Accepted(&mut contact.credentials),
                _ => PeerRecord::Requested(&mut contact.credentials),
            },
        }
    }

    /// Returns true if a session whose peer is `card` stands for a
    /// requested or accepted contact.
    fn stands(&self, card: &ContactCard) -> bool {
        self.credentials()
            .is_some_and(|credentials| credentials.authorizes(card))
    }
}

/// A session admitted as the contact's.
#[derive(Debug)]
struct Tracked {
    withdrawal: Withdrawal,
    card: ContactCard,
}

#[derive(Debug)]
struct Entry {
    record: Record,
    sessions: Vec<Tracked>,
}

impl Entry {
    /// Returns true if the session of `card` and `withdrawal` stands for
    /// this very record: it was admitted as the contact's here (a session
    /// of an earlier record, made again since, is not tracked here), it
    /// was not withdrawn, and its card still stands.
    fn stands_for(&self, card: &ContactCard, withdrawal: &Withdrawal) -> bool {
        !withdrawal.is_withdrawn()
            && self
                .sessions
                .iter()
                .any(|session| session.withdrawal.same_link(withdrawal))
            && self.record.stands(card)
    }

    /// Withdraws every session admitted as the contact's that no longer
    /// stands for it, and forgets those and the links that ended.
    fn reconcile(&mut self) {
        let record = &self.record;
        self.sessions.retain(|tracked| {
            if tracked.withdrawal.is_ended() {
                return false;
            }
            if record.stands(&tracked.card) {
                return true;
            }
            tracked.withdrawal.withdraw();
            false
        });
    }
}

#[derive(Debug)]
struct Slot(Mutex<Entry>);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every change under these locks is made in one step that cannot
    // panic halfway, so a poisoned lock still guards a consistent state.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Counts that limit the store, kept beside the slots.
#[derive(Debug, Default)]
struct Counts {
    contacts: usize,
    blocked: usize,
    /// Declined identities, oldest first.
    declined: VecDeque<IdentityPublicKey>,
}

/// What a card did to a contact when the user imported it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportOutcome {
    /// The identity had no record, or was declined: it is now a requested
    /// contact with this card.
    Created,
    /// The card was judged against the contact's credentials.
    Evaluated {
        /// What the card is.
        relation: Option<CardRelation>,
        /// What it did.
        change: CredentialChange,
    },
}

/// What the store holds about one remote identity, for a front end or a
/// test. A copy; it changes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactView {
    /// The kind of record.
    pub kind: RecordKind,
    /// For a contact: its credentials.
    pub credentials: Option<Credentials>,
    /// For a contact: the card confirmed for dialing.
    pub dial: Option<ContactCard>,
    /// The fingerprint was verified out of band.
    pub verified: bool,
    /// The local alias.
    pub alias: Option<DisplayName>,
    /// The successor of the local rotation in progress was announced to
    /// this contact. What was announced in an earlier rotation does not
    /// count.
    pub successor_announced: bool,
    /// This contact confirmed a session with the new key of the local
    /// rotation in progress.
    pub successor_promoted: bool,
    /// Sessions admitted as the contact's that are still open.
    pub sessions: usize,
}

/// The contact store of one local identity.
pub(crate) struct ContactStore {
    local: IdentityPublicKey,
    slots: Mutex<HashMap<IdentityPublicKey, Arc<Slot>>>,
    counts: Mutex<Counts>,
    durability: Arc<Durability>,
    /// The local identity was deleted: the store refuses everything.
    closed: AtomicBool,
    /// What the withdrawal of every session this store admits is bound
    /// to, so that the store applies what arrives on its own sessions
    /// only. One per store: a store made again for the same keys has
    /// another.
    owner: Arc<Owner>,
    /// The number of the next contact record this store makes.
    instances: AtomicU64,
    /// Steps a test runs at named points, with the identity of the slot
    /// at hand, until one returns true: to place another operation exactly
    /// there.
    #[cfg(test)]
    #[allow(clippy::type_complexity)]
    hooks: Mutex<HashMap<&'static str, Box<dyn FnMut(&IdentityPublicKey) -> bool + Send>>>,
}

impl fmt::Debug for ContactStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContactStore").finish_non_exhaustive()
    }
}

impl ContactStore {
    pub(crate) fn new(local: IdentityPublicKey, durability: Arc<Durability>) -> Self {
        Self {
            local,
            slots: Mutex::new(HashMap::new()),
            counts: Mutex::new(Counts::default()),
            durability,
            closed: AtomicBool::new(false),
            instances: AtomicU64::new(0),
            owner: Arc::new(Owner),
            #[cfg(test)]
            hooks: Mutex::new(HashMap::new()),
        }
    }

    /// Runs the step a test placed at `point`. It is taken out while it
    /// runs, so that it may call the store, and put back unless it is done.
    #[cfg(test)]
    fn run_hook(&self, point: &'static str, identity: &IdentityPublicKey) {
        let step = lock(&self.hooks).remove(point);
        if let Some(mut step) = step {
            if !step(identity) {
                lock(&self.hooks).insert(point, step);
            }
        }
    }

    /// A number for a new contact record.
    fn next_instance(&self) -> u64 {
        self.instances.fetch_add(1, Ordering::SeqCst)
    }

    /// Which contact the record of `identity` is now, if it is one.
    pub(crate) fn contact_instance(&self, identity: &IdentityPublicKey) -> Option<u64> {
        self.with_entry(identity, false, |entry| match entry {
            Some(Entry {
                record: Record::Contact(contact),
                ..
            }) => Some(contact.instance),
            _ => None,
        })
    }

    /// Records that the successor of `rotation` was sent to the contact
    /// `instance` on the session of `card` and `withdrawal`, if all of them
    /// still hold: the record of the identity is that very contact, and the
    /// session was admitted as its own and still stands for it. Returns
    /// the generation the mark depends on, or `None` if it was not made: a
    /// completion for a contact that was deleted, blocked or made again, or
    /// on a session withdrawn meanwhile, counts for nothing. The caller
    /// holds the keys of the identity, with `rotation` in progress.
    pub(crate) fn mark_announced_on(
        &self,
        card: &ContactCard,
        withdrawal: &Withdrawal,
        instance: u64,
        rotation: RotationId,
    ) -> Result<Option<u64>, StoreError> {
        self.mark_on(
            card,
            withdrawal,
            Some(instance),
            rotation,
            Progress::Announced,
        )
    }

    /// Records that the contact confirmed the new key of `rotation` on the
    /// session of `card` and `withdrawal`, made with that key, if the
    /// session was admitted as the own of the contact the record of the
    /// identity is now and still stands for it. Read and marked in one
    /// step: a confirmation does not count for a contact deleted, blocked
    /// or made again after its session was admitted. Returns `None` if the
    /// mark was not made. The caller holds the keys of the identity, with
    /// `rotation` in progress.
    pub(crate) fn mark_promoted_on(
        &self,
        card: &ContactCard,
        withdrawal: &Withdrawal,
        rotation: RotationId,
    ) -> Result<Option<u64>, StoreError> {
        self.mark_on(card, withdrawal, None, rotation, Progress::Promoted)
    }

    /// Marks `progress` of `rotation` at the contact `instance` (any, if
    /// `None`) while the session of `card` and `withdrawal` stands for it.
    fn mark_on(
        &self,
        card: &ContactCard,
        withdrawal: &Withdrawal,
        instance: Option<u64>,
        rotation: RotationId,
        progress: Progress,
    ) -> Result<Option<u64>, StoreError> {
        self.check_failed()?;
        Ok(self.with_entry(card.identity(), false, |entry| {
            let entry = entry?;
            let stands = entry.stands_for(card, withdrawal);
            let Record::Contact(contact) = &mut entry.record else {
                return None;
            };
            if instance.is_some_and(|instance| contact.instance != instance) || !stands {
                return None;
            }
            let mark = match progress {
                Progress::Announced => &mut contact.announced,
                Progress::Promoted => &mut contact.promoted,
            };
            if *mark != Some(rotation) {
                *mark = Some(rotation);
                self.durability.bump();
            }
            Some(self.depends())
        }))
    }

    /// Returns true if this store admitted the session of `withdrawal`.
    pub(crate) fn admitted(&self, withdrawal: &Withdrawal) -> bool {
        withdrawal.admitted_by(&self.owner)
    }

    /// Rebuilds a store from the vault. The records were validated when
    /// the payload was read.
    pub(crate) fn load(
        local: IdentityPublicKey,
        durability: Arc<Durability>,
        rotation: Option<RotationId>,
        contacts: Vec<StoredContact>,
        blocked: &[IdentityPublicKey],
        declined: &[IdentityPublicKey],
    ) -> Result<Self, StoreError> {
        let store = Self::new(local, durability);
        {
            let mut slots = lock(&store.slots);
            let mut counts = lock(&store.counts);
            let mut insert = |identity: IdentityPublicKey, record: Record| {
                if identity == local || slots.contains_key(&identity) {
                    return Err(StoreError::Protocol(ProtocolError::InvalidValue));
                }
                slots.insert(
                    identity,
                    Arc::new(Slot(Mutex::new(Entry {
                        record,
                        sessions: Vec::new(),
                    }))),
                );
                Ok(())
            };
            for stored in contacts {
                let identity = *stored.credentials.identity();
                insert(
                    identity,
                    Record::Contact(Box::new(ContactRecord {
                        instance: store.next_instance(),
                        kind: stored.kind,
                        credentials: stored.credentials,
                        dial: stored.dial,
                        verified: stored.verified,
                        alias: stored.alias,
                        // The marks stored are those of the rotation stored
                        // with them, read back with it.
                        announced: rotation.filter(|_| stored.successor_announced),
                        promoted: rotation.filter(|_| stored.successor_promoted),
                    })),
                )?;
                counts.contacts = counts.contacts.saturating_add(1);
            }
            for identity in blocked {
                insert(*identity, Record::Blocked)?;
                counts.blocked = counts.blocked.saturating_add(1);
            }
            for identity in declined {
                insert(*identity, Record::Declined)?;
                counts.declined.push_back(*identity);
            }
        }
        Ok(store)
    }

    fn check_failed(&self) -> Result<(), StoreError> {
        if self.durability.is_failed() || self.is_closed() {
            Err(StoreError::Failed)
        } else {
            Ok(())
        }
    }

    /// Returns true once the local identity was deleted.
    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Closes the store of a local identity that is being deleted: from
    /// now on it refuses every operation and admission, and every session
    /// it admitted as a contact's is withdrawn. The flag is set before the
    /// sessions are withdrawn, so an operation that passed its check
    /// earlier and admits afterwards is refused when it waits for
    /// durability (`LocalIdentity::wait_durable`).
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.withdraw_all();
    }

    /// Runs `operation` on the entry of `identity` under its lock. Without
    /// a slot, `operation` gets `None`, under the lock of the map, unless
    /// `create` is true; then an empty slot is made first. A slot whose
    /// record is left empty, with no session, is taken out afterwards.
    fn with_entry<R>(
        &self,
        identity: &IdentityPublicKey,
        create: bool,
        operation: impl FnOnce(Option<&mut Entry>) -> R,
    ) -> R {
        let mut slots = lock(&self.slots);
        let slot = match slots.get(identity) {
            Some(slot) => slot.clone(),
            None if create => {
                let slot = Arc::new(Slot(Mutex::new(Entry {
                    record: Record::None,
                    sessions: Vec::new(),
                })));
                slots.insert(*identity, slot.clone());
                slot
            }
            None => return operation(None),
        };
        let mut entry = lock(&slot.0);
        drop(slots);
        let result = operation(Some(&mut entry));
        let empty = entry.record == Record::None && entry.sessions.is_empty();
        drop(entry);
        if empty {
            let mut slots = lock(&self.slots);
            if slots
                .get(identity)
                .is_some_and(|held| Arc::ptr_eq(held, &slot))
            {
                let entry = lock(&slot.0);
                if entry.record == Record::None && entry.sessions.is_empty() {
                    drop(entry);
                    slots.remove(identity);
                }
            }
        }
        result
    }

    /// The generation a decision made now depends on.
    fn depends(&self) -> u64 {
        self.durability.applied()
    }

    /// Admits the initiator of an inbound session: the record of its
    /// identity as it is now, the card it presented, one step. A session
    /// admitted as a contact's is tracked with its withdrawal. Returns the
    /// admission and the generation it depends on.
    pub(crate) fn admit_inbound(
        &self,
        peer: InboundPeer,
        withdrawal: &Withdrawal,
    ) -> Result<(Admitted, u64), SessionError> {
        self.check_failed().map_err(|_| SessionError::Internal)?;
        withdrawal.bind(&self.owner);
        let identity = *peer.card().identity();
        let card = peer.card().clone();
        let admitted = self.with_entry(&identity, false, |entry| match entry {
            None => peer.admit(PeerRecord::None),
            Some(entry) => {
                let before = entry.record.clone();
                let admitted = peer.admit(entry.record.peer_record());
                let admitted = self.kept_on_error(entry, &before, admitted)?;
                self.after_admission(entry, &before, &card, admitted.1.standing, withdrawal);
                Ok(admitted)
            }
        })?;
        Ok((admitted, self.depends()))
    }

    /// Admits the responder of an outbound session the same way. Message
    /// 3 is in the result only if the responder may learn the local
    /// identity.
    pub(crate) fn admit_outbound(
        &self,
        peer: OutboundPeer,
        withdrawal: &Withdrawal,
    ) -> Result<(OutboundAdmission, u64), SessionError> {
        self.check_failed().map_err(|_| SessionError::Internal)?;
        withdrawal.bind(&self.owner);
        let identity = *peer.card().identity();
        let card = peer.card().clone();
        let admitted = self.with_entry(&identity, false, |entry| match entry {
            None => peer.admit(PeerRecord::None),
            Some(entry) => {
                let before = entry.record.clone();
                let admitted = peer.admit(entry.record.peer_record());
                let admitted = self.kept_on_error(entry, &before, admitted)?;
                let standing = match &admitted {
                    OutboundAdmission::Granted { admission, .. }
                    | OutboundAdmission::Refused(admission) => admission.standing,
                };
                self.after_admission(entry, &before, &card, standing, withdrawal);
                Ok(admitted)
            }
        })?;
        Ok((admitted, self.depends()))
    }

    /// An admission that failed after it changed the record (no path does
    /// today: what fails after the credentials changed is internal) keeps
    /// the change as any other: stamped, so that the write under way or the
    /// next one holds it (nothing waits for it here), and with the sessions
    /// that no longer stand withdrawn.
    fn kept_on_error<T>(
        &self,
        entry: &mut Entry,
        before: &Record,
        admitted: Result<T, SessionError>,
    ) -> Result<T, SessionError> {
        if admitted.is_err() && entry.record != *before {
            self.durability.bump();
            entry.reconcile();
        }
        admitted
    }

    /// What follows an admission in the same step: the generation if the
    /// record changed, the session tracked if it is a contact's, and the
    /// withdrawal of every session that no longer stands.
    fn after_admission(
        &self,
        entry: &mut Entry,
        before: &Record,
        card: &ContactCard,
        standing: monolith_protocol::session::Standing,
        withdrawal: &Withdrawal,
    ) {
        if entry.record != *before {
            self.durability.bump();
        }
        if standing.is_contact_record() && entry.record.stands(card) {
            entry.sessions.push(Tracked {
                withdrawal: withdrawal.clone(),
                card: card.clone(),
            });
        }
        entry.reconcile();
    }

    /// Applies `decision`, made for `entry`, to it.
    fn apply(&self, entry: &mut Entry, decision: &Decision) {
        if let (Record::Contact(contact), Some(next)) =
            (&mut entry.record, decision.next_credentials())
        {
            contact.credentials = next.clone();
        }
    }

    /// The user imported `card`. For an identity without a record, or a
    /// declined one, it makes a requested contact with the card, which is
    /// also its card for dialing. For a contact the card is judged by
    /// `contact::decide`: for a requested contact it is the user's choice
    /// of the card to use, for an accepted one never a key change. A card
    /// that changes the active card is confirmed for dialing by the import.
    pub(crate) fn import(&self, card: &ContactCard) -> Result<(ImportOutcome, u64), StoreError> {
        self.check_failed()?;
        if card.identity() == &self.local {
            return Err(StoreError::OwnIdentity);
        }
        self.with_entry(card.identity(), true, |entry| {
            let entry = entry.ok_or(StoreError::NotFound)?;
            let kind = entry.record.kind();
            let decision =
                match contact::decide(kind, entry.record.credentials(), card, Context::Import)? {
                    Ok(decision) => decision,
                    Err(Refusal::Blocked) => return Err(StoreError::Blocked),
                };
            let outcome = if decision.before == decision.after {
                self.apply(entry, &decision);
                if decision.updates_active_card() {
                    if let Record::Contact(contact) = &mut entry.record {
                        contact.dial = contact.credentials.active().clone();
                    }
                }
                ImportOutcome::Evaluated {
                    relation: decision.relation(),
                    change: decision.change().unwrap_or(CredentialChange::Unchanged),
                }
            } else {
                let credentials = decision
                    .next_credentials()
                    .cloned()
                    .ok_or(StoreError::Protocol(ProtocolError::InvalidValue))?;
                let mut counts = lock(&self.counts);
                if counts.contacts >= MAX_CONTACTS {
                    return Err(StoreError::Full);
                }
                counts.contacts = counts.contacts.saturating_add(1);
                if kind == RecordKind::Declined {
                    counts.declined.retain(|held| held != card.identity());
                }
                drop(counts);
                entry.record = Record::Contact(Box::new(ContactRecord {
                    instance: self.next_instance(),
                    kind: decision.after,
                    credentials,
                    dial: card.clone(),
                    verified: false,
                    alias: None,
                    announced: None,
                    promoted: None,
                }));
                ImportOutcome::Created
            };
            if decision.changes_state() {
                self.durability.bump();
            }
            entry.reconcile();
            Ok((outcome, self.depends()))
        })
    }

    /// The user confirmed `card`, shown as the pending successor of the
    /// contact: it takes over, the previous key is retired, its sessions
    /// are withdrawn, and the card is confirmed for dialing.
    pub(crate) fn confirm_pending(&self, card: &ContactCard) -> Result<u64, StoreError> {
        self.check_failed()?;
        self.with_entry(card.identity(), false, |entry| {
            let entry = entry.ok_or(StoreError::NotFound)?;
            // Only a contact has a pending successor to confirm.
            if !entry.record.kind().is_contact() {
                return Err(StoreError::NotFound);
            }
            let decision = contact::decide(
                entry.record.kind(),
                entry.record.credentials(),
                card,
                Context::Confirmation,
            )
            .map_err(|error| match error {
                ProtocolError::InvalidValue => StoreError::NotThatCard,
                other => StoreError::Protocol(other),
            })?
            .map_err(|_| StoreError::Blocked)?;
            self.apply(entry, &decision);
            if let Record::Contact(contact) = &mut entry.record {
                contact.dial = card.clone();
            }
            self.durability.bump();
            entry.reconcile();
            Ok(self.depends())
        })
    }

    /// The user confirmed `card` for dialing. It has to be the active card
    /// of the contact: confirming where to connect never changes which key
    /// stands for it.
    pub(crate) fn confirm_dial(&self, card: &ContactCard) -> Result<u64, StoreError> {
        self.check_failed()?;
        self.with_entry(card.identity(), false, |entry| {
            let Some(Entry {
                record: Record::Contact(contact),
                ..
            }) = entry
            else {
                return Err(StoreError::NotFound);
            };
            if contact.credentials.relation(card)? != CardRelation::Same {
                return Err(StoreError::NotThatCard);
            }
            if contact.dial != *card {
                contact.dial = card.clone();
                self.durability.bump();
            }
            Ok(self.depends())
        })
    }

    /// Blocks `identity`: whatever it was, it is blocked now, and every
    /// session admitted as its contact's is withdrawn in the same step.
    pub(crate) fn block(&self, identity: &IdentityPublicKey) -> Result<u64, StoreError> {
        self.check_failed()?;
        if identity == &self.local {
            return Err(StoreError::OwnIdentity);
        }
        self.with_entry(identity, true, |entry| {
            let entry = entry.ok_or(StoreError::NotFound)?;
            if entry.record == Record::Blocked {
                return Ok(self.depends());
            }
            let mut counts = lock(&self.counts);
            if counts.blocked >= MAX_BLOCKED_IDENTITIES {
                return Err(StoreError::Full);
            }
            counts.blocked = counts.blocked.saturating_add(1);
            match entry.record {
                Record::Contact(_) => counts.contacts = counts.contacts.saturating_sub(1),
                Record::Declined => counts.declined.retain(|held| held != identity),
                Record::None | Record::Blocked => {}
            }
            drop(counts);
            entry.record = Record::Blocked;
            self.durability.bump();
            entry.reconcile();
            Ok(self.depends())
        })
    }

    /// Unblocks `identity`: it has no record afterwards.
    pub(crate) fn unblock(&self, identity: &IdentityPublicKey) -> Result<u64, StoreError> {
        self.check_failed()?;
        self.with_entry(identity, false, |entry| {
            let entry = entry.ok_or(StoreError::NotFound)?;
            if entry.record != Record::Blocked {
                return Err(StoreError::NotFound);
            }
            let mut counts = lock(&self.counts);
            counts.blocked = counts.blocked.saturating_sub(1);
            drop(counts);
            entry.record = Record::None;
            self.durability.bump();
            entry.reconcile();
            Ok(self.depends())
        })
    }

    /// Deletes the contact `identity`: it has no record afterwards, and
    /// every session admitted as its contact's is withdrawn in the same
    /// step. A later card of the identity is a new contact.
    pub(crate) fn delete(&self, identity: &IdentityPublicKey) -> Result<u64, StoreError> {
        self.check_failed()?;
        self.with_entry(identity, false, |entry| {
            let entry = entry.ok_or(StoreError::NotFound)?;
            if !matches!(entry.record, Record::Contact(_)) {
                return Err(StoreError::NotFound);
            }
            let mut counts = lock(&self.counts);
            counts.contacts = counts.contacts.saturating_sub(1);
            drop(counts);
            entry.record = Record::None;
            self.durability.bump();
            entry.reconcile();
            Ok(self.depends())
        })
    }

    /// Declines `identity`, which has no record. When the list is full the
    /// oldest entry is dropped.
    pub(crate) fn decline(&self, identity: &IdentityPublicKey) -> Result<u64, StoreError> {
        self.check_failed()?;
        // Two slots change, the new entry and the oldest one it pushes out,
        // in one step under the lock of the map, which a snapshot holds
        // while it reads every slot: no snapshot sees one change without
        // the other. The slots are locked one after the other.
        let mut slots = lock(&self.slots);
        let slot = match slots.get(identity) {
            Some(slot) => slot.clone(),
            None => {
                let slot = Arc::new(Slot(Mutex::new(Entry {
                    record: Record::None,
                    sessions: Vec::new(),
                })));
                slots.insert(*identity, slot.clone());
                slot
            }
        };
        let dropped = {
            let mut entry = lock(&slot.0);
            if entry.record != Record::None {
                return Err(StoreError::NotFound);
            }
            let mut counts = lock(&self.counts);
            counts.declined.push_back(*identity);
            let dropped = if counts.declined.len() > MAX_DECLINED_IDENTITIES {
                counts.declined.pop_front()
            } else {
                None
            };
            drop(counts);
            entry.record = Record::Declined;
            dropped
        };
        #[cfg(test)]
        self.run_hook("decline", identity);
        if let Some(oldest) = dropped {
            if let Some(old) = slots.get(&oldest).cloned() {
                let mut entry = lock(&old.0);
                if entry.record == Record::Declined {
                    entry.record = Record::None;
                    if entry.sessions.is_empty() {
                        drop(entry);
                        slots.remove(&oldest);
                    }
                }
            }
        }
        self.durability.bump();
        drop(slots);
        Ok(self.depends())
    }

    /// The user accepted the request of the identity of `card`: it becomes
    /// an accepted contact with that card. Fails for a blocked identity
    /// and leaves an existing contact as it is.
    pub(crate) fn accept_request(&self, card: &ContactCard) -> Result<u64, StoreError> {
        self.check_failed()?;
        if card.identity() == &self.local {
            return Err(StoreError::OwnIdentity);
        }
        self.with_entry(card.identity(), true, |entry| {
            let entry = entry.ok_or(StoreError::NotFound)?;
            match entry.record {
                Record::Blocked => return Err(StoreError::Blocked),
                Record::Contact(_) => return Ok(self.depends()),
                Record::None | Record::Declined => {}
            }
            let mut counts = lock(&self.counts);
            if counts.contacts >= MAX_CONTACTS {
                return Err(StoreError::Full);
            }
            counts.contacts = counts.contacts.saturating_add(1);
            counts.declined.retain(|held| held != card.identity());
            drop(counts);
            entry.record = Record::Contact(Box::new(ContactRecord {
                instance: self.next_instance(),
                kind: RecordKind::Accepted,
                credentials: Credentials::new(card.clone()),
                dial: card.clone(),
                verified: false,
                alias: None,
                announced: None,
                promoted: None,
            }));
            self.durability.bump();
            Ok(self.depends())
        })
    }

    /// The peer of a session admitted as a requested contact accepted
    /// (`Action::MarkAccepted`). Applied only while the session still
    /// stands for the record it was admitted for: not for a contact made
    /// again since.
    pub(crate) fn mark_accepted(
        &self,
        session: &ContactCard,
        withdrawal: &Withdrawal,
    ) -> Result<u64, StoreError> {
        self.check_failed()?;
        self.with_entry(session.identity(), false, |entry| {
            let entry = entry.ok_or(StoreError::Withdrawn)?;
            if !entry.stands_for(session, withdrawal) {
                return Err(StoreError::Withdrawn);
            }
            if let Record::Contact(contact) = &mut entry.record {
                if contact.kind == RecordKind::Requested {
                    contact.kind = RecordKind::Accepted;
                    self.durability.bump();
                }
            }
            Ok(self.depends())
        })
    }

    /// An EndpointUpdate with `card` arrived on a session whose peer is
    /// `session`. Applied only while the session stands for the record it
    /// was admitted for; `contact::decide` judges the card, and only a
    /// session of the active
    /// key carries continuity.
    pub(crate) fn announce(
        &self,
        session: &ContactCard,
        card: &ContactCard,
        withdrawal: &Withdrawal,
    ) -> Result<(CredentialChange, u64), StoreError> {
        self.check_failed()?;
        self.with_entry(session.identity(), false, |entry| {
            let entry = entry.ok_or(StoreError::Withdrawn)?;
            if !entry.stands_for(session, withdrawal) {
                return Err(StoreError::Withdrawn);
            }
            let decision = contact::decide(
                entry.record.kind(),
                entry.record.credentials(),
                card,
                Context::Announcement(session),
            )?
            .map_err(|_| StoreError::Blocked)?;
            self.apply(entry, &decision);
            if decision.changes_state() {
                self.durability.bump();
            }
            entry.reconcile();
            Ok((
                decision.change().unwrap_or(CredentialChange::NoContinuity),
                self.depends(),
            ))
        })
    }

    /// Marks the verification of a contact.
    pub(crate) fn set_verified(
        &self,
        identity: &IdentityPublicKey,
        verified: bool,
    ) -> Result<u64, StoreError> {
        self.check_failed()?;
        self.with_entry(identity, false, |entry| {
            let Some(Entry {
                record: Record::Contact(contact),
                ..
            }) = entry
            else {
                return Err(StoreError::NotFound);
            };
            if contact.verified != verified {
                contact.verified = verified;
                self.durability.bump();
            }
            Ok(self.depends())
        })
    }

    /// Records the progress of the local rotation `rotation` at a contact:
    /// its successor was sent to it, or it confirmed the new key. The
    /// caller holds the keys of the identity, so that `rotation` is the
    /// one in progress while the mark is made.
    #[cfg(test)]
    pub(crate) fn mark_rotation(
        &self,
        identity: &IdentityPublicKey,
        rotation: RotationId,
        progress: Progress,
    ) -> Result<u64, StoreError> {
        self.check_failed()?;
        self.with_entry(identity, false, |entry| {
            let Some(Entry {
                record: Record::Contact(contact),
                ..
            }) = entry
            else {
                return Err(StoreError::NotFound);
            };
            let mark = match progress {
                Progress::Announced => &mut contact.announced,
                Progress::Promoted => &mut contact.promoted,
            };
            if *mark != Some(rotation) {
                *mark = Some(rotation);
                self.durability.bump();
            }
            Ok(self.depends())
        })
    }

    /// Returns true if `condition` holds for the marks of `rotation` at
    /// every accepted contact, read in one cut through the store: the lock
    /// of the map is held while every slot is read, so a contact accepted
    /// meanwhile is either seen or accepted after the answer.
    pub(crate) fn every_accepted(&self, rotation: RotationId, progress: Progress) -> bool {
        let slots = lock(&self.slots);
        slots.values().all(|slot| match &lock(&slot.0).record {
            Record::Contact(contact) if contact.kind == RecordKind::Accepted => {
                let mark = match progress {
                    Progress::Announced => contact.announced,
                    Progress::Promoted => contact.promoted,
                };
                mark == Some(rotation)
            }
            _ => true,
        })
    }

    /// Returns true if the session whose peer is `card`, admitted with
    /// `withdrawal`, still stands for the contact it was admitted for.
    pub(crate) fn session_stands(&self, card: &ContactCard, withdrawal: &Withdrawal) -> bool {
        self.with_entry(card.identity(), false, |entry| {
            entry.is_some_and(|entry| entry.stands_for(card, withdrawal))
        })
    }

    /// The kind of record of `identity` now.
    pub(crate) fn kind(&self, identity: &IdentityPublicKey) -> RecordKind {
        self.with_entry(identity, false, |entry| {
            entry.map_or(RecordKind::None, |entry| entry.record.kind())
        })
    }

    /// A copy of what is held about `identity`, or `None` without a record,
    /// with the progress of the local rotation `rotation`, the one in
    /// progress.
    pub(crate) fn view(
        &self,
        identity: &IdentityPublicKey,
        rotation: Option<RotationId>,
    ) -> Option<ContactView> {
        self.with_entry(identity, false, |entry| {
            let entry = entry?;
            entry.reconcile();
            let sessions = entry.sessions.len();
            Some(match &entry.record {
                Record::Contact(contact) => ContactView {
                    kind: contact.kind,
                    credentials: Some(contact.credentials.clone()),
                    dial: Some(contact.dial.clone()),
                    verified: contact.verified,
                    alias: contact.alias.clone(),
                    successor_announced: rotation.is_some() && contact.announced == rotation,
                    successor_promoted: rotation.is_some() && contact.promoted == rotation,
                    sessions,
                },
                other => ContactView {
                    kind: other.kind(),
                    credentials: None,
                    dial: None,
                    verified: false,
                    alias: None,
                    successor_announced: false,
                    successor_promoted: false,
                    sessions,
                },
            })
        })
    }

    /// Every remote identity with a record.
    pub(crate) fn identities(&self) -> Vec<IdentityPublicKey> {
        let mut identities: Vec<IdentityPublicKey> = lock(&self.slots).keys().copied().collect();
        identities.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        identities
    }

    /// Withdraws every session of every contact: the installation failed
    /// or the identity is being removed.
    pub(crate) fn withdraw_all(&self) {
        let slots: Vec<Arc<Slot>> = lock(&self.slots).values().cloned().collect();
        for slot in slots {
            let mut entry = lock(&slot.0);
            for tracked in entry.sessions.drain(..) {
                tracked.withdrawal.withdraw();
            }
        }
    }

    /// The records to store: contacts, blocked, declined (oldest first),
    /// with the progress of the local rotation `rotation`, the one stored
    /// with them. Progress of another rotation is not stored.
    pub(crate) fn snapshot(
        &self,
        rotation: Option<RotationId>,
    ) -> (
        Vec<StoredContact>,
        Vec<IdentityPublicKey>,
        Vec<IdentityPublicKey>,
    ) {
        // One cut through the store: the lock of the map is held while
        // every slot is read. An operation that holds a slot when the
        // snapshot begins ends before that slot is read, and one that comes
        // later waits for the map, so every limit that holds after each
        // operation holds in the snapshot too. Operations that change two
        // slots hold the map throughout (`Self::decline`).
        let slots = lock(&self.slots);
        let mut contacts = Vec::new();
        let mut blocked = Vec::new();
        let mut declined = Vec::new();
        for (identity, slot) in slots.iter() {
            let identity = *identity;
            let entry = lock(&slot.0);
            match &entry.record {
                Record::Contact(contact) => contacts.push(StoredContact {
                    kind: contact.kind,
                    credentials: contact.credentials.clone(),
                    dial: contact.dial.clone(),
                    verified: contact.verified,
                    alias: contact.alias.clone(),
                    successor_announced: rotation.is_some() && contact.announced == rotation,
                    successor_promoted: rotation.is_some() && contact.promoted == rotation,
                }),
                Record::Blocked => blocked.push(identity),
                Record::Declined => declined.push(identity),
                Record::None => {}
            }
            drop(entry);
            #[cfg(test)]
            self.run_hook("snapshot", &identity);
        }
        // Declined in the order of the list; what the list holds and what
        // the slots say agree, since both change in one step.
        let order: HashMap<IdentityPublicKey, usize> = lock(&self.counts)
            .declined
            .iter()
            .enumerate()
            .map(|(position, identity)| (*identity, position))
            .collect();
        drop(slots);
        declined.sort_by_key(|identity| order.get(identity).copied().unwrap_or(usize::MAX));
        contacts.sort_by(|a, b| {
            a.credentials
                .identity()
                .as_bytes()
                .cmp(b.credentials.identity().as_bytes())
        });
        blocked.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        (contacts, blocked, declined)
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::arithmetic_side_effects,
        clippy::indexing_slicing,
        clippy::cast_possible_truncation
    )]

    use std::sync::atomic::AtomicUsize;
    use std::time::{Duration, Instant};

    use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};
    use monolith_protocol::card::EndpointSet;
    use monolith_session::{LocalParty, TransportSecretKey};

    use super::*;

    /// A step placed at a point of the store (`ContactStore::hooks`).
    type SlotHook = Box<dyn FnMut(&IdentityPublicKey) -> bool + Send>;

    fn seed(n: u32, salt: u8) -> [u8; 32] {
        let mut seed = [salt; 32];
        seed[..4].copy_from_slice(&n.to_be_bytes());
        seed
    }

    fn id(n: u32) -> IdentityPublicKey {
        IdentitySecretKey::from_seed(&seed(n, 1)).public_key()
    }

    fn card(n: u32) -> ContactCard {
        let endpoint = OnionServiceKey::from_bytes(
            IdentitySecretKey::from_seed(&seed(n, 2))
                .public_key()
                .as_bytes(),
        )
        .unwrap();
        LocalParty::issue(
            &IdentitySecretKey::from_seed(&seed(n, 1)),
            TransportSecretKey::from_bytes(&seed(n, 3)).unwrap(),
            EndpointEpoch::FIRST,
            EndpointSet::single(endpoint),
        )
        .unwrap()
        .card()
        .clone()
    }

    #[test]
    fn no_operation_runs_between_the_reads_of_a_snapshot() {
        // A snapshot is one cut through the store because it holds the
        // lock of the map from the first slot it reads to the last: no
        // operation, which takes the map first, can run in between. The
        // hook looks at that at every slot the snapshot reads, in whatever
        // order the map yields them: the lock must be held. Where it is
        // not, an operation does run there, a decline of the full list
        // that takes a new slot and pushes the oldest out, and the cut the
        // snapshot returns can show it.
        let store = Arc::new(ContactStore::new(id(0), Durability::new()));
        for n in 0..MAX_DECLINED_IDENTITIES as u32 {
            store.decline(&id(10_000 + n)).unwrap();
        }
        let free_at = Arc::new(Mutex::new(Vec::new()));
        let step: SlotHook = Box::new({
            let store = Arc::downgrade(&store);
            let free_at = free_at.clone();
            let mut reads = 0_usize;
            move |_: &IdentityPublicKey| {
                reads += 1;
                if let Some(store) = store.upgrade() {
                    let free = store.slots.try_lock().is_ok();
                    if free {
                        lock(&free_at).push(reads);
                        let _ = store.decline(&id(100_000 + reads as u32));
                    }
                }
                false
            }
        });
        lock(&store.hooks).insert("snapshot", step);
        let (_, _, declined) = store.snapshot(None);
        assert_eq!(
            *lock(&free_at),
            Vec::<usize>::new(),
            "the snapshot let go of the map between its reads"
        );
        assert_eq!(declined.len(), MAX_DECLINED_IDENTITIES);
    }

    #[test]
    fn no_snapshot_runs_between_the_two_slots_of_a_decline() {
        // The declined list is full; a decline takes a slot of its own and
        // pushes the oldest declined identity out. Between the two, a
        // snapshot is taken if it can be: it must see the list full, not
        // one over the bound, which the vault refuses to read.
        let store = Arc::new(ContactStore::new(id(0), Durability::new()));
        for n in 0..MAX_DECLINED_IDENTITIES as u32 {
            store.decline(&id(10_000 + n)).unwrap();
        }
        let seen = Arc::new(Mutex::new(None));
        let step: SlotHook = Box::new({
            let store = Arc::downgrade(&store);
            let seen = seen.clone();
            move |_: &IdentityPublicKey| {
                if let Some(store) = store.upgrade() {
                    let free = store.slots.try_lock().is_ok();
                    if free {
                        let (_, _, declined) = store.snapshot(None);
                        *lock(&seen) = Some(declined.len());
                    }
                }
                true
            }
        });
        lock(&store.hooks).insert("decline", step);
        store.decline(&id(99_999)).unwrap();
        let seen = *lock(&seen);
        assert!(
            seen.is_none_or(|count| count == MAX_DECLINED_IDENTITIES),
            "{seen:?}"
        );
        let (_, _, declined) = store.snapshot(None);
        assert_eq!(declined.len(), MAX_DECLINED_IDENTITIES);
    }

    #[test]
    fn the_progress_of_a_rotation_counts_only_for_that_rotation() {
        let store = ContactStore::new(id(0), Durability::new());
        store.accept_request(&card(1)).unwrap();
        store.accept_request(&card(2)).unwrap();
        let (first, second) = (RotationId::for_test(7, 0), RotationId::for_test(7, 1));
        for progress in [Progress::Announced, Progress::Promoted] {
            store.mark_rotation(&id(1), first, progress).unwrap();
            store.mark_rotation(&id(2), first, progress).unwrap();
            assert!(store.every_accepted(first, progress));
            assert!(!store.every_accepted(second, progress));
            // One contact made the step again for the second rotation.
            store.mark_rotation(&id(1), second, progress).unwrap();
            assert!(!store.every_accepted(second, progress));
            let marked = |rotation, contact: u32| {
                let view = store.view(&id(contact), Some(rotation)).unwrap();
                match progress {
                    Progress::Announced => view.successor_announced,
                    Progress::Promoted => view.successor_promoted,
                }
            };
            assert!(marked(second, 1));
            assert!(!marked(second, 2));
            assert!(!marked(first, 1));
            // Stored with the second rotation: only what was done for it.
            let (held, _, _) = store.snapshot(Some(second));
            let stored = |contact: u32| {
                let record = held
                    .iter()
                    .find(|record| record.credentials.identity() == &id(contact))
                    .unwrap();
                match progress {
                    Progress::Announced => record.successor_announced,
                    Progress::Promoted => record.successor_promoted,
                }
            };
            assert!(stored(1));
            assert!(!stored(2));
            // Without a rotation nothing is marked.
            let (held, _, _) = store.snapshot(None);
            assert!(
                held.iter()
                    .all(|record| !record.successor_announced && !record.successor_promoted)
            );
        }
    }

    /// The fewest and the most of something a snapshot held.
    #[derive(Debug, PartialEq, Eq)]
    struct Range {
        fewest: usize,
        most: usize,
    }

    impl Range {
        const fn new() -> Self {
            Self {
                fewest: usize::MAX,
                most: 0,
            }
        }

        fn add(&mut self, count: usize) {
            self.fewest = self.fewest.min(count);
            self.most = self.most.max(count);
        }
    }

    /// Runs `round` until `snapshots` snapshots were taken meanwhile on
    /// another thread, or 20 seconds passed, and returns how many contacts
    /// and declined identities they held.
    fn race(
        store: &Arc<ContactStore>,
        snapshots: usize,
        mut round: impl FnMut(u32),
    ) -> (Range, Range) {
        let stop = Arc::new(AtomicBool::new(false));
        let taken = Arc::new(AtomicUsize::new(0));
        let reader = {
            let (store, stop, taken) = (store.clone(), stop.clone(), taken.clone());
            std::thread::spawn(move || {
                let (mut contacts, mut declined) = (Range::new(), Range::new());
                while !stop.load(Ordering::SeqCst) {
                    let (held, _, refused) = store.snapshot(None);
                    contacts.add(held.len());
                    declined.add(refused.len());
                    taken.fetch_add(1, Ordering::SeqCst);
                }
                (contacts, declined)
            })
        };
        let started = Instant::now();
        let mut n = 0;
        while taken.load(Ordering::SeqCst) < snapshots
            && started.elapsed() < Duration::from_secs(20)
        {
            round(n);
            n += 1;
        }
        stop.store(true, Ordering::SeqCst);
        reader.join().unwrap()
    }

    #[test]
    fn a_snapshot_is_one_cut_through_the_store() {
        // Operations that keep the lists at their bounds run on one thread
        // while snapshots are taken on another. A snapshot sees every
        // operation whole, so it never holds more contacts or declined
        // identities than the bounds, which the vault refuses to read.
        let store = Arc::new(ContactStore::new(id(0), Durability::new()));
        let cards: Vec<ContactCard> = (1..=MAX_CONTACTS as u32 + 1).map(card).collect();
        for card in &cards[..MAX_CONTACTS] {
            store.import(card).unwrap();
        }
        let spare = MAX_CONTACTS;
        store.decline(cards[spare].identity()).unwrap();
        // A contact goes and a declined identity, which has a slot already,
        // becomes one, then the one that went is declined: two identities
        // take turns, and the contacts stay at the bound.
        let mut out = spare;
        let (contacts, _) = race(&store, 25, |n| {
            let going = (n as usize) % MAX_CONTACTS;
            let going = if going == out {
                (going + 1) % MAX_CONTACTS
            } else {
                going
            };
            store.delete(cards[going].identity()).unwrap();
            store.import(&cards[out]).unwrap();
            store.decline(cards[going].identity()).unwrap();
            out = going;
        });

        // Every snapshot is a state between two operations: all contacts,
        // or all but the one between its deletion and the import.
        assert_eq!(
            contacts,
            Range {
                fewest: MAX_CONTACTS - 1,
                most: MAX_CONTACTS
            }
        );

        // A full declined list: each decline pushes the oldest out.
        let store = Arc::new(ContactStore::new(id(0), Durability::new()));
        for n in 0..MAX_DECLINED_IDENTITIES as u32 {
            store.decline(&id(10_000 + n)).unwrap();
        }
        let (_, declined) = race(&store, 3000, |n| {
            store.decline(&id(100_000 + n)).unwrap();
        });
        assert_eq!(
            declined,
            Range {
                fewest: MAX_DECLINED_IDENTITIES,
                most: MAX_DECLINED_IDENTITIES
            }
        );
    }
}

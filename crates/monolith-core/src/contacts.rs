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

use crate::link::Withdrawal;
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
    kind: RecordKind,
    credentials: Credentials,
    /// The card the user confirmed for dialing (`docs/PROTOCOL.md` section
    /// 11.4, dialing). Kept apart from the credentials: confirming where to
    /// connect never changes which key stands for the contact.
    dial: ContactCard,
    verified: bool,
    alias: Option<DisplayName>,
    successor_announced: bool,
    successor_promoted: bool,
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
    /// The successor of a local rotation was announced to this contact.
    pub successor_announced: bool,
    /// This contact confirmed a session with the new local key.
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
        }
    }

    /// Rebuilds a store from the vault. The records were validated when
    /// the payload was read.
    pub(crate) fn load(
        local: IdentityPublicKey,
        durability: Arc<Durability>,
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
                        kind: stored.kind,
                        credentials: stored.credentials,
                        dial: stored.dial,
                        verified: stored.verified,
                        alias: stored.alias,
                        successor_announced: stored.successor_announced,
                        successor_promoted: stored.successor_promoted,
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
        if self.durability.is_failed() {
            Err(StoreError::Failed)
        } else {
            Ok(())
        }
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
        let identity = *peer.card().identity();
        let card = peer.card().clone();
        let admitted = self.with_entry(&identity, false, |entry| match entry {
            None => peer.admit(PeerRecord::None),
            Some(entry) => {
                let before = entry.record.clone();
                let admitted = peer.admit(entry.record.peer_record())?;
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
        let identity = *peer.card().identity();
        let card = peer.card().clone();
        let admitted = self.with_entry(&identity, false, |entry| match entry {
            None => peer.admit(PeerRecord::None),
            Some(entry) => {
                let before = entry.record.clone();
                let admitted = peer.admit(entry.record.peer_record())?;
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
                    kind: decision.after,
                    credentials,
                    dial: card.clone(),
                    verified: false,
                    alias: None,
                    successor_announced: false,
                    successor_promoted: false,
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
        let dropped = self.with_entry(identity, true, |entry| {
            let entry = entry.ok_or(StoreError::NotFound)?;
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
            self.durability.bump();
            Ok(dropped)
        })?;
        if let Some(oldest) = dropped {
            // Another slot: taken after the first was let go.
            self.with_entry(&oldest, false, |entry| {
                if let Some(entry) = entry {
                    if entry.record == Record::Declined {
                        entry.record = Record::None;
                        self.durability.bump();
                    }
                }
            });
        }
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
                kind: RecordKind::Accepted,
                credentials: Credentials::new(card.clone()),
                dial: card.clone(),
                verified: false,
                alias: None,
                successor_announced: false,
                successor_promoted: false,
            }));
            self.durability.bump();
            Ok(self.depends())
        })
    }

    /// The peer of a session admitted as a requested contact accepted
    /// (`Action::MarkAccepted`). Applied only while the session still
    /// stands for the contact.
    pub(crate) fn mark_accepted(
        &self,
        session: &ContactCard,
        withdrawal: &Withdrawal,
    ) -> Result<u64, StoreError> {
        self.check_failed()?;
        self.with_entry(session.identity(), false, |entry| {
            let entry = entry.ok_or(StoreError::Withdrawn)?;
            if withdrawal.is_withdrawn() || !entry.record.stands(session) {
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
    /// `session`. Applied only while the session stands for the contact;
    /// `contact::decide` judges the card, and only a session of the active
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
            if withdrawal.is_withdrawn() || !entry.record.stands(session) {
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

    /// Records the progress of a local rotation at a contact.
    pub(crate) fn set_rotation_marks(
        &self,
        identity: &IdentityPublicKey,
        announced: Option<bool>,
        promoted: Option<bool>,
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
            let before = (contact.successor_announced, contact.successor_promoted);
            if let Some(announced) = announced {
                contact.successor_announced = announced;
            }
            if let Some(promoted) = promoted {
                contact.successor_promoted = promoted;
            }
            if before != (contact.successor_announced, contact.successor_promoted) {
                self.durability.bump();
            }
            Ok(self.depends())
        })
    }

    /// Clears the rotation marks of every contact: a new rotation begins,
    /// or the last one ended.
    pub(crate) fn clear_rotation_marks(&self) {
        for identity in self.identities() {
            let _ = self.set_rotation_marks(&identity, Some(false), Some(false));
        }
    }

    /// Returns true if the session whose peer is `card`, admitted with
    /// `withdrawal`, still stands for a contact.
    pub(crate) fn session_stands(&self, card: &ContactCard, withdrawal: &Withdrawal) -> bool {
        !withdrawal.is_withdrawn()
            && self.with_entry(card.identity(), false, |entry| {
                entry.is_some_and(|entry| entry.record.stands(card))
            })
    }

    /// The kind of record of `identity` now.
    pub(crate) fn kind(&self, identity: &IdentityPublicKey) -> RecordKind {
        self.with_entry(identity, false, |entry| {
            entry.map_or(RecordKind::None, |entry| entry.record.kind())
        })
    }

    /// A copy of what is held about `identity`, or `None` without a record.
    pub(crate) fn view(&self, identity: &IdentityPublicKey) -> Option<ContactView> {
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
                    successor_announced: contact.successor_announced,
                    successor_promoted: contact.successor_promoted,
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

    /// The records to store: contacts, blocked, declined (oldest first).
    pub(crate) fn snapshot(
        &self,
    ) -> (
        Vec<StoredContact>,
        Vec<IdentityPublicKey>,
        Vec<IdentityPublicKey>,
    ) {
        let slots: Vec<(IdentityPublicKey, Arc<Slot>)> = lock(&self.slots)
            .iter()
            .map(|(identity, slot)| (*identity, slot.clone()))
            .collect();
        let mut contacts = Vec::new();
        let mut blocked = Vec::new();
        let mut declined = Vec::new();
        for (identity, slot) in slots {
            let entry = lock(&slot.0);
            match &entry.record {
                Record::Contact(contact) => contacts.push(StoredContact {
                    kind: contact.kind,
                    credentials: contact.credentials.clone(),
                    dial: contact.dial.clone(),
                    verified: contact.verified,
                    alias: contact.alias.clone(),
                    successor_announced: contact.successor_announced,
                    successor_promoted: contact.successor_promoted,
                }),
                Record::Blocked => blocked.push(identity),
                Record::Declined => declined.push(identity),
                Record::None => {}
            }
        }
        // Declined in the order of the list; what the list holds and what
        // the slots say agree, since both change in one step.
        let order = lock(&self.counts).declined.clone();
        declined.sort_by_key(|identity| order.iter().position(|held| held == identity));
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

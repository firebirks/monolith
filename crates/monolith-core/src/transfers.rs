//! The file transfers of the local identities
//! (`docs/adr/0013-file-transfer.md`, `docs/adr/0014-transfer-staging.md`).
//!
//! [`TransferState`] is the state machine: plain data and synchronous
//! transitions, no I/O, no clock, no random source; the time is an
//! argument, in seconds since the Unix epoch. A transfer is keyed by its
//! local identity, its contact record, its direction and the identifier
//! the offerer drew ([`Key`]): the same 16 bytes from two contact records,
//! or in two directions, are two transfers, and a contact made again for
//! the same person inherits nothing. Its transitions, each with the
//! authority that makes it:
//!
//! | From | To | Authority |
//! | --- | --- | --- |
//! | (none) | outgoing waiting | the user, once the file is staged |
//! | waiting | sending | an acceptance of the peer, on a session the offer was made on |
//! | waiting, sending | delivered | a completion of the peer with the digest staged |
//! | waiting, sending | failed | a completion with another digest, a staged file that does not read, no progress for the stall limit |
//! | waiting, sending | rejected | a rejection of the peer |
//! | waiting, sending | cancelled | the user, an abort of the peer, the end of the contact |
//! | cancelled | delivered | a completion of the peer with the digest staged |
//! | (none) | incoming offered | an offer of the peer within the limits |
//! | offered | accepted | the user, with the blob reserved |
//! | offered | rejected | the user |
//! | offered | expired | `FILE_OFFER_TIMEOUT` |
//! | offered, accepted | cancelled | the user (accepted only), an abort of the peer, the end of the contact |
//! | accepted | complete | the whole size staged and its digest verified |
//! | accepted | failed | a digest that differs, a blob that cannot be written, no progress, the offerer gone |
//!
//! Every other change of a record changes a flag or a deadline: that the
//! peer read an offer or an answer, that a file was saved or removed from
//! the interface, a stall deadline moved by progress. A record of an
//! ended incoming transfer goes when its retention ends; an ended outgoing
//! one when the user removes it or room is made. Every limit is checked
//! before anything changes, and nothing that is not ended, nor anything
//! the peer may still need, is removed to make room.
//!
//! What a record does not hold is kept beside it, in memory, and rebuilt
//! after a restart: how much of a transfer is known to be staged or
//! acknowledged, a transfer held after a full disk, the sessions in a row
//! without an offer.
//!
//! `Transfers` holds the state of one installation under one lock, stamps
//! every change with a generation of its own and writes it to the transfer
//! store with the write job of [`crate::persist`]. A change is used (a
//! message written, a command reported done, an event raised) only once
//! its generation is durable.

use core::fmt;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use monolith_protocol::limits::{
    ENDED_TRANSFER_RETENTION, FILE_OFFER_TIMEOUT, MAX_ACTIVE_TRANSFERS,
    MAX_ACTIVE_TRANSFERS_PER_CONTACT, MAX_EPHEMERAL_STAGED_BYTES, MAX_PENDING_FILE_OFFERS,
    MAX_PENDING_FILE_OFFERS_PER_CONTACT, MAX_STAGED_INCOMING_BYTES, MAX_STAGED_OUTGOING_BYTES,
    MAX_TRANSFER_RECORDS, MAX_TRANSFER_RECORDS_PER_CONTACT, MAX_TRANSFER_SIZE, MAX_WAITING_OFFERS,
    MAX_WAITING_OFFERS_PER_CONTACT, TRANSFER_HOLD, TRANSFER_ID_LEN, TRANSFER_PROGRESS_BYTES,
    TRANSFER_PROGRESS_RECORD_INTERVAL, TRANSFER_STALL_LIMIT, TRANSFER_UNOFFERED_SESSIONS,
};
use monolith_protocol::text::Filename;
use monolith_storage::StorageError;
use monolith_storage::blob::BlobId;
use monolith_storage::record::{ContactId, IdentityId};
use monolith_storage::transfers::{
    Direction, Flags, Reason, State, TransferRecord, TransferRecords,
};
use tokio::sync::watch;

use crate::api::{TransferReason, TransferRef, TransferStatus, TransferView};
use crate::blobs::Blobs;
use crate::persist::{CommitError, Durability, Store, Write};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every transition is one step that cannot panic halfway, so a
    // poisoned lock still guards a consistent state.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

const STALL: u64 = TRANSFER_STALL_LIMIT.as_secs();
const RETENTION: u64 = ENDED_TRANSFER_RETENTION.as_secs();
const OFFER_TIMEOUT: u64 = FILE_OFFER_TIMEOUT.as_secs();
const RECORD_INTERVAL: u64 = TRANSFER_PROGRESS_RECORD_INTERVAL.as_secs();
const HOLD: u64 = TRANSFER_HOLD.as_secs();
/// An ended incoming record older than this is one no offerer can still
/// offer: the stall limit of the offerer, and a day for clocks that differ.
const UNOFFERABLE_AFTER: u64 = STALL.saturating_add(24 * 60 * 60);

/// A transfer: its local identity, its contact record, its direction and
/// the identifier the offerer drew.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Key {
    pub(crate) identity: IdentityId,
    pub(crate) contact: ContactId,
    pub(crate) direction: Direction,
    pub(crate) transfer: [u8; TRANSFER_ID_LEN],
}

impl Key {
    fn of(record: &TransferRecord) -> Self {
        Self {
            identity: record.identity,
            contact: record.contact,
            direction: record.direction,
            transfer: record.transfer,
        }
    }

    /// The handle a front end names the transfer by.
    pub(crate) const fn reference(&self) -> TransferRef {
        TransferRef::new(
            self.identity,
            self.contact,
            matches!(self.direction, Direction::Incoming),
            self.transfer,
        )
    }

    /// The transfer a front end names.
    pub(crate) const fn named(reference: &TransferRef) -> Self {
        Self {
            identity: reference.identity,
            contact: reference.contact,
            direction: if reference.incoming {
                Direction::Incoming
            } else {
                Direction::Outgoing
            },
            transfer: reference.transfer(),
        }
    }

    fn matches(&self, record: &TransferRecord) -> bool {
        record.identity == self.identity
            && record.contact == self.contact
            && record.direction == self.direction
            && record.transfer == self.transfer
    }
}

/// What is known of a transfer beside its record, in memory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Memo {
    /// The stall deadline that holds now: the record's, or a later one
    /// that progress set and the record will get with its next write.
    deadline: u64,
    /// When the record's deadline was written last; 0 when not known.
    recorded: u64,
    /// The bytes the progress counted last was counted at.
    progress: u64,
    /// The bytes acknowledged (outgoing) or staged durably (incoming), if
    /// known.
    held: Option<u64>,
    /// Incoming accepted: held after a write failed for a full disk or a
    /// held file, until then.
    hold_until: Option<u64>,
    /// Sessions in a row that counted and brought no offer of it.
    unoffered: usize,
    /// An incoming cancellation the offerer did not read no longer claims
    /// the scheduler.
    unclaimed: bool,
    /// Outgoing: offered on a session since the installation opened.
    offered: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    record: TransferRecord,
    memo: Memo,
}

impl Memo {
    /// What is known beside `record` when nothing more is.
    fn of(record: &TransferRecord) -> Self {
        Self {
            deadline: record.deadline,
            held: (record.direction == Direction::Incoming && record.state == State::Complete)
                .then_some(record.size),
            ..Self::default()
        }
    }
}

impl Entry {
    fn new(record: TransferRecord) -> Self {
        let memo = Memo::of(&record);
        Self { record, memo }
    }

    fn key(&self) -> Key {
        Key::of(&self.record)
    }

    fn outgoing(&self) -> bool {
        self.record.direction == Direction::Outgoing
    }

    fn unread(&self) -> bool {
        self.record.flags.has(Flags::UNREAD)
    }

    /// An outgoing record that is ended: nothing more is sent or told for
    /// it.
    fn is_ended_outgoing(&self) -> bool {
        self.outgoing()
            && match self.record.state {
                State::Delivered | State::Rejected | State::Failed => true,
                State::Cancelled => !self.unread(),
                _ => false,
            }
    }

    /// When an ended incoming record ended.
    fn ended_at(&self) -> u64 {
        self.record.deadline.saturating_sub(RETENTION)
    }

    /// The peer need not be told any more: the flag goes, and an ended
    /// outgoing record has no deadline.
    fn untell(&mut self) -> Change {
        if !self.unread() {
            return Change::default();
        }
        self.record.flags = self.record.flags.with(Flags::UNREAD, false);
        if self.outgoing() {
            self.record.deadline = 0;
            self.memo.deadline = 0;
        }
        Change {
            changed: true,
            ..Change::default()
        }
    }

    /// Takes the blob away from the record, for removal once the change is
    /// durable.
    fn free(&mut self) -> Option<BlobId> {
        self.record.blob.take()
    }

    /// Ends the transfer in `state`, with `reason`, as of `now`: its blob
    /// goes, the flag that the peer has to be told is set if `tell`, and
    /// the deadline is what the new state needs.
    fn end(&mut self, state: State, reason: Option<Reason>, tell: bool, now: u64) -> Change {
        let freed = if state == State::Complete {
            None
        } else {
            self.free()
        };
        self.record.state = state;
        self.record.reason = reason;
        self.record.flags = self.record.flags.with(Flags::UNREAD, tell);
        if !self.outgoing() && state != State::Complete {
            self.record.digest = [0; 32];
            self.record.flags = self.record.flags.with(Flags::SAVING, false);
        }
        self.record.deadline = if self.outgoing() {
            if tell { now.saturating_add(STALL) } else { 0 }
        } else if state == State::Complete {
            0
        } else {
            now.saturating_add(RETENTION)
        };
        self.memo.deadline = self.record.deadline;
        self.memo.recorded = now;
        self.memo.hold_until = None;
        self.memo.unoffered = 0;
        self.memo.unclaimed = false;
        Change {
            changed: true,
            freed,
            progressed: true,
        }
    }

    /// `bytes` are acknowledged or staged now. Progress counts when they
    /// grew by `TRANSFER_PROGRESS_BYTES` since it counted last, or reached
    /// the size: the stall deadline moves, and the record gets it if its
    /// deadline was not written within `TRANSFER_PROGRESS_RECORD_INTERVAL`.
    fn advance(&mut self, bytes: u64, now: u64) -> Change {
        self.memo.held = Some(bytes);
        let size = self.record.size;
        let grew = bytes >= self.memo.progress.saturating_add(TRANSFER_PROGRESS_BYTES)
            || (bytes == size && self.memo.progress < size);
        if !grew {
            return Change::default();
        }
        self.memo.progress = bytes;
        let deadline = now.saturating_add(STALL);
        self.memo.deadline = deadline;
        let write = now.saturating_sub(self.memo.recorded) >= RECORD_INTERVAL;
        if write {
            self.record.deadline = deadline;
            self.memo.recorded = now;
        }
        Change {
            changed: write,
            freed: None,
            progressed: true,
        }
    }
}

/// What a transition did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Change {
    /// A record changed: the change is to be made durable, and reported.
    pub(crate) changed: bool,
    /// A blob no record names any more, to remove once the change is
    /// durable.
    pub(crate) freed: Option<BlobId>,
    /// The transfer made progress: a change of its state, or bytes that
    /// count (see [`Entry::advance`]).
    pub(crate) progressed: bool,
}

/// Why a transition was refused. Nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The size is above `MAX_TRANSFER_SIZE`.
    TooLarge,
    /// Too many offers wait for an answer.
    TooManyWaiting,
    /// Too many incoming transfers are accepted and not complete.
    TooManyActive,
    /// The declared sizes staged would pass their limit.
    StagingFull,
    /// No record can be removed to make room.
    ListFull,
    /// The identifier is held for the contact already. Another is drawn.
    IdTaken,
    /// No such transfer, or not in a state the transition applies to.
    Unknown,
    /// A save of the file is under way.
    Busy,
}

/// What the receiver answers to an offer, from what it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Nothing.
    Nothing,
    /// FileAccept with the bytes held durably.
    Accept(u64),
    /// FileComplete with the digest verified.
    Complete([u8; 32]),
    /// FileReject.
    Reject,
    /// FileAbort.
    Abort,
}

/// What became of an offer of the peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Offer {
    /// A new offer, recorded: to make durable and show to the user.
    New,
    /// To answer from what is held, or refused without a record.
    Answer(Answer),
    /// How much of the transfer is staged is not known yet: its blob is
    /// scanned, and the offer looked at again.
    Scan(BlobId),
    /// The offer differs from the record of its key.
    Violation,
}

/// An outgoing transfer to offer on a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Outgoing {
    pub(crate) transfer: [u8; TRANSFER_ID_LEN],
    pub(crate) size: u64,
    pub(crate) name: Filename,
    /// Cancelled while the receiver could not be told: FileAbort follows
    /// the offer.
    pub(crate) cancelled: bool,
}

/// An outgoing transfer once its file is staged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Staged {
    pub(crate) identity: IdentityId,
    pub(crate) contact: ContactId,
    pub(crate) transfer: [u8; TRANSFER_ID_LEN],
    pub(crate) size: u64,
    pub(crate) name: Filename,
    pub(crate) digest: [u8; 32],
    pub(crate) blob: BlobId,
}

/// A complete incoming transfer whose save began.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Saving {
    pub(crate) blob: BlobId,
    pub(crate) size: u64,
    pub(crate) digest: [u8; 32],
}

/// The limits that bound what is added (`docs/RESOURCE_LIMITS.md`). Tests
/// lower them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
    pub(crate) size: u64,
    pub(crate) pending_per_contact: usize,
    pub(crate) pending: usize,
    pub(crate) active_per_contact: usize,
    pub(crate) active: usize,
    pub(crate) waiting_per_contact: usize,
    pub(crate) waiting: usize,
    pub(crate) staged_outgoing: u64,
    pub(crate) staged_incoming: u64,
    /// Both directions together, for an installation that stages in
    /// memory.
    pub(crate) staged_in_memory: Option<u64>,
    pub(crate) records_per_contact: usize,
    pub(crate) records: usize,
    pub(crate) unoffered_sessions: usize,
}

impl Limits {
    /// The limits of an installation that stages on disk.
    pub(crate) const fn persistent() -> Self {
        Self {
            size: MAX_TRANSFER_SIZE,
            pending_per_contact: MAX_PENDING_FILE_OFFERS_PER_CONTACT,
            pending: MAX_PENDING_FILE_OFFERS,
            active_per_contact: MAX_ACTIVE_TRANSFERS_PER_CONTACT,
            active: MAX_ACTIVE_TRANSFERS,
            waiting_per_contact: MAX_WAITING_OFFERS_PER_CONTACT,
            waiting: MAX_WAITING_OFFERS,
            staged_outgoing: MAX_STAGED_OUTGOING_BYTES,
            staged_incoming: MAX_STAGED_INCOMING_BYTES,
            staged_in_memory: None,
            records_per_contact: MAX_TRANSFER_RECORDS_PER_CONTACT,
            records: MAX_TRANSFER_RECORDS,
            unoffered_sessions: TRANSFER_UNOFFERED_SESSIONS,
        }
    }

    /// The limits of an ephemeral installation, which stages in memory.
    pub(crate) const fn ephemeral() -> Self {
        let mut limits = Self::persistent();
        limits.staged_in_memory = Some(MAX_EPHEMERAL_STAGED_BYTES);
        limits
    }
}

/// The transfers of every local identity of one installation. See the
/// module documentation for its rules.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct TransferState {
    entries: Vec<Entry>,
    limits: Limits,
}

impl fmt::Debug for TransferState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransferState")
            .field("records", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl TransferState {
    /// An empty state.
    pub(crate) const fn new(limits: Limits) -> Self {
        Self {
            entries: Vec::new(),
            limits,
        }
    }

    /// The state of a transfer store as read; its records were checked
    /// when they were decoded.
    pub(crate) fn load(records: TransferRecords, limits: Limits) -> Self {
        Self {
            entries: records.records.into_iter().map(Entry::new).collect(),
            limits,
        }
    }

    /// What the transfer store holds, to write.
    pub(crate) fn records(&self) -> TransferRecords {
        TransferRecords {
            records: self
                .entries
                .iter()
                .map(|entry| entry.record.clone())
                .collect(),
        }
    }

    fn entry(&self, key: &Key) -> Option<&Entry> {
        self.entries.iter().find(|entry| key.matches(&entry.record))
    }

    fn entry_mut(&mut self, key: &Key) -> Option<&mut Entry> {
        self.entries
            .iter_mut()
            .find(|entry| key.matches(&entry.record))
    }

    /// The record of `key`.
    pub(crate) fn record(&self, key: &Key) -> Option<&TransferRecord> {
        self.entry(key).map(|entry| &entry.record)
    }

    fn count(&self, counted: impl Fn(&Entry) -> bool) -> usize {
        self.entries.iter().filter(|entry| counted(entry)).count()
    }

    fn sum(&self, counted: impl Fn(&Entry) -> bool) -> u64 {
        self.entries
            .iter()
            .filter(|entry| counted(entry))
            .fold(0_u64, |sum, entry| sum.saturating_add(entry.record.size))
    }

    fn of_contact(entry: &Entry, identity: &IdentityId, contact: &ContactId) -> bool {
        entry.record.identity == *identity && entry.record.contact == *contact
    }

    fn pending(entry: &Entry) -> bool {
        !entry.outgoing() && entry.record.state == State::Offered
    }

    fn active(entry: &Entry) -> bool {
        !entry.outgoing() && entry.record.state == State::Accepted
    }

    /// An outgoing transfer not yet accepted: waiting, or cancelled while
    /// the receiver could not be told.
    fn waiting(entry: &Entry) -> bool {
        entry.outgoing()
            && (entry.record.state == State::Waiting
                || (entry.record.state == State::Cancelled && entry.unread()))
    }

    fn staged_outgoing(entry: &Entry) -> bool {
        entry.outgoing() && entry.record.has_blob()
    }

    fn staged_incoming(entry: &Entry) -> bool {
        !entry.outgoing() && entry.record.has_blob()
    }

    /// Whether `size` more bytes may be staged in `direction`.
    fn room_to_stage(&self, direction: Direction, size: u64) -> bool {
        if let Some(memory) = self.limits.staged_in_memory {
            let staged = self.sum(|entry| entry.record.has_blob());
            return staged.saturating_add(size) <= memory;
        }
        match direction {
            Direction::Outgoing => {
                self.sum(Self::staged_outgoing).saturating_add(size) <= self.limits.staged_outgoing
            }
            Direction::Incoming => {
                self.sum(Self::staged_incoming).saturating_add(size) <= self.limits.staged_incoming
            }
        }
    }

    /// The record that goes first to make room among those `among` takes,
    /// leaving out `planned`: an ended outgoing record, of any age; then
    /// an ended incoming record that ended so long ago that no offerer can
    /// still offer it; then an ended incoming record whose final answer the
    /// peer read. Oldest first within each.
    fn evictable(
        &self,
        now: u64,
        among: impl Fn(&Entry) -> bool,
        planned: &[usize],
    ) -> Option<usize> {
        let candidates = || {
            self.entries
                .iter()
                .enumerate()
                .filter(|(index, entry)| !planned.contains(index) && among(entry))
        };
        let outgoing = candidates()
            .filter(|(_, entry)| entry.is_ended_outgoing())
            .min_by_key(|(_, entry)| entry.record.created)
            .map(|(index, _)| index);
        if outgoing.is_some() {
            return outgoing;
        }
        let ended = |entry: &Entry| !entry.outgoing() && entry.record.is_ended_incoming();
        let old = candidates()
            .filter(|(_, entry)| {
                ended(entry) && now.saturating_sub(entry.ended_at()) > UNOFFERABLE_AFTER
            })
            .min_by_key(|(_, entry)| entry.ended_at())
            .map(|(index, _)| index);
        if old.is_some() {
            return old;
        }
        candidates()
            .filter(|(_, entry)| ended(entry) && !entry.unread())
            .min_by_key(|(_, entry)| entry.ended_at())
            .map(|(index, _)| index)
    }

    /// The records to remove so that one more record of `contact` of
    /// `identity` fits, or `None` if that cannot be done.
    fn room(&self, identity: &IdentityId, contact: &ContactId, now: u64) -> Option<Vec<usize>> {
        let mut planned = Vec::new();
        let mut of_contact = self.count(|entry| Self::of_contact(entry, identity, contact));
        let mut total = self.entries.len();
        while of_contact >= self.limits.records_per_contact {
            let index = self.evictable(
                now,
                |entry| Self::of_contact(entry, identity, contact),
                &planned,
            )?;
            planned.push(index);
            of_contact = of_contact.saturating_sub(1);
            total = total.saturating_sub(1);
        }
        while total >= self.limits.records {
            let index = self.evictable(now, |_| true, &planned)?;
            planned.push(index);
            total = total.saturating_sub(1);
        }
        Some(planned)
    }

    fn remove_planned(&mut self, mut planned: Vec<usize>) {
        planned.sort_unstable();
        for index in planned.into_iter().rev() {
            if index < self.entries.len() {
                self.entries.remove(index);
            }
        }
    }

    // --- Outgoing ---------------------------------------------------------

    /// Whether an outgoing transfer of `size` bytes to `contact` would be
    /// taken: checked before its file is staged, and again when it is
    /// recorded.
    pub(crate) fn may_offer(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
        size: u64,
        now: u64,
    ) -> Result<(), Refusal> {
        if size > self.limits.size {
            return Err(Refusal::TooLarge);
        }
        let waiting = |entry: &Entry| Self::waiting(entry) && entry.record.identity == *identity;
        if self.count(|entry| waiting(entry) && entry.record.contact == *contact)
            >= self.limits.waiting_per_contact
            || self.count(waiting) >= self.limits.waiting
        {
            return Err(Refusal::TooManyWaiting);
        }
        if !self.room_to_stage(Direction::Outgoing, size) {
            return Err(Refusal::StagingFull);
        }
        if self.room(identity, contact, now).is_none() {
            return Err(Refusal::ListFull);
        }
        Ok(())
    }

    /// Records an outgoing transfer whose file is staged, waiting for an
    /// answer. Its identifier must not be held for the contact in either
    /// direction.
    pub(crate) fn add_outgoing(&mut self, staged: Staged, now: u64) -> Result<Key, Refusal> {
        self.may_offer(&staged.identity, &staged.contact, staged.size, now)?;
        if staged.transfer == [0; TRANSFER_ID_LEN]
            || self.entries.iter().any(|entry| {
                Self::of_contact(entry, &staged.identity, &staged.contact)
                    && entry.record.transfer == staged.transfer
            })
        {
            return Err(Refusal::IdTaken);
        }
        let planned = self
            .room(&staged.identity, &staged.contact, now)
            .ok_or(Refusal::ListFull)?;
        self.remove_planned(planned);
        let record = TransferRecord {
            transfer: staged.transfer,
            identity: staged.identity,
            contact: staged.contact,
            direction: Direction::Outgoing,
            state: State::Waiting,
            flags: Flags::default(),
            reason: None,
            size: staged.size,
            name: staged.name,
            digest: staged.digest,
            blob: Some(staged.blob),
            created: now,
            deadline: now.saturating_add(STALL),
        };
        let key = Key::of(&record);
        let mut entry = Entry::new(record);
        entry.memo.recorded = now;
        entry.memo.held = Some(0);
        self.entries.push(entry);
        Ok(key)
    }

    /// The outgoing transfers of `contact` to offer on a new session, in
    /// the order of the ADR: those being sent, then those cancelled while
    /// the receiver could not be told, then those waiting, oldest first.
    pub(crate) fn to_offer(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
        now: u64,
    ) -> Vec<Outgoing> {
        let mut offers: Vec<(u8, u64, Outgoing)> = self
            .entries
            .iter()
            .filter(|entry| entry.outgoing() && Self::of_contact(entry, identity, contact))
            .filter_map(|entry| {
                let rank = match entry.record.state {
                    State::Sending => 0,
                    State::Cancelled if entry.unread() && entry.memo.deadline > now => 1,
                    State::Waiting => 2,
                    _ => return None,
                };
                Some((
                    rank,
                    entry.record.created,
                    Outgoing {
                        transfer: entry.record.transfer,
                        size: entry.record.size,
                        name: entry.record.name.clone(),
                        cancelled: rank == 1,
                    },
                ))
            })
            .collect();
        offers.sort_by_key(|(rank, created, _)| (*rank, *created));
        offers.into_iter().map(|(_, _, offer)| offer).collect()
    }

    /// The offer of `key` was written on a session.
    pub(crate) fn mark_offered(&mut self, key: &Key) {
        if let Some(entry) = self.entry_mut(key) {
            entry.memo.offered = true;
        }
    }

    /// The peer read the offer of `key`: a Pong answered a Ping written
    /// after it.
    pub(crate) fn offer_read(&mut self, key: &Key) -> Change {
        match self.entry_mut(key) {
            Some(entry) if entry.outgoing() && !entry.record.flags.has(Flags::OFFER_READ) => {
                entry.record.flags = entry.record.flags.with(Flags::OFFER_READ, true);
                Change {
                    changed: true,
                    ..Change::default()
                }
            }
            _ => Change::default(),
        }
    }

    /// The peer accepted the offer of `key`, holding `offset` bytes: a
    /// waiting transfer is being sent from now on; one being sent made
    /// progress if the offset counts.
    pub(crate) fn accepted_by_peer(&mut self, key: &Key, offset: u64, now: u64) -> Change {
        let Some(entry) = self.entry_mut(key).filter(|entry| entry.outgoing()) else {
            return Change::default();
        };
        match entry.record.state {
            State::Waiting => {
                entry.record.state = State::Sending;
                // An acceptance shows that the offer was read.
                entry.record.flags = entry.record.flags.with(Flags::OFFER_READ, true);
                let deadline = now.saturating_add(STALL);
                entry.record.deadline = deadline;
                entry.memo.deadline = deadline;
                entry.memo.recorded = now;
                entry.memo.progress = offset;
                entry.memo.held = Some(offset);
                Change {
                    changed: true,
                    freed: None,
                    progressed: true,
                }
            }
            State::Sending => entry.advance(offset, now),
            _ => Change::default(),
        }
    }

    /// The peer completed `key` with `digest`: delivered if it is the
    /// digest staged, also after a cancellation the peer did not read;
    /// failed if it differs while the transfer was waiting or being sent.
    /// The second value says whether the digest matched.
    pub(crate) fn completed_by_peer(
        &mut self,
        key: &Key,
        digest: &[u8; 32],
        now: u64,
    ) -> (Change, bool) {
        let Some(entry) = self.entry_mut(key).filter(|entry| entry.outgoing()) else {
            return (Change::default(), false);
        };
        let matched = entry.record.digest == *digest;
        let change = match (entry.record.state, matched) {
            (State::Waiting | State::Sending | State::Cancelled, true) => {
                entry.memo.held = Some(entry.record.size);
                entry.end(State::Delivered, None, false, now)
            }
            (State::Waiting | State::Sending, false) => {
                entry.end(State::Failed, Some(Reason::Digest), false, now)
            }
            _ => Change::default(),
        };
        (change, matched)
    }

    /// The peer rejected the offer of `key`. A cancellation it never read
    /// needs telling no more.
    pub(crate) fn rejected_by_peer(&mut self, key: &Key, now: u64) -> Change {
        let Some(entry) = self.entry_mut(key).filter(|entry| entry.outgoing()) else {
            return Change::default();
        };
        match entry.record.state {
            State::Waiting | State::Sending => entry.end(State::Rejected, None, false, now),
            State::Cancelled => entry.untell(),
            _ => Change::default(),
        }
    }

    // --- Incoming ---------------------------------------------------------

    /// What is answered for `key` from what is held: the answer to a
    /// repeated offer, and what a session owes for a decision.
    pub(crate) fn answer(&self, key: &Key, now: u64) -> Offer {
        let Some(entry) = self.entry(key).filter(|entry| !entry.outgoing()) else {
            return Offer::Answer(Answer::Nothing);
        };
        Offer::Answer(match entry.record.state {
            State::Offered => Answer::Nothing,
            State::Accepted => {
                if entry.memo.hold_until.is_some_and(|until| until > now) {
                    Answer::Nothing
                } else {
                    match (entry.memo.held, entry.record.blob) {
                        (Some(held), _) => Answer::Accept(held),
                        (None, Some(blob)) => return Offer::Scan(blob),
                        (None, None) => Answer::Nothing,
                    }
                }
            }
            State::Complete => Answer::Complete(entry.record.digest),
            State::Rejected | State::Expired => Answer::Reject,
            State::Cancelled | State::Failed => Answer::Abort,
            State::Waiting | State::Sending | State::Delivered => Answer::Nothing,
        })
    }

    /// An offer of the peer: of a transfer held, answered from what is
    /// held; otherwise recorded, refused with FileReject (above the size
    /// limit or the pending limit of the contact) or with nothing (above
    /// the other limits), without a record.
    pub(crate) fn offered(&mut self, key: &Key, size: u64, name: &Filename, now: u64) -> Offer {
        if let Some(entry) = self.entry(key) {
            if entry.record.size != size || entry.record.name != *name {
                return Offer::Violation;
            }
            return self.answer(key, now);
        }
        // The identifier of a transfer of this side to the contact: an offer
        // of the peer's own carries one it drew at random, so this is a
        // violation, as an offer back of it on the session that made it is.
        if self.entries.iter().any(|entry| {
            Self::of_contact(entry, &key.identity, &key.contact)
                && entry.record.transfer == key.transfer
        }) {
            return Offer::Violation;
        }
        // An identifier of zero bytes, which no random source gives and no
        // record may hold, is refused as an offer above the limits is.
        if size > self.limits.size || key.transfer == [0; TRANSFER_ID_LEN] {
            return Offer::Answer(Answer::Reject);
        }
        let pending = |entry: &Entry| Self::pending(entry) && entry.record.identity == key.identity;
        if self.count(|entry| pending(entry) && entry.record.contact == key.contact)
            >= self.limits.pending_per_contact
        {
            return Offer::Answer(Answer::Reject);
        }
        if self.count(pending) >= self.limits.pending {
            return Offer::Answer(Answer::Nothing);
        }
        let Some(planned) = self.room(&key.identity, &key.contact, now) else {
            return Offer::Answer(Answer::Nothing);
        };
        self.remove_planned(planned);
        let record = TransferRecord {
            transfer: key.transfer,
            identity: key.identity,
            contact: key.contact,
            direction: Direction::Incoming,
            state: State::Offered,
            flags: Flags::default(),
            reason: None,
            size,
            name: name.clone(),
            digest: [0; 32],
            blob: None,
            created: now,
            deadline: now.saturating_add(OFFER_TIMEOUT),
        };
        let mut entry = Entry::new(record);
        entry.memo.recorded = now;
        self.entries.push(entry);
        Offer::New
    }

    /// Whether the offer of `key` may be accepted now: the limits of active
    /// transfers and of staged bytes. Returns its size, which the blob is
    /// reserved for.
    pub(crate) fn may_accept(&self, key: &Key) -> Result<u64, Refusal> {
        let entry = self
            .entry(key)
            .filter(|entry| Self::pending(entry))
            .ok_or(Refusal::Unknown)?;
        let active = |held: &Entry| Self::active(held) && held.record.identity == key.identity;
        if self.count(|held| active(held) && held.record.contact == key.contact)
            >= self.limits.active_per_contact
            || self.count(active) >= self.limits.active
        {
            return Err(Refusal::TooManyActive);
        }
        if !self.room_to_stage(Direction::Incoming, entry.record.size) {
            return Err(Refusal::StagingFull);
        }
        Ok(entry.record.size)
    }

    /// The user accepted the offer of `key`, whose blob is reserved and
    /// durable. The first FileAccept after it is to be read by the peer.
    pub(crate) fn accept(&mut self, key: &Key, blob: BlobId, now: u64) -> Result<Change, Refusal> {
        self.may_accept(key)?;
        let entry = self.entry_mut(key).ok_or(Refusal::Unknown)?;
        entry.record.state = State::Accepted;
        entry.record.blob = Some(blob);
        entry.record.flags = entry.record.flags.with(Flags::UNREAD, true);
        let deadline = now.saturating_add(STALL);
        entry.record.deadline = deadline;
        entry.memo = Memo {
            deadline,
            recorded: now,
            held: Some(0),
            ..Memo::default()
        };
        Ok(Change {
            changed: true,
            freed: None,
            progressed: true,
        })
    }

    /// The user rejected the offer of `key`. Rejecting twice is rejecting
    /// once.
    pub(crate) fn reject(&mut self, key: &Key, now: u64) -> Result<Change, Refusal> {
        let entry = self
            .entry_mut(key)
            .filter(|entry| !entry.outgoing())
            .ok_or(Refusal::Unknown)?;
        match entry.record.state {
            State::Offered => Ok(entry.end(State::Rejected, None, true, now)),
            State::Rejected => Ok(Change::default()),
            _ => Err(Refusal::Unknown),
        }
    }

    /// How much of the blob of `key` is staged, as a scan found it after a
    /// start: what counts from now on. It is not progress.
    pub(crate) fn scanned(&mut self, key: &Key, bytes: u64) {
        if let Some(entry) = self.entry_mut(key).filter(|entry| Self::active(entry)) {
            entry.memo.held = Some(bytes);
            entry.memo.progress = entry.memo.progress.max(bytes);
        }
    }

    /// `bytes` of `key` are staged durably now, after a sync of its blob.
    pub(crate) fn staged(&mut self, key: &Key, bytes: u64, now: u64) -> Change {
        match self.entry_mut(key).filter(|entry| Self::active(entry)) {
            Some(entry) => entry.advance(bytes, now),
            None => Change::default(),
        }
    }

    /// The blob of `key` was missing or did not match its record: `blob`,
    /// new, reserved and durable, replaces it, with nothing staged.
    pub(crate) fn replace_blob(&mut self, key: &Key, blob: BlobId) -> Change {
        let Some(entry) = self.entry_mut(key).filter(|entry| Self::active(entry)) else {
            return Change::default();
        };
        let freed = entry.record.blob.replace(blob);
        entry.memo.held = Some(0);
        entry.memo.progress = 0;
        Change {
            changed: true,
            freed,
            progressed: false,
        }
    }

    /// The whole of `key` is staged and its digest verified: complete. Its
    /// FileComplete is to be read by the peer.
    pub(crate) fn complete(&mut self, key: &Key, digest: [u8; 32], now: u64) -> Change {
        let Some(entry) = self.entry_mut(key).filter(|entry| Self::active(entry)) else {
            return Change::default();
        };
        entry.record.digest = digest;
        entry.memo.held = Some(entry.record.size);
        entry.end(State::Complete, None, true, now)
    }

    /// The writes of `key` wait: a write failed for a full disk or a held
    /// file. Until `TRANSFER_HOLD` passes or the user resumes it, its
    /// offers are answered with nothing and it claims no session.
    pub(crate) fn hold(&mut self, key: &Key, now: u64) {
        if let Some(entry) = self.entry_mut(key).filter(|entry| Self::active(entry)) {
            entry.memo.hold_until = Some(now.saturating_add(HOLD));
            // What was written after the last sync counts as not written;
            // how much is held is read again.
            entry.memo.held = None;
        }
    }

    /// The user resumes `key` held after a failed write.
    pub(crate) fn resume(&mut self, key: &Key) -> Result<(), Refusal> {
        let entry = self
            .entry_mut(key)
            .filter(|entry| Self::active(entry))
            .ok_or(Refusal::Unknown)?;
        entry.memo.hold_until = None;
        Ok(())
    }

    /// A save of the complete file of `key` begins: recorded before
    /// anything is written. One at a time per file.
    pub(crate) fn begin_save(&mut self, key: &Key) -> Result<Saving, Refusal> {
        let entry = self
            .entry_mut(key)
            .filter(|entry| !entry.outgoing() && entry.record.state == State::Complete)
            .ok_or(Refusal::Unknown)?;
        if entry.record.flags.has(Flags::SAVING) {
            return Err(Refusal::Busy);
        }
        let blob = entry.record.blob.ok_or(Refusal::Unknown)?;
        entry.record.flags = entry.record.flags.with(Flags::SAVING, true);
        Ok(Saving {
            blob,
            size: entry.record.size,
            digest: entry.record.digest,
        })
    }

    /// The save of `key` ended: the file is saved, and its blob goes, or
    /// it was not, and the blob stays.
    pub(crate) fn end_save(&mut self, key: &Key, saved: bool, now: u64) -> Change {
        let Some(entry) = self.entry_mut(key).filter(|entry| {
            !entry.outgoing()
                && entry.record.state == State::Complete
                && entry.record.flags.has(Flags::SAVING)
        }) else {
            return Change::default();
        };
        entry.record.flags = entry.record.flags.with(Flags::SAVING, false);
        if !saved {
            return Change {
                changed: true,
                ..Change::default()
            };
        }
        entry.record.flags = entry.record.flags.with(Flags::SAVED, true);
        let freed = entry.free();
        entry.record.deadline = now.saturating_add(RETENTION);
        entry.memo.deadline = entry.record.deadline;
        Change {
            changed: true,
            freed,
            progressed: false,
        }
    }

    /// The saves that were under way when the installation last closed:
    /// their flags are cleared, and the user is told.
    pub(crate) fn interrupted_saves(&mut self) -> Vec<Key> {
        self.entries
            .iter_mut()
            .filter(|entry| entry.record.flags.has(Flags::SAVING))
            .map(|entry| {
                entry.record.flags = entry.record.flags.with(Flags::SAVING, false);
                entry.key()
            })
            .collect()
    }

    // --- Both directions --------------------------------------------------

    /// The peer aborted `key`. A complete incoming transfer stays
    /// complete; a cancellation of this side it never read needs telling
    /// no more.
    pub(crate) fn aborted_by_peer(&mut self, key: &Key, now: u64) -> Change {
        let Some(entry) = self.entry_mut(key) else {
            return Change::default();
        };
        match (entry.record.direction, entry.record.state) {
            (Direction::Outgoing, State::Waiting | State::Sending)
            | (Direction::Incoming, State::Offered | State::Accepted) => {
                entry.end(State::Cancelled, Some(Reason::ByPeer), false, now)
            }
            (Direction::Outgoing, State::Cancelled) => entry.untell(),
            _ => Change::default(),
        }
    }

    /// The user cancels `key`. Cancelling twice is cancelling once. An
    /// undecided offer is rejected; a complete incoming transfer cannot be
    /// cancelled, only removed. The peer is to be told of a cancellation
    /// of an outgoing transfer it may have seen, and of an accepted one.
    pub(crate) fn cancel(&mut self, key: &Key, now: u64) -> Result<Change, Refusal> {
        let entry = self.entry_mut(key).ok_or(Refusal::Unknown)?;
        Ok(match (entry.record.direction, entry.record.state) {
            (Direction::Outgoing, State::Waiting | State::Sending) => {
                let seen = entry.record.flags.has(Flags::OFFER_READ) || entry.memo.offered;
                entry.end(State::Cancelled, Some(Reason::ByUs), seen, now)
            }
            (Direction::Incoming, State::Offered) => entry.end(State::Rejected, None, true, now),
            (Direction::Incoming, State::Accepted) => {
                entry.end(State::Cancelled, Some(Reason::ByUs), true, now)
            }
            (Direction::Incoming, State::Complete) => return Err(Refusal::Unknown),
            _ => Change::default(),
        })
    }

    /// The transfer of `key` fails for `reason`.
    pub(crate) fn fail(&mut self, key: &Key, reason: Reason, now: u64) -> Change {
        let Some(entry) = self.entry_mut(key) else {
            return Change::default();
        };
        match (entry.record.direction, entry.record.state) {
            (Direction::Outgoing, State::Waiting | State::Sending)
            | (Direction::Incoming, State::Accepted) => {
                entry.end(State::Failed, Some(reason), false, now)
            }
            // A complete file whose blob no longer reads.
            (Direction::Incoming, State::Complete) if entry.record.has_blob() => {
                entry.memo.held = None;
                entry.end(State::Failed, Some(reason), false, now)
            }
            _ => Change::default(),
        }
    }

    /// The peer read what this side last told it of `key` while the
    /// transfer was in `state`: an answer, a decision or a cancellation.
    /// The flag that it has to be told is cleared, unless the state moved
    /// on and owes the peer something new.
    pub(crate) fn told(&mut self, key: &Key, state: State) -> Change {
        match self
            .entry_mut(key)
            .filter(|entry| entry.record.state == state)
        {
            Some(entry) => entry.untell(),
            None => Change::default(),
        }
    }

    /// The user removes `key` from the interface: an ended outgoing record
    /// goes, also a cancellation the peer was not told yet, which the peer
    /// then learns as a file no longer offered; an ended or complete
    /// incoming one is hidden, its blob goes, and its record stays for its
    /// retention so that a repeated offer finds it.
    pub(crate) fn remove(&mut self, key: &Key, now: u64) -> Result<Change, Refusal> {
        let index = self
            .entries
            .iter()
            .position(|entry| key.matches(&entry.record))
            .ok_or(Refusal::Unknown)?;
        let entry = self.entries.get_mut(index).ok_or(Refusal::Unknown)?;
        if entry.outgoing() {
            if !entry.is_ended_outgoing() && entry.record.state != State::Cancelled {
                return Err(Refusal::Unknown);
            }
            self.entries.remove(index);
            return Ok(Change {
                changed: true,
                ..Change::default()
            });
        }
        let ended = matches!(
            entry.record.state,
            State::Rejected | State::Expired | State::Cancelled | State::Failed | State::Complete
        );
        if !ended {
            return Err(Refusal::Unknown);
        }
        if entry.record.flags.has(Flags::SAVING) {
            return Err(Refusal::Busy);
        }
        if entry.record.flags.has(Flags::REMOVED) {
            return Ok(Change::default());
        }
        entry.record.flags = entry.record.flags.with(Flags::REMOVED, true);
        let freed = entry.free();
        if entry.record.deadline == 0 {
            entry.record.deadline = now.saturating_add(RETENTION);
            entry.memo.deadline = entry.record.deadline;
        }
        Ok(Change {
            changed: true,
            freed,
            progressed: false,
        })
    }

    // --- Sessions and the scheduler ----------------------------------------

    /// A session of `contact` that counts ended: confirmed, and the only
    /// one of the contact for `TRANSFER_GRACE` of reading time or closed
    /// by the peer while it was. `offered` holds the identifiers the peer
    /// offered on it. An accepted transfer it did not offer on enough such
    /// sessions in a row fails: the offerer no longer has it. A
    /// cancellation of this side it did not offer stops claiming the
    /// scheduler the same way.
    pub(crate) fn session_counted(
        &mut self,
        identity: &IdentityId,
        contact: &ContactId,
        offered: &HashSet<[u8; TRANSFER_ID_LEN]>,
        now: u64,
    ) -> Vec<(Key, Change)> {
        let limit = self.limits.unoffered_sessions;
        let mut changes = Vec::new();
        for entry in &mut self.entries {
            if entry.outgoing() || !Self::of_contact(entry, identity, contact) {
                continue;
            }
            let accepted = entry.record.state == State::Accepted;
            let cancelling =
                entry.record.state == State::Cancelled && entry.unread() && !entry.memo.unclaimed;
            if !accepted && !cancelling {
                continue;
            }
            if offered.contains(&entry.record.transfer) {
                entry.memo.unoffered = 0;
                continue;
            }
            entry.memo.unoffered = entry.memo.unoffered.saturating_add(1);
            if entry.memo.unoffered < limit {
                continue;
            }
            if accepted {
                let change = entry.end(State::Failed, Some(Reason::OffererGone), false, now);
                changes.push((entry.key(), change));
            } else {
                entry.memo.unclaimed = true;
            }
        }
        changes
    }

    /// Whether a transfer of `contact` needs a session (ADR 0013,
    /// "Scheduling"): an offer not yet read, a transfer being sent or a
    /// cancellation not yet read, outgoing; an accepted transfer not held,
    /// or a cancellation of this side not yet read, incoming.
    pub(crate) fn claims(&self, identity: &IdentityId, contact: &ContactId, now: u64) -> bool {
        self.entries
            .iter()
            .any(|entry| Self::of_contact(entry, identity, contact) && Self::claiming(entry, now))
    }

    fn claiming(entry: &Entry, now: u64) -> bool {
        let record = &entry.record;
        match (record.direction, record.state) {
            (Direction::Outgoing, State::Waiting) => !record.flags.has(Flags::OFFER_READ),
            (Direction::Outgoing, State::Sending) => true,
            (Direction::Outgoing, State::Cancelled) => entry.unread() && entry.memo.deadline > now,
            (Direction::Incoming, State::Accepted) => {
                entry.memo.hold_until.is_none_or(|until| until <= now)
            }
            (Direction::Incoming, State::Cancelled) => {
                entry.unread()
                    && record.reason == Some(Reason::ByUs)
                    && !entry.memo.unclaimed
                    && entry.ended_at().saturating_add(STALL) > now
            }
            _ => false,
        }
    }

    /// The contact records of `identity` a transfer claims a session for.
    pub(crate) fn claiming_contacts(&self, identity: &IdentityId, now: u64) -> Vec<ContactId> {
        let mut contacts: Vec<ContactId> = self
            .entries
            .iter()
            .filter(|entry| entry.record.identity == *identity && Self::claiming(entry, now))
            .map(|entry| entry.record.contact)
            .collect();
        contacts.sort_unstable();
        contacts.dedup();
        contacts
    }

    // --- Time ---------------------------------------------------------------

    /// The earliest time at which [`Self::tick`] changes something.
    pub(crate) fn next_due(&self) -> Option<u64> {
        self.entries
            .iter()
            .filter_map(|entry| {
                let record = &entry.record;
                match (record.direction, record.state) {
                    (Direction::Incoming, State::Offered) => Some(record.deadline),
                    (Direction::Outgoing, State::Waiting | State::Sending) => {
                        Some(entry.memo.deadline)
                    }
                    (Direction::Outgoing, State::Cancelled) if entry.unread() => {
                        Some(entry.memo.deadline)
                    }
                    (Direction::Incoming, State::Accepted) => Some(
                        entry
                            .memo
                            .hold_until
                            .map_or(entry.memo.deadline, |until| until.min(entry.memo.deadline)),
                    ),
                    (Direction::Incoming, State::Cancelled)
                        if entry.unread() && !entry.memo.unclaimed =>
                    {
                        Some(entry.ended_at().saturating_add(STALL).min(record.deadline))
                    }
                    _ if record.is_ended_incoming() => Some(record.deadline),
                    _ => None,
                }
            })
            .min()
    }

    /// Applies what the time does at `now`: an offer not decided expires,
    /// a transfer without progress fails, a cancellation not read is told
    /// no more, a hold ends, the record of an ended incoming transfer goes
    /// at the end of its retention. Returns the transfers whose records
    /// changed or went.
    pub(crate) fn tick(&mut self, now: u64) -> Vec<(Key, Change)> {
        let mut changes = Vec::new();
        for entry in &mut self.entries {
            let record = &entry.record;
            let change = match (record.direction, record.state) {
                (Direction::Incoming, State::Offered) if record.deadline <= now => {
                    entry.end(State::Expired, None, true, now)
                }
                (Direction::Outgoing, State::Waiting | State::Sending)
                    if entry.memo.deadline <= now =>
                {
                    entry.end(State::Failed, Some(Reason::Stalled), false, now)
                }
                (Direction::Outgoing, State::Cancelled)
                    if entry.unread() && entry.memo.deadline <= now =>
                {
                    entry.untell()
                }
                (Direction::Incoming, State::Accepted) if entry.memo.deadline <= now => {
                    entry.end(State::Failed, Some(Reason::Stalled), false, now)
                }
                (Direction::Incoming, State::Accepted) => {
                    if entry.memo.hold_until.is_some_and(|until| until <= now) {
                        entry.memo.hold_until = None;
                    }
                    continue;
                }
                (Direction::Incoming, State::Cancelled)
                    if entry.unread()
                        && !entry.memo.unclaimed
                        && entry.ended_at().saturating_add(STALL) <= now
                        && record.deadline > now =>
                {
                    entry.memo.unclaimed = true;
                    continue;
                }
                _ => continue,
            };
            changes.push((entry.key(), change));
        }
        let gone: Vec<Key> = self
            .entries
            .iter()
            .filter(|entry| entry.record.is_ended_incoming() && entry.record.deadline <= now)
            .map(Entry::key)
            .collect();
        self.entries
            .retain(|entry| !(entry.record.is_ended_incoming() && entry.record.deadline <= now));
        changes.extend(gone.into_iter().map(|key| {
            (
                key,
                Change {
                    changed: true,
                    ..Change::default()
                },
            )
        }));
        changes
    }

    // --- Contacts and identities -----------------------------------------

    /// The contact record `contact` of `identity` ended. Blocked: every
    /// transfer that is not ended is cancelled, and a complete file not
    /// saved goes, without telling the peer. Deleted: its records go. In
    /// both cases every blob of the contact goes.
    pub(crate) fn contact_ended(
        &mut self,
        identity: &IdentityId,
        contact: &ContactId,
        delete: bool,
        now: u64,
    ) -> Vec<(Key, Change)> {
        let mut changes = Vec::new();
        if delete {
            let (gone, kept): (Vec<Entry>, Vec<Entry>) = core::mem::take(&mut self.entries)
                .into_iter()
                .partition(|entry| Self::of_contact(entry, identity, contact));
            self.entries = kept;
            for mut entry in gone {
                let freed = entry.free();
                changes.push((
                    entry.key(),
                    Change {
                        changed: true,
                        freed,
                        progressed: false,
                    },
                ));
            }
            return changes;
        }
        for entry in &mut self.entries {
            if !Self::of_contact(entry, identity, contact) {
                continue;
            }
            let reason = Some(Reason::ContactEnded);
            let change = match (entry.record.direction, entry.record.state) {
                (Direction::Outgoing, State::Waiting | State::Sending)
                | (Direction::Incoming, State::Offered | State::Accepted) => {
                    entry.end(State::Cancelled, reason, false, now)
                }
                (Direction::Incoming, State::Complete) if entry.record.has_blob() => {
                    entry.record.flags = entry
                        .record
                        .flags
                        .with(Flags::SAVING, false)
                        .with(Flags::UNREAD, false)
                        .with(Flags::REMOVED, true);
                    entry.record.deadline = now.saturating_add(RETENTION);
                    entry.memo.deadline = entry.record.deadline;
                    Change {
                        changed: true,
                        freed: entry.free(),
                        progressed: false,
                    }
                }
                _ if entry.unread() => entry.untell(),
                _ => continue,
            };
            changes.push((entry.key(), change));
        }
        changes
    }

    /// Whether a record of `identity` is held.
    pub(crate) fn holds_identity(&self, identity: &IdentityId) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.record.identity == *identity)
    }

    /// Removes every transfer of `identity`. Returns their blobs.
    pub(crate) fn remove_identity(&mut self, identity: &IdentityId) -> Vec<BlobId> {
        let (gone, kept): (Vec<Entry>, Vec<Entry>) = core::mem::take(&mut self.entries)
            .into_iter()
            .partition(|entry| entry.record.identity == *identity);
        self.entries = kept;
        gone.into_iter()
            .filter_map(|entry| entry.record.blob)
            .collect()
    }

    /// Matches the state with the vault it belongs to: a record of a local
    /// identity or a contact record the vault does not hold goes, what a
    /// crash left of a deletion; a record of a contact of another local
    /// identity is refused as corrupt. `records` lists, for every identity
    /// in the vault, its contact records. Returns true if anything went.
    pub(crate) fn reconcile(
        &mut self,
        records: &HashMap<IdentityId, Vec<ContactId>>,
    ) -> Result<bool, StorageError> {
        let mut owners: HashMap<ContactId, IdentityId> = HashMap::new();
        for (identity, contacts) in records {
            for contact in contacts {
                owners.insert(*contact, *identity);
            }
        }
        if self.entries.iter().any(|entry| {
            owners
                .get(&entry.record.contact)
                .is_some_and(|owner| *owner != entry.record.identity)
        }) {
            return Err(StorageError::BadFormat);
        }
        let before = self.entries.len();
        self.entries
            .retain(|entry| owners.contains_key(&entry.record.contact));
        Ok(self.entries.len() != before)
    }

    /// Every blob a record names.
    pub(crate) fn blobs(&self) -> HashSet<BlobId> {
        self.entries
            .iter()
            .filter_map(|entry| entry.record.blob)
            .collect()
    }

    // --- Queries ------------------------------------------------------------

    /// The transfers of `identity` the interface shows, oldest first.
    pub(crate) fn views(&self, identity: &IdentityId, now: u64) -> Vec<TransferView> {
        let mut views: Vec<TransferView> = self
            .entries
            .iter()
            .filter(|entry| {
                entry.record.identity == *identity && !entry.record.flags.has(Flags::REMOVED)
            })
            .map(|entry| Self::view_of(entry, now))
            .collect();
        views.sort_by_key(|view| view.created);
        views
    }

    /// The transfer `key`, unless it was removed from the interface.
    pub(crate) fn view(&self, key: &Key, now: u64) -> Option<TransferView> {
        self.entry(key)
            .filter(|entry| !entry.record.flags.has(Flags::REMOVED))
            .map(|entry| Self::view_of(entry, now))
    }

    /// The transfers of `identity` as the transfer store holds them in
    /// `written`, with what is known beside them now, oldest first.
    pub(crate) fn written_views(
        &self,
        written: &[TransferRecord],
        identity: &IdentityId,
        now: u64,
    ) -> Vec<TransferView> {
        let mut views: Vec<TransferView> = written
            .iter()
            .filter(|record| record.identity == *identity && !record.flags.has(Flags::REMOVED))
            .map(|record| self.written_view_of(record, now))
            .collect();
        views.sort_by_key(|view| view.created);
        views
    }

    /// The transfer `key` as the transfer store holds it in `written`,
    /// unless it was removed from the interface.
    pub(crate) fn written_view(
        &self,
        written: &[TransferRecord],
        key: &Key,
        now: u64,
    ) -> Option<TransferView> {
        written
            .iter()
            .find(|record| key.matches(record) && !record.flags.has(Flags::REMOVED))
            .map(|record| self.written_view_of(record, now))
    }

    fn written_view_of(&self, record: &TransferRecord, now: u64) -> TransferView {
        match self.entry(&Key::of(record)) {
            Some(entry) => Self::record_view(record, &entry.memo, now),
            None => Self::record_view(record, &Memo::of(record), now),
        }
    }

    fn view_of(entry: &Entry, now: u64) -> TransferView {
        Self::record_view(&entry.record, &entry.memo, now)
    }

    fn record_view(record: &TransferRecord, memo: &Memo, now: u64) -> TransferView {
        TransferView {
            reference: Key::of(record).reference(),
            status: match record.state {
                State::Waiting => TransferStatus::Waiting,
                State::Sending => TransferStatus::Sending,
                State::Delivered => TransferStatus::Completed,
                State::Offered => TransferStatus::Offered,
                State::Accepted => TransferStatus::Receiving,
                State::Complete => TransferStatus::Received,
                State::Expired => TransferStatus::Expired,
                State::Rejected => TransferStatus::Rejected,
                State::Cancelled => TransferStatus::Cancelled,
                State::Failed => TransferStatus::Failed,
            },
            reason: record.reason.map(|reason| match reason {
                Reason::Storage => TransferReason::Storage,
                Reason::Digest => TransferReason::Digest,
                Reason::Stalled => TransferReason::Stalled,
                Reason::OffererGone => TransferReason::OffererGone,
                Reason::ByUs => TransferReason::ByUs,
                Reason::ByPeer => TransferReason::ByPeer,
                Reason::ContactEnded => TransferReason::ContactEnded,
            }),
            size: record.size,
            name: record.name.as_str().to_owned(),
            bytes: match record.state {
                State::Delivered | State::Complete => Some(record.size),
                // Nothing was accepted yet.
                State::Waiting | State::Offered => Some(0),
                _ => memo.held,
            },
            created: record.created,
            held: memo.hold_until.is_some_and(|until| until > now),
            saved: record.flags.has(Flags::SAVED),
        }
    }
}

/// The time of the transfers: the wall clock when the installation opened,
/// advanced by the clock of the runtime, so that a deadline counts the
/// time the process runs and tests can move it.
#[derive(Clone, Copy, Debug)]
struct Clock {
    unix: u64,
    start: tokio::time::Instant,
}

impl Clock {
    fn new() -> Self {
        Self {
            unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |since| since.as_secs()),
            start: tokio::time::Instant::now(),
        }
    }

    fn now(&self) -> u64 {
        self.unix.saturating_add(
            tokio::time::Instant::now()
                .saturating_duration_since(self.start)
                .as_secs(),
        )
    }
}

/// The transfer state of one installation, and where it is written.
pub(crate) struct Transfers {
    state: Mutex<TransferState>,
    /// What the transfer store holds, which the interface is shown; `None`
    /// without a vault, where every change is durable at once.
    written: Option<Mutex<Written>>,
    durability: Arc<Durability>,
    store: Store,
    /// Turns over after every change, so that the tasks of sessions look
    /// at their transfers again.
    changes: watch::Sender<u64>,
    clock: Clock,
    /// The staged files.
    pub(crate) blobs: Blobs,
    /// The saves that were under way when the installation last closed,
    /// for the runtime to report.
    interrupted: Mutex<Vec<Key>>,
}

impl fmt::Debug for Transfers {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Transfers").finish_non_exhaustive()
    }
}

/// The records of the transfer store as the last write that completed
/// left them, and those of the write under way with the generation it
/// covers. The interface is shown the first: never a change that a crash
/// could still undo, such as a completion, a cancellation or an offer
/// whose write has not finished.
struct Written {
    durable: Vec<TransferRecord>,
    writing: Option<(u64, Vec<TransferRecord>)>,
}

impl Written {
    /// The write under way is durable once `durable` covers it.
    fn settle(&mut self, durable: u64) {
        if self
            .writing
            .as_ref()
            .is_some_and(|(covered, _)| *covered <= durable)
        {
            if let Some((_, records)) = self.writing.take() {
                self.durable = records;
            }
        }
    }
}

impl Transfers {
    pub(crate) fn new(
        store: Store,
        state: TransferState,
        blobs: Blobs,
        interrupted: Vec<Key>,
    ) -> Arc<Self> {
        // What a vault holds as the installation opens is durable.
        let written = (!matches!(store, Store::Ephemeral)).then(|| {
            Mutex::new(Written {
                durable: state.records().records,
                writing: None,
            })
        });
        Arc::new(Self {
            state: Mutex::new(state),
            written,
            durability: Durability::new(),
            store,
            changes: watch::Sender::new(0),
            clock: Clock::new(),
            blobs,
            interrupted: Mutex::new(interrupted),
        })
    }

    /// The saves that were under way when the installation last closed.
    pub(crate) fn interrupted(&self) -> Vec<Key> {
        lock(&self.interrupted).clone()
    }

    /// The time of the transfers, in seconds since the Unix epoch.
    pub(crate) fn now(&self) -> u64 {
        self.clock.now()
    }

    /// Runs `change` on the state under its lock, at the time of the
    /// transfers. If it returns a change, the change is stamped with the
    /// next generation in the same step and the tasks waiting for changes
    /// are woken. Returns what `change` returned, with the generation that
    /// what it decided depends on.
    pub(crate) fn change<R>(
        &self,
        change: impl FnOnce(&mut TransferState, u64) -> (R, bool),
    ) -> Result<(R, u64), CommitError> {
        let now = self.now();
        let generation;
        let result;
        let changed;
        {
            let mut state = lock(&self.state);
            if self.durability.is_failed() {
                return Err(CommitError::Failed);
            }
            let value;
            (value, changed) = change(&mut state, now);
            generation = if changed {
                self.durability.bump()
            } else {
                self.durability.applied()
            };
            result = value;
        }
        if changed {
            self.wake();
        }
        Ok((result, generation))
    }

    /// Wakes the tasks that follow the transfers: also for what changed in
    /// memory only, such as a hold that ended.
    pub(crate) fn wake(&self) {
        self.changes
            .send_modify(|count| *count = count.wrapping_add(1));
    }

    /// Reads the state under its lock, at the time of the transfers.
    pub(crate) fn read<R>(&self, read: impl FnOnce(&TransferState, u64) -> R) -> R {
        let now = self.now();
        read(&lock(&self.state), now)
    }

    /// Reads the state, with the records the transfer store holds (`None`
    /// without a vault, where the state is durable), at the time of the
    /// transfers: what the interface is shown.
    pub(crate) fn show<R>(
        &self,
        read: impl FnOnce(&TransferState, Option<&[TransferRecord]>, u64) -> R,
    ) -> R {
        let now = self.now();
        let state = lock(&self.state);
        match &self.written {
            Some(written) => {
                let mut written = lock(written);
                written.settle(self.durability.durable());
                read(&state, Some(&written.durable), now)
            }
            None => read(&state, None, now),
        }
    }

    /// Follows the changes.
    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Returns true once a write failed: nothing more is stored.
    pub(crate) fn is_failed(&self) -> bool {
        self.durability.is_failed()
    }

    /// Waits until `generation` is durable.
    pub(crate) async fn commit(self: &Arc<Self>, generation: u64) -> Result<(), CommitError> {
        self.store
            .wait(&self.durability, generation, || self.write())
            .await
    }

    /// Waits until every change applied so far is durable. A session that
    /// read the state calls it before it writes what it read: an offer, an
    /// answer or a cancellation goes out only once a crash can no longer
    /// take it back, since what it read is no newer than what is applied.
    pub(crate) async fn settled(self: &Arc<Self>) -> Result<(), CommitError> {
        self.commit(self.durability.applied()).await
    }

    /// What a write of the transfer store needs.
    fn write(self: &Arc<Self>) -> Write {
        let reading: Weak<Self> = Arc::downgrade(self);
        let failing: Weak<Self> = Arc::downgrade(self);
        Write {
            snapshot: Box::new(move || {
                let transfers = reading.upgrade().ok_or(StorageError::Internal)?;
                // The generation first: the state read after it holds every
                // change up to it.
                let covered = transfers.durability.applied();
                let records = lock(&transfers.state).records();
                let encoded = records.encode()?;
                // Shown once the write completed. The job writes one
                // snapshot at a time, so the one before is settled first.
                if let Some(written) = &transfers.written {
                    let mut written = lock(written);
                    written.settle(transfers.durability.durable());
                    written.writing = Some((covered, records.records));
                }
                Ok((covered, encoded))
            }),
            failed: Box::new(move || {
                // Nothing more is stored or sent; the tasks that wait for a
                // change look again and find it failed.
                if let Some(transfers) = failing.upgrade() {
                    transfers.wake();
                }
            }),
        }
    }
}

#[cfg(test)]
mod tests;

//! The file transfers of the sessions of the runtime
//! (`docs/adr/0013-file-transfer.md`).
//!
//! A session task writes, after what it owes for what it read and after
//! the head of the chat queue: the answers and decisions the durable state
//! of its transfers owes the peer, then the offers of this side at their
//! pace, then at most one chunk, before which it takes every frame that
//! arrived. Every answer is read from the transfer store, never kept by a
//! session: a decision the user made while no session was bound to its
//! transfer goes out with the next offer of it. A change of state that a
//! file message causes is durable before the next frame is read, and a
//! FileAccept never counts what a sync of the blob did not cover.
//!
//! A Ping written after a message whose reading matters, and the Pong
//! that answers it, show that the peer read and recorded it: the flags of
//! the records say so only then.
//!
//! The sessions of one identity share which session each incoming
//! transfer is bound to, and which session carried the latest offer of
//! each outgoing one ([`Bindings`]). Everything here acts on the transfers
//! of the identity and the contact record of the session, and on nothing
//! once the identity is closed or the record ended.

use std::collections::HashSet;
use std::io::Read;

use monolith_protocol::body::{FileChunk, TransferId};
use monolith_protocol::limits::{
    FILE_OFFER_RATE, MAX_FILE_CHUNK_LEN, TRANSFER_GRACE, TRANSFER_ID_LEN,
};
use monolith_protocol::text::Filename;
use monolith_storage::StorageError;
use monolith_storage::blob::{BlobId, held_bytes};
use monolith_storage::transfers::{Direction, Flags, Reason, State};

use super::*;
use crate::api::{TransferError, TransferRef, TransferView};
use crate::blobs::{SaveReader, StageError, Verified};
use crate::exchange::{Exchange, Received};
use crate::rates::Bucket;
use crate::transfers::{Answer, Change, Key, Offer, Refusal, Staged, TransferState, Transfers};

/// L, the full chunk size.
const CHUNK: u64 = MAX_FILE_CHUNK_LEN as u64;

type Id = [u8; TRANSFER_ID_LEN];

/// The violation that ends a session without a Close.
fn violation() -> LinkError {
    LinkError::Session(monolith_session::SessionError::Protocol(
        ProtocolError::MessageNotPermitted,
    ))
}

/// The transfer a file message names.
fn named(message: &Message) -> Option<Id> {
    match message {
        Message::FileOffer { transfer, .. }
        | Message::FileAccept { transfer, .. }
        | Message::FileReject(transfer)
        | Message::FileComplete { transfer, .. }
        | Message::FileAbort(transfer) => Some(*transfer.as_bytes()),
        Message::FileChunk(chunk) => Some(*chunk.transfer().as_bytes()),
        _ => None,
    }
}

/// A random nonce for a Ping of a barrier. A failing random source gives a
/// fixed one: a session has at most one Ping outstanding.
fn nonce() -> [u8; PING_NONCE_LEN] {
    let mut bytes = [0_u8; PING_NONCE_LEN];
    if getrandom::fill(&mut bytes).is_err() {
        bytes = [0xB5; PING_NONCE_LEN];
    }
    bytes
}

/// What a command on a transfer is refused with, for a refusal of the
/// state.
const fn refused(refusal: Refusal) -> TransferError {
    match refusal {
        Refusal::TooLarge => TransferError::TooLarge,
        Refusal::TooManyWaiting => TransferError::TooManyWaiting,
        Refusal::TooManyActive => TransferError::TooManyActive,
        Refusal::StagingFull => TransferError::StagingFull,
        Refusal::ListFull => TransferError::ListFull,
        Refusal::IdTaken => TransferError::Randomness,
        Refusal::Unknown => TransferError::UnknownTransfer,
        Refusal::Busy => TransferError::Busy,
    }
}

/// What a command on a transfer is refused with, for a failure of a staged
/// file.
const fn staging_error(error: StorageError) -> TransferError {
    match error {
        StorageError::Full => TransferError::DiskFull,
        StorageError::Busy => TransferError::Busy,
        StorageError::Randomness => TransferError::Randomness,
        _ => TransferError::Storage,
    }
}

/// What the reading of the peer shows, once a Pong answers a Ping written
/// after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Fact {
    /// The peer read the offer.
    OfferRead(Key),
    /// The peer read what this side told it while the transfer was in this
    /// state.
    Told(Key, State),
}

/// What one session task holds of its transfers.
#[derive(Debug, Default)]
pub(super) struct Files {
    /// The transfers offered on the session (`crate::exchange`).
    pub(super) exchange: Exchange,
    /// What was written and waits for a Ping to cover it.
    barrier: Vec<Fact>,
    /// What the Ping outstanding covers.
    covered: Vec<Fact>,
    /// The transfers of the store the peer offered on the session.
    counted: HashSet<Id>,
    /// The reading time the session spent as the only one of its contact.
    pub(super) sole: Duration,
    /// The peer closed the session while it was the only one of its
    /// contact.
    pub(super) peer_closed: bool,
    /// The session is to end with a Close: a write that failed for a full
    /// disk or a held file, or a staged file that no longer reads, after
    /// which an offset sent here could not go down.
    pub(super) close: bool,
    /// When the pace allows the next offer.
    paced: Option<Instant>,
}

impl Files {
    /// A Ping was written: it covers what was written before it.
    pub(super) fn ping_written(&mut self) {
        self.covered.append(&mut self.barrier);
    }

    /// When the task has to look at its transfers again by itself.
    pub(super) fn deadline(&self) -> Option<Instant> {
        [self.exchange.progress_deadline(), self.paced]
            .into_iter()
            .flatten()
            .min()
    }
}

/// A session as the transfers of its identity know it.
#[derive(Debug)]
struct Seen {
    /// When it was registered, among the sessions of the identity.
    order: u64,
    contact: ContactId,
    control: Arc<SessionControl>,
    /// The offers of the peer on it.
    offered: HashSet<Id>,
}

/// What the sessions of one identity share of their transfers.
#[derive(Debug, Default)]
pub(super) struct Bindings {
    sessions: HashMap<u64, Seen>,
    /// Incoming: the session each transfer is bound to.
    bound: HashMap<(ContactId, Id), u64>,
    /// Outgoing: the session of the latest offer of each transfer.
    latest: HashMap<(ContactId, Id), u64>,
    registered: u64,
    /// The pace of the offers of this side, per contact.
    pace: HashMap<ContactId, Bucket>,
    /// The contacts a session of which brought progress to a transfer:
    /// what lets a dial for transfers start the delays again.
    pub(super) progressed: HashSet<ContactId>,
}

impl Bindings {
    /// The session `serial` of `contact` is registered.
    pub(super) fn register(
        &mut self,
        serial: u64,
        contact: ContactId,
        control: Arc<SessionControl>,
    ) {
        self.registered = self.registered.saturating_add(1);
        self.sessions.insert(
            serial,
            Seen {
                order: self.registered,
                contact,
                control,
                offered: HashSet::new(),
            },
        );
    }

    /// The peer offered `transfer` on the session `serial`. The transfer
    /// is bound to it if it is bound nowhere, or to a session registered
    /// before it.
    fn offer_seen(&mut self, serial: u64, contact: ContactId, transfer: Id) {
        let Some(order) = self.sessions.get_mut(&serial).map(|seen| {
            seen.offered.insert(transfer);
            seen.order
        }) else {
            return;
        };
        let bound = self.bound.get(&(contact, transfer)).copied();
        let moves = match bound.and_then(|serial| self.sessions.get(&serial)) {
            Some(seen) => seen.order < order,
            None => true,
        };
        if moves {
            self.bound.insert((contact, transfer), serial);
        }
    }

    fn is_bound(&self, serial: u64, contact: ContactId, transfer: Id) -> bool {
        self.bound.get(&(contact, transfer)) == Some(&serial)
    }

    fn offered_out(&mut self, serial: u64, contact: ContactId, transfer: Id) {
        self.latest.insert((contact, transfer), serial);
    }

    fn is_latest(&self, serial: u64, contact: ContactId, transfer: Id) -> bool {
        self.latest.get(&(contact, transfer)) == Some(&serial)
    }

    /// The session `serial` ended. Each transfer bound to it passes to the
    /// session registered last among the live ones it was offered on,
    /// which is woken to write what it owes for it.
    pub(super) fn session_gone(&mut self, serial: u64) {
        if self.sessions.remove(&serial).is_none() {
            return;
        }
        self.latest.retain(|_, held| *held != serial);
        let orphans: Vec<(ContactId, Id)> = self
            .bound
            .iter()
            .filter(|(_, held)| **held == serial)
            .map(|(key, _)| *key)
            .collect();
        for (contact, transfer) in orphans {
            self.bound.remove(&(contact, transfer));
            let heir = self
                .sessions
                .iter()
                .filter(|(_, seen)| seen.contact == contact && seen.offered.contains(&transfer))
                .max_by_key(|(_, seen)| seen.order)
                .map(|(serial, seen)| (*serial, seen.control.clone()));
            if let Some((heir, control)) = heir {
                self.bound.insert((contact, transfer), heir);
                control.wake();
            }
        }
    }
}

impl<B: TorBackend + 'static> Inner<B> {
    /// Makes `generation` of the transfers durable. False, with the failed
    /// store reported, if it could not be.
    async fn settle(&self, transfers: &Arc<Transfers>, generation: u64) -> bool {
        if transfers.commit(generation).await.is_ok() {
            true
        } else {
            self.storage_failure();
            false
        }
    }

    /// Removes a blob no record names any more, in the background: the
    /// removal waits for whatever reads it, a save included.
    fn release(&self, transfers: &Arc<Transfers>, blob: Option<BlobId>) {
        if let Some(blob) = blob {
            let transfers = transfers.clone();
            self.spawn(async move {
                transfers.blobs.remove(blob).await;
                transfers.blobs.retry().await;
            });
        }
    }

    /// Runs `step` on the transfers; a change of a record is made durable,
    /// reported, and what it freed removed. `None` if the store failed.
    async fn apply<R>(
        &self,
        transfers: &Arc<Transfers>,
        key: &Key,
        step: impl FnOnce(&mut TransferState, u64) -> (R, Change),
    ) -> Option<(R, Change)> {
        let ((result, change), generation) = transfers
            .change(|state, now| {
                let (result, change) = step(state, now);
                ((result, change), change.changed)
            })
            .ok()?;
        if change.changed {
            if !self.settle(transfers, generation).await {
                return None;
            }
            self.emit(Event::TransferChanged(key.reference()));
            // A removal that failed is tried again with each write of the
            // transfer store.
            if change.freed.is_none() && transfers.blobs.has_stale() {
                let transfers = transfers.clone();
                self.spawn(async move { transfers.blobs.retry().await });
            }
        }
        self.release(transfers, change.freed);
        Some((result, change))
    }

    /// Whether a transfer of `contact` needs a session.
    pub(super) fn transfers_claim(&self, runtime: &IdentityRuntime, contact: &ContactId) -> bool {
        let identity = &runtime.identity;
        identity.transfers().is_some_and(|transfers| {
            transfers.read(|state, now| state.claims(&identity.id(), contact, now))
        })
    }

    /// Starts the scheduler of every contact of `runtime` a transfer
    /// claims a session for and that has none.
    pub(super) fn schedule_transfers(self: &Arc<Self>, runtime: &Arc<IdentityRuntime>) {
        let identity = &runtime.identity;
        let Some(transfers) = identity.transfers() else {
            return;
        };
        let contacts = transfers.read(|state, now| state.claiming_contacts(&identity.id(), now));
        for contact in contacts {
            if let Some(peer) = identity.current_record(&contact) {
                if !runtime.manager.has_session(&contact) {
                    self.ensure_scheduler(runtime, contact, peer, Duration::ZERO);
                }
            }
        }
    }

    /// Whether `contact` is an accepted contact of `identity` now: files
    /// go only between accepted contacts.
    fn accepted_contact(identity: &LocalIdentity, contact: &ContactId) -> bool {
        identity
            .current_record(contact)
            .and_then(|peer| identity.contact(&peer))
            .is_some_and(|view| view.kind == monolith_protocol::contact::RecordKind::Accepted)
    }

    // --- What the peer sent --------------------------------------------------

    /// Handles a file message that arrived on the session.
    pub(super) async fn on_file<S>(
        self: &Arc<Self>,
        runtime: &Arc<IdentityRuntime>,
        link: &mut Link<S>,
        running: &mut Running,
        message: &Message,
    ) -> Result<(), LinkError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        // A transfer whose binding moved to a later session ended here,
        // with its window still on the way.
        if let (Some(contact), Some(transfer)) = (running.record, named(message)) {
            if running
                .files
                .exchange
                .is_live(&transfer, Direction::Incoming)
                && !lock(&runtime.bindings).is_bound(running.serial, contact, transfer)
            {
                running.files.exchange.end(&transfer, true);
            }
        }
        let Some(step) = running.files.exchange.receive(message) else {
            return Ok(());
        };
        if step.received == Received::Violation {
            return Err(violation());
        }
        let identity = runtime.identity.clone();
        let (Some(contact), Some(transfers)) = (running.record, identity.transfers()) else {
            return Ok(());
        };
        let key = Key {
            identity: identity.id(),
            contact,
            direction: step.direction,
            transfer: step.transfer,
        };
        // An offer counts as made on the session whatever its answer.
        if matches!(message, Message::FileOffer { .. })
            && transfers.read(|state, _| state.record(&key).is_some())
        {
            running.files.counted.insert(step.transfer);
        }
        let transfer = TransferId::from_bytes(step.transfer);
        match step.received {
            Received::Violation | Received::Discard => Ok(()),
            Received::Offer { size, name } => {
                self.on_offer(runtime, link, running, &transfers, key, size, &name)
                    .await
            }
            Received::Accept { offset, .. } => {
                running.activity = Instant::now();
                if let Some((_, change)) = self
                    .apply(&transfers, &key, |state, now| {
                        ((), state.accepted_by_peer(&key, offset, now))
                    })
                    .await
                {
                    if change.progressed {
                        lock(&runtime.bindings).progressed.insert(contact);
                    }
                    self.emit(Event::TransferProgressed {
                        reference: key.reference(),
                        bytes: offset,
                    });
                }
                Ok(())
            }
            Received::Reject => {
                running.files.exchange.end(&key.transfer, false);
                self.apply(&transfers, &key, |state, now| {
                    ((), state.rejected_by_peer(&key, now))
                })
                .await;
                Ok(())
            }
            Received::Abort => {
                running.files.exchange.end(&key.transfer, false);
                self.apply(&transfers, &key, |state, now| {
                    ((), state.aborted_by_peer(&key, now))
                })
                .await;
                Ok(())
            }
            Received::Completed { digest } => {
                running.files.exchange.end(&key.transfer, false);
                let Some((matched, change)) = self
                    .apply(&transfers, &key, |state, now| {
                        let (change, matched) = state.completed_by_peer(&key, &digest, now);
                        (matched, change)
                    })
                    .await
                else {
                    return Ok(());
                };
                if change.progressed {
                    lock(&runtime.bindings).progressed.insert(contact);
                }
                if change.changed && !matched {
                    // Another file than the one staged: the receiver is told.
                    link.send(&Message::FileAbort(transfer)).await?;
                }
                Ok(())
            }
            Received::Chunk { offset } => {
                let Message::FileChunk(chunk) = message else {
                    return Ok(());
                };
                self.on_chunk(runtime, link, running, &transfers, key, offset, chunk)
                    .await
            }
            Received::Verify { digest } => {
                self.on_verify(runtime, link, running, &transfers, key, digest)
                    .await
            }
        }
    }

    /// An offer of the peer, new on the session.
    #[expect(clippy::too_many_arguments, reason = "the parts of one offer")]
    async fn on_offer<S>(
        &self,
        runtime: &Arc<IdentityRuntime>,
        link: &mut Link<S>,
        running: &mut Running,
        transfers: &Arc<Transfers>,
        key: Key,
        size: u64,
        name: &Filename,
    ) -> Result<(), LinkError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        // Only between accepted contacts: an offer of another is answered
        // with nothing and leaves no record.
        if !Self::accepted_contact(&runtime.identity, &key.contact) {
            return Ok(());
        }
        lock(&runtime.bindings).offer_seen(running.serial, key.contact, key.transfer);
        let Ok(((offer, held), generation)) = transfers.change(|state, now| {
            let offer = state.offered(&key, size, name, now);
            let held = state.record(&key).is_some();
            ((offer, held), offer == Offer::New)
        }) else {
            return Ok(());
        };
        match offer {
            Offer::New => {
                if self.settle(transfers, generation).await {
                    self.emit(Event::TransferOffered(key.reference()));
                }
            }
            Offer::Violation => return Err(violation()),
            Offer::Answer(Answer::Reject) if !held => {
                // Refused without a record: the same offer gets the same
                // answer every time.
                link.send(&Message::FileReject(TransferId::from_bytes(key.transfer)))
                    .await?;
                running.files.exchange.end(&key.transfer, true);
            }
            // Answered from what is held, by the session the transfer is
            // bound to, before the next frame is read.
            Offer::Answer(_) | Offer::Scan(_) => {}
        }
        Ok(())
    }

    /// The next chunk of an offer of the peer, at `offset`: written to its
    /// blob before the next frame is read, and acknowledged with the
    /// progress once a sync covers it.
    #[expect(clippy::too_many_arguments, reason = "the parts of one chunk")]
    async fn on_chunk<S>(
        &self,
        runtime: &Arc<IdentityRuntime>,
        link: &mut Link<S>,
        running: &mut Running,
        transfers: &Arc<Transfers>,
        key: Key,
        offset: u64,
        chunk: &FileChunk,
    ) -> Result<(), LinkError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let Some((blob, size)) = Self::accepted_blob(transfers, &key) else {
            // Cancelled or failed meanwhile: the data is dropped, and the
            // session tells the peer.
            return Ok(());
        };
        running.activity = Instant::now();
        let index = offset.checked_div(CHUNK).unwrap_or(0);
        if let Err(error) = transfers
            .blobs
            .write_chunk(blob, size, index, chunk.data().to_vec())
            .await
        {
            self.write_failed(running, transfers, &key, error).await;
            return Ok(());
        }
        if running
            .files
            .exchange
            .progress_due(Instant::now())
            .contains(&key.transfer)
        {
            self.confirm_progress(runtime, link, running, transfers, &key)
                .await?;
        }
        Ok(())
    }

    /// The blob and the size of an accepted incoming transfer.
    fn accepted_blob(transfers: &Transfers, key: &Key) -> Option<(BlobId, u64)> {
        transfers.read(|state, _| {
            state
                .record(key)
                .filter(|record| record.state == State::Accepted)
                .and_then(|record| record.blob.map(|blob| (blob, record.size)))
        })
    }

    /// A write to the blob of `key` failed. For a full disk or a file held
    /// by another program the transfer is held, nothing more is
    /// acknowledged, and the session ends once; any other failure fails
    /// the transfer, which the session then tells the peer.
    async fn write_failed(
        &self,
        running: &mut Running,
        transfers: &Arc<Transfers>,
        key: &Key,
        error: StorageError,
    ) {
        if matches!(error, StorageError::Full | StorageError::Busy) {
            let _ = transfers.change(|state, now| {
                state.hold(key, now);
                ((), false)
            });
            transfers.wake();
            running.files.close = true;
            self.emit(Event::TransferChanged(key.reference()));
        } else {
            self.apply(transfers, key, |state, now| {
                ((), state.fail(key, Reason::Storage, now))
            })
            .await;
        }
    }

    /// Acknowledges the progress of `key`: the blob is synced, and only
    /// then is FileAccept written with every byte the sync covered.
    async fn confirm_progress<S>(
        &self,
        runtime: &Arc<IdentityRuntime>,
        link: &mut Link<S>,
        running: &mut Running,
        transfers: &Arc<Transfers>,
        key: &Key,
    ) -> Result<(), LinkError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let Some((blob, size)) = Self::accepted_blob(transfers, key) else {
            return Ok(());
        };
        let Some(bytes) = running.files.exchange.expected(&key.transfer) else {
            return Ok(());
        };
        if let Err(error) = transfers.blobs.sync(blob, size).await {
            self.write_failed(running, transfers, key, error).await;
            return Ok(());
        }
        let Some((_, change)) = self
            .apply(transfers, key, |state, now| {
                ((), state.staged(key, bytes, now))
            })
            .await
        else {
            return Ok(());
        };
        if change.progressed {
            lock(&runtime.bindings).progressed.insert(key.contact);
        }
        link.send(&Message::FileAccept {
            transfer: TransferId::from_bytes(key.transfer),
            offset: bytes,
        })
        .await?;
        running
            .files
            .exchange
            .accepting(&key.transfer, bytes, Instant::now());
        self.emit(Event::TransferProgressed {
            reference: key.reference(),
            bytes,
        });
        Ok(())
    }

    /// The offerer sent every byte of `key`: the receiver checks the size,
    /// computes the digest of what it staged, and on a match makes the
    /// completion durable before it answers with FileComplete.
    async fn on_verify<S>(
        &self,
        _runtime: &Arc<IdentityRuntime>,
        link: &mut Link<S>,
        running: &mut Running,
        transfers: &Arc<Transfers>,
        key: Key,
        digest: [u8; 32],
    ) -> Result<(), LinkError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let Some((blob, size)) = Self::accepted_blob(transfers, &key) else {
            return Ok(());
        };
        running.activity = Instant::now();
        let transfer = TransferId::from_bytes(key.transfer);
        match transfers.blobs.verify(blob, size).await {
            Ok(Verified::Digest(held)) if held == digest => {
                if self
                    .apply(transfers, &key, |state, now| {
                        ((), state.complete(&key, held, now))
                    })
                    .await
                    .is_none()
                {
                    return Ok(());
                }
                // Durable: only now is the completion acknowledged.
                link.send(&Message::FileComplete {
                    transfer,
                    digest: held,
                })
                .await?;
                running.files.exchange.end(&key.transfer, true);
                running.files.barrier.push(Fact::Told(key, State::Complete));
            }
            Ok(Verified::Digest(_)) => {
                if self
                    .apply(transfers, &key, |state, now| {
                        ((), state.fail(&key, Reason::Digest, now))
                    })
                    .await
                    .is_none()
                {
                    return Ok(());
                }
                link.send(&Message::FileAbort(transfer)).await?;
                running.files.exchange.end(&key.transfer, true);
            }
            Ok(Verified::Damaged(good)) => {
                // Everything from the first segment that does not read on
                // counts as not written; the offset sent here cannot go
                // down, so the session ends and the next one starts lower.
                let _ = transfers.change(|state, _| {
                    state.scanned(&key, held_bytes(good, size));
                    ((), false)
                });
                running.files.close = true;
            }
            Err(error) => self.write_failed(running, transfers, &key, error).await,
        }
        Ok(())
    }

    /// A Pong answered the Ping outstanding: what it covers is known read.
    pub(super) async fn files_read(&self, runtime: &IdentityRuntime, running: &mut Running) {
        let facts = core::mem::take(&mut running.files.covered);
        let Some(transfers) = runtime.identity.transfers() else {
            return;
        };
        for fact in facts {
            match fact {
                Fact::OfferRead(key) => {
                    self.apply(&transfers, &key, |state, _| ((), state.offer_read(&key)))
                        .await;
                }
                Fact::Told(key, held) => {
                    self.apply(&transfers, &key, |state, _| ((), state.told(&key, held)))
                        .await;
                }
            }
        }
    }

    // --- What this side writes ----------------------------------------------

    /// Writes what the durable state owes the peer on this session: for an
    /// offer of the peer bound here, its answer or the decision of the
    /// user; for an offer of this side, its cancellation or failure. Then
    /// the offers of this side, and a Ping if what was written needs one.
    pub(super) async fn files_write<S>(
        self: &Arc<Self>,
        runtime: &Arc<IdentityRuntime>,
        link: &mut Link<S>,
        running: &mut Running,
    ) -> Result<(), LinkError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let identity = runtime.identity.clone();
        let (Some(contact), Some(transfers)) = (running.record, identity.transfers()) else {
            return Ok(());
        };
        for transfer in running.files.exchange.incoming() {
            if !lock(&runtime.bindings).is_bound(running.serial, contact, transfer) {
                continue;
            }
            let key = Key {
                identity: identity.id(),
                contact,
                direction: Direction::Incoming,
                transfer,
            };
            self.answer(link, running, &transfers, key).await?;
        }
        for transfer in running.files.exchange.outgoing() {
            let key = Key {
                identity: identity.id(),
                contact,
                direction: Direction::Outgoing,
                transfer,
            };
            let held = transfers.read(|state, _| {
                state
                    .record(&key)
                    .map(|record| (record.state, record.flags.has(Flags::UNREAD)))
            });
            if let Some((state @ (State::Cancelled | State::Failed), unread)) = held {
                link.send(&Message::FileAbort(TransferId::from_bytes(transfer)))
                    .await?;
                running.files.exchange.end(&transfer, true);
                if unread {
                    running.files.barrier.push(Fact::Told(key, state));
                }
            }
        }
        self.offer(runtime, link, running, &transfers).await?;
        if running.ping.is_none() && !running.files.barrier.is_empty() {
            let nonce = nonce();
            link.send(&Message::Ping(nonce)).await?;
            running.ping = Some((nonce, PONG_TIMEOUT));
            running.files.ping_written();
        }
        Ok(())
    }

    /// Writes the answer the durable state owes for the offer `key` of the
    /// peer, bound to this session, if it was not written here yet.
    async fn answer<S>(
        &self,
        link: &mut Link<S>,
        running: &mut Running,
        transfers: &Arc<Transfers>,
        key: Key,
    ) -> Result<(), LinkError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let mut answer = transfers.read(|state, now| state.answer(&key, now));
        if let Offer::Scan(blob) = answer {
            answer = self.scan(running, transfers, &key, blob).await;
        }
        let Offer::Answer(answer) = answer else {
            return Ok(());
        };
        let held = transfers.read(|state, _| {
            state
                .record(&key)
                .map(|record| (record.state, record.flags.has(Flags::UNREAD)))
        });
        let Some((state, unread)) = held else {
            return Ok(());
        };
        let transfer = TransferId::from_bytes(key.transfer);
        let message = match answer {
            Answer::Nothing => return Ok(()),
            Answer::Accept(offset) => {
                if running.files.exchange.accepted(&key.transfer).is_some() {
                    return Ok(());
                }
                link.send(&Message::FileAccept { transfer, offset }).await?;
                running
                    .files
                    .exchange
                    .accepting(&key.transfer, offset, Instant::now());
                if unread {
                    running.files.barrier.push(Fact::Told(key, state));
                }
                return Ok(());
            }
            Answer::Complete(digest) => Message::FileComplete { transfer, digest },
            Answer::Reject => Message::FileReject(transfer),
            Answer::Abort => Message::FileAbort(transfer),
        };
        link.send(&message).await?;
        running.files.exchange.end(&key.transfer, true);
        if unread {
            running.files.barrier.push(Fact::Told(key, state));
        }
        Ok(())
    }

    /// Finds how much of the accepted transfer `key` its blob holds, the
    /// first time it is used after the installation opened. A blob that is
    /// missing or does not match its record is replaced with a new one,
    /// empty; one that cannot be reserved holds the transfer.
    async fn scan(
        &self,
        running: &mut Running,
        transfers: &Arc<Transfers>,
        key: &Key,
        blob: BlobId,
    ) -> Offer {
        let Some(size) = transfers.read(|state, _| state.record(key).map(|record| record.size))
        else {
            return Offer::Answer(Answer::Nothing);
        };
        match transfers.blobs.scan(blob, size).await {
            Ok(Some(segments)) => {
                let _ = transfers.change(|state, _| {
                    state.scanned(key, held_bytes(segments, size));
                    ((), false)
                });
            }
            Ok(None) => match transfers.blobs.create(size).await {
                Ok(new) => {
                    if self
                        .apply(transfers, key, |state, _| {
                            ((), state.replace_blob(key, new))
                        })
                        .await
                        .is_none()
                    {
                        return Offer::Answer(Answer::Nothing);
                    }
                }
                Err(error) => self.write_failed(running, transfers, key, error).await,
            },
            Err(error) => self.write_failed(running, transfers, key, error).await,
        }
        let answer = transfers.read(|state, now| state.answer(key, now));
        if matches!(answer, Offer::Scan(_)) {
            Offer::Answer(Answer::Nothing)
        } else {
            answer
        }
    }

    /// Writes the offers of this side the session owes, if it is the
    /// preferred and only session of its contact: those being sent, those
    /// cancelled while the peer could not be told, each followed by its
    /// FileAbort, then those waiting, at the pace of `FILE_OFFER_RATE`.
    async fn offer<S>(
        &self,
        runtime: &Arc<IdentityRuntime>,
        link: &mut Link<S>,
        running: &mut Running,
        transfers: &Arc<Transfers>,
    ) -> Result<(), LinkError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        running.files.paced = None;
        let Some(contact) = running.record else {
            return Ok(());
        };
        let identity = &runtime.identity;
        if link.session().state() != SessionState::AuthenticatedContact
            || !runtime.manager.is_preferred(&contact, running.serial)
            || runtime.manager.sessions(&contact) != 1
            || !Self::accepted_contact(identity, &contact)
        {
            return Ok(());
        }
        let offers = transfers.read(|state, now| state.to_offer(&identity.id(), &contact, now));
        for offer in offers {
            if !running.files.exchange.may_offer(&offer.transfer) {
                continue;
            }
            let now = Instant::now();
            let paced = lock(&runtime.bindings)
                .pace
                .entry(contact)
                .or_insert_with(|| Bucket::new(now))
                .take(FILE_OFFER_RATE, now);
            if let Err(at) = paced {
                running.files.paced = Some(at);
                break;
            }
            let key = Key {
                identity: identity.id(),
                contact,
                direction: Direction::Outgoing,
                transfer: offer.transfer,
            };
            let transfer = TransferId::from_bytes(offer.transfer);
            link.send(&Message::FileOffer {
                transfer,
                size: offer.size,
                filename: offer.name.clone(),
            })
            .await?;
            running.files.exchange.offered(offer.transfer, offer.size);
            lock(&runtime.bindings).offered_out(running.serial, contact, offer.transfer);
            let read = transfers
                .change(|state, _| {
                    state.mark_offered(&key);
                    let read = state
                        .record(&key)
                        .is_some_and(|record| record.flags.has(Flags::OFFER_READ));
                    (read, false)
                })
                .map(|(read, _)| read)
                .unwrap_or(true);
            if !read {
                running.files.barrier.push(Fact::OfferRead(key));
            }
            if offer.cancelled {
                link.send(&Message::FileAbort(transfer)).await?;
                running.files.exchange.end(&offer.transfer, true);
                running
                    .files
                    .barrier
                    .push(Fact::Told(key, State::Cancelled));
            }
        }
        Ok(())
    }

    /// Writes the completions due and at most one chunk of the transfer
    /// that sends, if the window has room and the session is the
    /// preferred one and that of the latest offer of the transfer. Before
    /// the chunk it takes every complete frame that has arrived, without
    /// waiting for one that has not. True if it wrote a chunk.
    pub(super) async fn files_pump<S>(
        self: &Arc<Self>,
        runtime: &Arc<IdentityRuntime>,
        link: &mut Link<S>,
        running: &mut Running,
    ) -> Result<bool, LinkError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let identity = runtime.identity.clone();
        let (Some(contact), Some(transfers)) = (running.record, identity.transfers()) else {
            return Ok(false);
        };
        if !runtime.manager.is_preferred(&contact, running.serial) {
            return Ok(false);
        }
        let sending = |transfer: Id| {
            let key = Key {
                identity: identity.id(),
                contact,
                direction: Direction::Outgoing,
                transfer,
            };
            transfers.read(|state, _| {
                state
                    .record(&key)
                    .filter(|record| record.state == State::Sending)
                    .and_then(|record| record.blob.map(|blob| (blob, record.size, record.digest)))
            })
        };
        for transfer in running.files.exchange.completion_due() {
            if !lock(&runtime.bindings).is_latest(running.serial, contact, transfer) {
                continue;
            }
            if let Some((_, _, digest)) = sending(transfer) {
                link.send(&Message::FileComplete {
                    transfer: TransferId::from_bytes(transfer),
                    digest,
                })
                .await?;
                running.files.exchange.completion_written(&transfer);
            }
        }
        if running.files.exchange.next_chunk().is_none() {
            return Ok(false);
        }
        // Every frame that has arrived first, so that with transfers both
        // ways neither side stops reading.
        loop {
            let ready = {
                let mut receive = pin!(link.receive());
                poll_fn(|cx| match receive.as_mut().poll(cx) {
                    Poll::Ready(received) => Poll::Ready(Some(received)),
                    Poll::Pending => Poll::Ready(None),
                })
                .await
            };
            match ready {
                None => break,
                Some(Err(error)) => return Err(error),
                Some(Ok(arrived)) => {
                    if self.pace(runtime, running, &arrived).await == Waited::Stop {
                        return Err(LinkError::Session(monolith_session::SessionError::Closed));
                    }
                    Box::pin(self.handle(runtime, link, running, &arrived)).await?;
                    if running.files.close || running.control.closing() {
                        return Ok(false);
                    }
                }
            }
        }
        let Some((transfer, offset, len)) = running.files.exchange.next_chunk() else {
            return Ok(false);
        };
        if !lock(&runtime.bindings).is_latest(running.serial, contact, transfer) {
            return Ok(false);
        }
        let Some((blob, size, _)) = sending(transfer) else {
            return Ok(false);
        };
        let key = Key {
            identity: identity.id(),
            contact,
            direction: Direction::Outgoing,
            transfer,
        };
        let index = offset.checked_div(CHUNK).unwrap_or(0);
        let data = match transfers.blobs.read_chunk(blob, size, index).await {
            Ok(data) if u64::try_from(data.len()).ok() == Some(len) => data,
            _ => {
                // The staged file no longer reads: the transfer fails, and
                // the receiver is told.
                self.apply(&transfers, &key, |state, now| {
                    ((), state.fail(&key, Reason::Storage, now))
                })
                .await;
                return Ok(false);
            }
        };
        let chunk = FileChunk::new(TransferId::from_bytes(transfer), data)
            .map_err(|error| LinkError::Session(monolith_session::SessionError::Protocol(error)))?;
        link.send(&Message::FileChunk(chunk)).await?;
        running.files.exchange.chunk_written(&transfer, len);
        running.activity = Instant::now();
        Ok(true)
    }

    /// Acknowledges the progress whose interval passed.
    pub(super) async fn files_progress<S>(
        self: &Arc<Self>,
        runtime: &Arc<IdentityRuntime>,
        link: &mut Link<S>,
        running: &mut Running,
    ) -> Result<(), LinkError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let identity = runtime.identity.clone();
        let (Some(contact), Some(transfers)) = (running.record, identity.transfers()) else {
            return Ok(());
        };
        for transfer in running.files.exchange.progress_due(Instant::now()) {
            let key = Key {
                identity: identity.id(),
                contact,
                direction: Direction::Incoming,
                transfer,
            };
            self.confirm_progress(runtime, link, running, &transfers, &key)
                .await?;
        }
        Ok(())
    }

    /// The session `running` ended. A session that counted (confirmed, and
    /// the only one of its contact for `TRANSFER_GRACE` of reading time or
    /// closed by the peer while it was) tells the transfers which offers
    /// the peer made on it. Runs when the task is dropped; with no tokio
    /// runtime left it does nothing.
    pub(super) fn files_ended(
        self: &Arc<Self>,
        runtime: &Arc<IdentityRuntime>,
        running: &mut Running,
    ) {
        lock(&runtime.bindings).session_gone(running.serial);
        let Some(contact) = running.record else {
            return;
        };
        let counts = running.files.sole >= TRANSFER_GRACE || running.files.peer_closed;
        if !counts || tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let offered = core::mem::take(&mut running.files.counted);
        let inner = self.clone();
        let identity = runtime.identity.clone();
        self.spawn(async move {
            let Some(transfers) = identity.transfers() else {
                return;
            };
            let Ok((changes, generation)) = transfers.change(|state, now| {
                let changes = state.session_counted(&identity.id(), &contact, &offered, now);
                let changed = !changes.is_empty();
                (changes, changed)
            }) else {
                return;
            };
            if changes.is_empty() || !inner.settle(&transfers, generation).await {
                return;
            }
            for (key, change) in changes {
                inner.emit(Event::TransferChanged(key.reference()));
                inner.release(&transfers, change.freed);
            }
        });
    }

    /// Follows the deadlines of the transfers: an offer expires, a
    /// transfer without progress fails, a hold ends, an ended record goes.
    pub(super) async fn follow_transfers(self: Arc<Self>) {
        let transfers = self.installation.transfers();
        let mut changes = transfers.subscribe();
        let mut shutdown = self.shutdown.subscribe();
        for key in transfers.interrupted() {
            self.emit(Event::SaveInterrupted(key.reference()));
        }
        loop {
            if *shutdown.borrow_and_update() || transfers.is_failed() {
                return;
            }
            changes.borrow_and_update();
            let (due, now) = transfers.read(|state, now| (state.next_due(), now));
            if due.is_some_and(|due| due <= now) {
                let Ok((ticked, generation)) = transfers.change(|state, now| {
                    let ticked = state.tick(now);
                    let changed = ticked.iter().any(|(_, change)| change.changed);
                    (ticked, changed)
                }) else {
                    return;
                };
                if ticked.iter().any(|(_, change)| change.changed)
                    && !self.settle(&transfers, generation).await
                {
                    return;
                }
                for (key, change) in &ticked {
                    if change.changed {
                        self.emit(Event::TransferChanged(key.reference()));
                    }
                    self.release(&transfers, change.freed);
                }
                // A hold that ended, a claim that went: the schedulers and
                // the sessions look again.
                transfers.wake();
                let held: Vec<Arc<IdentityRuntime>> =
                    lock(&self.identities).values().cloned().collect();
                for runtime in &held {
                    self.schedule_transfers(runtime);
                }
                continue;
            }
            let wait = due.map(|due| {
                after(
                    Instant::now(),
                    Duration::from_secs(due.saturating_sub(now).max(1)),
                )
            });
            let gone = {
                let mut timer = pin!(async {
                    match wait {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => core::future::pending::<()>().await,
                    }
                });
                let mut changed = pin!(changes.changed());
                let mut stopping = pin!(shutdown.changed());
                poll_fn(|cx| {
                    if timer.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(false);
                    }
                    if let Poll::Ready(result) = changed.as_mut().poll(cx) {
                        return Poll::Ready(result.is_err());
                    }
                    if let Poll::Ready(result) = stopping.as_mut().poll(cx) {
                        return Poll::Ready(result.is_err());
                    }
                    Poll::Pending
                })
                .await
            };
            if gone {
                return;
            }
        }
    }

    /// The contact record `contact` of `runtime` ended: blocked, its
    /// transfers are cancelled; deleted, they go. Nothing is told to the
    /// peer, and every staged file of the contact goes.
    pub(super) async fn contact_transfers_ended(
        &self,
        runtime: &IdentityRuntime,
        contact: ContactId,
        delete: bool,
    ) {
        let identity = &runtime.identity;
        let transfers = self.installation.transfers();
        let Ok((changes, generation)) = transfers.change(|state, now| {
            let changes = state.contact_ended(&identity.id(), &contact, delete, now);
            let changed = !changes.is_empty();
            (changes, changed)
        }) else {
            return;
        };
        if changes.is_empty() || !self.settle(&transfers, generation).await {
            return;
        }
        for (key, change) in changes {
            self.emit(Event::TransferChanged(key.reference()));
            self.release(&transfers, change.freed);
        }
    }

    // --- Commands ----------------------------------------------------------------

    /// Stages and records an outgoing transfer: the work of
    /// [`Runtime::offer_file`] once its arguments were checked.
    async fn offer_file(
        self: &Arc<Self>,
        held: &Arc<IdentityRuntime>,
        contact: ContactId,
        name: Filename,
        size: u64,
        source: Box<dyn Read + Send>,
    ) -> Result<TransferRef, TransferError> {
        let identity = held.identity.clone();
        let peer = identity
            .current_record(&contact)
            .filter(|_| Self::accepted_contact(&identity, &contact))
            .ok_or(TransferError::NotAContact)?;
        let transfers = identity.transfers().ok_or(TransferError::UnknownIdentity)?;
        if transfers.is_failed() {
            return Err(TransferError::Storage);
        }
        transfers
            .read(|state, now| state.may_offer(&identity.id(), &contact, size, now))
            .map_err(refused)?;
        let (blob, digest) =
            transfers
                .blobs
                .stage(size, source)
                .await
                .map_err(|error| match error {
                    StageError::Source => TransferError::Source,
                    StageError::Storage(error) => staging_error(error),
                })?;
        let mut draws = 0_u8;
        let recorded = loop {
            let mut transfer = [0_u8; TRANSFER_ID_LEN];
            if getrandom::fill(&mut transfer).is_err() {
                break Err(TransferError::Randomness);
            }
            let staged = Staged {
                identity: identity.id(),
                contact,
                transfer,
                size,
                name: name.clone(),
                digest,
                blob,
            };
            let changed = transfers.change(|state, now| {
                // The identity and the record are looked at again where
                // the record is made: a record that ended meanwhile, or a
                // closed identity, gets no transfer.
                let result = if identity.is_closed() || !identity.is_current(&contact, &peer) {
                    Err(Refusal::Unknown)
                } else {
                    state.add_outgoing(staged, now)
                };
                let changed = result.is_ok();
                (result, changed)
            });
            match changed {
                Ok((Ok(key), generation)) => break Ok((key, generation)),
                Ok((Err(Refusal::IdTaken), _)) if draws < 3 => draws = draws.saturating_add(1),
                Ok((Err(Refusal::Unknown), _)) => break Err(TransferError::NotAContact),
                Ok((Err(refusal), _)) => break Err(refused(refusal)),
                Err(_) => break Err(TransferError::Storage),
            }
        };
        let (key, generation) = match recorded {
            Ok(recorded) => recorded,
            Err(error) => {
                transfers.blobs.remove(blob).await;
                return Err(error);
            }
        };
        if !self.settle(&transfers, generation).await {
            return Err(TransferError::Storage);
        }
        self.emit(Event::TransferChanged(key.reference()));
        if !held.manager.has_session(&contact) {
            self.ensure_scheduler(held, contact, peer, Duration::ZERO);
        }
        Ok(key.reference())
    }

    /// Accepts the offer `key`: the blob is reserved and durable before the
    /// record names it.
    async fn accept_transfer(
        self: &Arc<Self>,
        held: &Arc<IdentityRuntime>,
        key: Key,
    ) -> Result<(), TransferError> {
        let identity = held.identity.clone();
        let peer = identity
            .current_record(&key.contact)
            .filter(|_| Self::accepted_contact(&identity, &key.contact))
            .ok_or(TransferError::NotAContact)?;
        let transfers = identity.transfers().ok_or(TransferError::UnknownIdentity)?;
        let size = transfers
            .read(|state, _| state.may_accept(&key))
            .map_err(refused)?;
        let blob = transfers.blobs.create(size).await.map_err(staging_error)?;
        let accepted = transfers.change(|state, now| {
            let result = if identity.is_closed() || !identity.is_current(&key.contact, &peer) {
                Err(Refusal::Unknown)
            } else {
                state.accept(&key, blob, now)
            };
            let changed = result.is_ok();
            (result, changed)
        });
        let generation = match accepted {
            Ok((Ok(_), generation)) => generation,
            Ok((Err(refusal), _)) => {
                transfers.blobs.remove(blob).await;
                // A contact that ended meanwhile is told as such.
                return Err(if identity.is_current(&key.contact, &peer) {
                    refused(refusal)
                } else {
                    TransferError::NotAContact
                });
            }
            Err(_) => {
                transfers.blobs.remove(blob).await;
                return Err(TransferError::Storage);
            }
        };
        if !self.settle(&transfers, generation).await {
            return Err(TransferError::Storage);
        }
        self.emit(Event::TransferChanged(key.reference()));
        if !held.manager.has_session(&key.contact) {
            self.ensure_scheduler(held, key.contact, peer, Duration::ZERO);
        }
        Ok(())
    }

    /// Runs a decision of the user on `key`, made durable before it
    /// returns; the session bound to the transfer tells the peer.
    async fn decide(
        self: &Arc<Self>,
        held: &Arc<IdentityRuntime>,
        key: Key,
        decision: impl FnOnce(&mut TransferState, &Key, u64) -> Result<Change, Refusal>,
    ) -> Result<(), TransferError> {
        let transfers = held
            .identity
            .transfers()
            .ok_or(TransferError::UnknownIdentity)?;
        let ((result, change), generation) = transfers
            .change(|state, now| {
                let result = decision(state, &key, now);
                let change = result.unwrap_or_default();
                ((result, change), change.changed)
            })
            .map_err(|_| TransferError::Storage)?;
        result.map_err(refused)?;
        if change.changed {
            if !self.settle(&transfers, generation).await {
                return Err(TransferError::Storage);
            }
            self.emit(Event::TransferChanged(key.reference()));
        }
        self.release(&transfers, change.freed);
        if let Some(peer) = held.identity.current_record(&key.contact) {
            if !held.manager.has_session(&key.contact) && self.transfers_claim(held, &key.contact) {
                self.ensure_scheduler(held, key.contact, peer, Duration::ZERO);
            }
        }
        Ok(())
    }
}

impl<B: TorBackend + 'static> Runtime<B> {
    fn transfer_held(&self, identity: &IdentityId) -> Result<Arc<IdentityRuntime>, TransferError> {
        self.held(identity).map_err(|error| match error {
            CommandError::ShuttingDown => TransferError::ShuttingDown,
            _ => TransferError::UnknownIdentity,
        })
    }

    /// Offers a file to the contact record `contact` of `identity`, an
    /// accepted contact. `name` is the name of the local file; it is
    /// offered under [`offer_name`] of it, never a path. `source` gives the
    /// `size` bytes of the file once, as they are staged, encrypted, before
    /// the offer exists (`docs/adr/0014-transfer-staging.md`): a source
    /// that gives fewer or more fails the offer. Succeeds once the offer is
    /// durable; it is made when the contact is reached. One file is staged
    /// at a time.
    pub async fn offer_file(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
        name: &str,
        size: u64,
        source: Box<dyn Read + Send>,
    ) -> Result<TransferRef, TransferError> {
        let held = self.transfer_held(identity)?;
        let name = Filename::from_local_name(name);
        let inner = self.inner.clone();
        let contact = *contact;
        self.inner
            .detached(async move { inner.offer_file(&held, contact, name, size, source).await })
            .await
            .unwrap_or(Err(TransferError::ShuttingDown))
    }

    /// Accepts the offer `transfer`: its file is staged as it arrives,
    /// within the limits of active transfers and staged bytes and with the
    /// room it needs reserved on the disk. Durable when this returns.
    pub async fn accept_transfer(&self, transfer: &TransferRef) -> Result<(), TransferError> {
        if !transfer.incoming {
            return Err(TransferError::UnknownTransfer);
        }
        let held = self.transfer_held(&transfer.identity)?;
        let inner = self.inner.clone();
        let key = Key::named(transfer);
        self.inner
            .detached(async move { inner.accept_transfer(&held, key).await })
            .await
            .unwrap_or(Err(TransferError::ShuttingDown))
    }

    /// Rejects the offer `transfer`. Rejecting twice is rejecting once.
    pub async fn reject_transfer(&self, transfer: &TransferRef) -> Result<(), TransferError> {
        if !transfer.incoming {
            return Err(TransferError::UnknownTransfer);
        }
        self.transfer_decision(transfer, TransferState::reject)
            .await
    }

    /// Cancels `transfer`, in either direction, unless it is complete: an
    /// undecided offer is rejected. Its staged file goes, and the contact
    /// is told. Cancelling twice is cancelling once.
    pub async fn cancel_transfer(&self, transfer: &TransferRef) -> Result<(), TransferError> {
        self.transfer_decision(transfer, TransferState::cancel)
            .await
    }

    /// Removes the ended `transfer`, or a received file, from the list; a
    /// file not saved goes.
    pub async fn remove_transfer(&self, transfer: &TransferRef) -> Result<(), TransferError> {
        self.transfer_decision(transfer, TransferState::remove)
            .await
    }

    /// Goes on with a received transfer held after its disk was full or
    /// its staged file was held by another program.
    pub async fn resume_transfer(&self, transfer: &TransferRef) -> Result<(), TransferError> {
        if !transfer.incoming {
            return Err(TransferError::UnknownTransfer);
        }
        self.transfer_decision(transfer, |state, key, _| {
            state.resume(key).map(|()| Change::default())
        })
        .await
    }

    async fn transfer_decision(
        &self,
        transfer: &TransferRef,
        decision: impl FnOnce(&mut TransferState, &Key, u64) -> Result<Change, Refusal> + Send + 'static,
    ) -> Result<(), TransferError> {
        let held = self.transfer_held(&transfer.identity)?;
        let inner = self.inner.clone();
        let key = Key::named(transfer);
        self.inner
            .detached(async move {
                let decided = inner.decide(&held, key, decision).await;
                if let Some(transfers) = held.identity.transfers() {
                    // A hold that ended changed nothing durable: the
                    // sessions look again all the same.
                    transfers.wake();
                }
                decided
            })
            .await
            .unwrap_or(Err(TransferError::ShuttingDown))
    }

    /// The transfers of `identity`, oldest first, but those removed from
    /// the list.
    pub fn transfers(&self, identity: &IdentityId) -> Result<Vec<TransferView>, TransferError> {
        let held = self.transfer_held(identity)?;
        let transfers = held
            .identity
            .transfers()
            .ok_or(TransferError::UnknownIdentity)?;
        let id = held.identity.id();
        Ok(transfers.read(|state, now| state.views(&id, now)))
    }

    /// One transfer, unless it was removed from the list.
    pub fn transfer(&self, transfer: &TransferRef) -> Result<Option<TransferView>, TransferError> {
        let held = self.transfer_held(&transfer.identity)?;
        let transfers = held
            .identity
            .transfers()
            .ok_or(TransferError::UnknownIdentity)?;
        let key = Key::named(transfer);
        Ok(transfers.read(|state, now| state.view(&key, now)))
    }

    /// Begins a save of the received file of `transfer`
    /// (`docs/adr/0014-transfer-staging.md`, "Saving a received file"):
    /// that a save began is durable first. The reader gives the file and,
    /// after its last byte, an error unless the file has its size and its
    /// digest. The caller writes it to a temporary file in the folder the
    /// user chose, syncs it, gives it its final name without replacing
    /// anything, and reports the outcome with [`Self::end_save`]. One save
    /// or staging runs at a time.
    pub async fn begin_save(&self, transfer: &TransferRef) -> Result<SaveReader, TransferError> {
        if !transfer.incoming {
            return Err(TransferError::UnknownTransfer);
        }
        let held = self.transfer_held(&transfer.identity)?;
        let inner = self.inner.clone();
        let key = Key::named(transfer);
        self.inner
            .detached(async move {
                let transfers = held
                    .identity
                    .transfers()
                    .ok_or(TransferError::UnknownIdentity)?;
                let ((saving, changed), generation) = transfers
                    .change(|state, _| {
                        let saving = state.begin_save(&key);
                        let changed = saving.is_ok();
                        ((saving, changed), changed)
                    })
                    .map_err(|_| TransferError::Storage)?;
                let saving = saving.map_err(refused)?;
                if changed && !inner.settle(&transfers, generation).await {
                    return Err(TransferError::Storage);
                }
                match transfers
                    .blobs
                    .reader(saving.blob, saving.size, saving.digest)
                    .await
                {
                    Ok(reader) => Ok(reader),
                    // A staged file that is missing or does not match its
                    // record fails the transfer, and the user is told.
                    Err(
                        StorageError::NotFound
                        | StorageError::BadFormat
                        | StorageError::AuthenticationFailed,
                    ) => {
                        let _ = inner
                            .decide(&held, key, |state, key, now| {
                                Ok(state.fail(key, Reason::Storage, now))
                            })
                            .await;
                        Err(TransferError::Storage)
                    }
                    Err(error) => {
                        let _ = inner
                            .decide(&held, key, |state, key, now| {
                                Ok(state.end_save(key, false, now))
                            })
                            .await;
                        Err(staging_error(error))
                    }
                }
            })
            .await
            .unwrap_or(Err(TransferError::ShuttingDown))
    }

    /// The saves of `identity` that were under way when the installation
    /// last closed: a file named `monolith-partial-` and 16 hexadecimal
    /// digits may be left in the folder each went to. The same list as the
    /// [`Event::SaveInterrupted`] raised when the runtime started.
    pub fn interrupted_saves(
        &self,
        identity: &IdentityId,
    ) -> Result<Vec<TransferRef>, TransferError> {
        let held = self.transfer_held(identity)?;
        Ok(self
            .inner
            .installation
            .transfers()
            .interrupted()
            .into_iter()
            .filter(|key| key.identity == held.identity.id())
            .map(|key| key.reference())
            .collect())
    }

    /// Ends the save of `transfer` that [`Self::begin_save`] began: the
    /// file is saved, and its staged copy goes, or it was not, and the
    /// staged copy stays.
    ///
    /// A save that failed because the staged file did not read as it was
    /// verified fails the transfer instead (`docs/adr/0014`, "Missing and
    /// damaged blobs").
    pub async fn end_save(&self, transfer: &TransferRef, saved: bool) -> Result<(), TransferError> {
        let held = self.transfer_held(&transfer.identity)?;
        let key = Key::named(transfer);
        let damaged = !saved
            && held.identity.transfers().is_some_and(|transfers| {
                transfers
                    .read(|state, _| state.record(&key).and_then(|record| record.blob))
                    .is_some_and(|blob| transfers.blobs.take_damaged(blob))
            });
        self.transfer_decision(transfer, move |state, key, now| {
            Ok(if damaged {
                state.fail(key, Reason::Storage, now)
            } else {
                state.end_save(key, saved, now)
            })
        })
        .await
    }
}

/// The filename under which a local file named `name` is offered: its last
/// component, made a valid filename (`docs/adr/0013-file-transfer.md`,
/// "Names"). What the user is shown before the offer is made.
pub fn offer_name(name: &str) -> String {
    Filename::from_local_name(name).as_str().to_owned()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use monolith_storage::record::RECORD_ID_LEN;

    use super::*;

    const TRANSFER: Id = [7; TRANSFER_ID_LEN];

    fn contact(byte: u8) -> ContactId {
        ContactId::from_bytes([byte; RECORD_ID_LEN]).unwrap()
    }

    /// Bindings with the sessions `serials` of contact 1, registered in
    /// that order.
    fn registered(serials: &[u64]) -> Bindings {
        let mut bindings = Bindings::default();
        for serial in serials {
            bindings.register(*serial, contact(1), Arc::new(SessionControl::default()));
        }
        bindings
    }

    #[test]
    fn a_transfer_is_bound_to_its_first_offer_and_moves_only_to_a_session_registered_later() {
        let mut bindings = registered(&[10, 20]);
        bindings.offer_seen(20, contact(1), TRANSFER);
        assert!(bindings.is_bound(20, contact(1), TRANSFER));
        // An offer on a session registered before it moves nothing.
        bindings.offer_seen(10, contact(1), TRANSFER);
        assert!(bindings.is_bound(20, contact(1), TRANSFER));
        assert!(!bindings.is_bound(10, contact(1), TRANSFER));
        let mut bindings = registered(&[10, 20]);
        bindings.offer_seen(10, contact(1), TRANSFER);
        assert!(bindings.is_bound(10, contact(1), TRANSFER));
        bindings.offer_seen(20, contact(1), TRANSFER);
        assert!(bindings.is_bound(20, contact(1), TRANSFER));
        assert!(!bindings.is_bound(10, contact(1), TRANSFER));
        // The same identifier of another contact is another transfer.
        assert!(!bindings.is_bound(20, contact(2), TRANSFER));
    }

    #[test]
    fn when_the_bound_session_ends_the_binding_passes_to_the_latest_live_one_it_was_offered_on() {
        let mut bindings = registered(&[1, 2, 3, 4]);
        bindings.offer_seen(1, contact(1), TRANSFER);
        bindings.offer_seen(2, contact(1), TRANSFER);
        bindings.offer_seen(4, contact(1), TRANSFER);
        assert!(bindings.is_bound(4, contact(1), TRANSFER));
        bindings.session_gone(4);
        // Session 3 never had the offer; 2 is the latest that did.
        assert!(bindings.is_bound(2, contact(1), TRANSFER));
        bindings.session_gone(2);
        assert!(bindings.is_bound(1, contact(1), TRANSFER));
        bindings.session_gone(1);
        assert!(!bindings.is_bound(3, contact(1), TRANSFER));
        // A session that was not registered changes nothing.
        bindings.session_gone(99);
        // An offer on the one left binds it again.
        bindings.offer_seen(3, contact(1), TRANSFER);
        assert!(bindings.is_bound(3, contact(1), TRANSFER));
    }

    #[test]
    fn the_session_of_the_latest_offer_of_this_side_is_the_one_that_sends() {
        let mut bindings = registered(&[1, 2]);
        bindings.offered_out(1, contact(1), TRANSFER);
        assert!(bindings.is_latest(1, contact(1), TRANSFER));
        bindings.offered_out(2, contact(1), TRANSFER);
        assert!(bindings.is_latest(2, contact(1), TRANSFER));
        assert!(!bindings.is_latest(1, contact(1), TRANSFER));
        // Once that session ends, no session sends it until it is offered
        // again.
        bindings.session_gone(2);
        assert!(!bindings.is_latest(1, contact(1), TRANSFER));
        assert!(!bindings.is_latest(2, contact(1), TRANSFER));
    }
}

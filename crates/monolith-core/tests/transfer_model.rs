//! File transfers between two runtimes against a model written here
//! (`docs/adr/0013-file-transfer.md`, `docs/TEST_PLAN.md` T-FT-MODEL): two
//! peers on persistent installations over the in-memory Tor, and any
//! sequence of offering files of several sizes from either side,
//! accepting, rejecting, cancelling from either side, removing, saving,
//! crashing either side at once or at a later step of its storage under
//! every crash outcome, Tor going away and coming back, closing the
//! sessions, letting time pass, and one side blocking or deleting the
//! other. The model is what each user did and the transitions a transfer
//! may make, written here from the table of the ADR; nothing of the state
//! machine is used to say what is right.
//!
//! After every step, and as a side stops at a crash, what each side lists:
//!
//! - every transfer is one the model knows: an outgoing one that side
//!   offered, an incoming one the other side offered, with its size and
//!   name; one record each, of that side's identity and contact record;
//!   an offer reported made is listed until its user removes it or its
//!   contact ends, and a transfer that went is never listed again;
//! - from what was last seen, its status moved only along the table, in
//!   any number of steps, and an ended transfer never goes on: what was
//!   shown stays shown across every crash (the one exception is a
//!   cancelled outgoing transfer that the receiver had completed);
//! - each status has its authority: received, receiving, sending,
//!   completed or failed after an acceptance of the receiver's user,
//!   rejected after its rejection, expired after the offer timeout,
//!   cancelled by the user it names (or by the end of the contact at the
//!   side that ended it), a failure only because the offerer no longer has
//!   the file, and saved only by the receiver's user;
//! - the bytes shown never exceed the size, and a received or completed
//!   transfer shows them all; one shown completed at its sender is shown
//!   received at its receiver, unless that user removed it.
//!
//! A command answers from that truth: an acceptance is refused only when
//! the transfer is no longer offered or two are being received, a
//! cancellation only for a received file (an undecided offer is rejected),
//! a removal only for a transfer not ended. A command decides on the state
//! in memory and the list shows what is durable, so a refusal is right
//! once what refused it is shown, moments later. A received file reads
//! back exactly what was offered, once.
//!
//! At the end, with Tor back, both running, every offer still open
//! accepted and the users chatting now and then, a day passes: every
//! transfer is ended at both sides, or was never listed at the receiver
//! because its sender cancelled it first; a file is received at one side
//! exactly when it is completed at the other, and reads back whole; any
//! other end at one side is an end at the other, the two named apart only
//! when the users' decisions crossed. After a block or a deletion nothing
//! more changes but an offer expiring.
//!
//! Time is paused, so the reconnect delays pass at once.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation
)]

mod common;

use core::time::Duration;
use std::io::{Cursor, Read};
use std::sync::Arc;

use common::{fixed_endpoint, keys, run_paused};
use monolith_core::api::{
    ContactId, IdentityId, MessageStatus, TransferError, TransferReason, TransferRef,
    TransferStatus, TransferView,
};
use monolith_core::budget::Budgets;
use monolith_core::identity::{Installation, LocalIdentity};
use monolith_core::runtime::{Runtime, RuntimeOptions};
use monolith_protocol::limits::{
    FILE_OFFER_TIMEOUT, MAX_ACTIVE_TRANSFERS_PER_CONTACT, MAX_FILE_CHUNK_LEN,
    MAX_WAITING_OFFERS_PER_CONTACT,
};
use monolith_storage::dir::{CrashOutcome, MemoryDir};
use monolith_storage::vault::{KdfParams, Passphrase};
use monolith_tor::{MockNetwork, MockTorBackend};
use proptest::prelude::*;
use tokio::time::Instant;

const L: u64 = MAX_FILE_CHUNK_LEN as u64;

/// The sizes offered: empty, one byte, around one chunk, several chunks
/// with a short last one, and more than twice the window.
const SIZES: [u64; 6] = [0, 1, L - 1, L + 1, 3 * L + 5, 9 * L];

fn passphrase() -> Passphrase {
    Passphrase::new("correct horse battery staple").unwrap()
}

fn options() -> RuntimeOptions {
    RuntimeOptions {
        startup_spread: Duration::ZERO,
        ..RuntimeOptions::default()
    }
}

/// Waits, with paused time, until `condition` holds, for up to `limit` of
/// test time. Returns whether it did.
async fn within(limit: Duration, condition: impl Fn() -> bool) -> bool {
    let step = Duration::from_millis(250);
    let mut waited = Duration::ZERO;
    while waited < limit {
        if condition() {
            return true;
        }
        tokio::time::sleep(step).await;
        waited += step;
    }
    condition()
}

/// One side: its directory, its installation, its identity, its runtime.
struct Side {
    seed: u8,
    dir: MemoryDir,
    installation: Installation,
    identity: Arc<LocalIdentity>,
    runtime: Option<Runtime<MockTorBackend>>,
    /// The record of the other side.
    other: ContactId,
}

impl Side {
    /// A new installation with the identity of `seed`.
    async fn installed(seed: u8) -> (MemoryDir, Installation, Arc<LocalIdentity>) {
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let identity = installation
            .restore_identity(keys(seed, seed, 1, fixed_endpoint(seed)), None)
            .await
            .unwrap();
        (dir, installation, identity)
    }

    fn runtime(&self) -> &Runtime<MockTorBackend> {
        self.runtime.as_ref().expect("started")
    }

    fn id(&self) -> IdentityId {
        self.identity.id()
    }

    async fn start(&mut self, network: &MockNetwork) {
        self.runtime = Some(Runtime::start(
            self.installation.clone(),
            network.backend(),
            Budgets::new(),
            options(),
        ));
        let endpoint = fixed_endpoint(self.seed);
        assert!(within(Duration::from_secs(600), || network.is_published(&endpoint)).await);
    }

    /// A crash: every task ends where it is, and the installation is
    /// opened again from what the directory holds as `outcome` says.
    async fn crash(&mut self, network: &MockNetwork, outcome: CrashOutcome) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown(Duration::ZERO).await;
        }
        let dir = self.dir.restart(outcome);
        let (installation, _) = Installation::open(Box::new(dir.clone()), &passphrase()).unwrap();
        let identity = installation.identities().into_iter().next().unwrap();
        self.dir = dir;
        self.installation = installation;
        self.identity = identity;
        self.start(network).await;
    }

    fn views(&self) -> Vec<TransferView> {
        self.runtime().transfers(&self.id()).unwrap()
    }

    fn status(&self, reference: &TransferRef) -> Option<TransferStatus> {
        self.runtime()
            .transfer(reference)
            .unwrap()
            .map(|view| view.status)
    }
}

/// Saves the received file of `reference`, reading it whole.
async fn read_back(side: &Side, reference: &TransferRef) -> Result<Vec<u8>, TransferError> {
    let mut reader = side.runtime().begin_save(reference).await?;
    let mut bytes = Vec::new();
    let read = reader.read_to_end(&mut bytes);
    drop(reader);
    side.runtime().end_save(reference, read.is_ok()).await?;
    assert!(read.is_ok(), "a received file does not read: {read:?}");
    Ok(bytes)
}

fn is_outgoing_end(status: TransferStatus) -> bool {
    matches!(
        status,
        TransferStatus::Completed
            | TransferStatus::Rejected
            | TransferStatus::Cancelled
            | TransferStatus::Failed
    )
}

fn is_incoming_end(status: TransferStatus) -> bool {
    matches!(
        status,
        TransferStatus::Received
            | TransferStatus::Rejected
            | TransferStatus::Expired
            | TransferStatus::Cancelled
            | TransferStatus::Failed
    )
}

/// Whether a transfer may go from `from` to `to` in any number of steps,
/// itself included: the table of `docs/adr/0013-file-transfer.md`.
fn reaches(outgoing: bool, from: TransferStatus, to: TransferStatus) -> bool {
    if from == to {
        return true;
    }
    if outgoing {
        match from {
            TransferStatus::Waiting => matches!(
                to,
                TransferStatus::Sending
                    | TransferStatus::Completed
                    | TransferStatus::Rejected
                    | TransferStatus::Cancelled
                    | TransferStatus::Failed
            ),
            TransferStatus::Sending => matches!(
                to,
                TransferStatus::Completed
                    | TransferStatus::Rejected
                    | TransferStatus::Cancelled
                    | TransferStatus::Failed
            ),
            // A completion that crossed the cancellation.
            TransferStatus::Cancelled => to == TransferStatus::Completed,
            _ => false,
        }
    } else {
        match from {
            TransferStatus::Offered => matches!(
                to,
                TransferStatus::Receiving
                    | TransferStatus::Received
                    | TransferStatus::Rejected
                    | TransferStatus::Expired
                    | TransferStatus::Cancelled
                    | TransferStatus::Failed
            ),
            TransferStatus::Receiving => matches!(
                to,
                TransferStatus::Received | TransferStatus::Cancelled | TransferStatus::Failed
            ),
            _ => false,
        }
    }
}

/// A file offered, and what the users did with it.
struct Transfer {
    /// The side that offered it.
    from: usize,
    name: String,
    data: Vec<u8>,
    offered: Instant,
    /// Its handle at each side, once known.
    handles: [Option<TransferRef>; 2],
    /// What each side last listed of it.
    seen: [Option<TransferStatus>; 2],
    /// It was listed at that side and is no longer.
    gone: [bool; 2],
    /// The receiver's user accepted or rejected it.
    accepted: bool,
    rejected: bool,
    /// The user of each side cancelled or removed it.
    cancelled: [bool; 2],
    removed: [bool; 2],
    /// The receiver saved it, and its staged copy went.
    saved: bool,
}

impl Transfer {
    fn receiver(&self) -> usize {
        1 - self.from
    }

    fn timed_out(&self) -> bool {
        Instant::now().saturating_duration_since(self.offered) >= FILE_OFFER_TIMEOUT
    }

    /// Ended at the sender, or taken off its list.
    fn ended_at_sender(&self) -> bool {
        self.removed[self.from] || self.seen[self.from].is_some_and(is_outgoing_end)
    }

    /// Ended at the receiver, or taken off its list; or never listed there
    /// because its sender cancelled it before the receiver read the offer.
    fn ended_at_receiver(&self) -> bool {
        let receiver = self.receiver();
        self.removed[receiver]
            || self.seen[receiver].is_some_and(is_incoming_end)
            || (self.handles[receiver].is_none() && self.cancelled[self.from])
    }
}

/// How the contact of A with B ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ended {
    No,
    Blocked,
    Deleted,
}

struct World {
    network: MockNetwork,
    sides: [Side; 2],
    transfers: Vec<Transfer>,
    ended: Ended,
}

impl World {
    /// Checks what each side lists against the model, and records it.
    fn observe(&mut self) {
        for side in 0..2 {
            let views = self.sides[side].views();
            let mut listed = vec![false; self.transfers.len()];
            for view in views {
                let index = self.place(side, &view);
                assert!(!listed[index], "listed twice at {side}: {view:?}");
                listed[index] = true;
                self.check(side, index, &view);
            }
            for (index, transfer) in self.transfers.iter_mut().enumerate() {
                if listed[index] {
                    continue;
                }
                let offered_here = transfer.from == side;
                // A deleted contact's records go at once, a blocked one's
                // when the installation opens again, as its messages do.
                let deleted = side == 0 && self.ended != Ended::No;
                if transfer.seen[side].is_some()
                    || (offered_here && transfer.handles[side].is_some())
                {
                    // It went away: only by its user, or with its contact.
                    assert!(
                        transfer.removed[side] || deleted,
                        "{} went away at {side} after {:?}",
                        transfer.name,
                        transfer.seen[side]
                    );
                    transfer.gone[side] = true;
                    transfer.seen[side] = None;
                }
            }
        }
        // Completed at the sender: received at the receiver.
        for transfer in &self.transfers {
            let receiver = transfer.receiver();
            if transfer.seen[transfer.from] == Some(TransferStatus::Completed)
                && !transfer.removed[receiver]
                && !(receiver == 0 && self.ended != Ended::No)
            {
                assert_eq!(
                    transfer.seen[receiver],
                    Some(TransferStatus::Received),
                    "{} completed at its sender",
                    transfer.name
                );
            }
        }
    }

    /// The transfer of the model `view` is, at `side`.
    fn place(&mut self, side: usize, view: &TransferView) -> usize {
        if let Some(index) = self
            .transfers
            .iter()
            .position(|transfer| transfer.handles[side] == Some(view.reference))
        {
            return index;
        }
        assert!(
            view.reference.incoming,
            "an outgoing transfer at {side} that was never offered: {view:?}"
        );
        let index = self
            .transfers
            .iter()
            .position(|transfer| {
                transfer.from != side
                    && transfer.handles[side].is_none()
                    && transfer.name == view.name
            })
            .unwrap_or_else(|| panic!("an incoming transfer at {side} nobody offered: {view:?}"));
        self.transfers[index].handles[side] = Some(view.reference);
        index
    }

    fn check(&mut self, side: usize, index: usize, view: &TransferView) {
        let ended = self.ended;
        let other_side_ended = {
            let transfer = &self.transfers[index];
            let from = transfer.from;
            transfer.seen[from].is_some_and(is_outgoing_end)
                || transfer.cancelled[from]
                || transfer.removed[from]
        };
        let me = &self.sides[side];
        let transfer = &mut self.transfers[index];
        let outgoing = transfer.from == side;
        let name = &transfer.name;
        assert!(
            !transfer.gone[side],
            "{name} listed again at {side}: {view:?}"
        );
        assert_eq!(view.reference.incoming, !outgoing, "{name}: {view:?}");
        assert_eq!(view.reference.identity, me.id(), "{name}: {view:?}");
        assert_eq!(view.reference.contact, me.other, "{name}: {view:?}");
        assert_eq!(view.size, transfer.data.len() as u64, "{name}: {view:?}");
        assert_eq!(&view.name, name, "{view:?}");
        if let Some(bytes) = view.bytes {
            assert!(bytes <= view.size, "{name}: {bytes} of {view:?}");
        }
        if matches!(
            view.status,
            TransferStatus::Received | TransferStatus::Completed
        ) {
            assert_eq!(view.bytes, Some(view.size), "{name}: {view:?}");
        }
        // Saved: by the receiver's user, and only once that is durable.
        assert_eq!(view.saved, !outgoing && transfer.saved, "{name}: {view:?}");
        // Along the table, from what was last seen.
        let first = if outgoing {
            TransferStatus::Waiting
        } else {
            TransferStatus::Offered
        };
        let from = transfer.seen[side].unwrap_or(first);
        assert!(
            reaches(outgoing, from, view.status),
            "{name} at {side}: {from:?} to {:?} ({:?})",
            view.status,
            view.reason
        );
        // Its authority.
        let by_us = transfer.cancelled[side];
        let by_peer = transfer.cancelled[1 - side];
        match view.status {
            TransferStatus::Receiving
            | TransferStatus::Received
            | TransferStatus::Sending
            | TransferStatus::Completed => {
                assert!(transfer.accepted, "{name} at {side}: {view:?} not accepted");
            }
            TransferStatus::Rejected => {
                assert!(
                    transfer.rejected || (outgoing && transfer.timed_out()),
                    "{name} at {side}: {view:?} not rejected"
                );
            }
            TransferStatus::Expired => {
                assert!(transfer.timed_out(), "{name} at {side}: {view:?} early");
            }
            TransferStatus::Cancelled => {
                let by = match view.reason {
                    Some(TransferReason::ByUs) => by_us,
                    Some(TransferReason::ByPeer) => by_peer,
                    Some(TransferReason::ContactEnded) => side == 0 && ended != Ended::No,
                    _ => false,
                };
                assert!(by, "{name} at {side}: {view:?} by nobody");
            }
            TransferStatus::Failed => {
                // An honest pair whose disks fail only by crashing: the one
                // failure is a receiver whose offerer ended the transfer
                // without being able to tell it.
                assert!(
                    !outgoing
                        && view.reason == Some(TransferReason::OffererGone)
                        && transfer.accepted
                        && (other_side_ended || ended != Ended::No),
                    "{name} at {side}: {view:?} failed"
                );
            }
            TransferStatus::Waiting | TransferStatus::Offered => {}
        }
        transfer.seen[side] = Some(view.status);
    }

    fn pick(&self, pick: u8) -> Option<usize> {
        if self.transfers.is_empty() {
            None
        } else {
            Some(usize::from(pick) % self.transfers.len())
        }
    }

    /// The status of `handle` at `side` once `justified` holds, or after
    /// ten seconds: a command decides on the state in memory and the list
    /// shows what is durable, which follows within moments. A refusal is
    /// right only if what refused it is shown.
    async fn settled(
        &self,
        side: usize,
        handle: &TransferRef,
        justified: impl Fn(Option<TransferStatus>) -> bool,
    ) -> Option<TransferStatus> {
        let at = &self.sides[side];
        within(Duration::from_secs(10), || justified(at.status(handle))).await;
        at.status(handle)
    }

    /// Accepts the offer of `index` at its receiver, if it was seen there.
    /// Returns whether the acceptance was taken.
    async fn accept(&mut self, index: usize) -> bool {
        let receiver = self.transfers[index].receiver();
        let Some(handle) = self.transfers[index].handles[receiver] else {
            return false;
        };
        // Accepted and not seen ended: at most that many are being
        // received.
        let active = self
            .transfers
            .iter()
            .filter(|transfer| {
                transfer.receiver() == receiver
                    && transfer.accepted
                    && !transfer.gone[receiver]
                    && !transfer.seen[receiver].is_some_and(is_incoming_end)
            })
            .count();
        let result = self.sides[receiver]
            .runtime()
            .accept_transfer(&handle)
            .await;
        let name = &self.transfers[index].name;
        match result {
            Ok(()) => {
                self.transfers[index].accepted = true;
                true
            }
            Err(TransferError::UnknownTransfer) => {
                let offered = |now| now == Some(TransferStatus::Offered);
                let now = self.settled(receiver, &handle, |now| !offered(now)).await;
                assert!(!offered(now), "{name}: refused");
                false
            }
            Err(TransferError::TooManyActive) => {
                assert!(
                    active >= MAX_ACTIVE_TRANSFERS_PER_CONTACT,
                    "{name}: refused with {active} being received"
                );
                false
            }
            Err(TransferError::NotAContact) => {
                assert!(
                    receiver == 0 && self.ended != Ended::No,
                    "{name}: not a contact"
                );
                false
            }
            Err(error) => panic!("{name}: acceptance failed: {error:?}"),
        }
    }

    async fn step(&mut self, number: usize, step: Step) {
        match step {
            Step::Offer { from_a, size } => {
                let from = usize::from(!from_a);
                let name = format!("f{number}.bin");
                let len = SIZES[usize::from(size) % SIZES.len()];
                let data: Vec<u8> = (0..len)
                    .map(|i| (i.wrapping_mul(31).wrapping_add(number as u64) % 251) as u8)
                    .collect();
                let waiting = self
                    .transfers
                    .iter()
                    .filter(|transfer| {
                        transfer.from == from
                            && matches!(
                                transfer.seen[from],
                                None | Some(TransferStatus::Waiting | TransferStatus::Cancelled)
                            )
                            && !transfer.gone[from]
                    })
                    .count();
                let side = &self.sides[from];
                let result = side
                    .runtime()
                    .offer_file(
                        &side.id(),
                        &side.other,
                        &name,
                        len,
                        Box::new(Cursor::new(data.clone())),
                    )
                    .await;
                match result {
                    Ok(handle) => self.transfers.push(Transfer {
                        from,
                        name,
                        data,
                        offered: Instant::now(),
                        handles: if from == 0 {
                            [Some(handle), None]
                        } else {
                            [None, Some(handle)]
                        },
                        seen: [None, None],
                        gone: [false, false],
                        accepted: false,
                        rejected: false,
                        cancelled: [false, false],
                        removed: [false, false],
                        saved: false,
                    }),
                    Err(TransferError::TooManyWaiting) => {
                        assert!(
                            waiting >= MAX_WAITING_OFFERS_PER_CONTACT,
                            "{name}: refused with {waiting} waiting"
                        );
                    }
                    Err(TransferError::NotAContact) => {
                        assert!(
                            from == 0 && self.ended != Ended::No,
                            "{name}: not a contact"
                        );
                    }
                    Err(error) => panic!("{name}: offer failed: {error:?}"),
                }
            }
            Step::Accept(pick) => {
                if let Some(index) = self.pick(pick) {
                    self.accept(index).await;
                }
            }
            Step::Reject(pick) => {
                let Some(index) = self.pick(pick) else { return };
                let receiver = self.transfers[index].receiver();
                let Some(handle) = self.transfers[index].handles[receiver] else {
                    return;
                };
                let result = self.sides[receiver]
                    .runtime()
                    .reject_transfer(&handle)
                    .await;
                let name = &self.transfers[index].name;
                match result {
                    Ok(()) => self.transfers[index].rejected = true,
                    Err(TransferError::UnknownTransfer) => {
                        let offered = |now| now == Some(TransferStatus::Offered);
                        let now = self.settled(receiver, &handle, |now| !offered(now)).await;
                        assert!(!offered(now), "{name}: refused");
                    }
                    Err(TransferError::NotAContact) => {
                        assert!(receiver == 0 && self.ended != Ended::No, "{name}");
                    }
                    Err(error) => panic!("{name}: rejection failed: {error:?}"),
                }
            }
            Step::Cancel { at_a, pick } => {
                let side = usize::from(!at_a);
                let Some(index) = self.pick(pick) else { return };
                let Some(handle) = self.transfers[index].handles[side] else {
                    return;
                };
                let outgoing = self.transfers[index].from == side;
                let result = self.sides[side].runtime().cancel_transfer(&handle).await;
                let name = &self.transfers[index].name;
                match result {
                    Ok(()) => {
                        // Cancelling an undecided offer rejects it;
                        // cancelling an ended transfer changes nothing.
                        self.transfers[index].cancelled[side] = true;
                        if !outgoing {
                            self.transfers[index].rejected = true;
                        }
                    }
                    Err(TransferError::UnknownTransfer) => {
                        // Only a received file cannot be cancelled, or one
                        // no longer listed.
                        let right = |now: Option<TransferStatus>| {
                            now.is_none() || (!outgoing && now == Some(TransferStatus::Received))
                        };
                        let now = self.settled(side, &handle, right).await;
                        assert!(right(now), "{name}: cancellation refused at {now:?}");
                    }
                    Err(TransferError::NotAContact) => {
                        assert!(side == 0 && self.ended != Ended::No, "{name}");
                    }
                    Err(error) => panic!("{name}: cancellation failed: {error:?}"),
                }
            }
            Step::Remove { at_a, pick } => {
                let side = usize::from(!at_a);
                let Some(index) = self.pick(pick) else { return };
                let Some(handle) = self.transfers[index].handles[side] else {
                    return;
                };
                let outgoing = self.transfers[index].from == side;
                let result = self.sides[side].runtime().remove_transfer(&handle).await;
                let now = self.sides[side].status(&handle);
                let name = &self.transfers[index].name;
                match result {
                    Ok(()) => {
                        assert_eq!(now, None, "{name}: removed and still listed");
                        self.transfers[index].removed[side] = true;
                    }
                    Err(TransferError::UnknownTransfer) => {
                        let ended = now.is_some_and(if outgoing {
                            is_outgoing_end
                        } else {
                            is_incoming_end
                        });
                        assert!(!ended, "{name}: removal refused at {now:?}");
                    }
                    Err(TransferError::NotAContact) => {
                        assert!(side == 0 && self.ended != Ended::No, "{name}");
                    }
                    Err(error) => panic!("{name}: removal failed: {error:?}"),
                }
            }
            Step::Save(pick) => {
                let Some(index) = self.pick(pick) else { return };
                let receiver = self.transfers[index].receiver();
                let Some(handle) = self.transfers[index].handles[receiver] else {
                    return;
                };
                let before = self.sides[receiver].status(&handle);
                let result = read_back(&self.sides[receiver], &handle).await;
                let transfer = &mut self.transfers[index];
                match result {
                    Ok(bytes) => {
                        assert!(bytes == transfer.data, "{}: other bytes", transfer.name);
                        transfer.saved = true;
                    }
                    // A file saved is saved once: its staged copy went.
                    Err(error) => assert!(
                        before != Some(TransferStatus::Received) || transfer.saved,
                        "{}: a received file did not save: {error:?}",
                        transfer.name
                    ),
                }
            }
            Step::Crash { a, outcome } => {
                let side = usize::from(!a);
                let outcome = CrashOutcome::ALL[usize::from(outcome) % CrashOutcome::ALL.len()];
                self.sides[side].crash(&self.network, outcome).await;
            }
            Step::CrashLater { a, after, outcome } => {
                let side = usize::from(!a);
                let outcome = CrashOutcome::ALL[usize::from(outcome) % CrashOutcome::ALL.len()];
                let dir = self.sides[side].dir.clone();
                dir.crash_at(dir.steps() + 1 + u64::from(after));
                // The side runs until the step comes, or for a minute; what
                // it lists as it stops is what it showed.
                within(Duration::from_secs(60), || dir.crashed()).await;
                self.observe();
                self.sides[side].crash(&self.network, outcome).await;
            }
            Step::Tor(up) => self.network.set_socks_available(up),
            Step::Close(a) => {
                let side = &self.sides[usize::from(!a)];
                let _ = side.runtime().close_sessions(&side.id(), &side.other);
            }
            Step::Wait(seconds) => {
                tokio::time::sleep(Duration::from_secs(u64::from(seconds))).await;
            }
            Step::End { delete } => {
                if self.ended == Ended::No {
                    let side = &self.sides[0];
                    if delete {
                        side.runtime()
                            .delete_contact(&side.id(), &side.other)
                            .await
                            .unwrap();
                        self.ended = Ended::Deleted;
                    } else {
                        side.runtime()
                            .block_contact(&side.id(), &side.other)
                            .await
                            .unwrap();
                        self.ended = Ended::Blocked;
                    }
                }
            }
        }
    }

    /// Every transfer the model knows, as each side lists it now.
    fn statuses(&self) -> Vec<[Option<TransferStatus>; 2]> {
        self.transfers
            .iter()
            .map(|transfer| {
                [0, 1].map(|side| {
                    transfer.handles[side].and_then(|handle| self.sides[side].status(&handle))
                })
            })
            .collect()
    }

    /// The end: Tor back, both running, every open offer accepted as room
    /// allows, a day of time.
    async fn finish(&mut self) {
        self.network.set_socks_available(true);
        if self.ended != Ended::No {
            // Nothing more moves: once what was under way has settled, a
            // day passes and nothing changes.
            tokio::time::sleep(Duration::from_secs(60 * 60)).await;
            self.observe();
            let before = self.statuses();
            tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
            self.observe();
            // An offer the other side did not decide may expire.
            for ((transfer, before), after) in
                self.transfers.iter().zip(before).zip(self.statuses())
            {
                for side in 0..2 {
                    assert!(
                        before[side] == after[side]
                            || (before[side] == Some(TransferStatus::Offered)
                                && after[side] == Some(TransferStatus::Expired)
                                && transfer.timed_out()),
                        "{} moved at {side} after the end: {:?} to {:?}",
                        transfer.name,
                        before[side],
                        after[side]
                    );
                }
            }
            return;
        }
        for round in 0..(24 * 6) {
            for index in 0..self.transfers.len() {
                let receiver = self.transfers[index].receiver();
                if self.transfers[index].seen[receiver] == Some(TransferStatus::Offered) {
                    self.accept(index).await;
                }
            }
            // The users chat now and then: a rejection, an expiry or a
            // final answer makes no dial, and the offerer learns it on the
            // next session (ADR 0013, "Scheduling").
            if round % 6 == 0 {
                for side in &self.sides {
                    side.runtime()
                        .send_message(&side.id(), &side.other, "hello")
                        .await
                        .unwrap();
                }
            }
            tokio::time::sleep(Duration::from_secs(10 * 60)).await;
            self.observe();
            let settled = self
                .transfers
                .iter()
                .all(|transfer| transfer.ended_at_sender() && transfer.ended_at_receiver());
            if settled {
                break;
            }
        }
        for index in 0..self.transfers.len() {
            let transfer = &self.transfers[index];
            let receiver = transfer.receiver();
            let name = transfer.name.clone();
            let sent = transfer.seen[transfer.from];
            let got = transfer.seen[receiver];
            assert!(
                transfer.removed[transfer.from] || sent.is_some_and(is_outgoing_end),
                "{name} not ended at its sender: {sent:?} / {got:?}"
            );
            assert!(
                transfer.ended_at_receiver(),
                "{name} not ended at its receiver: {sent:?} / {got:?}"
            );
            if transfer.removed[transfer.from]
                || transfer.removed[receiver]
                || transfer.handles[receiver].is_none()
            {
                continue;
            }
            // An end at one side is matched at the other.
            match got {
                Some(TransferStatus::Received) => {
                    assert_eq!(sent, Some(TransferStatus::Completed), "{name}");
                    if !transfer.saved {
                        let handle = transfer.handles[receiver].unwrap();
                        let data = transfer.data.clone();
                        let bytes = read_back(&self.sides[receiver], &handle).await.unwrap();
                        assert!(bytes == data, "{name}: other bytes");
                    }
                }
                Some(TransferStatus::Rejected | TransferStatus::Expired) => {
                    // Unless the sender cancelled before it heard.
                    assert!(
                        sent == Some(TransferStatus::Rejected)
                            || (sent == Some(TransferStatus::Cancelled)
                                && transfer.cancelled[transfer.from]),
                        "{name}: {sent:?} / {got:?}"
                    );
                }
                Some(TransferStatus::Cancelled | TransferStatus::Failed) => {
                    assert!(
                        matches!(
                            sent,
                            Some(TransferStatus::Cancelled | TransferStatus::Failed)
                        ),
                        "{name}: {sent:?} / {got:?}"
                    );
                }
                _ => panic!("{name}: {sent:?} / {got:?}"),
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Step {
    /// A side offers a file of one of the sizes.
    Offer { from_a: bool, size: u8 },
    /// The receiver of a transfer accepts it.
    Accept(u8),
    /// The receiver of a transfer rejects it.
    Reject(u8),
    /// A side cancels a transfer.
    Cancel { at_a: bool, pick: u8 },
    /// A side removes a transfer from its list.
    Remove { at_a: bool, pick: u8 },
    /// The receiver of a transfer saves it.
    Save(u8),
    /// A side crashes now and starts again.
    Crash { a: bool, outcome: u8 },
    /// A side crashes at a later step of its storage, when that step comes
    /// or after a minute, and starts again.
    CrashLater { a: bool, after: u8, outcome: u8 },
    /// Tor's SOCKS endpoint goes away, or comes back.
    Tor(bool),
    /// A side closes its sessions.
    Close(bool),
    /// Time passes, in seconds.
    Wait(u16),
    /// A blocks or deletes B.
    End { delete: bool },
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        5 => (any::<bool>(), any::<u8>()).prop_map(|(from_a, size)| Step::Offer { from_a, size }),
        5 => any::<u8>().prop_map(Step::Accept),
        1 => any::<u8>().prop_map(Step::Reject),
        2 => (any::<bool>(), any::<u8>()).prop_map(|(at_a, pick)| Step::Cancel { at_a, pick }),
        1 => (any::<bool>(), any::<u8>()).prop_map(|(at_a, pick)| Step::Remove { at_a, pick }),
        2 => any::<u8>().prop_map(Step::Save),
        2 => (any::<bool>(), any::<u8>()).prop_map(|(a, outcome)| Step::Crash { a, outcome }),
        4 => (any::<bool>(), 0..48_u8, any::<u8>())
            .prop_map(|(a, after, outcome)| Step::CrashLater { a, after, outcome }),
        1 => any::<bool>().prop_map(Step::Tor),
        1 => any::<bool>().prop_map(Step::Close),
        3 => (0..600_u16).prop_map(Step::Wait),
        1 => any::<bool>().prop_map(|delete| Step::End { delete }),
    ]
}

/// Two contacts who accepted each other, with no session left.
async fn world() -> World {
    let network = MockNetwork::new();
    let (dir_a, installation_a, identity_a) = Side::installed(1).await;
    let (dir_b, installation_b, identity_b) = Side::installed(2).await;
    identity_a.import(&identity_b.card()).await.unwrap();
    identity_b.import(&identity_a.card()).await.unwrap();
    let at_a = identity_a.contact_id(identity_b.identity()).unwrap();
    let at_b = identity_b.contact_id(identity_a.identity()).unwrap();
    let mut sides = [
        Side {
            seed: 1,
            dir: dir_a,
            installation: installation_a,
            identity: identity_a,
            runtime: None,
            other: at_a,
        },
        Side {
            seed: 2,
            dir: dir_b,
            installation: installation_b,
            identity: identity_b,
            runtime: None,
            other: at_b,
        },
    ];
    for side in &mut sides {
        side.start(&network).await;
    }
    // A message each way makes each an accepted contact of the other.
    for side in &sides {
        side.runtime()
            .send_message(&side.id(), &side.other, "hello")
            .await
            .unwrap();
    }
    let delivered = within(Duration::from_secs(600), || {
        sides.iter().all(|side| {
            side.runtime()
                .messages(&side.id(), &side.other, None, 10)
                .unwrap()
                .iter()
                .filter(|message| message.status == MessageStatus::Delivered)
                .count()
                == 1
        })
    })
    .await;
    assert!(delivered);
    World {
        network,
        sides,
        transfers: Vec::new(),
        ended: Ended::No,
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 32,
        max_shrink_iters: 128,
        ..ProptestConfig::default()
    })]

    #[test]
    fn transfers_between_two_runtimes_follow_the_model(steps in prop::collection::vec(step(), 1..32)) {
        run_paused(async {
            let mut world = world().await;
            for (number, step) in steps.iter().enumerate() {
                world.step(number, *step).await;
                world.observe();
            }
            world.finish().await;
            for side in &mut world.sides {
                if let Some(runtime) = side.runtime.take() {
                    runtime.shutdown(Duration::ZERO).await;
                }
            }
        });
    }
}

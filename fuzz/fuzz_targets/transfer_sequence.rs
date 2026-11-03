//! The transfer state machine (`monolith_core::fuzzing::Transfers`) under
//! any sequence of operations on the transfers of two local identities
//! with two contact records each, in both directions: offers of the user
//! and of the peer, the user's acceptances, rejections, cancellations and
//! removals, the peer's acceptances, completions with the right digest or
//! another, rejections and cancellations, staged bytes, verified
//! completions, failures, the time going on by hours or days, contacts
//! blocked or deleted, identities removed, and restarts through the
//! encoded store. The model is written here from the table of
//! `docs/adr/0013-file-transfer.md`: what was done to each transfer and
//! by whom.
//!
//! After every operation, for each identity: every transfer listed is of
//! that identity; its status moved only along the table, and each status
//! has its authority (received after an acceptance and a verified
//! completion, completed after a completion with the staged digest,
//! rejected, cancelled or failed only by what can reject, cancel or fail
//! it, expired only after the offer timeout); the bytes shown never pass
//! the size, and a received or completed transfer shows them all; offers
//! waiting for a decision, and transfers being received, stay within the
//! limits of a contact; a transfer the user removed is not listed again
//! while its record lasts; and a restart through the encoded store lists
//! the same transfers with the same statuses.
//!
//! Every operation is four bytes: the operation (modulo 17), the transfer
//! (`Named::from_byte`), and two arguments.

#![no_main]

use std::collections::BTreeMap;

use libfuzzer_sys::fuzz_target;
use monolith_core::api::{TransferReason, TransferStatus, TransferView};
use monolith_core::fuzzing::{Named, Offered, Transfers};
use monolith_protocol::limits::{
    ENDED_TRANSFER_RETENTION, FILE_OFFER_TIMEOUT, MAX_ACTIVE_TRANSFERS_PER_CONTACT,
    MAX_PENDING_FILE_OFFERS_PER_CONTACT, MAX_TRANSFER_SIZE, MAX_WAITING_OFFERS_PER_CONTACT,
    TRANSFER_STALL_LIMIT,
};

const START: u64 = 1_800_000_000;

fn reaches(incoming: bool, from: TransferStatus, to: TransferStatus) -> bool {
    use TransferStatus as S;
    if from == to {
        return true;
    }
    if incoming {
        match from {
            S::Offered => matches!(
                to,
                S::Receiving | S::Received | S::Rejected | S::Expired | S::Cancelled | S::Failed
            ),
            S::Receiving => matches!(to, S::Received | S::Cancelled | S::Failed),
            // A received file that no longer reads fails.
            S::Received => to == S::Failed,
            _ => false,
        }
    } else {
        match from {
            S::Waiting => matches!(
                to,
                S::Sending | S::Completed | S::Rejected | S::Cancelled | S::Failed
            ),
            S::Sending => matches!(to, S::Completed | S::Rejected | S::Cancelled | S::Failed),
            S::Cancelled => to == S::Completed,
            _ => false,
        }
    }
}

/// What was done to one transfer since its record was made.
#[derive(Clone, Debug, Default)]
struct Done {
    seen: Option<TransferStatus>,
    size: u64,
    made: u64,
    accepted: bool,
    rejected: bool,
    cancelled: bool,
    removed: bool,
    peer_accepted: bool,
    peer_completed: bool,
    peer_wrong_digest: bool,
    peer_rejected: bool,
    peer_aborted: bool,
    verified: bool,
    failed: bool,
    ended_contact: bool,
}

struct Run {
    transfers: Transfers,
    done: BTreeMap<Named, Done>,
    now: u64,
}

impl Run {
    fn check(&mut self) {
        let now = self.now;
        let mut listed_now: Vec<Named> = Vec::new();
        for identity in 0..2_u8 {
            let views = self.transfers.views(identity, now);
            let mut pending = [0_usize; 2];
            let mut active = [0_usize; 2];
            let mut waiting = [0_usize; 2];
            for view in &views {
                let named = Named::of(view).expect("a transfer these numbers name");
                assert_eq!(named.identity, identity, "{view:?} listed for another identity");
                assert!(!listed_now.contains(&named), "listed twice: {named:?}");
                listed_now.push(named);
                let contact = usize::from(named.contact);
                match view.status {
                    TransferStatus::Offered => pending[contact] += 1,
                    TransferStatus::Receiving => active[contact] += 1,
                    TransferStatus::Waiting => waiting[contact] += 1,
                    _ => {}
                }
                self.check_view(named, view);
            }
            for contact in 0..2 {
                assert!(pending[contact] <= MAX_PENDING_FILE_OFFERS_PER_CONTACT);
                assert!(active[contact] <= MAX_ACTIVE_TRANSFERS_PER_CONTACT);
                assert!(waiting[contact] <= MAX_WAITING_OFFERS_PER_CONTACT);
            }
        }
        for (named, done) in &mut self.done {
            if !listed_now.contains(named) {
                done.seen = None;
            }
        }
    }

    fn check_view(&mut self, named: Named, view: &TransferView) {
        let now = self.now;
        let done = self.done.get_mut(&named).expect("a transfer the model knows");
        assert!(!done.removed, "{named:?} listed again after its removal");
        assert_eq!(view.size, done.size, "{named:?}");
        if let Some(bytes) = view.bytes {
            assert!(bytes <= view.size, "{named:?}: {bytes} of {}", view.size);
        }
        if matches!(view.status, TransferStatus::Received | TransferStatus::Completed) {
            assert_eq!(view.bytes, Some(view.size), "{named:?}");
        }
        let first = if named.incoming {
            TransferStatus::Offered
        } else {
            TransferStatus::Waiting
        };
        let from = done.seen.unwrap_or(first);
        assert!(
            reaches(named.incoming, from, view.status),
            "{named:?}: {from:?} to {:?}",
            view.status
        );
        // Progress may or may not move the stall deadline: the model counts
        // from the record's making, which no deadline comes before.
        let stalled = now.saturating_sub(done.made) >= TRANSFER_STALL_LIMIT.as_secs();
        let authority = match (named.incoming, view.status) {
            (true, TransferStatus::Receiving) => done.accepted,
            (true, TransferStatus::Received) => done.accepted && done.verified,
            (true, TransferStatus::Rejected) => done.rejected,
            (true, TransferStatus::Expired) => {
                now.saturating_sub(done.made) >= FILE_OFFER_TIMEOUT.as_secs()
            }
            (true, TransferStatus::Cancelled) => match view.reason {
                Some(TransferReason::ByUs) => done.cancelled,
                Some(TransferReason::ByPeer) => done.peer_aborted,
                Some(TransferReason::ContactEnded) => done.ended_contact,
                _ => false,
            },
            (true, TransferStatus::Failed) => done.failed || (done.accepted && stalled),
            (false, TransferStatus::Sending) => done.peer_accepted,
            (false, TransferStatus::Completed) => done.peer_completed,
            (false, TransferStatus::Rejected) => done.peer_rejected,
            (false, TransferStatus::Cancelled) => match view.reason {
                Some(TransferReason::ByUs) => done.cancelled,
                Some(TransferReason::ByPeer) => done.peer_aborted,
                Some(TransferReason::ContactEnded) => done.ended_contact,
                _ => false,
            },
            (false, TransferStatus::Failed) => done.failed || done.peer_wrong_digest || stalled,
            (true, TransferStatus::Offered) | (false, TransferStatus::Waiting) => true,
            _ => false,
        };
        assert!(authority, "{named:?} {:?} {:?} with {done:?}", view.status, view.reason);
        done.seen = Some(view.status);
    }

    /// A record that went: its model goes too, so that the same numbers
    /// may name a new transfer.
    fn forget_gone(&mut self) {
        let now = self.now;
        let held: Vec<Named> = self
            .done
            .keys()
            .copied()
            .filter(|named| {
                // The record still exists if the state answers for it: a
                // removed incoming record is kept for its retention.
                let listed = self.transfers.status(*named, now).is_some();
                listed || !self.dropped(*named)
            })
            .collect();
        self.done.retain(|named, _| held.contains(named));
    }

    /// Whether the record of `named` may have gone by now.
    fn dropped(&self, named: Named) -> bool {
        let Some(done) = self.done.get(&named) else {
            return true;
        };
        if !named.incoming {
            return done.removed || done.ended_contact;
        }
        // An incoming record goes at the end of its retention, or with its
        // contact; the model does not know when it ended, so it lets it go
        // only once the retention has passed since it was made.
        self.now.saturating_sub(done.made) >= ENDED_TRANSFER_RETENTION.as_secs()
            || done.ended_contact
    }
}

fuzz_target!(|data: &[u8]| {
    let mut run = Run {
        transfers: Transfers::new(),
        done: BTreeMap::new(),
        now: START,
    };
    for op in data.chunks_exact(4) {
        let named = Named::from_byte(op[1]);
        let (a, b) = (op[2], op[3]);
        let now = run.now;
        let size = if b == 0xFF {
            MAX_TRANSFER_SIZE + 1
        } else {
            u64::from(a) * 1000
        };
        match op[0] % 17 {
            0 => {
                let named = Named {
                    incoming: false,
                    ..named
                };
                let fresh = !run.done.contains_key(&named);
                if run.transfers.offer(named, size, now) {
                    assert!(fresh || run.dropped(named), "{named:?} recorded twice");
                    run.done.insert(
                        named,
                        Done {
                            size,
                            made: now,
                            ..Done::default()
                        },
                    );
                }
            }
            1 => {
                let named = Named {
                    incoming: true,
                    ..named
                };
                match run.transfers.offered(named, size, now) {
                    Offered::New => {
                        assert!(size <= MAX_TRANSFER_SIZE, "an offer above the limit recorded");
                        run.done.insert(
                            named,
                            Done {
                                size,
                                made: now,
                                progress: now,
                                ..Done::default()
                            },
                        );
                    }
                    Offered::Complete => {
                        if let Some(done) = run.done.get(&named) {
                            assert!(done.verified, "{named:?} answered complete");
                        }
                    }
                    _ => {}
                }
            }
            2 => {
                if run.transfers.accept(named, now) {
                    let done = run.done.get_mut(&named).expect("an offer the model knows");
                    done.accepted = true;
                }
            }
            3 => {
                if run.transfers.reject(named, now) {
                    if let Some(done) = run.done.get_mut(&named) {
                        done.rejected = true;
                    }
                }
            }
            4 => {
                if run.transfers.cancel(named, now) {
                    if let Some(done) = run.done.get_mut(&named) {
                        done.cancelled = true;
                        // Cancelling an undecided offer rejects it.
                        done.rejected = true;
                    }
                }
            }
            5 => {
                if run.transfers.remove(named, now) {
                    if let Some(done) = run.done.get_mut(&named) {
                        done.removed = true;
                    }
                }
            }
            6 => {
                run.transfers
                    .accepted_by_peer(named, u64::from(a) * 1000, now);
                if let Some(done) = run.done.get_mut(&named) {
                    done.peer_accepted = true;
                }
            }
            7 => {
                let right = b & 1 == 0;
                let matched = run.transfers.completed_by_peer(named, right, now);
                if let Some(done) = run.done.get_mut(&named) {
                    if matched {
                        assert!(right, "a completion with another digest matched");
                        done.peer_completed = true;
                    } else if !right {
                        done.peer_wrong_digest = true;
                    }
                }
            }
            8 => {
                run.transfers.rejected_by_peer(named, now);
                if let Some(done) = run.done.get_mut(&named) {
                    done.peer_rejected = true;
                }
            }
            9 => {
                run.transfers.aborted_by_peer(named, now);
                if let Some(done) = run.done.get_mut(&named) {
                    done.peer_aborted = true;
                }
            }
            10 => {
                run.transfers.staged(named, u64::from(a) * 1000, now);
            }
            11 => {
                run.transfers.complete(named, now);
                if let Some(done) = run.done.get_mut(&named) {
                    done.verified = true;
                }
            }
            12 => {
                run.transfers.fail(named, now);
                if let Some(done) = run.done.get_mut(&named) {
                    done.failed = true;
                }
            }
            13 => {
                let unit = if b & 1 == 0 { 3600 } else { 24 * 3600 };
                run.now = run.now.saturating_add(u64::from(a) * unit);
                run.transfers.tick(run.now);
            }
            14 => {
                let delete = b & 1 == 1;
                run.transfers
                    .contact_ended(named.identity, named.contact, delete, now);
                for (other, done) in &mut run.done {
                    if other.identity == named.identity && other.contact == named.contact {
                        done.ended_contact = true;
                    }
                }
            }
            15 => {
                let before: Vec<(u8, Vec<(TransferStatus, Option<TransferReason>)>)> = (0..2)
                    .map(|identity| {
                        (
                            identity,
                            run.transfers
                                .views(identity, now)
                                .iter()
                                .map(|view| (view.status, view.reason))
                                .collect(),
                        )
                    })
                    .collect();
                assert!(run.transfers.restart(), "the store does not encode or decode");
                for (identity, statuses) in before {
                    let after: Vec<(TransferStatus, Option<TransferReason>)> = run
                        .transfers
                        .views(identity, now)
                        .iter()
                        .map(|view| (view.status, view.reason))
                        .collect();
                    assert_eq!(after, statuses, "a restart changed what is listed");
                }
            }
            _ => {
                run.transfers.remove_identity(named.identity);
                for (other, done) in &mut run.done {
                    if other.identity == named.identity {
                        done.ended_contact = true;
                        done.removed = true;
                    }
                }
            }
        }
        run.check();
        run.forget_gone();
    }
    // The blobs the records name are those of transfers with staged data.
    assert!(run.transfers.blobs() <= run.transfers.records());
});

//! Any sequence of transitions, on a few transfers of two contacts of two
//! identities, against the rules the state keeps: every record valid and
//! read back as written; the limits never passed; a state reached only
//! along an edge of the table of the module; a record that is not ended
//! never removed but by the end of its contact or identity, and an
//! ended one only by its retention, the user, or room made for another;
//! a blob reported free exactly when no record names it any more, and
//! never one a record still names.

use std::collections::BTreeMap;

use super::*;

#[derive(Clone, Debug)]
enum Op {
    AddOutgoing { transfer: u8, size: u64 },
    Offered { transfer: u8, size: u64 },
    Accept { transfer: u8 },
    Reject { transfer: u8 },
    Cancel { direction: bool, transfer: u8 },
    AcceptedByPeer { transfer: u8, offset: u64 },
    CompletedByPeer { transfer: u8, good: bool },
    RejectedByPeer { transfer: u8 },
    AbortedByPeer { direction: bool, transfer: u8 },
    OfferRead { transfer: u8 },
    MarkOffered { transfer: u8 },
    Staged { transfer: u8, bytes: u64 },
    Complete { transfer: u8 },
    Fail { direction: bool, transfer: u8 },
    Told { direction: bool, transfer: u8 },
    Hold { transfer: u8 },
    Remove { direction: bool, transfer: u8 },
    Save { transfer: u8, saved: bool },
    SessionCounted { offered: Vec<u8> },
    Tick { advance: u64 },
    ContactEnded { delete: bool },
    Restart,
}

/// The key of `transfer`: its low bit picks the contact, the next one the
/// identity, so that the few transfers spread over four contact records.
fn which(direction: bool, transfer: u8) -> Key {
    Key {
        identity: identity(1 + ((transfer >> 1) & 1)),
        contact: contact(10 + (transfer & 1)),
        direction: if direction {
            Direction::Outgoing
        } else {
            Direction::Incoming
        },
        transfer: [transfer; 16],
    }
}

fn op() -> impl proptest::strategy::Strategy<Value = Op> {
    use proptest::prelude::*;
    // Zero included: an identifier no record may hold.
    let transfer = || 0_u8..12;
    let size = || prop_oneof![Just(0_u64), 1_u64..=3 * L];
    prop_oneof![
        (transfer(), size()).prop_map(|(transfer, size)| Op::AddOutgoing { transfer, size }),
        (transfer(), size()).prop_map(|(transfer, size)| Op::Offered { transfer, size }),
        transfer().prop_map(|transfer| Op::Accept { transfer }),
        transfer().prop_map(|transfer| Op::Reject { transfer }),
        (any::<bool>(), transfer()).prop_map(|(direction, transfer)| Op::Cancel {
            direction,
            transfer
        }),
        (transfer(), 0_u64..4).prop_map(|(transfer, chunks)| Op::AcceptedByPeer {
            transfer,
            offset: chunks * L
        }),
        (transfer(), any::<bool>())
            .prop_map(|(transfer, good)| Op::CompletedByPeer { transfer, good }),
        transfer().prop_map(|transfer| Op::RejectedByPeer { transfer }),
        (any::<bool>(), transfer()).prop_map(|(direction, transfer)| Op::AbortedByPeer {
            direction,
            transfer
        }),
        transfer().prop_map(|transfer| Op::OfferRead { transfer }),
        transfer().prop_map(|transfer| Op::MarkOffered { transfer }),
        (transfer(), 0_u64..4).prop_map(|(transfer, chunks)| Op::Staged {
            transfer,
            bytes: chunks * L
        }),
        transfer().prop_map(|transfer| Op::Complete { transfer }),
        (any::<bool>(), transfer()).prop_map(|(direction, transfer)| Op::Fail {
            direction,
            transfer
        }),
        (any::<bool>(), transfer()).prop_map(|(direction, transfer)| Op::Told {
            direction,
            transfer
        }),
        transfer().prop_map(|transfer| Op::Hold { transfer }),
        (any::<bool>(), transfer()).prop_map(|(direction, transfer)| Op::Remove {
            direction,
            transfer
        }),
        (transfer(), any::<bool>()).prop_map(|(transfer, saved)| Op::Save { transfer, saved }),
        proptest::collection::vec(transfer(), 0..4)
            .prop_map(|offered| Op::SessionCounted { offered }),
        prop_oneof![
            Just(0_u64),
            1_u64..10_000,
            Just(HOLD),
            Just(STALL),
            Just(RETENTION),
            Just(OFFER_TIMEOUT)
        ]
        .prop_map(|advance| Op::Tick { advance }),
        any::<bool>().prop_map(|delete| Op::ContactEnded { delete }),
        Just(Op::Restart),
    ]
}

/// Whether `to` may follow `from` for one record: the table of the module,
/// plus the changes of a flag or a deadline, which keep the state.
fn edge(direction: Direction, from: State, to: State) -> bool {
    use State::{
        Accepted, Cancelled, Complete, Delivered, Expired, Failed, Offered, Rejected, Sending,
        Waiting,
    };
    if from == to {
        return true;
    }
    match direction {
        Direction::Outgoing => matches!(
            (from, to),
            (Waiting, Sending | Delivered | Failed | Rejected | Cancelled)
                | (Sending, Delivered | Failed | Rejected | Cancelled)
                | (Cancelled, Delivered)
        ),
        Direction::Incoming => matches!(
            (from, to),
            (Offered, Accepted | Rejected | Expired | Cancelled)
                | (Accepted, Complete | Failed | Cancelled)
                | (Complete, Failed)
        ),
    }
}

fn ended(record: &TransferRecord) -> bool {
    match record.direction {
        Direction::Outgoing => match record.state {
            State::Delivered | State::Rejected | State::Failed => true,
            State::Cancelled => !record.flags.has(Flags::UNREAD),
            _ => false,
        },
        Direction::Incoming => record.is_ended_incoming(),
    }
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(512))]

    #[test]
    fn every_sequence_of_transitions_keeps_the_rules(
        ops in proptest::collection::vec(op(), 1..120),
    ) {
        let mut limits = Limits::persistent();
        // Low enough for the few transfers here to reach every limit.
        limits.records_per_contact = 3;
        limits.records = 7;
        limits.pending_per_contact = 2;
        limits.pending = 3;
        limits.active_per_contact = 1;
        limits.active = 2;
        limits.waiting_per_contact = 2;
        limits.waiting = 3;
        limits.staged_outgoing = 5 * L;
        limits.staged_incoming = 4 * L;
        let mut state = TransferState::new(limits);
        let mut now = NOW;
        let mut next_blob = 1_u8;
        for op in ops {
            let before: BTreeMap<(u8, bool, IdentityId, ContactId), TransferRecord> = state
                .records()
                .records
                .into_iter()
                .map(|record| {
                    (
                        (
                            record.transfer[0],
                            record.direction == Direction::Outgoing,
                            record.identity,
                            record.contact,
                        ),
                        record,
                    )
                })
                .collect();
            let blobs_before = state.blobs();
            let mut freed = Vec::new();
            // What may remove a record that is not ended.
            let mut ending: Option<(IdentityId, ContactId)> = None;
            let mut lawful_removal = false;
            let mut fresh_blob = || {
                let id = blob(next_blob);
                next_blob = next_blob.wrapping_add(1).max(1);
                id
            };
            match op.clone() {
                Op::AddOutgoing { transfer, size } => {
                    let key = which(true, transfer);
                    let _ = state.add_outgoing(
                        Staged {
                            identity: key.identity,
                            contact: key.contact,
                            transfer: key.transfer,
                            size,
                            name: name(),
                            digest: DIGEST,
                            blob: fresh_blob(),
                        },
                        now,
                    );
                }
                Op::Offered { transfer, size } => {
                    let _ = state.offered(&which(false, transfer), size, &name(), now);
                }
                Op::Accept { transfer } => {
                    let blob = fresh_blob();
                    let _ = state.accept(&which(false, transfer), blob, now);
                }
                Op::Reject { transfer } => {
                    let _ = state.reject(&which(false, transfer), now);
                }
                Op::Cancel { direction, transfer } => {
                    if let Ok(change) = state.cancel(&which(direction, transfer), now) {
                        freed.extend(change.freed);
                    }
                }
                Op::AcceptedByPeer { transfer, offset } => {
                    let key = which(true, transfer);
                    let size = state.record(&key).map_or(0, |record| record.size);
                    let change = state.accepted_by_peer(&key, offset.min(size), now);
                    freed.extend(change.freed);
                }
                Op::CompletedByPeer { transfer, good } => {
                    let digest = if good { DIGEST } else { [3; 32] };
                    let (change, _) = state.completed_by_peer(&which(true, transfer), &digest, now);
                    freed.extend(change.freed);
                }
                Op::RejectedByPeer { transfer } => {
                    freed.extend(state.rejected_by_peer(&which(true, transfer), now).freed);
                }
                Op::AbortedByPeer { direction, transfer } => {
                    freed.extend(state.aborted_by_peer(&which(direction, transfer), now).freed);
                }
                Op::OfferRead { transfer } => {
                    state.offer_read(&which(true, transfer));
                }
                Op::MarkOffered { transfer } => state.mark_offered(&which(true, transfer)),
                Op::Staged { transfer, bytes } => {
                    let key = which(false, transfer);
                    let size = state.record(&key).map_or(0, |record| record.size);
                    state.staged(&key, bytes.min(size), now);
                }
                Op::Complete { transfer } => {
                    state.complete(&which(false, transfer), DIGEST, now);
                }
                Op::Fail { direction, transfer } => {
                    freed.extend(state.fail(&which(direction, transfer), Reason::Storage, now).freed);
                }
                Op::Told { direction, transfer } => {
                    let key = which(direction, transfer);
                    if let Some(record) = state.record(&key) {
                        let state_now = record.state;
                        state.told(&key, state_now);
                    }
                }
                Op::Hold { transfer } => state.hold(&which(false, transfer), now),
                Op::Remove { direction, transfer } => {
                    lawful_removal = true;
                    if let Ok(change) = state.remove(&which(direction, transfer), now) {
                        freed.extend(change.freed);
                    }
                }
                Op::Save { transfer, saved } => {
                    let key = which(false, transfer);
                    if state.begin_save(&key).is_ok() {
                        freed.extend(state.end_save(&key, saved, now).freed);
                    }
                }
                Op::SessionCounted { offered } => {
                    let offered: HashSet<[u8; 16]> =
                        offered.into_iter().map(|byte| [byte; 16]).collect();
                    for (_, change) in
                        state.session_counted(&identity(1), &contact(10), &offered, now)
                    {
                        freed.extend(change.freed);
                    }
                }
                Op::Tick { advance } => {
                    now += advance;
                    for (_, change) in state.tick(now) {
                        freed.extend(change.freed);
                    }
                }
                Op::ContactEnded { delete } => {
                    ending = Some((identity(1), contact(11)));
                    for (_, change) in state.contact_ended(&identity(1), &contact(11), delete, now) {
                        freed.extend(change.freed);
                    }
                }
                Op::Restart => {
                    let records = state.records();
                    state = TransferState::load(records, limits);
                }
            }
            // Valid, and read back as written.
            let records = state.records();
            proptest::prop_assert!(records.check().is_ok(), "{op:?}");
            let payload = records.encode().unwrap();
            proptest::prop_assert_eq!(TransferRecords::decode(&payload).unwrap(), records.clone());
            // The limits.
            let count = |test: &dyn Fn(&TransferRecord) -> bool| {
                records.records.iter().filter(|record| test(record)).count()
            };
            proptest::prop_assert!(records.records.len() <= limits.records);
            for byte in [10, 11] {
                for id in [1, 2] {
                    let of = |record: &TransferRecord| {
                        record.contact == contact(byte) && record.identity == identity(id)
                    };
                    proptest::prop_assert!(count(&of) <= limits.records_per_contact);
                    proptest::prop_assert!(
                        count(&|record| of(record)
                            && record.direction == Direction::Incoming
                            && record.state == State::Accepted)
                            <= limits.active_per_contact
                    );
                    proptest::prop_assert!(
                        count(&|record| of(record)
                            && record.direction == Direction::Incoming
                            && record.state == State::Offered)
                            <= limits.pending_per_contact
                    );
                }
            }
            let staged = |direction| {
                records
                    .records
                    .iter()
                    .filter(|record| record.direction == direction && record.has_blob())
                    .map(|record| record.size)
                    .sum::<u64>()
            };
            proptest::prop_assert!(staged(Direction::Outgoing) <= limits.staged_outgoing);
            proptest::prop_assert!(staged(Direction::Incoming) <= limits.staged_incoming);
            // The edges, and the removals.
            for (place, old) in &before {
                let after = records.records.iter().find(|record| {
                    (
                        record.transfer[0],
                        record.direction == Direction::Outgoing,
                        record.identity,
                        record.contact,
                    ) == *place
                });
                match after {
                    Some(new) => proptest::prop_assert!(
                        edge(old.direction, old.state, new.state),
                        "{op:?}: {:?} -> {:?}",
                        old.state,
                        new.state
                    ),
                    None => {
                        let by_contact = ending == Some((old.identity, old.contact));
                        let retention = matches!(op, Op::Tick { .. })
                            && old.direction == Direction::Incoming
                            && old.is_ended_incoming();
                        // Room made for a new record takes an ended one.
                        let room = matches!(op, Op::AddOutgoing { .. } | Op::Offered { .. })
                            && ended(old);
                        proptest::prop_assert!(
                            by_contact || retention || room || (lawful_removal && ended(old)),
                            "{op:?} removed {old:?}"
                        );
                    }
                }
            }
            // Blobs: freed exactly when no record names them any more.
            let blobs_after = state.blobs();
            let gone: HashSet<BlobId> = blobs_before.difference(&blobs_after).copied().collect();
            let freed: HashSet<BlobId> = freed.into_iter().collect();
            proptest::prop_assert_eq!(&gone, &freed, "{:?}", op);
            proptest::prop_assert!(freed.is_disjoint(&blobs_after));
        }
    }
}

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation
)]

use super::*;
use monolith_storage::record::RECORD_ID_LEN;

mod model;

const NOW: u64 = 1_800_000_000;
const DIGEST: [u8; 32] = [7; 32];
const L: u64 = monolith_protocol::limits::MAX_FILE_CHUNK_LEN as u64;

fn identity(byte: u8) -> IdentityId {
    IdentityId::from_bytes([byte; RECORD_ID_LEN]).unwrap()
}

fn contact(byte: u8) -> ContactId {
    ContactId::from_bytes([byte; RECORD_ID_LEN]).unwrap()
}

fn blob(byte: u8) -> BlobId {
    BlobId::from_bytes([byte; 16]).unwrap()
}

fn name() -> Filename {
    Filename::new("report.pdf").unwrap()
}

fn key(direction: Direction, transfer: u8) -> Key {
    Key {
        identity: identity(1),
        contact: contact(2),
        direction,
        transfer: [transfer; 16],
    }
}

fn outgoing(transfer: u8) -> Key {
    key(Direction::Outgoing, transfer)
}

fn incoming(transfer: u8) -> Key {
    key(Direction::Incoming, transfer)
}

fn staged(transfer: u8, size: u64) -> Staged {
    Staged {
        identity: identity(1),
        contact: contact(2),
        transfer: [transfer; 16],
        size,
        name: name(),
        digest: DIGEST,
        blob: blob(transfer),
    }
}

fn state() -> TransferState {
    TransferState::new(Limits::persistent())
}

/// The records are valid, read back as written, and the state rebuilt
/// from them holds the same records.
fn checked(state: &TransferState) {
    let records = state.records();
    records.check().unwrap();
    let payload = records.encode().unwrap();
    assert_eq!(TransferRecords::decode(&payload).unwrap(), records);
    assert_eq!(
        TransferState::load(records.clone(), state.limits).records(),
        records
    );
}

fn status(state: &TransferState, key: &Key) -> TransferStatus {
    state.view(key, NOW).unwrap().status
}

#[test]
fn an_outgoing_transfer_is_waiting_then_sending_then_delivered() {
    let mut state = state();
    let key = state.add_outgoing(staged(1, 3 * L + 5), NOW).unwrap();
    assert_eq!(status(&state, &key), TransferStatus::Waiting);
    // A new offer claims a session until the peer read it.
    assert!(state.claims(&identity(1), &contact(2), NOW));
    assert!(state.offer_read(&key).changed);
    assert!(!state.offer_read(&key).changed);
    assert!(!state.claims(&identity(1), &contact(2), NOW));
    checked(&state);

    let change = state.accepted_by_peer(&key, L, NOW + 10);
    assert!(change.changed && change.progressed);
    assert_eq!(status(&state, &key), TransferStatus::Sending);
    assert_eq!(state.view(&key, NOW).unwrap().bytes, L);
    assert!(state.claims(&identity(1), &contact(2), NOW));
    // The same acceptance on another session changes nothing durable.
    assert!(!state.accepted_by_peer(&key, L, NOW + 20).changed);
    checked(&state);

    let (change, matched) = state.completed_by_peer(&key, &DIGEST, NOW + 30);
    assert!(matched && change.changed);
    assert_eq!(change.freed, Some(blob(1)));
    assert_eq!(status(&state, &key), TransferStatus::Completed);
    assert_eq!(state.view(&key, NOW).unwrap().bytes, 3 * L + 5);
    assert!(!state.claims(&identity(1), &contact(2), NOW));
    assert!(state.to_offer(&identity(1), &contact(2), NOW).is_empty());
    checked(&state);
    // A completion again, on a later session: nothing.
    assert_eq!(
        state.completed_by_peer(&key, &DIGEST, NOW + 40).0,
        Change::default()
    );
}

#[test]
fn a_completion_with_another_digest_fails_the_transfer() {
    let mut state = state();
    let key = state.add_outgoing(staged(1, 10), NOW).unwrap();
    state.accepted_by_peer(&key, 0, NOW);
    let (change, matched) = state.completed_by_peer(&key, &[8; 32], NOW);
    assert!(!matched && change.changed);
    assert_eq!(change.freed, Some(blob(1)));
    let view = state.view(&key, NOW).unwrap();
    assert_eq!(view.status, TransferStatus::Failed);
    assert_eq!(view.reason, Some(TransferReason::Digest));
    checked(&state);
    // The right digest later does not bring it back.
    assert_eq!(
        state.completed_by_peer(&key, &DIGEST, NOW).0,
        Change::default()
    );
}

#[test]
fn a_rejection_or_an_abort_of_the_peer_ends_the_offer() {
    let mut state = state();
    let rejected = state.add_outgoing(staged(1, 10), NOW).unwrap();
    let aborted = state.add_outgoing(staged(2, 10), NOW).unwrap();
    let change = state.rejected_by_peer(&rejected, NOW);
    assert!(change.changed);
    assert_eq!(change.freed, Some(blob(1)));
    assert_eq!(status(&state, &rejected), TransferStatus::Rejected);
    let change = state.aborted_by_peer(&aborted, NOW);
    assert_eq!(change.freed, Some(blob(2)));
    let view = state.view(&aborted, NOW).unwrap();
    assert_eq!(view.status, TransferStatus::Cancelled);
    assert_eq!(view.reason, Some(TransferReason::ByPeer));
    assert!(state.to_offer(&identity(1), &contact(2), NOW).is_empty());
    checked(&state);
}

#[test]
fn a_cancellation_is_told_only_if_the_offer_may_have_been_seen() {
    let mut state = state();
    // Never offered: nothing to tell.
    let quiet = state.add_outgoing(staged(1, 10), NOW).unwrap();
    let change = state.cancel(&quiet, NOW).unwrap();
    assert_eq!(change.freed, Some(blob(1)));
    assert!(!state.record(&quiet).unwrap().flags.has(Flags::UNREAD));
    // Offered on a session: told on the next one, offer and FileAbort.
    let told = state.add_outgoing(staged(2, 10), NOW).unwrap();
    state.mark_offered(&told);
    state.cancel(&told, NOW).unwrap();
    assert!(state.record(&told).unwrap().flags.has(Flags::UNREAD));
    assert!(state.claims(&identity(1), &contact(2), NOW));
    let offers = state.to_offer(&identity(1), &contact(2), NOW);
    assert_eq!(offers.len(), 1);
    assert!(offers[0].cancelled);
    assert_eq!(offers[0].transfer, [2; 16]);
    checked(&state);
    // Cancelling twice is cancelling once.
    assert_eq!(state.cancel(&told, NOW).unwrap(), Change::default());
    // The peer read it.
    assert!(state.told(&told, State::Cancelled).changed);
    assert!(!state.claims(&identity(1), &contact(2), NOW));
    assert!(state.to_offer(&identity(1), &contact(2), NOW).is_empty());
    checked(&state);
    // A cancellation of a transfer the peer acknowledged reading.
    let read = state.add_outgoing(staged(3, 10), NOW).unwrap();
    state.offer_read(&read);
    state.cancel(&read, NOW).unwrap();
    assert!(state.record(&read).unwrap().flags.has(Flags::UNREAD));
    checked(&state);
}

#[test]
fn a_receiver_that_completed_before_reading_a_cancellation_delivered_the_file() {
    let mut state = state();
    let key = state.add_outgoing(staged(1, 10), NOW).unwrap();
    state.mark_offered(&key);
    state.accepted_by_peer(&key, 0, NOW);
    state.cancel(&key, NOW).unwrap();
    let (change, matched) = state.completed_by_peer(&key, &DIGEST, NOW);
    assert!(matched && change.changed);
    assert_eq!(status(&state, &key), TransferStatus::Completed);
    assert!(!state.record(&key).unwrap().flags.has(Flags::UNREAD));
    checked(&state);
    // A completion with another digest after a cancellation changes
    // nothing: the transfer stays cancelled.
    let other = state.add_outgoing(staged(2, 10), NOW).unwrap();
    state.mark_offered(&other);
    state.cancel(&other, NOW).unwrap();
    assert_eq!(
        state.completed_by_peer(&other, &[9; 32], NOW).0,
        Change::default()
    );
    assert_eq!(status(&state, &other), TransferStatus::Cancelled);
}

#[test]
fn offers_are_made_in_the_order_of_the_adr() {
    let mut state = state();
    let waiting_old = state.add_outgoing(staged(1, 10), NOW).unwrap();
    let sending = state.add_outgoing(staged(2, 10), NOW + 1).unwrap();
    let cancelled = state.add_outgoing(staged(3, 10), NOW + 2).unwrap();
    let waiting_new = state.add_outgoing(staged(4, 10), NOW + 3).unwrap();
    state.accepted_by_peer(&sending, 0, NOW + 4);
    state.mark_offered(&cancelled);
    state.cancel(&cancelled, NOW + 5).unwrap();
    let order: Vec<u8> = state
        .to_offer(&identity(1), &contact(2), NOW + 6)
        .iter()
        .map(|offer| offer.transfer[0])
        .collect();
    assert_eq!(
        order,
        vec![
            sending.transfer[0],
            cancelled.transfer[0],
            waiting_old.transfer[0],
            waiting_new.transfer[0]
        ]
    );
    // Another contact sees none of them.
    assert!(state.to_offer(&identity(1), &contact(3), NOW).is_empty());
}

#[test]
fn an_outgoing_transfer_is_refused_above_its_limits() {
    let mut state = state();
    assert_eq!(
        state.may_offer(&identity(1), &contact(2), MAX_TRANSFER_SIZE + 1, NOW),
        Err(Refusal::TooLarge)
    );
    for transfer in 1..=MAX_WAITING_OFFERS_PER_CONTACT as u8 {
        state.add_outgoing(staged(transfer, 10), NOW).unwrap();
    }
    assert_eq!(
        state.add_outgoing(staged(100, 10), NOW),
        Err(Refusal::TooManyWaiting)
    );
    // An identifier the contact holds already, in either direction.
    let mut state = super::TransferState::new(Limits::persistent());
    state.add_outgoing(staged(1, 10), NOW).unwrap();
    let mut again = staged(1, 10);
    again.blob = blob(50);
    assert_eq!(state.add_outgoing(again, NOW), Err(Refusal::IdTaken));
    assert_eq!(state.offered(&incoming(9), 10, &name(), NOW), Offer::New);
    let mut theirs = staged(9, 10);
    theirs.blob = blob(51);
    assert_eq!(state.add_outgoing(theirs, NOW), Err(Refusal::IdTaken));
    // The staged bytes.
    let mut limits = Limits::persistent();
    limits.staged_outgoing = 100;
    let mut state = TransferState::new(limits);
    state.add_outgoing(staged(1, 60), NOW).unwrap();
    assert_eq!(
        state.may_offer(&identity(1), &contact(2), 41, NOW),
        Err(Refusal::StagingFull)
    );
    assert_eq!(state.may_offer(&identity(1), &contact(2), 40, NOW), Ok(()));
    // Delivered, its bytes are free again.
    state.completed_by_peer(&outgoing(1), &DIGEST, NOW);
    assert_eq!(state.may_offer(&identity(1), &contact(2), 100, NOW), Ok(()));
}

#[test]
fn an_ephemeral_installation_stages_both_directions_within_one_budget() {
    let mut limits = Limits::ephemeral();
    limits.staged_in_memory = Some(100);
    let mut state = TransferState::new(limits);
    state.add_outgoing(staged(1, 60), NOW).unwrap();
    assert_eq!(state.offered(&incoming(2), 50, &name(), NOW), Offer::New);
    assert_eq!(state.may_accept(&incoming(2)), Err(Refusal::StagingFull));
    assert_eq!(state.offered(&incoming(3), 40, &name(), NOW), Offer::New);
    assert_eq!(state.may_accept(&incoming(3)), Ok(40));
}

#[test]
fn an_incoming_offer_is_recorded_and_answered_from_what_is_held() {
    let mut state = state();
    let key = incoming(1);
    assert_eq!(state.offered(&key, 3 * L, &name(), NOW), Offer::New);
    assert_eq!(status(&state, &key), TransferStatus::Offered);
    checked(&state);
    // Again, on a later session: nothing, and the deadline stays.
    let deadline = state.record(&key).unwrap().deadline;
    assert_eq!(
        state.offered(&key, 3 * L, &name(), NOW + 100),
        Offer::Answer(Answer::Nothing)
    );
    assert_eq!(state.record(&key).unwrap().deadline, deadline);
    // Another size or another name for the same key: a violation.
    assert_eq!(
        state.offered(&key, 3 * L + 1, &name(), NOW),
        Offer::Violation
    );
    assert_eq!(
        state.offered(&key, 3 * L, &Filename::new("other.pdf").unwrap(), NOW),
        Offer::Violation
    );
    // Accepted: FileAccept with what is held.
    assert_eq!(state.may_accept(&key), Ok(3 * L));
    assert!(state.accept(&key, blob(1), NOW).unwrap().changed);
    assert!(state.record(&key).unwrap().flags.has(Flags::UNREAD));
    assert_eq!(state.answer(&key, NOW), Offer::Answer(Answer::Accept(0)));
    state.staged(&key, 2 * L, NOW);
    assert_eq!(
        state.offered(&key, 3 * L, &name(), NOW),
        Offer::Answer(Answer::Accept(2 * L))
    );
    // The first FileAccept was read.
    assert!(state.told(&key, State::Accepted).changed);
    checked(&state);
    // Complete: FileComplete with the digest verified, again and again.
    let change = state.complete(&key, DIGEST, NOW);
    assert!(change.changed && change.freed.is_none());
    assert_eq!(status(&state, &key), TransferStatus::Received);
    assert_eq!(
        state.offered(&key, 3 * L, &name(), NOW),
        Offer::Answer(Answer::Complete(DIGEST))
    );
    assert!(state.record(&key).unwrap().flags.has(Flags::UNREAD));
    // An abort of the offerer for a complete transfer is discarded.
    assert_eq!(state.aborted_by_peer(&key, NOW), Change::default());
    checked(&state);
}

#[test]
fn after_a_restart_an_accepted_transfer_is_scanned_before_it_is_answered() {
    let mut state = state();
    let key = incoming(1);
    state.offered(&key, 3 * L, &name(), NOW);
    state.accept(&key, blob(1), NOW).unwrap();
    state.staged(&key, 2 * L, NOW);
    let mut state = TransferState::load(state.records(), Limits::persistent());
    assert_eq!(state.answer(&key, NOW), Offer::Scan(blob(1)));
    assert_eq!(state.view(&key, NOW).unwrap().bytes, 0);
    state.scanned(&key, L);
    assert_eq!(state.answer(&key, NOW), Offer::Answer(Answer::Accept(L)));
    // A scan is not progress: the deadline does not move for it.
    let deadline = state.record(&key).unwrap().deadline;
    assert!(!state.staged(&key, L, NOW + 5000).progressed);
    assert_eq!(state.record(&key).unwrap().deadline, deadline);
    // A blob that was missing is replaced, with nothing staged.
    let change = state.replace_blob(&key, blob(2));
    assert_eq!(change.freed, Some(blob(1)));
    assert_eq!(state.answer(&key, NOW), Offer::Answer(Answer::Accept(0)));
    checked(&state);
}

#[test]
fn a_rejected_expired_cancelled_or_failed_offer_is_answered_for_good() {
    let mut state = state();
    state.offered(&incoming(1), 10, &name(), NOW);
    state.reject(&incoming(1), NOW).unwrap();
    // Rejecting twice is rejecting once; accepting it now fails.
    assert_eq!(state.reject(&incoming(1), NOW), Ok(Change::default()));
    assert_eq!(state.may_accept(&incoming(1)), Err(Refusal::Unknown));
    assert_eq!(
        state.answer(&incoming(1), NOW),
        Offer::Answer(Answer::Reject)
    );

    state.offered(&incoming(2), 10, &name(), NOW);
    let expiry = state.record(&incoming(2)).unwrap().deadline;
    assert_eq!(state.next_due(), Some(expiry));
    let changes = state.tick(expiry);
    assert_eq!(changes.len(), 1);
    assert_eq!(status(&state, &incoming(2)), TransferStatus::Expired);
    assert!(state.record(&incoming(2)).unwrap().flags.has(Flags::UNREAD));
    assert_eq!(
        state.answer(&incoming(2), NOW),
        Offer::Answer(Answer::Reject)
    );

    state.offered(&incoming(3), 10, &name(), NOW);
    state.accept(&incoming(3), blob(3), NOW).unwrap();
    let change = state.cancel(&incoming(3), NOW).unwrap();
    assert_eq!(change.freed, Some(blob(3)));
    assert_eq!(
        state.answer(&incoming(3), NOW),
        Offer::Answer(Answer::Abort)
    );

    state.offered(&incoming(4), 10, &name(), NOW);
    state.accept(&incoming(4), blob(4), NOW).unwrap();
    let change = state.fail(&incoming(4), Reason::Digest, NOW);
    assert_eq!(change.freed, Some(blob(4)));
    assert_eq!(
        state.answer(&incoming(4), NOW),
        Offer::Answer(Answer::Abort)
    );
    // A failure needs no telling.
    assert!(!state.record(&incoming(4)).unwrap().flags.has(Flags::UNREAD));
    checked(&state);
}

#[test]
fn an_offer_above_the_limits_is_refused_without_a_record() {
    let mut state = state();
    // Above the size: FileReject, every time.
    for _ in 0..2 {
        assert_eq!(
            state.offered(&incoming(1), MAX_TRANSFER_SIZE + 1, &name(), NOW),
            Offer::Answer(Answer::Reject)
        );
    }
    assert!(state.record(&incoming(1)).is_none());
    // An identifier of zero bytes, which no record may hold: the same.
    assert_eq!(
        state.offered(&incoming(0), 1, &name(), NOW),
        Offer::Answer(Answer::Reject)
    );
    assert!(state.record(&incoming(0)).is_none());
    let zero = Staged {
        transfer: [0; 16],
        ..staged(200, 1)
    };
    assert_eq!(state.add_outgoing(zero, NOW), Err(Refusal::IdTaken));
    // Above the pending offers of the contact: FileReject.
    for transfer in 10..10 + MAX_PENDING_FILE_OFFERS_PER_CONTACT as u8 {
        assert_eq!(
            state.offered(&incoming(transfer), 1, &name(), NOW),
            Offer::New
        );
    }
    assert_eq!(
        state.offered(&incoming(99), 1, &name(), NOW),
        Offer::Answer(Answer::Reject)
    );
    assert!(state.record(&incoming(99)).is_none());
    // Above the pending offers of the identity: nothing; repeated later.
    let mut state = TransferState::new(Limits::persistent());
    let mut made = 0;
    'fill: for record in 10..=200_u8 {
        for transfer in 1..=MAX_PENDING_FILE_OFFERS_PER_CONTACT as u8 {
            if made == MAX_PENDING_FILE_OFFERS {
                break 'fill;
            }
            let key = Key {
                contact: contact(record),
                ..incoming(transfer)
            };
            assert_eq!(state.offered(&key, 1, &name(), NOW), Offer::New);
            made += 1;
        }
    }
    let key = Key {
        contact: contact(250),
        ..incoming(1)
    };
    assert_eq!(
        state.offered(&key, 1, &name(), NOW),
        Offer::Answer(Answer::Nothing)
    );
    assert!(state.record(&key).is_none());
    // Another local identity is not affected.
    let other = Key {
        identity: identity(9),
        ..key
    };
    assert_eq!(state.offered(&other, 1, &name(), NOW), Offer::New);
    checked(&state);
}

#[test]
fn accepting_needs_a_free_place_and_room_to_stage() {
    let mut state = state();
    for transfer in 1..=3 {
        state.offered(&incoming(transfer), 10, &name(), NOW);
    }
    for transfer in 1..=MAX_ACTIVE_TRANSFERS_PER_CONTACT as u8 {
        state
            .accept(&incoming(transfer), blob(transfer), NOW)
            .unwrap();
    }
    assert_eq!(state.may_accept(&incoming(3)), Err(Refusal::TooManyActive));
    assert_eq!(
        state.accept(&incoming(3), blob(3), NOW),
        Err(Refusal::TooManyActive)
    );
    // The refused offer stays pending.
    assert_eq!(status(&state, &incoming(3)), TransferStatus::Offered);
    // Complete, a transfer is no longer active, but its bytes are staged.
    state.complete(&incoming(1), DIGEST, NOW);
    assert_eq!(state.may_accept(&incoming(3)), Ok(10));
    let mut limits = Limits::persistent();
    limits.staged_incoming = 25;
    let mut state = TransferState::new(limits);
    for transfer in 1..=3 {
        state.offered(&incoming(transfer), 10, &name(), NOW);
    }
    state.accept(&incoming(1), blob(1), NOW).unwrap();
    state.complete(&incoming(1), DIGEST, NOW);
    state.accept(&incoming(2), blob(2), NOW).unwrap();
    assert_eq!(state.may_accept(&incoming(3)), Err(Refusal::StagingFull));
    // Saved, the complete file takes nothing any more.
    state.begin_save(&incoming(1)).unwrap();
    let change = state.end_save(&incoming(1), true, NOW);
    assert_eq!(change.freed, Some(blob(1)));
    assert_eq!(state.may_accept(&incoming(3)), Ok(10));
    checked(&state);
}

#[test]
fn room_is_made_from_ended_records_in_the_order_of_the_adr_and_never_from_others() {
    let mut limits = Limits::persistent();
    limits.records_per_contact = 4;
    limits.pending_per_contact = 4;
    let mut state = TransferState::new(limits);
    // An incoming record ended long ago with its answer unread, a recent
    // one whose answer was read, and an ended outgoing one.
    state.offered(&incoming(1), 1, &name(), NOW);
    state.reject(&incoming(1), NOW).unwrap();
    state.offered(&incoming(2), 1, &name(), NOW);
    state.reject(&incoming(2), NOW + UNOFFERABLE_AFTER).unwrap();
    state.told(&incoming(2), State::Rejected);
    state.add_outgoing(staged(3, 1), NOW).unwrap();
    state.rejected_by_peer(&outgoing(3), NOW);
    state.offered(&incoming(4), 1, &name(), NOW);
    let later = NOW + UNOFFERABLE_AFTER + 10;
    // First the ended outgoing record.
    assert_eq!(state.offered(&incoming(5), 1, &name(), later), Offer::New);
    assert!(state.record(&outgoing(3)).is_none());
    // Then the old ended incoming record, though its answer is unread.
    assert_eq!(state.offered(&incoming(6), 1, &name(), later), Offer::New);
    assert!(state.record(&incoming(1)).is_none());
    // Then the one whose answer was read.
    state.reject(&incoming(5), later).unwrap();
    state.reject(&incoming(6), later).unwrap();
    assert_eq!(state.offered(&incoming(7), 1, &name(), later), Offer::New);
    assert!(state.record(&incoming(2)).is_none());
    // Left: an offer pending and records whose answers are unread and
    // recent. Nothing can go.
    assert_eq!(
        state.offered(&incoming(8), 1, &name(), later),
        Offer::Answer(Answer::Nothing)
    );
    assert_eq!(
        state.may_offer(&identity(1), &contact(2), 1, later),
        Err(Refusal::ListFull)
    );
    assert_eq!(state.records().records.len(), 4);
    checked(&state);
}

#[test]
fn progress_moves_the_stall_deadline_and_writes_it_at_most_once_an_hour() {
    let mut state = state();
    let key = state.add_outgoing(staged(1, 300 * L), NOW).unwrap();
    state.accepted_by_peer(&key, 0, NOW);
    let first = state.record(&key).unwrap().deadline;
    assert_eq!(first, NOW + STALL);
    // Less than the progress bytes: not progress.
    let change = state.accepted_by_peer(&key, L, NOW + 10);
    assert!(!change.progressed);
    // Enough bytes, within the hour: progress, the deadline moves in
    // memory only.
    let bytes = TRANSFER_PROGRESS_BYTES.div_ceil(L) * L;
    let change = state.accepted_by_peer(&key, bytes, NOW + 20);
    assert!(change.progressed && !change.changed);
    assert_eq!(state.record(&key).unwrap().deadline, first);
    assert_eq!(state.next_due(), Some(NOW + 20 + STALL));
    // An hour later: written.
    let change = state.accepted_by_peer(&key, 2 * bytes, NOW + RECORD_INTERVAL);
    assert!(change.progressed && change.changed);
    assert_eq!(
        state.record(&key).unwrap().deadline,
        NOW + RECORD_INTERVAL + STALL
    );
    // An offset acknowledged again is not progress.
    assert!(
        !state
            .accepted_by_peer(&key, 2 * bytes, NOW + 2 * RECORD_INTERVAL)
            .progressed
    );
    // Reaching the size is.
    assert!(
        state
            .accepted_by_peer(&key, 300 * L, NOW + 2 * RECORD_INTERVAL)
            .progressed
    );
    checked(&state);
}

#[test]
fn a_transfer_without_progress_fails_at_its_deadline_on_either_side() {
    let mut state = state();
    let sent = state.add_outgoing(staged(1, 10), NOW).unwrap();
    state.offered(&incoming(2), 10, &name(), NOW);
    state.accept(&incoming(2), blob(2), NOW).unwrap();
    let told = state.add_outgoing(staged(3, 10), NOW).unwrap();
    state.mark_offered(&told);
    state.cancel(&told, NOW).unwrap();
    assert_eq!(state.next_due(), Some(NOW + STALL));
    assert!(state.tick(NOW + STALL - 1).is_empty());
    let changes = state.tick(NOW + STALL);
    assert_eq!(changes.len(), 3);
    for key in [sent, incoming(2)] {
        let view = state.view(&key, NOW).unwrap();
        assert_eq!(view.status, TransferStatus::Failed);
        assert_eq!(view.reason, Some(TransferReason::Stalled));
    }
    // The cancellation is told no more.
    assert!(!state.record(&told).unwrap().flags.has(Flags::UNREAD));
    assert!(!state.claims(&identity(1), &contact(2), NOW + STALL));
    let freed: Vec<Option<BlobId>> = changes.iter().map(|(_, change)| change.freed).collect();
    assert!(freed.contains(&Some(blob(1))) && freed.contains(&Some(blob(2))));
    checked(&state);
}

#[test]
fn an_ended_incoming_record_goes_at_the_end_of_its_retention() {
    let mut state = state();
    state.offered(&incoming(1), 10, &name(), NOW);
    state.reject(&incoming(1), NOW).unwrap();
    assert_eq!(state.next_due(), Some(NOW + RETENTION));
    assert!(state.tick(NOW + RETENTION - 1).is_empty());
    let changes = state.tick(NOW + RETENTION);
    assert_eq!(changes.len(), 1);
    assert!(state.record(&incoming(1)).is_none());
    assert_eq!(state.next_due(), None);
}

#[test]
fn an_accepted_transfer_the_offerer_stops_offering_fails() {
    let mut state = state();
    state.offered(&incoming(1), 10, &name(), NOW);
    state.accept(&incoming(1), blob(1), NOW).unwrap();
    state.offered(&incoming(2), 10, &name(), NOW);
    state.accept(&incoming(2), blob(2), NOW).unwrap();
    let only_two: HashSet<[u8; 16]> = [[2; 16]].into_iter().collect();
    for _ in 1..TRANSFER_UNOFFERED_SESSIONS {
        assert!(
            state
                .session_counted(&identity(1), &contact(2), &only_two, NOW)
                .is_empty()
        );
    }
    let changes = state.session_counted(&identity(1), &contact(2), &only_two, NOW);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].0, incoming(1));
    assert_eq!(changes[0].1.freed, Some(blob(1)));
    let view = state.view(&incoming(1), NOW).unwrap();
    assert_eq!(view.reason, Some(TransferReason::OffererGone));
    assert_eq!(status(&state, &incoming(2)), TransferStatus::Receiving);
    checked(&state);
    // An offer in between starts the count again.
    let mut state = TransferState::new(Limits::persistent());
    state.offered(&incoming(1), 10, &name(), NOW);
    state.accept(&incoming(1), blob(1), NOW).unwrap();
    let none = HashSet::new();
    let one: HashSet<[u8; 16]> = [[1; 16]].into_iter().collect();
    for round in 0..10 {
        let offered = if round % 2 == 0 { &none } else { &one };
        assert!(
            state
                .session_counted(&identity(1), &contact(2), offered, NOW)
                .is_empty()
        );
    }
}

#[test]
fn a_cancellation_of_the_receiver_stops_claiming_when_the_offerer_is_gone() {
    let mut state = state();
    state.offered(&incoming(1), 10, &name(), NOW);
    state.accept(&incoming(1), blob(1), NOW).unwrap();
    state.cancel(&incoming(1), NOW).unwrap();
    assert!(state.claims(&identity(1), &contact(2), NOW));
    for _ in 0..TRANSFER_UNOFFERED_SESSIONS {
        state.session_counted(&identity(1), &contact(2), &HashSet::new(), NOW);
    }
    assert!(!state.claims(&identity(1), &contact(2), NOW));
    // The record stays, with its flag, until its retention ends.
    assert!(state.record(&incoming(1)).unwrap().flags.has(Flags::UNREAD));
    // Or the stall limit after it was made.
    let mut state = TransferState::new(Limits::persistent());
    state.offered(&incoming(1), 10, &name(), NOW);
    state.accept(&incoming(1), blob(1), NOW).unwrap();
    state.cancel(&incoming(1), NOW).unwrap();
    assert_eq!(state.next_due(), Some(NOW + STALL));
    assert!(state.tick(NOW + STALL).is_empty());
    assert!(!state.claims(&identity(1), &contact(2), NOW + STALL));
    assert_eq!(state.next_due(), Some(NOW + RETENTION));
}

#[test]
fn a_held_transfer_is_answered_with_nothing_and_claims_no_session_until_it_ends() {
    let mut state = state();
    state.offered(&incoming(1), 10, &name(), NOW);
    state.accept(&incoming(1), blob(1), NOW).unwrap();
    state.hold(&incoming(1), NOW);
    assert!(state.view(&incoming(1), NOW).unwrap().held);
    assert_eq!(
        state.answer(&incoming(1), NOW),
        Offer::Answer(Answer::Nothing)
    );
    assert!(!state.claims(&identity(1), &contact(2), NOW));
    assert_eq!(state.next_due(), Some(NOW + HOLD));
    assert!(state.tick(NOW + HOLD).is_empty());
    assert!(state.claims(&identity(1), &contact(2), NOW + HOLD));
    // What was written after the last sync is read again.
    assert_eq!(state.answer(&incoming(1), NOW + HOLD), Offer::Scan(blob(1)));
    // Or the user resumes it.
    state.hold(&incoming(1), NOW);
    state.resume(&incoming(1)).unwrap();
    assert!(state.claims(&identity(1), &contact(2), NOW));
}

#[test]
fn a_blocked_contact_has_its_transfers_cancelled_and_a_deleted_one_loses_them() {
    let mut state = state();
    let waiting = state.add_outgoing(staged(1, 10), NOW).unwrap();
    state.offered(&incoming(2), 10, &name(), NOW);
    state.accept(&incoming(2), blob(2), NOW).unwrap();
    state.offered(&incoming(3), 10, &name(), NOW);
    state.accept(&incoming(3), blob(3), NOW).unwrap();
    state.complete(&incoming(3), DIGEST, NOW);
    state.offered(&incoming(4), 10, &name(), NOW);
    // Another contact's transfer.
    let other = Key {
        contact: contact(9),
        ..incoming(5)
    };
    state.offered(&other, 10, &name(), NOW);
    let mut blocked = state.clone();
    let changes = blocked.contact_ended(&identity(1), &contact(2), false, NOW);
    let freed: HashSet<BlobId> = changes
        .iter()
        .filter_map(|(_, change)| change.freed)
        .collect();
    assert_eq!(freed, [blob(1), blob(2), blob(3)].into_iter().collect());
    for key in [waiting, incoming(2), incoming(4)] {
        let view = blocked.view(&key, NOW).unwrap();
        assert_eq!(view.status, TransferStatus::Cancelled);
        assert_eq!(view.reason, Some(TransferReason::ContactEnded));
    }
    // The complete file went with its blob.
    assert!(blocked.view(&incoming(3), NOW).is_none());
    assert!(!blocked.claims(&identity(1), &contact(2), NOW));
    assert_eq!(status(&blocked, &other), TransferStatus::Offered);
    // Nothing is told to the peer.
    assert!(
        blocked
            .records()
            .records
            .iter()
            .all(|record| !record.flags.has(Flags::UNREAD))
    );
    checked(&blocked);
    // Deleted: the records go.
    let changes = state.contact_ended(&identity(1), &contact(2), true, NOW);
    assert_eq!(changes.len(), 4);
    assert_eq!(state.records().records.len(), 1);
    assert_eq!(status(&state, &other), TransferStatus::Offered);
    checked(&state);
}

#[test]
fn a_contact_made_again_inherits_nothing() {
    let mut state = state();
    state.offered(&incoming(1), 10, &name(), NOW);
    state.reject(&incoming(1), NOW).unwrap();
    // The same person, the same transfer, a new contact record: a new
    // offer.
    let again = Key {
        contact: contact(3),
        ..incoming(1)
    };
    assert_eq!(state.offered(&again, 10, &name(), NOW), Offer::New);
    // And another local identity holds nothing of either.
    let elsewhere = Key {
        identity: identity(4),
        ..incoming(1)
    };
    assert_eq!(
        state.answer(&elsewhere, NOW),
        Offer::Answer(Answer::Nothing)
    );
    assert!(state.view(&elsewhere, NOW).is_none());
}

#[test]
fn reconciling_drops_what_the_vault_no_longer_holds_and_refuses_another_owner() {
    let mut state = state();
    state.offered(&incoming(1), 10, &name(), NOW);
    let gone = Key {
        contact: contact(5),
        ..incoming(2)
    };
    state.offered(&gone, 10, &name(), NOW);
    let mut records = HashMap::new();
    records.insert(identity(1), vec![contact(2)]);
    let mut kept = state.clone();
    assert_eq!(kept.reconcile(&records), Ok(true));
    assert!(kept.record(&gone).is_none());
    assert!(kept.record(&incoming(1)).is_some());
    assert_eq!(kept.reconcile(&records), Ok(false));
    // The contact of a record held by another local identity.
    let mut records = HashMap::new();
    records.insert(identity(1), Vec::new());
    records.insert(identity(7), vec![contact(2)]);
    assert_eq!(state.reconcile(&records), Err(StorageError::BadFormat));
    // A deleted identity loses everything, its blobs listed.
    let mut state = TransferState::new(Limits::persistent());
    state.offered(&incoming(1), 10, &name(), NOW);
    state.accept(&incoming(1), blob(1), NOW).unwrap();
    assert_eq!(state.remove_identity(&identity(1)), vec![blob(1)]);
    assert!(!state.holds_identity(&identity(1)));
}

#[test]
fn a_complete_file_is_saved_once_or_removed_and_its_record_stays_for_repeats() {
    let mut state = state();
    let key = incoming(1);
    state.offered(&key, 10, &name(), NOW);
    state.accept(&key, blob(1), NOW).unwrap();
    state.complete(&key, DIGEST, NOW);
    let saving = state.begin_save(&key).unwrap();
    assert_eq!(saving.blob, blob(1));
    assert_eq!(state.begin_save(&key), Err(Refusal::Busy));
    assert_eq!(state.remove(&key, NOW), Err(Refusal::Busy));
    checked(&state);
    // A save that did not end keeps the file.
    assert_eq!(state.end_save(&key, false, NOW).freed, None);
    assert!(state.begin_save(&key).is_ok());
    // Saved: the blob goes, the record stays for its retention.
    assert_eq!(state.end_save(&key, true, NOW).freed, Some(blob(1)));
    assert!(state.view(&key, NOW).unwrap().saved);
    assert_eq!(state.begin_save(&key), Err(Refusal::Unknown));
    assert_eq!(
        state.offered(&key, 10, &name(), NOW),
        Offer::Answer(Answer::Complete(DIGEST))
    );
    checked(&state);
    // Removed from the interface: hidden, still answered.
    assert!(state.remove(&key, NOW).unwrap().changed);
    assert!(state.view(&key, NOW).is_none());
    assert_eq!(state.remove(&key, NOW), Ok(Change::default()));
    assert_eq!(
        state.offered(&key, 10, &name(), NOW),
        Offer::Answer(Answer::Complete(DIGEST))
    );
    checked(&state);
    // A complete file never saved, removed: its blob goes.
    let other = incoming(2);
    state.offered(&other, 10, &name(), NOW);
    state.accept(&other, blob(2), NOW).unwrap();
    state.complete(&other, DIGEST, NOW);
    assert_eq!(state.cancel(&other, NOW), Err(Refusal::Unknown));
    assert_eq!(state.remove(&other, NOW).unwrap().freed, Some(blob(2)));
    assert_eq!(state.record(&other).unwrap().deadline, NOW + RETENTION);
    checked(&state);
}

#[test]
fn a_save_under_way_at_a_crash_is_reported_once_and_its_flag_cleared() {
    let mut state = state();
    let key = incoming(1);
    state.offered(&key, 10, &name(), NOW);
    state.accept(&key, blob(1), NOW).unwrap();
    state.complete(&key, DIGEST, NOW);
    state.begin_save(&key).unwrap();
    let mut after = TransferState::load(state.records(), Limits::persistent());
    assert_eq!(after.interrupted_saves(), vec![key]);
    assert!(after.interrupted_saves().is_empty());
    // The file can be saved again.
    assert!(after.begin_save(&key).is_ok());
    checked(&after);
}

#[test]
fn an_ended_outgoing_record_goes_when_the_user_removes_it() {
    let mut state = state();
    let key = state.add_outgoing(staged(1, 10), NOW).unwrap();
    assert_eq!(state.remove(&key, NOW), Err(Refusal::Unknown));
    state.mark_offered(&key);
    state.cancel(&key, NOW).unwrap();
    // Not while the peer has to be told.
    assert_eq!(state.remove(&key, NOW), Err(Refusal::Unknown));
    state.told(&key, State::Cancelled);
    assert!(state.remove(&key, NOW).unwrap().changed);
    assert!(state.record(&key).is_none());
}

#[test]
fn a_flag_that_the_peer_read_an_answer_is_cleared_only_for_that_answer() {
    let mut state = state();
    let key = incoming(1);
    state.offered(&key, 10, &name(), NOW);
    state.accept(&key, blob(1), NOW).unwrap();
    state.complete(&key, DIGEST, NOW);
    // A Pong of the Ping written after the first FileAccept arrives once
    // the transfer is complete: its FileComplete is still unread.
    assert_eq!(state.told(&key, State::Accepted), Change::default());
    assert!(state.record(&key).unwrap().flags.has(Flags::UNREAD));
    assert!(state.told(&key, State::Complete).changed);
}

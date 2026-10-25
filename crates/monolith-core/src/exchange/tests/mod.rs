#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation
)]

use monolith_protocol::body::{FileChunk, TransferId};

use super::*;

const L: u64 = CHUNK;

fn id(byte: u8) -> TransferId {
    TransferId::from_bytes([byte; 16])
}

fn offer(byte: u8, size: u64) -> Message {
    Message::FileOffer {
        transfer: id(byte),
        size,
        filename: Filename::new("a.txt").unwrap(),
    }
}

fn accept(byte: u8, offset: u64) -> Message {
    Message::FileAccept {
        transfer: id(byte),
        offset,
    }
}

fn chunk(byte: u8, len: u64) -> Message {
    Message::FileChunk(FileChunk::new(id(byte), vec![1; len as usize]).unwrap())
}

fn complete(byte: u8) -> Message {
    Message::FileComplete {
        transfer: id(byte),
        digest: [5; 32],
    }
}

fn received(exchange: &mut Exchange, message: &Message) -> Received {
    exchange.receive(message).unwrap().received
}

/// Every file message but an offer, for the transfer `byte`.
fn others(byte: u8) -> Vec<Message> {
    vec![
        accept(byte, 0),
        Message::FileReject(id(byte)),
        chunk(byte, 10),
        complete(byte),
        Message::FileAbort(id(byte)),
    ]
}

#[test]
fn other_messages_are_not_file_messages() {
    let mut exchange = Exchange::default();
    assert_eq!(exchange.receive(&Message::Ping([0; 8])), None);
    assert_eq!(exchange.receive(&Message::Close), None);
}

#[test]
fn a_file_message_for_a_transfer_never_offered_on_the_session_is_a_violation() {
    for message in others(1) {
        let mut exchange = Exchange::default();
        assert_eq!(
            received(&mut exchange, &message),
            Received::Violation,
            "{message:?}"
        );
    }
}

#[test]
fn a_second_offer_or_an_offer_of_a_transfer_this_side_offered_is_a_violation() {
    let mut exchange = Exchange::default();
    assert!(matches!(
        received(&mut exchange, &offer(1, 10)),
        Received::Offer { size: 10, .. }
    ));
    assert_eq!(received(&mut exchange, &offer(1, 10)), Received::Violation);
    exchange.offered([2; 16], 10);
    assert_eq!(received(&mut exchange, &offer(2, 10)), Received::Violation);
    // Not even once it ended on the session.
    let mut exchange = Exchange::default();
    exchange.offered([2; 16], 10);
    exchange.end(&[2; 16], false);
    assert_eq!(received(&mut exchange, &offer(2, 10)), Received::Violation);
}

#[test]
fn an_offer_refused_on_the_session_is_remembered_and_only_an_abort_or_itself_crosses_it() {
    // What Phase 7 asked of a receiver that rejects at once (P7-23),
    // still: the same offer again and an abort are discarded; a chunk, a
    // completion, and an answer from the side that made the offer are
    // violations.
    for message in others(1) {
        let mut exchange = Exchange::default();
        received(&mut exchange, &offer(1, 10));
        exchange.end(&[1; 16], true);
        let expected = if matches!(message, Message::FileAbort(_)) {
            Received::Discard
        } else {
            Received::Violation
        };
        assert_eq!(received(&mut exchange, &message), expected, "{message:?}");
        assert_eq!(received(&mut exchange, &offer(1, 10)), Received::Discard);
    }
}

#[test]
fn only_the_last_ended_transfers_are_remembered() {
    let mut exchange = Exchange::default();
    let count = u8::try_from(MAX_ENDED_TRANSFER_IDS).unwrap();
    for byte in 0..=count {
        received(&mut exchange, &offer(byte, 10));
        exchange.end(&[byte; 16], true);
    }
    // The first fell out; the last ones are still known.
    assert_eq!(
        received(&mut exchange, &Message::FileAbort(id(0))),
        Received::Violation
    );
    assert_eq!(
        received(&mut exchange, &Message::FileAbort(id(count))),
        Received::Discard
    );
    assert_eq!(
        received(&mut exchange, &Message::FileAbort(id(1))),
        Received::Discard
    );
}

#[test]
fn chunks_follow_the_acceptance_with_no_gap_and_no_overlap_to_the_size() {
    let size = 2 * L + 7;
    let mut exchange = Exchange::default();
    received(&mut exchange, &offer(1, size));
    // Before a FileAccept on the session, a chunk is a violation.
    assert_eq!(received(&mut exchange, &chunk(1, L)), Received::Violation);
    let mut exchange = Exchange::default();
    received(&mut exchange, &offer(1, size));
    exchange.accepting(&[1; 16], L, Instant::now());
    assert_eq!(exchange.expected(&[1; 16]), Some(L));
    // A chunk of the wrong length.
    let mut wrong = Exchange::default();
    received(&mut wrong, &offer(1, size));
    wrong.accepting(&[1; 16], L, Instant::now());
    assert_eq!(received(&mut wrong, &chunk(1, L - 1)), Received::Violation);
    assert_eq!(
        received(&mut exchange, &chunk(1, L)),
        Received::Chunk { offset: L }
    );
    // The last carries exactly what is left; then nothing more.
    let mut short = Exchange::default();
    received(&mut short, &offer(1, size));
    short.accepting(&[1; 16], L, Instant::now());
    received(&mut short, &chunk(1, L));
    assert_eq!(received(&mut short, &chunk(1, L)), Received::Violation);
    assert_eq!(
        received(&mut exchange, &chunk(1, 7)),
        Received::Chunk { offset: 2 * L }
    );
    assert_eq!(received(&mut exchange, &chunk(1, 7)), Received::Violation);
}

#[test]
fn a_completion_of_the_offerer_counts_only_with_every_byte_received() {
    let size = L + 1;
    let mut exchange = Exchange::default();
    received(&mut exchange, &offer(1, size));
    exchange.accepting(&[1; 16], 0, Instant::now());
    received(&mut exchange, &chunk(1, L));
    let mut early = Exchange::default();
    received(&mut early, &offer(1, size));
    early.accepting(&[1; 16], 0, Instant::now());
    received(&mut early, &chunk(1, L));
    assert_eq!(received(&mut early, &complete(1)), Received::Violation);
    received(&mut exchange, &chunk(1, 1));
    assert_eq!(
        received(&mut exchange, &complete(1)),
        Received::Verify { digest: [5; 32] }
    );
    // A file of size zero: complete at once after the acceptance.
    let mut empty = Exchange::default();
    received(&mut empty, &offer(2, 0));
    assert_eq!(received(&mut empty, &complete(2)), Received::Violation);
    let mut empty = Exchange::default();
    received(&mut empty, &offer(2, 0));
    empty.accepting(&[2; 16], 0, Instant::now());
    assert_eq!(received(&mut empty, &chunk(2, 1)), Received::Violation);
    let mut empty = Exchange::default();
    received(&mut empty, &offer(2, 0));
    empty.accepting(&[2; 16], 0, Instant::now());
    assert!(matches!(
        received(&mut empty, &complete(2)),
        Received::Verify { .. }
    ));
}

#[test]
fn an_acceptance_offset_that_is_stale_future_misaligned_or_of_another_transfer_is_refused() {
    let size = 5 * L + 3;
    let fresh = || {
        let mut exchange = Exchange::default();
        exchange.offered([1; 16], size);
        exchange
    };
    // Misaligned, beyond the size: violations.
    for offset in [1, L - 1, L + 1, size + 1, size + L, u64::MAX] {
        assert_eq!(
            received(&mut fresh(), &accept(1, offset)),
            Received::Violation,
            "{offset}"
        );
    }
    // Whole chunks, or the size: the first acceptance of the session sets
    // where the chunks start, whatever an earlier session held.
    for offset in [0, L, 5 * L, size] {
        assert_eq!(
            received(&mut fresh(), &accept(1, offset)),
            Received::Accept {
                offset,
                first: true
            }
        );
    }
    // Later: never lower, never beyond what was written on the session.
    let mut exchange = fresh();
    received(&mut exchange, &accept(1, L));
    assert_eq!(
        received(&mut exchange, &accept(1, 2 * L)),
        Received::Violation
    );
    exchange.chunk_written(&[1; 16], L);
    exchange.chunk_written(&[1; 16], L);
    assert_eq!(
        received(&mut exchange, &accept(1, 2 * L)),
        Received::Accept {
            offset: 2 * L,
            first: false
        }
    );
    assert_eq!(received(&mut exchange, &accept(1, L)), Received::Violation);
    // An acceptance of another transfer, never offered here.
    assert_eq!(received(&mut fresh(), &accept(9, 0)), Received::Violation);
    // An acceptance of an offer of the peer.
    let mut theirs = Exchange::default();
    received(&mut theirs, &offer(3, 10));
    assert_eq!(received(&mut theirs, &accept(3, 0)), Received::Violation);
}

#[test]
fn the_offerer_writes_at_most_the_window_beyond_the_acknowledged_offset() {
    let size = 10 * L + 1;
    let mut exchange = Exchange::default();
    exchange.offered([1; 16], size);
    // Nothing before an acceptance.
    assert_eq!(exchange.next_chunk(), None);
    received(&mut exchange, &accept(1, 2 * L));
    for n in 0..FILE_WINDOW_CHUNKS {
        let (transfer, offset, len) = exchange.next_chunk().unwrap();
        assert_eq!((transfer, offset, len), ([1; 16], (2 + n) * L, L));
        exchange.chunk_written(&transfer, len);
    }
    // The window is full until the receiver acknowledges.
    assert_eq!(exchange.next_chunk(), None);
    received(&mut exchange, &accept(1, 4 * L));
    for _ in 0..2 {
        let (transfer, _, len) = exchange.next_chunk().unwrap();
        exchange.chunk_written(&transfer, len);
    }
    assert_eq!(exchange.next_chunk(), None);
    // To the end: the last chunk is short, and FileComplete follows once.
    let mut written = 8 * L;
    let mut acked = 4 * L;
    while written < size {
        acked = written.min(acked + 2 * L) / L * L;
        received(&mut exchange, &accept(1, acked));
        while let Some((transfer, offset, len)) = exchange.next_chunk() {
            assert_eq!(offset, written);
            assert!(written - acked < FILE_WINDOW_CHUNKS * L);
            exchange.chunk_written(&transfer, len);
            written += len;
        }
    }
    assert_eq!(written, size);
    assert_eq!(exchange.completion_due(), vec![[1; 16]]);
    exchange.completion_written(&[1; 16]);
    assert!(exchange.completion_due().is_empty());
    // An acceptance with the whole size asks for nothing more.
    received(&mut exchange, &accept(1, size));
    assert_eq!(exchange.next_chunk(), None);
    assert!(exchange.completion_due().is_empty());
}

#[test]
fn an_acceptance_of_the_whole_size_asks_for_the_completion_at_once() {
    let mut exchange = Exchange::default();
    exchange.offered([1; 16], 3 * L);
    received(&mut exchange, &accept(1, 3 * L));
    assert_eq!(exchange.next_chunk(), None);
    assert_eq!(exchange.completion_due(), vec![[1; 16]]);
    let mut empty = Exchange::default();
    empty.offered([2; 16], 0);
    received(&mut empty, &accept(2, 0));
    assert_eq!(empty.completion_due(), vec![[2; 16]]);
}

#[test]
fn one_transfer_sends_at_a_time_the_one_accepted_first() {
    let mut exchange = Exchange::default();
    exchange.offered([1; 16], 3 * L);
    exchange.offered([2; 16], 3 * L);
    received(&mut exchange, &accept(2, 0));
    received(&mut exchange, &accept(1, 0));
    let mut sent = Vec::new();
    while let Some((transfer, _, len)) = exchange.next_chunk() {
        sent.push(transfer[0]);
        exchange.chunk_written(&transfer, len);
    }
    // The first accepted is written in full before the next starts, and
    // that one goes on to its window.
    assert_eq!(sent, vec![2, 2, 2, 1, 1, 1]);
    // With the first not written in full and its window full, the next
    // one waits.
    let mut exchange = Exchange::default();
    exchange.offered([1; 16], 9 * L);
    exchange.offered([2; 16], 9 * L);
    received(&mut exchange, &accept(2, 0));
    received(&mut exchange, &accept(1, 0));
    let mut sent = Vec::new();
    while let Some((transfer, _, len)) = exchange.next_chunk() {
        sent.push(transfer[0]);
        exchange.chunk_written(&transfer, len);
    }
    assert_eq!(sent, vec![2; FILE_WINDOW_CHUNKS as usize]);
    // Ended on the session, it gives way at once.
    let mut exchange = Exchange::default();
    exchange.offered([1; 16], 9 * L);
    exchange.offered([2; 16], 9 * L);
    received(&mut exchange, &accept(1, 0));
    received(&mut exchange, &accept(2, 0));
    exchange.end(&[1; 16], true);
    assert_eq!(
        exchange.next_chunk().map(|(transfer, ..)| transfer),
        Some([2; 16])
    );
}

#[test]
fn chunks_on_the_way_when_this_side_ended_a_transfer_are_discarded_within_the_window() {
    let size = 20 * L;
    let mut exchange = Exchange::default();
    received(&mut exchange, &offer(1, size));
    exchange.accepting(&[1; 16], 2 * L, Instant::now());
    received(&mut exchange, &chunk(1, L));
    // This side cancels, or the binding moved to another session: the
    // offerer may have three more chunks of its window on the way.
    exchange.end(&[1; 16], true);
    for _ in 0..FILE_WINDOW_CHUNKS - 1 {
        assert_eq!(received(&mut exchange, &chunk(1, L)), Received::Discard);
    }
    assert_eq!(received(&mut exchange, &chunk(1, L)), Received::Violation);
    // A completion may cross it too.
    let mut exchange = Exchange::default();
    received(&mut exchange, &offer(1, L));
    exchange.accepting(&[1; 16], 0, Instant::now());
    exchange.end(&[1; 16], true);
    assert_eq!(received(&mut exchange, &chunk(1, L)), Received::Discard);
    assert_eq!(received(&mut exchange, &complete(1)), Received::Discard);
    // Ended by the offerer, nothing more of it may come.
    let mut exchange = Exchange::default();
    received(&mut exchange, &offer(1, size));
    exchange.accepting(&[1; 16], 0, Instant::now());
    assert_eq!(
        received(&mut exchange, &Message::FileAbort(id(1))),
        Received::Abort
    );
    exchange.end(&[1; 16], false);
    assert_eq!(received(&mut exchange, &chunk(1, L)), Received::Violation);
}

#[test]
fn a_completion_that_crossed_a_cancellation_of_this_side_still_counts_once() {
    let mut exchange = Exchange::default();
    exchange.offered([1; 16], L);
    received(&mut exchange, &accept(1, 0));
    exchange.end(&[1; 16], true);
    assert_eq!(
        received(&mut exchange, &complete(1)),
        Received::Completed { digest: [5; 32] }
    );
    assert_eq!(received(&mut exchange, &complete(1)), Received::Discard);
    // Answers that crossed it are discarded; a chunk from the receiver is
    // a violation.
    assert_eq!(received(&mut exchange, &accept(1, L)), Received::Discard);
    assert_eq!(
        received(&mut exchange, &Message::FileReject(id(1))),
        Received::Discard
    );
    assert_eq!(received(&mut exchange, &chunk(1, 1)), Received::Violation);
    // Ended by the receiver's answer, a completion again is dropped.
    let mut exchange = Exchange::default();
    exchange.offered([2; 16], L);
    assert_eq!(
        received(&mut exchange, &Message::FileReject(id(2))),
        Received::Reject
    );
    exchange.end(&[2; 16], false);
    assert_eq!(received(&mut exchange, &complete(2)), Received::Discard);
}

#[test]
fn progress_is_acknowledged_after_two_chunks_or_after_its_interval() {
    let start = Instant::now();
    let mut exchange = Exchange::default();
    received(&mut exchange, &offer(1, 10 * L));
    exchange.accepting(&[1; 16], 0, start);
    assert!(exchange.progress_due(start).is_empty());
    assert_eq!(exchange.progress_deadline(), None);
    received(&mut exchange, &chunk(1, L));
    assert!(exchange.progress_due(start).is_empty());
    assert_eq!(
        exchange.progress_deadline(),
        Some(start + FILE_PROGRESS_INTERVAL)
    );
    assert_eq!(
        exchange.progress_due(start + FILE_PROGRESS_INTERVAL),
        vec![[1; 16]]
    );
    received(&mut exchange, &chunk(1, L));
    assert_eq!(exchange.progress_due(start), vec![[1; 16]]);
    exchange.accepting(&[1; 16], 2 * L, start);
    assert!(exchange.progress_due(start).is_empty());
    assert_eq!(exchange.accepted(&[1; 16]), Some(2 * L));
    // The chunks still start where the first FileAccept said.
    assert_eq!(exchange.expected(&[1; 16]), Some(2 * L));
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

    /// Whatever the receiver acknowledges and whenever, the offerer never
    /// has more than the window written beyond the last offset
    /// acknowledged, writes every byte once, in order, and asks for the
    /// completion once, after the last.
    #[test]
    fn the_window_holds_whatever_the_acknowledgements(
        chunks in 1_u64..40,
        tail in 0_u64..L,
        first in 0_u64..40,
        acks in proptest::collection::vec(0_u64..6, 1..200),
    ) {
        let size = chunks * L + tail;
        let mut exchange = Exchange::default();
        exchange.offered([1; 16], size);
        let start = (first.min(chunks)) * L;
        let start = if start > size { size } else { start };
        let got = received(&mut exchange, &accept(1, start));
        proptest::prop_assert!(
            matches!(got, Received::Accept { first: true, .. }),
            "first acceptance: {:?}",
            got
        );
        let mut acked = start;
        let mut written = start;
        for step in acks {
            while let Some((_, offset, len)) = exchange.next_chunk() {
                proptest::prop_assert_eq!(offset, written);
                proptest::prop_assert_eq!(len, (size - written).min(L));
                exchange.chunk_written(&[1; 16], len);
                written += len;
                proptest::prop_assert!((written - acked).div_ceil(L) <= FILE_WINDOW_CHUNKS);
            }
            // The receiver acknowledges some whole chunks of what arrived.
            let next = (acked + step * L).min(written);
            let next = if next == size { size } else { next / L * L };
            if next >= acked {
                let got = received(&mut exchange, &accept(1, next));
                proptest::prop_assert!(
                    matches!(got, Received::Accept { first: false, .. }),
                    "later acceptance: {:?}",
                    got
                );
                acked = next;
            }
        }
        if written == size {
            proptest::prop_assert_eq!(exchange.completion_due(), vec![[1; 16]]);
        } else {
            proptest::prop_assert!(exchange.completion_due().is_empty());
        }
    }
}

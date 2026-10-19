//! File messages on a session of a runtime that transfers no file
//! (`docs/PROTOCOL.md` section 13.1, `docs/DESIGN_QUESTIONS.md` P7-4).
//!
//! Monolith offers no file and accepts none. What section 13.1 asks of a
//! receiver is kept all the same, and nothing more: an offer is answered
//! at once with FileReject, as an offer above the pending limits is, and
//! its identifier is remembered as that of a transfer that ended on the
//! session, among the last [`MAX_ENDED_TRANSFER_IDS`]. A file message that
//! names a remembered identifier is discarded: it crossed the rejection. A
//! file message that names any other identifier is a violation, and the
//! session ends without a Close; with no offer of this side and none
//! pending or active, that is every FileAccept, FileReject, FileChunk,
//! FileComplete and FileAbort for an identifier this session did not
//! reject. Nothing of a file is stored, written or shown, and what a file
//! message names lives as long as its session: another session knows none
//! of it.

use std::collections::VecDeque;

use monolith_protocol::body::{Message, TransferId};
use monolith_protocol::limits::MAX_ENDED_TRANSFER_IDS;

/// What a session does with a file message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Answer {
    /// Send FileReject for this offer.
    Reject(TransferId),
    /// Nothing: the message names a transfer that ended on the session.
    Discard,
    /// End the session without a Close.
    Violation,
}

/// The transfers that ended on one session: the offers it rejected.
#[derive(Debug, Default)]
pub(crate) struct Transfers {
    ended: VecDeque<TransferId>,
}

impl Transfers {
    /// What to do with `message`, if it is a file message. `None` for any
    /// other message.
    pub(crate) fn receive(&mut self, message: &Message) -> Option<Answer> {
        let (transfer, offer) = match message {
            Message::FileOffer { transfer, .. } => (*transfer, true),
            Message::FileAccept(transfer)
            | Message::FileReject(transfer)
            | Message::FileAbort(transfer) => (*transfer, false),
            Message::FileChunk(chunk) => (*chunk.transfer(), false),
            Message::FileComplete { transfer, .. } => (*transfer, false),
            _ => return None,
        };
        if self.ended.contains(&transfer) {
            return Some(Answer::Discard);
        }
        if !offer {
            return Some(Answer::Violation);
        }
        if self.ended.len() >= MAX_ENDED_TRANSFER_IDS {
            self.ended.pop_front();
        }
        self.ended.push_back(transfer);
        Some(Answer::Reject(transfer))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use monolith_protocol::body::FileChunk;
    use monolith_protocol::text::Filename;

    use super::*;

    fn id(byte: u8) -> TransferId {
        TransferId::from_bytes([byte; 16])
    }

    fn offer(byte: u8) -> Message {
        Message::FileOffer {
            transfer: id(byte),
            size: 10,
            filename: Filename::new("a.txt").unwrap(),
        }
    }

    fn others(byte: u8) -> Vec<Message> {
        vec![
            Message::FileAccept(id(byte)),
            Message::FileReject(id(byte)),
            Message::FileAbort(id(byte)),
            Message::FileChunk(FileChunk::new(id(byte), vec![1]).unwrap()),
            Message::FileComplete {
                transfer: id(byte),
                digest: [0; 32],
            },
        ]
    }

    #[test]
    fn an_offer_is_rejected_and_what_crosses_the_rejection_is_discarded() {
        let mut transfers = Transfers::default();
        assert_eq!(transfers.receive(&offer(1)), Some(Answer::Reject(id(1))));
        // The same offer again names a transfer that ended.
        assert_eq!(transfers.receive(&offer(1)), Some(Answer::Discard));
        for message in others(1) {
            assert_eq!(transfers.receive(&message), Some(Answer::Discard));
        }
    }

    #[test]
    fn a_file_message_for_a_transfer_this_session_never_saw_is_a_violation() {
        let mut transfers = Transfers::default();
        for message in others(2) {
            assert_eq!(transfers.receive(&message), Some(Answer::Violation));
        }
        // Another session knows nothing of what this one rejected.
        let mut first = Transfers::default();
        assert_eq!(first.receive(&offer(3)), Some(Answer::Reject(id(3))));
        let mut second = Transfers::default();
        for message in others(3) {
            assert_eq!(second.receive(&message), Some(Answer::Violation));
        }
    }

    #[test]
    fn only_the_last_ended_transfers_are_remembered() {
        let mut transfers = Transfers::default();
        let count = u8::try_from(MAX_ENDED_TRANSFER_IDS).unwrap();
        for byte in 0..=count {
            assert_eq!(
                transfers.receive(&offer(byte)),
                Some(Answer::Reject(id(byte)))
            );
        }
        // The first one fell out; the last ones are still known.
        assert_eq!(
            transfers.receive(&Message::FileAbort(id(0))),
            Some(Answer::Violation)
        );
        assert_eq!(
            transfers.receive(&Message::FileAbort(id(count))),
            Some(Answer::Discard)
        );
        assert_eq!(
            transfers.receive(&Message::FileAbort(id(1))),
            Some(Answer::Discard)
        );
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

        /// Any sequence of file messages on one session against a model of
        /// what section 13.1 asks: the history of the offers rejected, of
        /// which the last `MAX_ENDED_TRANSFER_IDS` are remembered.
        #[test]
        fn every_file_message_is_answered_as_the_rejected_offers_say(
            steps in proptest::collection::vec(
                (
                    proptest::prop_oneof![3 => proptest::strategy::Just(0_u8), 1 => 1_u8..6],
                    0_u8..160,
                ),
                1..600,
            ),
        ) {
            let mut transfers = Transfers::default();
            let mut rejected: Vec<u8> = Vec::new();
            for (kind, byte) in steps {
                let message = if kind == 0 {
                    offer(byte)
                } else {
                    others(byte).swap_remove(usize::from(kind - 1))
                };
                let window = rejected.len().saturating_sub(MAX_ENDED_TRANSFER_IDS);
                let expected = if rejected[window..].contains(&byte) {
                    Answer::Discard
                } else if kind == 0 {
                    rejected.push(byte);
                    Answer::Reject(id(byte))
                } else {
                    Answer::Violation
                };
                proptest::prop_assert_eq!(transfers.receive(&message), Some(expected));
            }
        }
    }

    #[test]
    fn other_messages_are_not_file_messages() {
        let mut transfers = Transfers::default();
        assert_eq!(transfers.receive(&Message::Ping([0; 8])), None);
        assert_eq!(transfers.receive(&Message::Close), None);
    }
}

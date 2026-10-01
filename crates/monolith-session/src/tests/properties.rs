//! Property tests of the session layer: statements that hold for every
//! input, not for chosen examples.
//!
//! Every case runs real handshakes, so the number of cases is smaller than
//! in the protocol core.

use std::sync::LazyLock;

use monolith_protocol::SessionState;
use monolith_protocol::body::Message;
use monolith_protocol::limits::{HANDSHAKE_MSG1_LEN, HANDSHAKE_MSG2_LEN, HANDSHAKE_MSG3_LEN};
use monolith_protocol::session::{Action, PeerRecord, Standing};
use proptest::collection::vec;
use proptest::prelude::*;

use super::frames::{confirmed_with, limits};
use crate::testing::{
    ALICE, BOB, EPHEMERAL_I, EPHEMERAL_R, Transcript, card, chat, confirmed, connect, handshake,
    party, start,
};
use crate::{AuthenticatedSession, HandshakeInitiator, HandshakeResponder, SessionError};

/// The messages of the handshake between Alice and Bob with the fixed
/// ephemeral keys.
static TRANSCRIPT: LazyLock<Transcript> = LazyLock::new(|| handshake(&party(ALICE), &party(BOB)).2);

fn alice_dialing_bob() -> HandshakeInitiator {
    HandshakeInitiator::start_with_ephemeral(&party(ALICE), &card(BOB), start(), EPHEMERAL_I)
        .unwrap()
        .0
}

fn bob_waiting() -> HandshakeResponder {
    HandshakeResponder::new_with_ephemeral(&party(BOB), start(), EPHEMERAL_R).unwrap()
}

/// Feeds a stream to a session in pieces of the given sizes, repeated as
/// often as needed. Returns the messages that were delivered and the
/// error that stopped the session, if there was one.
fn feed(
    session: &mut AuthenticatedSession,
    stream: &[u8],
    pieces: &[usize],
) -> (Vec<Message>, Option<SessionError>) {
    let mut delivered = Vec::new();
    let mut rest = stream;
    let mut sizes = pieces.iter().cycle();
    while !rest.is_empty() {
        let size = sizes.next().copied().unwrap_or(1).clamp(1, rest.len());
        let (mut chunk, tail) = rest.split_at(size);
        rest = tail;
        while !chunk.is_empty() {
            match session.receive(chunk, start()) {
                Ok((used, received)) => {
                    assert!(used > 0 && used <= chunk.len());
                    chunk = &chunk[used..];
                    if let Some(received) = received {
                        assert_eq!(received.actions, vec![Action::Deliver]);
                        delivered.push(received.message);
                    }
                }
                Err(error) => return (delivered, Some(error)),
            }
        }
    }
    (delivered, None)
}

/// Chat messages whose bodies have the given lengths, so that frames of
/// one to three blocks occur.
fn messages(lengths: &[usize]) -> Vec<Message> {
    lengths
        .iter()
        .enumerate()
        .map(|(index, len)| {
            let letter = char::from(b'a' + u8::try_from(index % 26).unwrap());
            chat(&letter.to_string().repeat(*len))
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn a_handshake_message_with_any_bit_flipped_is_rejected(
        which in 0..3_usize,
        position in any::<prop::sample::Index>(),
        bit in 0..8_u8,
    ) {
        match which {
            0 => {
                let mut message = TRANSCRIPT.message_1;
                message[position.index(HANDSHAKE_MSG1_LEN)] ^= 1 << bit;
                prop_assert!(bob_waiting().read_message_1(&message, start()).is_err());
            }
            1 => {
                let mut message = TRANSCRIPT.message_2;
                message[position.index(HANDSHAKE_MSG2_LEN)] ^= 1 << bit;
                prop_assert!(alice_dialing_bob().read_message_2(&message, start()).is_err());
            }
            _ => {
                let mut message = TRANSCRIPT.message_3;
                message[position.index(HANDSHAKE_MSG3_LEN)] ^= 1 << bit;
                let (waiting, _) = bob_waiting()
                    .read_message_1(&TRANSCRIPT.message_1, start())
                    .unwrap();
                prop_assert!(waiting.read_message_3(&message, start()).is_err());
            }
        }
    }

    #[test]
    fn arbitrary_bytes_are_not_a_handshake_message(
        first in any::<[u8; HANDSHAKE_MSG1_LEN]>(),
        second in any::<[u8; HANDSHAKE_MSG2_LEN]>(),
        third in vec(any::<u8>(), HANDSHAKE_MSG3_LEN),
        valid_key in any::<bool>(),
    ) {
        // Arbitrary bytes, and arbitrary bytes behind a valid ephemeral
        // key, which gets the message past the key check to the tag.
        let mut first = first;
        let mut second = second;
        if valid_key {
            first[..32].copy_from_slice(&TRANSCRIPT.message_1[..32]);
            second[..32].copy_from_slice(&TRANSCRIPT.message_2[..32]);
        }
        prop_assert!(bob_waiting().read_message_1(&first, start()).is_err());
        prop_assert!(alice_dialing_bob().read_message_2(&second, start()).is_err());
        let third: [u8; HANDSHAKE_MSG3_LEN] = third.try_into().unwrap();
        let (waiting, _) = bob_waiting()
            .read_message_1(&TRANSCRIPT.message_1, start())
            .unwrap();
        prop_assert!(waiting.read_message_3(&third, start()).is_err());
    }

    #[test]
    fn a_stream_split_anywhere_delivers_the_same_messages(
        lengths in vec(1..2500_usize, 1..6),
        pieces in vec(1..4000_usize, 1..8),
    ) {
        let mut pair = confirmed();
        let sent = messages(&lengths);
        let mut stream = Vec::new();
        for message in &sent {
            stream.extend_from_slice(&pair.initiator.send(message, start()).unwrap());
        }
        let (delivered, error) = feed(&mut pair.responder, &stream, &pieces);
        prop_assert_eq!(error, None);
        prop_assert_eq!(delivered, sent);
        prop_assert_eq!(pair.responder.state(), SessionState::AuthenticatedContact);
    }

    #[test]
    fn a_changed_byte_delivers_nothing_from_the_frame_it_is_in_or_after(
        lengths in vec(1..2500_usize, 1..5),
        position in any::<prop::sample::Index>(),
        flip in 1..=255_u8,
        pieces in vec(1..4000_usize, 1..6),
    ) {
        // One byte of the stream is changed: ciphertext, tag or length
        // prefix. The frames before it are delivered. The frame it is in
        // and everything after it are not, whatever the change does to
        // the framing.
        let mut pair = confirmed();
        let sent = messages(&lengths);
        let mut stream = Vec::new();
        let mut ends = Vec::new();
        for message in &sent {
            stream.extend_from_slice(&pair.initiator.send(message, start()).unwrap());
            ends.push(stream.len());
        }
        let index = position.index(stream.len());
        stream[index] ^= flip;
        let intact = ends.iter().filter(|end| **end <= index).count();

        let (delivered, error) = feed(&mut pair.responder, &stream, &pieces);
        prop_assert_eq!(&delivered[..], &sent[..intact]);
        // The session is over, or it is still waiting for bytes that a
        // larger length prefix announced and that never come.
        if error.is_some() {
            prop_assert_eq!(pair.responder.state(), SessionState::Closed);
            prop_assert_eq!(
                pair.responder.receive(&[0], start()).err(),
                Some(SessionError::Closed)
            );
        }
    }

    #[test]
    fn a_dropped_or_repeated_frame_ends_the_session(
        count in 2..6_usize,
        victim in any::<prop::sample::Index>(),
        repeat in any::<bool>(),
    ) {
        let mut pair = confirmed();
        let sent = messages(&vec![10; count]);
        let frames: Vec<Vec<u8>> = sent
            .iter()
            .map(|message| pair.initiator.send(message, start()).unwrap())
            .collect();
        // Drop a frame that is followed by another one, or repeat any.
        let index = if repeat {
            victim.index(count)
        } else {
            victim.index(count - 1)
        };
        let mut stream = Vec::new();
        for (position, frame) in frames.iter().enumerate() {
            if position == index && !repeat {
                continue;
            }
            stream.extend_from_slice(frame);
            if position == index && repeat {
                stream.extend_from_slice(frame);
            }
        }
        let (delivered, error) = feed(&mut pair.responder, &stream, &[977]);
        let expected = if repeat { index + 1 } else { index };
        prop_assert_eq!(&delivered[..], &sent[..expected]);
        prop_assert!(error.is_some());
        prop_assert_eq!(pair.responder.state(), SessionState::Closed);
    }

    #[test]
    fn arbitrary_bytes_never_reach_the_application(
        bytes in vec(any::<u8>(), 1..3000),
        prefix in proptest::option::of(prop::sample::select(vec![[0x04_u8, 0x10], [0x08, 0x10]])),
        confirmed_session in any::<bool>(),
    ) {
        // Arbitrary bytes, and arbitrary bytes behind a length prefix that
        // is legal, which gets them to the cipher.
        let mut stream = prefix.map(|prefix| prefix.to_vec()).unwrap_or_default();
        stream.extend_from_slice(&bytes);
        let mut session = if confirmed_session {
            confirmed().responder
        } else {
            connect(Standing::Accepted, PeerRecord::Accepted(&card(ALICE))).responder
        };
        let before = session.state();
        let mut rest = &stream[..];
        while !rest.is_empty() {
            match session.receive(rest, start()) {
                Ok((used, received)) => {
                    prop_assert!(received.is_none());
                    rest = &rest[used..];
                }
                Err(_) => {
                    prop_assert_eq!(session.state(), SessionState::Closed);
                    break;
                }
            }
        }
        // Nothing changed the session but its end.
        prop_assert!(session.state() == before || session.state() == SessionState::Closed);
    }

    #[test]
    fn the_two_directions_do_not_depend_on_each_other(
        schedule in vec(any::<bool>(), 1..24),
    ) {
        // Frames are sent in an arbitrary interleaving of the two
        // directions and delivered at once. Each direction counts its own
        // frames, so every one of them is accepted.
        let mut pair = confirmed();
        for (index, from_initiator) in schedule.iter().enumerate() {
            let message = chat(&format!("message {index}"));
            let (sender, receiver) = if *from_initiator {
                (&mut pair.initiator, &mut pair.responder)
            } else {
                (&mut pair.responder, &mut pair.initiator)
            };
            let frame = sender.send(&message, start()).unwrap();
            let (used, received) = receiver.receive(&frame, start()).unwrap();
            prop_assert_eq!(used, frame.len());
            prop_assert_eq!(received.unwrap().message, message);
        }
    }

    #[test]
    fn a_session_never_sends_more_than_its_limits(
        max_frames in 2..12_u64,
        blocks in 2..40_u64,
        lengths in vec(1..3000_usize, 0..24),
    ) {
        // Whatever is sent, the frames and the ciphertext bytes of one
        // direction stay within the limits, a Close always fits, and the
        // receiver, which has the same limits, accepts all of it.
        let max_bytes = blocks * 1040;
        let mut pair = confirmed_with(limits(3600, 7200, max_frames, max_bytes));
        // The ContactAccept of the confirmation.
        let mut frames = 1_u64;
        let mut bytes = 1040_u64;
        let mut stream = Vec::new();
        let mut accepted = Vec::new();
        for message in messages(&lengths) {
            match pair.initiator.send(&message, start()) {
                Ok(frame) => {
                    frames += 1;
                    bytes += u64::try_from(frame.len() - 2).unwrap();
                    stream.extend_from_slice(&frame);
                    accepted.push(message);
                }
                Err(error) => prop_assert_eq!(error, SessionError::Expired),
            }
        }
        let close = pair.initiator.close();
        prop_assert!(close.is_some());
        frames += 1;
        bytes += 1040;
        prop_assert!(frames <= max_frames);
        prop_assert!(bytes <= max_bytes);

        let (delivered, error) = feed(&mut pair.responder, &stream, &[1500]);
        prop_assert_eq!(error, None);
        prop_assert_eq!(delivered, accepted);
        let (_, received) = pair.responder.receive(&close.unwrap(), start()).unwrap();
        prop_assert_eq!(received.unwrap().message, Message::Close);
    }
}

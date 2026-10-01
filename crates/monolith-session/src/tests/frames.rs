//! Frames on an authenticated session: what is accepted, what ends the
//! session, and what the limits do.

use core::time::Duration;

use monolith_protocol::body::{FileChunk, Message, TransferId};
use monolith_protocol::limits::{
    MAX_BYTES_PER_DIRECTION, MAX_CHAT_TEXT_LEN, MAX_FILE_CHUNK_LEN, MAX_FRAMES_PER_DIRECTION,
    MAX_SESSION_LIFETIME, MAX_SESSION_LIFETIME_WITH_TRANSFER,
};
use monolith_protocol::session::{Action, PeerRecord, Standing};
use monolith_protocol::{MessageType, ProtocolError, SessionState};

use crate::testing::{
    ALICE, BOB, MALLORY, Pair, after, card, card_of, chat, confirmed, connect, deliver, handshake,
    party, request_with, sample, start, transport_secret,
};
use crate::{AuthenticatedSession, LocalParty, SessionError, SessionLimits};

fn failed(error: ProtocolError) -> SessionError {
    SessionError::Protocol(error)
}

/// Two confirmed sessions whose local parties have the given limits.
pub(super) fn confirmed_with(limits: SessionLimits) -> Pair {
    let alice = party(ALICE).with_limits(limits);
    let bob = party(BOB).with_limits(limits);
    let (outbound, inbound, _) = handshake(&alice, &bob);
    let (mut initiator, initiator_first) = outbound.admit(Standing::Accepted).unwrap();
    let (mut responder, _, responder_first) =
        inbound.admit(PeerRecord::Accepted(&card(ALICE))).unwrap();
    let from_alice = initiator.send(&Message::ContactAccept, start()).unwrap();
    let from_bob = responder.send(&Message::ContactAccept, start()).unwrap();
    deliver(&mut responder, &from_alice).unwrap();
    deliver(&mut initiator, &from_bob).unwrap();
    Pair {
        initiator,
        responder,
        initiator_first,
        responder_first,
    }
}

pub(super) fn limits(age: u64, age_with_transfer: u64, frames: u64, bytes: u64) -> SessionLimits {
    SessionLimits::new(
        Duration::from_secs(age),
        Duration::from_secs(age_with_transfer),
        frames,
        bytes,
    )
    .unwrap()
}

/// Asserts that a session is over: it accepts nothing and sends nothing.
fn assert_dead(session: &mut AuthenticatedSession, valid_frame: &[u8]) {
    assert_eq!(session.state(), SessionState::Closed);
    assert_eq!(
        session.receive(valid_frame, start()).err(),
        Some(SessionError::Closed)
    );
    assert_eq!(
        session.send(&chat("x"), start()).err(),
        Some(SessionError::Closed)
    );
    assert!(!session.may_send(MessageType::ChatMessage));
    assert_eq!(session.close(), None);
}

#[test]
fn messages_cross_a_confirmed_session_in_both_directions() {
    let mut pair = confirmed();
    let hello = chat("hello");
    let frame = pair.initiator.send(&hello, start()).unwrap();
    assert_eq!(frame.len(), 1042);
    let received = deliver(&mut pair.responder, &frame).unwrap().unwrap();
    assert_eq!(received.message, hello);
    assert_eq!(received.actions, vec![Action::Deliver]);

    let reply = chat("hello yourself");
    let frame = pair.responder.send(&reply, start()).unwrap();
    let received = deliver(&mut pair.initiator, &frame).unwrap().unwrap();
    assert_eq!(received.message, reply);

    // The plaintext is not on the wire.
    let frame = pair
        .initiator
        .send(&chat("a recognizable sentence"), start())
        .unwrap();
    assert!(!frame.windows(12).any(|window| window == b"recognizable"));
    assert!(deliver(&mut pair.responder, &frame).unwrap().is_some());

    // One message of every type that a confirmed session carries.
    for message_type in MessageType::ALL {
        if !pair.initiator.may_send(message_type) {
            assert!(matches!(
                message_type,
                MessageType::Close | MessageType::ContactRequest
            ));
            continue;
        }
        let message = sample(message_type, ALICE);
        let frame = pair.initiator.send(&message, start()).unwrap();
        let received = deliver(&mut pair.responder, &frame).unwrap().unwrap();
        assert_eq!(received.message, message, "{message_type:?}");
    }
}

#[test]
fn frames_have_the_sizes_of_the_specification() {
    let mut pair = confirmed();
    let longest = chat(&"a".repeat(MAX_CHAT_TEXT_LEN));
    let chunk = Message::FileChunk(
        FileChunk::new(TransferId::from_bytes([3; 16]), vec![7; MAX_FILE_CHUNK_LEN]).unwrap(),
    );
    for (message, blocks) in [
        // One padding block.
        (Message::Ping([0; 8]), 1),
        // The longest chat message needs 17 blocks.
        (longest, 17),
        // A full file chunk fills the largest frame.
        (chunk, 63),
    ] {
        let frame = pair.initiator.send(&message, start()).unwrap();
        // The length prefix, the padded plaintext and the tag.
        assert_eq!(frame.len(), 2 + blocks * 1024 + 16);
        let declared = usize::from(u16::from_be_bytes([frame[0], frame[1]]));
        assert_eq!(declared, frame.len() - 2);
        let received = deliver(&mut pair.responder, &frame).unwrap().unwrap();
        assert_eq!(received.message, message);
    }
}

#[test]
fn a_frame_is_accepted_in_any_fragmentation() {
    let mut pair = confirmed();
    let messages = [chat("one"), chat(&"two".repeat(700)), Message::Ping([9; 8])];
    let mut stream = Vec::new();
    for message in &messages {
        stream.extend_from_slice(&pair.initiator.send(message, start()).unwrap());
    }
    for piece in [1, 2, 3, 7, 1041, 1042, 1043, 5000] {
        let mut receiver = confirmed().responder;
        let mut delivered = Vec::new();
        for chunk in stream.chunks(piece) {
            let mut rest = chunk;
            while !rest.is_empty() {
                let (used, received) = receiver.receive(rest, start()).unwrap();
                assert!(used > 0 && used <= rest.len());
                rest = &rest[used..];
                delivered.extend(received.map(|received| received.message));
            }
        }
        assert_eq!(delivered, messages, "pieces of {piece}");
    }
}

#[test]
fn a_tampered_frame_ends_the_session() {
    // T-FRAME-AUTH. One bit in the ciphertext, in the tag, anywhere.
    for position in [2, 3, 500, 1025, 1026, 1041] {
        let mut pair = confirmed();
        let mut frame = pair.initiator.send(&chat("hello"), start()).unwrap();
        frame[position] ^= 0x01;
        assert_eq!(
            deliver(&mut pair.responder, &frame).err(),
            Some(failed(ProtocolError::FrameAuthenticationFailed)),
            "byte {position}"
        );
        // Nothing was delivered, and the session takes nothing more, not
        // even a frame that is intact.
        let next = pair.initiator.send(&chat("again"), start()).unwrap();
        assert_dead(&mut pair.responder, &next);
    }
}

#[test]
fn a_modified_length_prefix_ends_the_session() {
    let mut pair = confirmed();
    let first = pair.initiator.send(&chat("hello"), start()).unwrap();
    let second = pair.initiator.send(&chat("world"), start()).unwrap();

    // A length that no frame has: refused before any payload is read.
    let mut bad = first.clone();
    bad[1] = 0x11;
    let mut responder = confirmed().responder;
    assert_eq!(
        responder.receive(&bad, start()).err(),
        Some(failed(ProtocolError::FrameLengthOutOfRange))
    );
    assert_dead(&mut responder, &first);

    // A length that is legal and wrong: two blocks. The decoder takes the
    // next frame's bytes into this one, and authentication fails.
    let mut stream = first.clone();
    stream[..2].copy_from_slice(&[0x08, 0x10]);
    stream.extend_from_slice(&second);
    let mut rest = &stream[..];
    let outcome = loop {
        match pair.responder.receive(rest, start()) {
            Ok((used, None)) => rest = &rest[used..],
            other => break other,
        }
    };
    assert_eq!(
        outcome.err(),
        Some(failed(ProtocolError::FrameAuthenticationFailed))
    );
    assert_dead(&mut pair.responder, &second);
}

#[test]
fn dropped_duplicated_and_reordered_frames_end_the_session() {
    let frames = |pair: &mut Pair| -> Vec<Vec<u8>> {
        ["one", "two", "three"]
            .iter()
            .map(|text| pair.initiator.send(&chat(text), start()).unwrap())
            .collect()
    };
    let rejected = Some(failed(ProtocolError::FrameAuthenticationFailed));

    // Dropped: the second frame never arrives.
    let mut pair = confirmed();
    let sent = frames(&mut pair);
    assert!(deliver(&mut pair.responder, &sent[0]).unwrap().is_some());
    assert_eq!(deliver(&mut pair.responder, &sent[2]).err(), rejected);
    assert_dead(&mut pair.responder, &sent[1]);

    // Duplicated: the first frame arrives twice.
    let mut pair = confirmed();
    let sent = frames(&mut pair);
    assert!(deliver(&mut pair.responder, &sent[0]).unwrap().is_some());
    assert_eq!(deliver(&mut pair.responder, &sent[0]).err(), rejected);
    assert_dead(&mut pair.responder, &sent[1]);

    // Reordered: the second frame arrives first.
    let mut pair = confirmed();
    let sent = frames(&mut pair);
    assert_eq!(deliver(&mut pair.responder, &sent[1]).err(), rejected);
    assert_dead(&mut pair.responder, &sent[0]);

    // In order, all three are delivered.
    let mut pair = confirmed();
    for frame in frames(&mut pair) {
        assert!(deliver(&mut pair.responder, &frame).unwrap().is_some());
    }
}

#[test]
fn a_frame_of_another_session_or_direction_is_rejected() {
    let rejected = Some(failed(ProtocolError::FrameAuthenticationFailed));

    // The sender's own frame, sent back to it.
    let mut pair = confirmed();
    let frame = pair.initiator.send(&chat("hello"), start()).unwrap();
    assert_eq!(deliver(&mut pair.initiator, &frame).err(), rejected);

    // A frame recorded on an earlier session between the same two parties.
    // The earlier session used other ephemeral keys.
    let old_frame = {
        let (first, message_1) =
            crate::HandshakeInitiator::start(&party(ALICE), &card(BOB), start()).unwrap();
        let waiting = crate::HandshakeResponder::new(&party(BOB), start()).unwrap();
        let (waiting, message_2) = waiting.read_message_1(&message_1, start()).unwrap();
        let (outbound, message_3) = first.read_message_2(&message_2, start()).unwrap();
        waiting.read_message_3(&message_3, start()).unwrap();
        let (mut old, _) = outbound.admit(Standing::Accepted).unwrap();
        old.send(&Message::ContactAccept, start()).unwrap()
    };
    let alice_card = card(ALICE);
    let mut fresh = connect(Standing::Accepted, PeerRecord::Accepted(&alice_card));
    assert_eq!(deliver(&mut fresh.responder, &old_frame).err(), rejected);
}

#[test]
fn nothing_is_processed_after_a_close() {
    // The local side closes. What the peer still had in flight is dropped
    // without being decoded: intact frames and garbage alike.
    let mut pair = confirmed();
    let in_flight = pair
        .responder
        .send(&chat("crossed the close"), start())
        .unwrap();
    let close = pair.initiator.close().unwrap();
    assert_eq!(close.len(), 1042);
    assert_eq!(pair.initiator.state(), SessionState::Closing);
    assert_eq!(
        pair.initiator.receive(&in_flight, start()),
        Ok((in_flight.len(), None))
    );
    assert_eq!(
        pair.initiator.receive(&[0xff; 70_000], start()),
        Ok((70_000, None))
    );
    // A Close is produced once, and nothing is sent after it.
    assert_eq!(pair.initiator.close(), None);
    assert_eq!(
        pair.initiator.send(&chat("x"), start()).err(),
        Some(SessionError::Closed)
    );
    assert!(!pair.initiator.may_send(MessageType::ChatMessage));

    // The peer receives the Close and is done.
    let received = deliver(&mut pair.responder, &close).unwrap().unwrap();
    assert_eq!(received.message, Message::Close);
    assert_eq!(received.actions, vec![Action::Disconnect]);
    assert_eq!(pair.responder.state(), SessionState::Closed);
    // Records after a Close are not processed. The session reports them
    // as consumed and does nothing with them.
    assert_eq!(
        pair.responder.receive(&in_flight, start()),
        Ok((in_flight.len(), None))
    );
    assert_eq!(
        pair.responder.send(&chat("x"), start()).err(),
        Some(SessionError::Closed)
    );
    assert_eq!(pair.responder.close(), None);

    pair.initiator.stream_closed();
    assert_eq!(pair.initiator.state(), SessionState::Closed);
}

#[test]
fn close_is_not_sent_like_a_message() {
    let mut pair = confirmed();
    assert_eq!(
        pair.initiator.send(&Message::Close, start()).err(),
        Some(SessionError::NotPermitted)
    );
    assert!(!pair.initiator.may_send(MessageType::Close));
    // The session is unaffected.
    assert_eq!(pair.initiator.state(), SessionState::AuthenticatedContact);
    let frame = pair.initiator.send(&chat("still here"), start()).unwrap();
    assert!(deliver(&mut pair.responder, &frame).unwrap().is_some());
}

#[test]
fn blocking_or_removing_a_contact_looks_like_any_other_close() {
    // T-ORACLE-7 on the wire. The three sessions are identical up to this
    // point, so the frame is the same byte for byte.
    let reference = confirmed().initiator.close().unwrap();
    assert_eq!(confirmed().initiator.block_peer().unwrap(), reference);
    assert_eq!(confirmed().initiator.remove_contact().unwrap(), reference);

    let mut blocked = confirmed().initiator;
    let _ = blocked.block_peer();
    assert_eq!(blocked.standing(), Standing::Blocked);
    assert_eq!(blocked.block_peer(), None);
}

#[test]
fn application_messages_are_refused_before_the_session_is_confirmed() {
    // S19. Both sides hold each other as accepted contacts, and neither
    // has confirmed yet.
    let alice_card = card(ALICE);
    let mut pair = connect(Standing::Accepted, PeerRecord::Accepted(&alice_card));
    assert_eq!(pair.initiator.state(), SessionState::AuthenticatedUnknown);

    // The local side cannot send one.
    for message_type in MessageType::ALL {
        let allowed = message_type == MessageType::ContactAccept;
        assert_eq!(
            pair.initiator.may_send(message_type),
            allowed,
            "{message_type:?}"
        );
        if !allowed {
            assert_eq!(
                pair.initiator
                    .send(&sample(message_type, ALICE), start())
                    .err(),
                Some(SessionError::NotPermitted),
                "{message_type:?}"
            );
        }
    }

    // A peer that sends one anyway ends the session. Nothing is delivered.
    let frame = pair.initiator.seal_unchecked(&chat("too early"));
    assert_eq!(
        deliver(&mut pair.responder, &frame).err(),
        Some(failed(ProtocolError::MessageNotPermitted))
    );
    let next = pair.initiator.seal_unchecked(&Message::ContactAccept);
    assert_dead(&mut pair.responder, &next);
}

#[test]
fn a_frame_of_more_than_one_block_is_refused_before_confirmation() {
    let alice_card = card(ALICE);
    let mut pair = connect(Standing::Accepted, PeerRecord::Accepted(&alice_card));
    let frame = pair.initiator.seal_unchecked(&chat(&"a".repeat(2000)));
    assert_eq!(frame.len(), 2 + 2 * 1024 + 16);
    // The length prefix alone is enough to refuse it.
    assert_eq!(
        pair.responder.receive(&frame[..2], start()).err(),
        Some(failed(ProtocolError::FrameLengthOutOfRange))
    );
    assert_eq!(pair.responder.state(), SessionState::Closed);
}

#[test]
fn a_card_of_another_identity_ends_the_session() {
    // The identity of a peer is fixed for the life of a session. A frame
    // that authenticates and carries somebody else's card does not change
    // it; it ends the session.
    let mut pair = confirmed();
    let update = Message::EndpointUpdate(Box::new(card(MALLORY)));
    let frame = pair.initiator.seal_unchecked(&update);
    assert_eq!(
        deliver(&mut pair.responder, &frame).err(),
        Some(failed(ProtocolError::IdentityMismatch))
    );
    assert_eq!(pair.responder.peer(), card(ALICE).identity());
    let next = pair.initiator.seal_unchecked(&chat("as somebody else"));
    assert_dead(&mut pair.responder, &next);

    // The same for a request, before confirmation, whatever the record.
    for record in [
        PeerRecord::None,
        PeerRecord::Blocked,
        PeerRecord::Accepted(&card(ALICE)),
    ] {
        let mut pair = connect(Standing::Requested, record);
        let frame = pair
            .initiator
            .seal_unchecked(&request_with(card(MALLORY), None));
        assert_eq!(
            deliver(&mut pair.responder, &frame).err(),
            Some(failed(ProtocolError::IdentityMismatch))
        );
        assert_eq!(pair.responder.state(), SessionState::Closed);
    }

    // And for a request of the initiator that carries another card of its
    // own identity than the one it presented in the handshake.
    let mut pair = connect(Standing::Requested, PeerRecord::None);
    let frame = pair
        .initiator
        .seal_unchecked(&request_with(card_of(ALICE, ALICE, 2, false), None));
    assert_eq!(
        deliver(&mut pair.responder, &frame).err(),
        Some(failed(ProtocolError::IdentityMismatch))
    );
}

#[test]
fn only_the_local_card_is_sent() {
    // The sending side of the rule above. A message with a card that is
    // not the local one is not encrypted at all, and the session goes on.
    let mut pair = confirmed();
    assert_eq!(
        pair.initiator
            .send(&Message::EndpointUpdate(Box::new(card(MALLORY))), start())
            .err(),
        Some(SessionError::InvalidMessage)
    );
    // A later card of the local identity may be announced.
    let later = Message::EndpointUpdate(Box::new(card_of(ALICE, MALLORY, 2, false)));
    let frame = pair.initiator.send(&later, start()).unwrap();
    assert_eq!(
        deliver(&mut pair.responder, &frame)
            .unwrap()
            .unwrap()
            .message,
        later
    );

    let mut pair = connect(Standing::Requested, PeerRecord::None);
    for foreign in [
        card(MALLORY),
        card_of(ALICE, ALICE, 2, false),
        card_of(ALICE, MALLORY, 1, false),
    ] {
        assert_eq!(
            pair.initiator
                .send(&request_with(foreign, None), start())
                .err(),
            Some(SessionError::InvalidMessage)
        );
    }
    // Nothing was sent, so the next frame is still the first one.
    let frame = pair
        .initiator
        .send(&request_with(card(ALICE), None), start())
        .unwrap();
    assert!(deliver(&mut pair.responder, &frame).unwrap().is_some());
}

#[test]
fn a_frame_that_authenticates_and_is_malformed_inside_ends_the_session() {
    let cases: Vec<(Vec<u8>, ProtocolError)> = vec![
        // Padding that is not zero.
        (
            {
                let mut plaintext = vec![0_u8; 1024];
                plaintext[..4].copy_from_slice(&[0x00, 0x03, 0x00, 0x08]);
                plaintext[1023] = 1;
                plaintext
            },
            ProtocolError::BadPadding,
        ),
        // A type code that is not assigned, among them the code of the
        // identity proof of earlier drafts.
        (
            {
                let mut plaintext = vec![0_u8; 1024];
                plaintext[..4].copy_from_slice(&[0x00, 0x01, 0x00, 0x00]);
                plaintext
            },
            ProtocolError::UnknownMessageType,
        ),
        // A Ping with a body of the wrong length.
        (
            {
                let mut plaintext = vec![0_u8; 1024];
                plaintext[..4].copy_from_slice(&[0x00, 0x03, 0x00, 0x07]);
                plaintext
            },
            ProtocolError::BadMessageLength,
        ),
        // More padding than the body needs.
        (
            {
                let mut plaintext = vec![0_u8; 2048];
                plaintext[..4].copy_from_slice(&[0x00, 0x03, 0x00, 0x08]);
                plaintext
            },
            ProtocolError::BadPadding,
        ),
    ];
    for (plaintext, expected) in cases {
        let mut pair = confirmed();
        let frame = pair.initiator.seal_raw(&plaintext);
        assert_eq!(
            deliver(&mut pair.responder, &frame).err(),
            Some(failed(expected))
        );
        assert_eq!(pair.responder.state(), SessionState::Closed);
    }
}

#[test]
fn session_limits_cannot_be_raised() {
    let day = MAX_SESSION_LIFETIME;
    let two_days = MAX_SESSION_LIFETIME_WITH_TRANSFER;
    let second = Duration::from_secs(1);
    assert_eq!(
        SessionLimits::new(
            day,
            two_days,
            MAX_FRAMES_PER_DIRECTION,
            MAX_BYTES_PER_DIRECTION
        ),
        Some(SessionLimits::PROTOCOL)
    );
    // Anything above the protocol's values.
    assert_eq!(SessionLimits::new(day + second, two_days, 1, 1040), None);
    assert_eq!(SessionLimits::new(day, two_days + second, 1, 1040), None);
    assert_eq!(
        SessionLimits::new(day, two_days, MAX_FRAMES_PER_DIRECTION + 1, 1040),
        None
    );
    assert_eq!(
        SessionLimits::new(day, two_days, 1, MAX_BYTES_PER_DIRECTION + 1),
        None
    );
    // A transfer does not shorten the life of a session.
    assert_eq!(SessionLimits::new(day, day - second, 1, 1040), None);
    assert!(SessionLimits::new(day, day, 1, 1040).is_some());
    // No room for a single frame.
    assert_eq!(SessionLimits::new(day, two_days, 0, 1040), None);
    assert_eq!(SessionLimits::new(day, two_days, 1, 1039), None);
    assert!(SessionLimits::new(second, second, 1, 1040).is_some());
    assert!(SessionLimits::new(Duration::ZERO, Duration::ZERO, 1, 1040).is_some());
}

#[test]
fn the_frame_limit_ends_a_session_and_leaves_room_for_close() {
    // Each side has sent one frame, the ContactAccept. Four are allowed.
    let mut pair = confirmed_with(limits(3600, 7200, 4, 1 << 30));
    let second = pair.initiator.send(&chat("two"), start()).unwrap();
    assert!(!pair.initiator.limit_reached(start()));
    let third = pair.initiator.send(&chat("three"), start()).unwrap();
    // The fourth frame is kept for the Close.
    assert!(pair.initiator.limit_reached(start()));
    assert_eq!(
        pair.initiator.send(&chat("four"), start()).err(),
        Some(SessionError::Expired)
    );
    // The refusal changed nothing: the session is open and the Close fits.
    assert_eq!(pair.initiator.state(), SessionState::AuthenticatedContact);
    let close = pair.initiator.close().unwrap();

    for frame in [&second, &third] {
        assert!(deliver(&mut pair.responder, frame).unwrap().is_some());
    }
    let received = deliver(&mut pair.responder, &close).unwrap().unwrap();
    assert_eq!(received.message, Message::Close);
}

#[test]
fn a_peer_that_exceeds_the_frame_limit_ends_the_session() {
    // Record counter exhaustion, with a reduced limit. The peer ignores
    // the limit and sends a fifth frame.
    let mut pair = confirmed_with(limits(3600, 7200, 4, 1 << 30));
    for text in ["two", "three", "four"] {
        let frame = pair.initiator.seal_unchecked(&chat(text));
        assert!(deliver(&mut pair.responder, &frame).unwrap().is_some());
    }
    let fifth = pair.initiator.seal_unchecked(&chat("five"));
    assert_eq!(
        deliver(&mut pair.responder, &fifth).err(),
        Some(failed(ProtocolError::SessionExpired))
    );
    assert_eq!(pair.responder.state(), SessionState::Closed);
}

#[test]
fn the_byte_limit_ends_a_session_and_leaves_room_for_close() {
    // Room for four frames of one block: the ContactAccept, two more, and
    // the Close.
    let mut pair = confirmed_with(limits(3600, 7200, 1000, 4 * 1040));
    let second = pair.initiator.send(&chat("two"), start()).unwrap();
    assert!(!pair.initiator.limit_reached(start()));
    // A frame of two blocks does not fit with the Close after it.
    assert_eq!(
        pair.initiator.send(&chat(&"a".repeat(1500)), start()).err(),
        Some(SessionError::Expired)
    );
    let third = pair.initiator.send(&chat("three"), start()).unwrap();
    assert!(pair.initiator.limit_reached(start()));
    assert_eq!(
        pair.initiator.send(&chat("four"), start()).err(),
        Some(SessionError::Expired)
    );
    let close = pair.initiator.close().unwrap();
    for frame in [&second, &third, &close] {
        assert!(deliver(&mut pair.responder, frame).unwrap().is_some());
    }

    // A peer that goes past the limit.
    let mut pair = confirmed_with(limits(3600, 7200, 1000, 4 * 1040));
    for text in ["two", "three", "four"] {
        let frame = pair.initiator.seal_unchecked(&chat(text));
        assert!(deliver(&mut pair.responder, &frame).unwrap().is_some());
    }
    let fifth = pair.initiator.seal_unchecked(&chat("five"));
    assert_eq!(
        deliver(&mut pair.responder, &fifth).err(),
        Some(failed(ProtocolError::SessionExpired))
    );
}

#[test]
fn the_age_limit_ends_a_session() {
    let hour = Duration::from_secs(3600);
    let just_before = after(hour - Duration::from_millis(1));
    let mut pair = confirmed_with(limits(3600, 7200, 1000, 1 << 30));
    assert_eq!(pair.initiator.expires_at(), Some(after(hour)));

    let frame = pair.initiator.send(&chat("in time"), just_before).unwrap();
    assert!(
        pair.responder
            .receive(&frame, just_before)
            .unwrap()
            .1
            .is_some()
    );
    assert!(!pair.initiator.limit_reached(just_before));

    // At the limit nothing but Close is sent.
    assert!(pair.initiator.limit_reached(after(hour)));
    assert_eq!(
        pair.initiator.send(&chat("late"), after(hour)).err(),
        Some(SessionError::Expired)
    );
    // A frame that arrives at the limit ends the session at the receiver.
    let late = pair.initiator.seal_unchecked(&chat("late"));
    assert_eq!(
        pair.responder.receive(&late, after(hour)).err(),
        Some(failed(ProtocolError::SessionExpired))
    );
    assert!(pair.initiator.close().is_some());
}

#[test]
fn an_active_transfer_extends_the_age_limit_and_no_further() {
    let hour = Duration::from_secs(3600);
    let mut pair = confirmed_with(limits(3600, 7200, 1000, 1 << 30));
    pair.initiator.set_transfer_active(true);
    pair.responder.set_transfer_active(true);
    assert_eq!(pair.initiator.expires_at(), Some(after(2 * hour)));

    let frame = pair
        .initiator
        .send(&chat("still transferring"), after(hour))
        .unwrap();
    assert!(
        pair.responder
            .receive(&frame, after(hour))
            .unwrap()
            .1
            .is_some()
    );

    assert_eq!(
        pair.initiator.send(&chat("late"), after(2 * hour)).err(),
        Some(SessionError::Expired)
    );
    // When the transfer ends, the shorter limit applies again at once.
    pair.initiator.set_transfer_active(false);
    assert!(pair.initiator.limit_reached(after(hour)));
    assert_eq!(pair.initiator.expires_at(), Some(after(hour)));
}

#[test]
fn the_protocol_limits_are_the_default() {
    let pair = confirmed();
    let day = Duration::from_secs(24 * 3600);
    assert_eq!(pair.initiator.expires_at(), Some(after(day)));
    assert!(
        !pair
            .initiator
            .limit_reached(after(day - Duration::from_secs(1)))
    );
    assert!(pair.initiator.limit_reached(after(day)));

    // A party keeps the limits it was given.
    let reduced = limits(60, 120, 10, 1 << 20);
    let local = LocalParty::new(card(ALICE), transport_secret(ALICE))
        .unwrap()
        .with_limits(reduced);
    let (outbound, _, _) = handshake(&local, &party(BOB));
    let (session, _) = outbound.admit(Standing::Accepted).unwrap();
    assert_eq!(session.expires_at(), Some(after(Duration::from_secs(60))));
}

//! The responder's side of the handshake: whatever arrives on a stream, a
//! session exists afterwards only if the bytes were the two genuine
//! messages of an initiator that holds its keys, and nothing reaches the
//! application before that.
//!
//! Input: the first byte selects the size of the pieces the stream arrives
//! in, the second the mode.
//!
//! - Even mode: the rest is the stream as it is.
//! - Odd mode: the rest is a damage position (two bytes), a damage value
//!   and a tail. The stream is the genuine first and third message of the
//!   handshake between the two fixed parties, followed by the tail. If the
//!   damage value is not zero, one byte of the two messages is XORed with
//!   it. Undamaged, the handshake must complete; damaged, it must fail.
//!
//! What follows the handshake on the stream is fed to the session as
//! frames. Arbitrary bytes cannot be a frame; a seed holds a genuine one.

#![no_main]

mod session_fixtures;

use libfuzzer_sys::fuzz_target;
use monolith_protocol::session::{Action, PeerRecord};
use monolith_protocol::{MessageType, SessionState};
use session_fixtures::{ALICE, Pieces, TRANSCRIPT, bob_waiting, card, start};

fuzz_target!(|data: &[u8]| {
    let [piece, mode, rest @ ..] = data else {
        return;
    };
    let piece = usize::from(*piece) + 1;

    let mut genuine = Vec::new();
    genuine.extend_from_slice(&TRANSCRIPT.message_1);
    genuine.extend_from_slice(&TRANSCRIPT.message_3);

    let (stream, damaged) = if mode % 2 == 0 {
        (rest.to_vec(), !rest.starts_with(&genuine))
    } else {
        let [high, low, damage, tail @ ..] = rest else {
            return;
        };
        let mut stream = genuine.clone();
        if *damage != 0 {
            let position = usize::from(u16::from_be_bytes([*high, *low])) % stream.len();
            stream[position] ^= damage;
        }
        stream.extend_from_slice(tail);
        (stream, *damage != 0)
    };

    // The stream arrives in pieces. Each handshake message is read as
    // exactly its size.
    let mut pieces = Pieces::new(&stream, piece);
    let Some(message_1) = pieces.message::<48>() else {
        return;
    };
    let Ok((waiting, reply)) = bob_waiting().read_message_1(&message_1, start()) else {
        assert!(damaged);
        return;
    };
    // A first message that is accepted is the genuine one or a replay of
    // it: nobody else knows a key to make another. The reply is then the
    // genuine second message, since Bob's ephemeral key is fixed here.
    assert_eq!(message_1, TRANSCRIPT.message_1);
    assert_eq!(reply, TRANSCRIPT.message_2);

    let Some(message_3) = pieces.message::<235>() else {
        return;
    };
    let Ok(inbound) = waiting.read_message_3(&message_3, start()) else {
        assert!(damaged);
        return;
    };
    assert!(!damaged);
    assert_eq!(message_3, TRANSCRIPT.message_3);
    assert_eq!(inbound.card(), &card(ALICE));

    let (mut session, admission, first_actions) = inbound.admit(PeerRecord::None).unwrap();
    assert!(admission.card.is_none());
    assert!(first_actions.is_empty());
    assert_eq!(session.state(), SessionState::AuthenticatedUnknown);

    // The rest of the stream is frames for a session with a stranger.
    while let Some(mut pending) = pieces.rest() {
        while !pending.is_empty() {
            let before = session.state();
            match session.receive(pending, start()) {
                Ok((used, received)) => {
                    assert!(used > 0 && used <= pending.len());
                    pending = &pending[used..];
                    if let Some(received) = received {
                        assert_eq!(before, SessionState::AuthenticatedUnknown);
                        assert!(matches!(
                            received.message.message_type(),
                            MessageType::ContactRequest
                                | MessageType::ContactAccept
                                | MessageType::Close
                        ));
                        for action in &received.actions {
                            assert!(matches!(
                                action,
                                Action::SendClose | Action::ConsiderRequest | Action::Disconnect
                            ));
                        }
                    }
                }
                Err(_) => {
                    assert_eq!(session.state(), SessionState::Closed);
                    assert!(session.receive(&[0], start()).is_err());
                    return;
                }
            }
        }
    }
});

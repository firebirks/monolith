//! The initiator's side of the handshake: whatever the endpoint answers,
//! the initiator makes its third message, which carries its identity, only
//! for a second message that Noise accepts, and ends with a session for
//! the identity it dialed or with none. The genuine second message is
//! always accepted. Another one could be accepted too, from whoever holds
//! the responder's key and another ephemeral key; nothing here assumes
//! that only the stored transcript is valid.
//!
//! Input: the first byte selects the size of the pieces the stream arrives
//! in, the second the mode.
//!
//! - Even mode: the rest is the stream from the responder as it is.
//! - Odd mode: the rest is a damage position (two bytes), a damage value
//!   and a tail. The stream is the genuine second message of the handshake
//!   between the two fixed parties, followed by the tail. If the damage
//!   value is not zero, one byte of the message is XORed with it.
//!   Undamaged, the handshake must complete; damaged, it must fail.
//!
//! What follows the second message on the stream is fed to the session as
//! frames.

#![no_main]

mod session_fixtures;

use libfuzzer_sys::fuzz_target;
use monolith_protocol::SessionState;
use monolith_protocol::credential::Credentials;
use monolith_protocol::session::{Action, PeerRecord, Standing};
use session_fixtures::{BOB, Pieces, TRANSCRIPT, alice_dialing, card, start};

fuzz_target!(|data: &[u8]| {
    let [piece, mode, rest @ ..] = data else {
        return;
    };
    let piece = usize::from(*piece) + 1;

    let stream = if mode % 2 == 0 {
        rest.to_vec()
    } else {
        let [high, low, damage, tail @ ..] = rest else {
            return;
        };
        let mut stream = TRANSCRIPT.message_2.to_vec();
        if *damage != 0 {
            let position = usize::from(u16::from_be_bytes([*high, *low])) % stream.len();
            stream[position] ^= damage;
        }
        stream.extend_from_slice(tail);
        stream
    };

    let (alice, message_1) = alice_dialing();
    // The first message says nothing about who sends it and is the same
    // on every run.
    assert_eq!(message_1, TRANSCRIPT.message_1);

    let mut pieces = Pieces::new(&stream, piece);
    let Some(message_2) = pieces.message::<48>() else {
        return;
    };

    let genuine = message_2 == TRANSCRIPT.message_2;
    let Ok((outbound, message_3)) = alice.read_message_2(&message_2, start()) else {
        // Nothing of Alice was sent: the third message was never made. The
        // genuine second message is never refused.
        assert!(!genuine);
        return;
    };
    // Whoever answered holds the key of the card that was dialed: the
    // session can only be one with Bob.
    assert_eq!(outbound.card(), &card(BOB));
    if genuine {
        // With both ephemeral keys fixed, the known answer.
        assert_eq!(message_3, TRANSCRIPT.message_3);
    }

    let (mut session, admission, first_actions) = outbound
        .admit(PeerRecord::Accepted(&mut Credentials::new(card(BOB))))
        .unwrap();
    assert_eq!(admission.standing, Standing::Accepted);
    assert_eq!(first_actions, [Action::SendContactAccept]);
    assert_eq!(session.peer(), card(BOB).identity());

    while let Some(mut pending) = pieces.rest() {
        while !pending.is_empty() {
            let before = session.state();
            match session.receive(pending, start()) {
                Ok((used, received)) => {
                    assert!(used > 0 && used <= pending.len());
                    pending = &pending[used..];
                    if let Some(received) = received {
                        // Application data is delivered on a confirmed
                        // session and nowhere else.
                        if received.actions.contains(&Action::Deliver) {
                            assert_eq!(before, SessionState::AuthenticatedContact);
                        }
                        assert_eq!(session.peer(), card(BOB).identity());
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

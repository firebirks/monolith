//! The responder's side of the handshake: whatever arrives on a stream, a
//! session exists afterwards only if Noise accepted a first and a third
//! message, and its peer is the identity of a valid card whose key Noise
//! authenticated; nothing reaches the application before that session is
//! confirmed. The genuine messages are always accepted. Others could be
//! too: anyone who knows the responder's card can make a first message
//! with an ephemeral key of its own, and a third with a key and card of
//! its own. Nothing here assumes that only the stored transcript is
//! valid.
//!
//! Input: the first byte selects what the responder holds about the
//! initiator and the size of the pieces the stream arrives in, the second
//! the mode.
//!
//! - First byte: the two high bits are the record (0 none, 1 blocked,
//!   2 requested, 3 accepted), the low six bits the piece size less one.
//! - Even mode: the rest is the stream as it is.
//! - Odd mode: the rest is a damage position (two bytes), a damage value
//!   and a tail. The stream is the genuine first and third message of the
//!   handshake between the two fixed parties, followed by the tail. If the
//!   damage value is not zero, one byte of the two messages is XORed with
//!   it. Undamaged, the handshake must complete; damaged, it must fail.
//!
//! The handshake does not look at the record: it completes or fails the
//! same way for all four. What follows the handshake on the stream is fed
//! to the session as frames, and every message that comes out is compared
//! with what the record allows (`docs/PROTOCOL.md` sections 6.2 to 6.4),
//! written out here a second time. Arbitrary bytes cannot be a frame; the
//! seeds hold genuine ones.

#![no_main]

mod session_fixtures;

use libfuzzer_sys::fuzz_target;
use monolith_protocol::credential::{CredentialChange, Credentials};
use monolith_protocol::session::{Action, PeerRecord, Standing};
use monolith_protocol::{MessageType, SessionState};
use session_fixtures::{ALICE, BOB, Pieces, TRANSCRIPT, bob_waiting, card, start};

/// What Bob holds about Alice.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Held {
    Nothing,
    Blocked,
    Requested,
    Accepted,
}

fuzz_target!(|data: &[u8]| {
    let [first, mode, rest @ ..] = data else {
        return;
    };
    let held = match first >> 6 {
        0 => Held::Nothing,
        1 => Held::Blocked,
        2 => Held::Requested,
        _ => Held::Accepted,
    };
    let piece = usize::from(first & 0x3F) + 1;

    let mut genuine = Vec::new();
    genuine.extend_from_slice(&TRANSCRIPT.message_1);
    genuine.extend_from_slice(&TRANSCRIPT.message_3);

    // Which genuine message had a byte changed on purpose. Every byte of
    // both is authenticated, so that message must then be refused.
    let (mut corrupted_1, mut corrupted_3) = (false, false);
    let stream = if mode % 2 == 0 {
        rest.to_vec()
    } else {
        let [high, low, damage, tail @ ..] = rest else {
            return;
        };
        let mut stream = genuine.clone();
        if *damage != 0 {
            let position = usize::from(u16::from_be_bytes([*high, *low])) % stream.len();
            stream[position] ^= damage;
            if position < TRANSCRIPT.message_1.len() {
                corrupted_1 = true;
            } else {
                corrupted_3 = true;
            }
        }
        stream.extend_from_slice(tail);
        stream
    };

    // The stream arrives in pieces. Each handshake message is read as
    // exactly its size.
    let mut pieces = Pieces::new(&stream, piece);
    let Some(message_1) = pieces.message::<48>() else {
        return;
    };
    let genuine_1 = message_1 == TRANSCRIPT.message_1;
    let Ok((waiting, reply)) = bob_waiting().read_message_1(&message_1, start()) else {
        // The genuine first message is never refused.
        assert!(!genuine_1);
        return;
    };
    // A first message of another ephemeral key may be accepted; the
    // genuine one with a byte changed may not.
    assert!(!corrupted_1);
    if genuine_1 {
        // Bob's ephemeral key is fixed here, so the reply to the genuine
        // first message is the known answer.
        assert_eq!(reply, TRANSCRIPT.message_2);
    }

    let Some(message_3) = pieces.message::<235>() else {
        return;
    };
    let genuine = genuine_1 && message_3 == TRANSCRIPT.message_3;
    let Ok(inbound) = waiting.read_message_3(&message_3, start()) else {
        // The genuine third message, after the genuine first, is never
        // refused.
        assert!(!genuine);
        return;
    };
    // The same for the third message: another initiator's may be
    // accepted, the genuine one with a byte changed may not.
    assert!(!corrupted_3);
    // Whoever it is, its card is valid, carries no capability and is not
    // Bob's identity.
    assert!(inbound.card().invitation().is_none());
    assert_ne!(inbound.card().identity(), card(BOB).identity());
    if genuine {
        assert_eq!(inbound.card(), &card(ALICE));
    }

    // Only now does the record play a part. The peer presented the card
    // that Bob holds of it, if he holds one.
    let alice = inbound.card().clone();
    let mut credentials = Credentials::new(alice.clone());
    let record = match held {
        Held::Nothing => PeerRecord::None,
        Held::Blocked => PeerRecord::Blocked,
        Held::Requested => PeerRecord::Requested(&mut credentials),
        Held::Accepted => PeerRecord::Accepted(&mut credentials),
    };
    let (mut session, admission, first_actions) = inbound.admit(record).unwrap();
    let (standing, change, first): (_, _, &[Action]) = match held {
        Held::Nothing => (Standing::None, None, &[]),
        Held::Blocked => (Standing::Blocked, None, &[]),
        Held::Requested => (
            Standing::Requested,
            Some(CredentialChange::Unchanged),
            &[Action::SendContactRequest],
        ),
        Held::Accepted => (
            Standing::Accepted,
            Some(CredentialChange::Unchanged),
            &[Action::SendContactAccept],
        ),
    };
    assert_eq!(admission.standing, standing);
    assert_eq!(admission.change, change);
    assert_eq!(first_actions, first);
    assert_eq!(session.state(), SessionState::AuthenticatedUnknown);
    assert_eq!(session.peer(), alice.identity());

    let contact = matches!(held, Held::Requested | Held::Accepted);
    // A requested contact is recorded as accepted once, by the first
    // request or acceptance that arrives from it.
    let mut to_mark = held == Held::Requested;

    // The rest of the stream is frames for that session.
    while let Some(mut pending) = pieces.rest() {
        while !pending.is_empty() {
            let before = session.state();
            match session.receive(pending, start()) {
                Ok((used, received)) => {
                    assert!(used > 0 && used <= pending.len());
                    pending = &pending[used..];
                    let Some(received) = received else {
                        continue;
                    };
                    let message_type = received.message.message_type();
                    let mut expected = Vec::new();
                    match (before, message_type) {
                        (
                            SessionState::AuthenticatedUnknown | SessionState::AuthenticatedContact,
                            MessageType::Close,
                        ) => expected.push(Action::Disconnect),
                        (
                            SessionState::AuthenticatedUnknown,
                            MessageType::ContactRequest | MessageType::ContactAccept,
                        ) => {
                            let request = message_type == MessageType::ContactRequest;
                            if contact {
                                if to_mark {
                                    expected.push(Action::MarkAccepted);
                                    expected.push(Action::SendContactAccept);
                                    to_mark = false;
                                }
                                if !request {
                                    expected.push(Action::Confirmed);
                                }
                            } else {
                                // A stranger gets a Close whatever it
                                // sent. Only a request from an identity
                                // without a record is looked at, later.
                                expected.push(Action::SendClose);
                                if request && held == Held::Nothing {
                                    expected.push(Action::ConsiderRequest);
                                }
                            }
                        }
                        // After confirmation these change nothing.
                        (
                            SessionState::AuthenticatedContact,
                            MessageType::ContactRequest | MessageType::ContactAccept,
                        ) => {}
                        // Application data, on a confirmed session only.
                        (SessionState::AuthenticatedContact, _) => expected.push(Action::Deliver),
                        _ => panic!("{message_type:?} taken in {before:?}"),
                    }
                    assert_eq!(received.actions, expected);

                    // Where the session is afterwards.
                    let after = if expected.contains(&Action::Disconnect) {
                        SessionState::Closed
                    } else if expected.contains(&Action::SendClose) {
                        SessionState::Closing
                    } else if expected.contains(&Action::Confirmed) {
                        SessionState::AuthenticatedContact
                    } else {
                        before
                    };
                    assert_eq!(session.state(), after);
                    // Only a contact reaches a confirmed session.
                    assert!(contact || after != SessionState::AuthenticatedContact);
                    assert_eq!(session.peer(), alice.identity());
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

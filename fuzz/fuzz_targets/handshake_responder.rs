//! The responder's side of the handshake: whatever arrives on a stream, a
//! session exists afterwards only if the bytes were the two genuine
//! messages of an initiator that holds its keys, and nothing reaches the
//! application before that session is confirmed.
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
use monolith_protocol::card::CardChange;
use monolith_protocol::session::{Action, PeerRecord, Standing};
use monolith_protocol::{MessageType, SessionState};
use session_fixtures::{ALICE, Pieces, TRANSCRIPT, bob_waiting, card, start};

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

    // Only now does the record play a part. Alice presented the card that
    // Bob holds of her, if he holds one.
    let alice = card(ALICE);
    let record = match held {
        Held::Nothing => PeerRecord::None,
        Held::Blocked => PeerRecord::Blocked,
        Held::Requested => PeerRecord::Requested(&alice),
        Held::Accepted => PeerRecord::Accepted(&alice),
    };
    let (mut session, admission, first_actions) = inbound.admit(record).unwrap();
    let (standing, change, first): (_, _, &[Action]) = match held {
        Held::Nothing => (Standing::None, None, &[]),
        Held::Blocked => (Standing::Blocked, None, &[]),
        Held::Requested => (
            Standing::Requested,
            Some(CardChange::Unchanged),
            &[Action::SendContactRequest],
        ),
        Held::Accepted => (
            Standing::Accepted,
            Some(CardChange::Unchanged),
            &[Action::SendContactAccept],
        ),
    };
    assert_eq!(admission.standing, standing);
    assert_eq!(admission.card, change);
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

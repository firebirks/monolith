//! Encrypted frames on a confirmed session: a message is delivered only if
//! the peer sent it, once, in the order it was sent, and nothing is
//! delivered from a stream that was dropped from, added to or changed.
//!
//! Two sessions between the fixed parties are connected and confirmed.
//! The input is a list of operations on them. Its first byte selects the
//! size of the pieces in which frames are delivered. Then every operation
//! is one byte, followed by the arguments it takes:
//!
//! | Code | Operation | Arguments |
//! | --- | --- | --- |
//! | 0, 1 | Alice or Bob sends a chat message | length selector |
//! | 2, 3 | the next frame to Bob or to Alice is delivered | |
//! | 4, 5 | the next frame to Bob or to Alice is delivered with one byte changed | position (two bytes), value |
//! | 6, 7 | the next frame to Bob or to Alice is dropped | |
//! | 8, 9 | the next frame to Bob or to Alice is delivered twice | |
//! | 10, 11 | bytes that nobody sent arrive at Bob or at Alice | length, then that many bytes |
//! | 12, 13 | Alice or Bob closes the session | |
//!
//! The target keeps its own account of what was sent and of which
//! direction was interfered with, and compares every delivery with it.

#![no_main]

mod session_fixtures;

use std::collections::VecDeque;

use libfuzzer_sys::fuzz_target;
use monolith_protocol::SessionState;
use monolith_protocol::body::{Message, MessageId};
use monolith_protocol::session::{Action, PeerRecord};
use monolith_protocol::text::ChatText;
use monolith_session::{AuthenticatedSession, SessionError};
use session_fixtures::{ALICE, BOB, card, handshake, start};

/// One direction of the stream, as the target sees it.
#[derive(Default)]
struct Direction {
    /// Frames that were sent and have not arrived.
    in_flight: VecDeque<Vec<u8>>,
    /// The messages in the frames that were sent, in order.
    sent: Vec<Message>,
    /// How many of them the receiver delivered.
    delivered: usize,
    /// Something happened to the stream: a frame was dropped, repeated or
    /// changed, or foreign bytes arrived. Nothing may be delivered from
    /// this direction afterwards.
    interfered: bool,
}

struct World {
    alice: AuthenticatedSession,
    bob: AuthenticatedSession,
    to_bob: Direction,
    to_alice: Direction,
    piece: usize,
}

fn rank(state: SessionState) -> u8 {
    match state {
        SessionState::Connecting => 0,
        SessionState::CryptoHandshake => 1,
        SessionState::IdentityAuth => 2,
        SessionState::AuthenticatedUnknown => 3,
        SessionState::AuthenticatedContact => 4,
        SessionState::Closing => 5,
        SessionState::Closed => 6,
    }
}

impl World {
    fn new(piece: usize) -> Self {
        let (outbound, inbound, _) = handshake();
        let (mut alice, _, _) = outbound.admit(PeerRecord::Accepted(&card(BOB))).unwrap();
        let (mut bob, _, _) = inbound.admit(PeerRecord::Accepted(&card(ALICE))).unwrap();
        let from_alice = alice.send(&Message::ContactAccept, start()).unwrap();
        let from_bob = bob.send(&Message::ContactAccept, start()).unwrap();
        assert!(bob.receive(&from_alice, start()).unwrap().1.is_some());
        assert!(alice.receive(&from_bob, start()).unwrap().1.is_some());
        assert_eq!(alice.state(), SessionState::AuthenticatedContact);
        assert_eq!(bob.state(), SessionState::AuthenticatedContact);
        Self {
            alice,
            bob,
            to_bob: Direction::default(),
            to_alice: Direction::default(),
            piece,
        }
    }

    /// The sender, the receiver and the direction between them.
    fn parts(
        &mut self,
        to_alice: bool,
    ) -> (
        &mut AuthenticatedSession,
        &mut AuthenticatedSession,
        &mut Direction,
    ) {
        if to_alice {
            (&mut self.bob, &mut self.alice, &mut self.to_alice)
        } else {
            (&mut self.alice, &mut self.bob, &mut self.to_bob)
        }
    }

    fn send(&mut self, from_bob: bool, selector: u8) {
        let (sender, _, direction) = self.parts(from_bob);
        // Bodies of one to three padding blocks.
        let len = 1 + usize::from(selector) * 11;
        let message = Message::ChatMessage {
            id: MessageId::from_bytes([selector; 16]),
            text: ChatText::new(&"m".repeat(len)).unwrap(),
        };
        match sender.send(&message, start()) {
            Ok(frame) => {
                direction.in_flight.push_back(frame);
                direction.sent.push(message);
            }
            // After a Close, or after the session failed.
            Err(error) => assert!(matches!(
                error,
                SessionError::Closed | SessionError::NotPermitted
            )),
        }
    }

    fn close(&mut self, bob: bool) {
        let (sender, _, direction) = self.parts(bob);
        if let Some(frame) = sender.close() {
            assert_eq!(frame.len(), 1042);
            direction.in_flight.push_back(frame);
            direction.sent.push(Message::Close);
        }
        assert!(sender.close().is_none());
        assert!(rank(sender.state()) >= rank(SessionState::Closing));
    }

    /// Gives bytes to the receiver of a direction and checks what comes
    /// out against the account of that direction.
    fn arrive(&mut self, to_alice: bool, bytes: &[u8]) {
        let piece = self.piece;
        let (_, receiver, direction) = self.parts(to_alice);
        for chunk in bytes.chunks(piece) {
            let mut rest = chunk;
            while !rest.is_empty() {
                let before = receiver.state();
                let result = receiver.receive(rest, start());
                assert!(rank(receiver.state()) >= rank(before));
                match result {
                    Ok((used, received)) => {
                        assert!(used > 0 && used <= rest.len());
                        rest = &rest[used..];
                        let Some(received) = received else {
                            continue;
                        };
                        // Delivered: then the stream was not interfered
                        // with, and this is the next message that was
                        // sent, once.
                        assert!(!direction.interfered);
                        assert_eq!(before, SessionState::AuthenticatedContact);
                        assert_eq!(
                            Some(&received.message),
                            direction.sent.get(direction.delivered)
                        );
                        direction.delivered += 1;
                        let expected = if received.message == Message::Close {
                            Action::Disconnect
                        } else {
                            Action::Deliver
                        };
                        assert_eq!(received.actions, [expected]);
                    }
                    Err(error) => {
                        // A violation is possible only on a stream that
                        // was interfered with: both ends follow the rules.
                        // A session that failed earlier reports that it
                        // is closed.
                        assert!(direction.interfered);
                        assert!(matches!(
                            error,
                            SessionError::Protocol(_) | SessionError::Closed
                        ));
                        assert_eq!(receiver.state(), SessionState::Closed);
                        assert_eq!(
                            receiver.receive(&[0], start()).err(),
                            Some(SessionError::Closed)
                        );
                        return;
                    }
                }
            }
        }
    }

    fn deliver(&mut self, to_alice: bool) {
        let (_, _, direction) = self.parts(to_alice);
        if let Some(frame) = direction.in_flight.pop_front() {
            self.arrive(to_alice, &frame);
        }
    }

    fn tamper(&mut self, to_alice: bool, position: usize, value: u8) {
        let (_, _, direction) = self.parts(to_alice);
        if let Some(mut frame) = direction.in_flight.pop_front() {
            let index = position % frame.len();
            // Never zero, so the frame always changes.
            frame[index] ^= value | 1;
            direction.interfered = true;
            self.arrive(to_alice, &frame);
        }
    }

    fn drop_frame(&mut self, to_alice: bool) {
        let (_, _, direction) = self.parts(to_alice);
        if direction.in_flight.pop_front().is_some() {
            direction.interfered = true;
        }
    }

    fn repeat(&mut self, to_alice: bool) {
        let (_, _, direction) = self.parts(to_alice);
        if let Some(frame) = direction.in_flight.pop_front() {
            self.arrive(to_alice, &frame);
            let (_, _, direction) = self.parts(to_alice);
            direction.interfered = true;
            self.arrive(to_alice, &frame);
        }
    }

    fn inject(&mut self, to_alice: bool, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let (_, _, direction) = self.parts(to_alice);
        direction.interfered = true;
        self.arrive(to_alice, bytes);
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((piece, mut rest)) = data.split_first() else {
        return;
    };
    let mut world = World::new(usize::from(*piece) * 17 + 1);

    while let Some((operation, tail)) = rest.split_first() {
        rest = tail;
        let second = operation % 2 == 1;
        match (operation % 14) / 2 {
            0 => {
                let Some((selector, tail)) = rest.split_first() else {
                    return;
                };
                rest = tail;
                world.send(second, *selector);
            }
            1 => world.deliver(second),
            2 => {
                let [high, low, value, tail @ ..] = rest else {
                    return;
                };
                rest = tail;
                world.tamper(
                    second,
                    usize::from(u16::from_be_bytes([*high, *low])),
                    *value,
                );
            }
            3 => world.drop_frame(second),
            4 => world.repeat(second),
            5 => {
                let Some((len, tail)) = rest.split_first() else {
                    return;
                };
                let len = usize::from(*len).min(tail.len());
                let (bytes, tail) = tail.split_at(len);
                rest = tail;
                world.inject(second, bytes);
            }
            _ => world.close(second),
        }
    }

    // In a direction nobody interfered with, what was delivered is exactly
    // the front of what was sent.
    for direction in [&world.to_bob, &world.to_alice] {
        assert!(direction.delivered <= direction.sent.len());
    }
});

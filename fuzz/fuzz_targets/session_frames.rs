//! Encrypted frames on a confirmed session: a message is delivered only if
//! the peer sent it, once, in the order it was sent, and nothing is
//! delivered from a stream that was dropped from, added to or changed, on
//! a session that has ended, or after the age limit and its grace.
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
//! | 14, 15 | time passes: hours, or seconds | how many |
//! | 16, 17 | Alice or Bob blocks the peer | |
//! | 18, 19 | Alice or Bob removes the contact | |
//! | 20, 21 | the stream of Alice or of Bob is reported closed | |
//! | 22, 23 | Alice or Bob withdraws the session: the peer's key was retired | |
//!
//! The target keeps its own account of what was sent, of which direction
//! was interfered with and of the time that has passed, and compares every
//! result with it. The age limit and the grace are written out here and
//! not taken from the code.

#![no_main]

mod session_fixtures;

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use libfuzzer_sys::fuzz_target;
use monolith_protocol::body::{Message, MessageId};
use monolith_protocol::credential::Credentials;
use monolith_protocol::session::{Action, PeerRecord, Standing};
use monolith_protocol::text::ChatText;
use monolith_protocol::{ProtocolError, SessionState};
use monolith_session::{AuthenticatedSession, SessionError};
use session_fixtures::{ALICE, BOB, card, handshake, start};

/// The age at which a session sends nothing but Close
/// (`docs/PROTOCOL.md` section 7.1).
const AGE_LIMIT: Duration = Duration::from_secs(24 * 60 * 60);

/// How long after the age limit a frame of the peer is still taken.
const CLOSE_GRACE: Duration = Duration::from_secs(120);

/// The clock of the target stops here, far past every limit.
const CLOCK_END: Duration = Duration::from_secs(30 * 24 * 60 * 60);

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
    /// The receiver of this direction reported a violation. Its session
    /// is over.
    failed: bool,
}

/// The ways the local side ends a session on purpose.
#[derive(Clone, Copy)]
enum End {
    Close,
    Block,
    Remove,
    Withdraw,
}

struct World {
    alice: AuthenticatedSession,
    bob: AuthenticatedSession,
    to_bob: Direction,
    to_alice: Direction,
    piece: usize,
    /// The time that has passed since both sessions were established.
    elapsed: Duration,
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
        let (mut alice, _, _) = outbound
            .admit(PeerRecord::Accepted(&mut Credentials::new(card(BOB))))
            .unwrap();
        let (mut bob, _, _) = inbound
            .admit(PeerRecord::Accepted(&mut Credentials::new(card(ALICE))))
            .unwrap();
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
            elapsed: Duration::ZERO,
        }
    }

    fn now(&self) -> Instant {
        start() + self.elapsed
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
        let now = self.now();
        let aged = self.elapsed >= AGE_LIMIT;
        let (sender, _, direction) = self.parts(from_bob);
        // Bodies of one to three padding blocks.
        let len = 1 + usize::from(selector) * 11;
        let message = Message::ChatMessage {
            id: MessageId::from_bytes([selector; 16]),
            text: ChatText::new(&"m".repeat(len)).unwrap(),
        };
        match sender.send(&message, now) {
            Ok(frame) => {
                // Nothing but Close is sent at the age limit.
                assert!(!aged);
                direction.in_flight.push_back(frame);
                direction.sent.push(message);
            }
            // The session is as it was and refuses for its age alone.
            Err(SessionError::Expired) => assert!(aged),
            // After a Close, or after the session failed.
            Err(error) => assert!(matches!(
                error,
                SessionError::Closed | SessionError::NotPermitted
            )),
        }
    }

    /// One side ends its session on purpose. The peer sees the same Close
    /// whatever the reason was.
    fn end(&mut self, bob: bool, how: End) {
        let (sender, _, direction) = self.parts(bob);
        let state = sender.state();
        let standing = sender.standing();
        let frame = match how {
            End::Close => sender.close(),
            End::Block => sender.block_peer(),
            End::Remove => sender.remove_contact(),
            End::Withdraw => sender.withdraw(),
        };
        // A Close is produced exactly when the session was still open,
        // also past the age limit.
        assert_eq!(frame.is_some(), state == SessionState::AuthenticatedContact);
        if let Some(frame) = frame {
            assert_eq!(frame.len(), 1042);
            direction.in_flight.push_back(frame);
            direction.sent.push(Message::Close);
        }
        assert!(sender.close().is_none());
        assert!(rank(sender.state()) >= rank(SessionState::Closing));
        assert!(rank(sender.state()) >= rank(state));
        let expected = match how {
            End::Close => standing,
            End::Block => Standing::Blocked,
            End::Remove => Standing::None,
            End::Withdraw => Standing::StaleCard,
        };
        assert_eq!(sender.standing(), expected);
    }

    /// The stream of one side is gone. Its session is over without a
    /// Close, and nothing is sent on it afterwards.
    fn stream_closed(&mut self, bob: bool) {
        let now = self.now();
        let (session, _, _) = self.parts(bob);
        session.stream_closed();
        assert_eq!(session.state(), SessionState::Closed);
        assert!(session.close().is_none());
        assert_eq!(
            session.send(&Message::Ping([0; 8]), now).err(),
            Some(SessionError::Closed)
        );
    }

    /// Time passes. That alone changes no state; a session that is open
    /// reports that its limit is reached from the age limit on.
    fn advance(&mut self, step: Duration) {
        self.elapsed = self.elapsed.saturating_add(step).min(CLOCK_END);
        let now = self.now();
        let aged = self.elapsed >= AGE_LIMIT;
        for session in [&self.alice, &self.bob] {
            assert_eq!(session.limit_reached(now), aged);
        }
    }

    /// Gives bytes to the receiver of a direction and checks what comes
    /// out against the account of that direction.
    fn arrive(&mut self, to_alice: bool, bytes: &[u8]) {
        let piece = self.piece;
        let now = self.now();
        let overdue = self.elapsed >= AGE_LIMIT + CLOSE_GRACE;
        let (_, receiver, direction) = self.parts(to_alice);
        for chunk in bytes.chunks(piece) {
            let mut rest = chunk;
            while !rest.is_empty() {
                let before = receiver.state();
                let result = receiver.receive(rest, now);
                assert!(rank(receiver.state()) >= rank(before));
                match result {
                    Ok((used, received)) => {
                        assert!(used > 0 && used <= rest.len());
                        rest = &rest[used..];
                        let Some(received) = received else {
                            continue;
                        };
                        // Delivered: then the stream was not interfered
                        // with, the frame arrived in time, and this is the
                        // next message that was sent, once.
                        assert!(!direction.interfered);
                        assert!(!direction.failed);
                        assert!(!overdue);
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
                        // Both ends follow the rules, so a violation has
                        // one of two causes: a frame that arrived after
                        // the age limit and its grace, or a stream that
                        // was interfered with. A session that failed
                        // earlier reports that it is closed.
                        match error {
                            SessionError::Closed => assert!(direction.failed),
                            SessionError::Protocol(ProtocolError::SessionExpired) => {
                                assert!(overdue);
                                assert!(!direction.failed);
                            }
                            SessionError::Protocol(_) => {
                                assert!(direction.interfered);
                                assert!(!direction.failed);
                            }
                            other => panic!("{other:?}"),
                        }
                        direction.failed = true;
                        assert_eq!(receiver.state(), SessionState::Closed);
                        assert_eq!(
                            receiver.receive(&[0], now).err(),
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
        match (operation % 24) / 2 {
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
            6 => world.end(second, End::Close),
            7 => {
                let Some((amount, tail)) = rest.split_first() else {
                    return;
                };
                rest = tail;
                let seconds = if second {
                    u64::from(*amount)
                } else {
                    u64::from(*amount) * 60 * 60
                };
                world.advance(Duration::from_secs(seconds));
            }
            8 => world.end(second, End::Block),
            9 => world.end(second, End::Remove),
            10 => world.stream_closed(second),
            _ => world.end(second, End::Withdraw),
        }
    }
});

//! The deadlines of a session: a frame has to complete, the peer has to
//! send something, a session leaves `AuthenticatedUnknown` and ends at its
//! age limit, and a peer that sends a frame a byte at a time moves none of
//! that.

use core::time::Duration;

use monolith_protocol::SessionState;
use monolith_protocol::body::Message;
use monolith_protocol::credential::Credentials;
use monolith_protocol::limits::{
    FRAME_READ_TIMEOUT, IDLE_TIMEOUT, UNKNOWN_FIRST_MESSAGE_TIMEOUT, UNKNOWN_SESSION_TIMEOUT,
};
use monolith_protocol::session::{Action, PeerRecord, Standing};

use crate::session::SessionLimits;
use crate::testing::{
    ALICE, BOB, after, card, chat, confirmed, connect, deliver, handshake, party, start,
};
use crate::{AuthenticatedSession, Expiry, SessionError};

fn secs(value: u64) -> Duration {
    Duration::from_secs(value)
}

/// A chat message from the initiator, as bytes for the responder.
fn frame_from(session: &mut AuthenticatedSession) -> Vec<u8> {
    session.send(&chat("hello"), start()).unwrap()
}

fn assert_over(session: &mut AuthenticatedSession) {
    assert!(session.is_over());
    assert_eq!(session.deadline(), None);
    // Nothing is delivered any more: a failed session refuses input, a
    // closed one drops it.
    assert!(matches!(
        session.receive(&[0; 4], start()),
        Ok((_, None)) | Err(SessionError::Closed)
    ));
    assert!(session.send(&chat("late"), start()).is_err());
}

#[test]
fn a_confirmed_session_waits_for_the_idle_limit() {
    let mut pair = confirmed();
    assert_eq!(pair.responder.deadline(), Some(after(IDLE_TIMEOUT)));
    assert_eq!(
        pair.responder.expire(after(IDLE_TIMEOUT - secs(1))),
        Expiry::Running
    );
    // A complete frame moves the idle limit.
    let frame = frame_from(&mut pair.initiator);
    let (used, received) = pair.responder.receive(&frame, after(secs(100))).unwrap();
    assert_eq!(used, frame.len());
    assert!(received.is_some());
    assert_eq!(
        pair.responder.deadline(),
        Some(after(secs(100) + IDLE_TIMEOUT))
    );
    // Then nothing: the session ends without a Close.
    assert_eq!(
        pair.responder.expire(after(secs(100) + IDLE_TIMEOUT)),
        Expiry::Silent
    );
    assert_over(&mut pair.responder);
}

#[test]
fn a_frame_sent_a_byte_at_a_time_does_not_hold_the_session() {
    // The peer sends one byte of a frame, and another before every 59
    // seconds pass. The frame has to complete within FRAME_READ_TIMEOUT of
    // its first byte, and the bytes that follow do not move that.
    let mut pair = confirmed();
    let frame = frame_from(&mut pair.initiator);
    let begun = after(secs(10));
    assert_eq!(pair.responder.receive(&frame[..1], begun), Ok((1, None)));
    assert_eq!(pair.responder.deadline(), Some(begun + FRAME_READ_TIMEOUT));
    for (index, byte) in frame[1..3].iter().enumerate() {
        let now = begun + secs(59) * u32::try_from(index + 1).unwrap() / 2;
        assert_eq!(
            pair.responder.receive(core::slice::from_ref(byte), now),
            Ok((1, None))
        );
        assert_eq!(pair.responder.deadline(), Some(begun + FRAME_READ_TIMEOUT));
    }
    assert_eq!(
        pair.responder.expire(begun + FRAME_READ_TIMEOUT - secs(1)),
        Expiry::Running
    );
    assert_eq!(
        pair.responder.expire(begun + FRAME_READ_TIMEOUT),
        Expiry::Silent
    );
    assert_over(&mut pair.responder);
}

#[test]
fn bytes_of_an_incomplete_frame_do_not_move_the_idle_limit() {
    // A frame begins a second before the idle limit. Its first bytes do
    // not move the limit, which comes before the frame deadline.
    let mut pair = confirmed();
    let frame = frame_from(&mut pair.initiator);
    let begun = after(IDLE_TIMEOUT - secs(1));
    assert_eq!(pair.responder.receive(&frame[..2], begun), Ok((2, None)));
    assert_eq!(pair.responder.deadline(), Some(after(IDLE_TIMEOUT)));
    assert_eq!(pair.responder.expire(after(IDLE_TIMEOUT)), Expiry::Silent);
    assert_over(&mut pair.responder);
}

#[test]
fn a_frame_that_completes_in_time_clears_its_deadline() {
    let mut pair = confirmed();
    let frame = frame_from(&mut pair.initiator);
    let begun = after(secs(10));
    pair.responder.receive(&frame[..1], begun).unwrap();
    let (_, received) = pair
        .responder
        .receive(&frame[1..], begun + secs(30))
        .unwrap();
    assert!(received.is_some());
    assert_eq!(
        pair.responder.deadline(),
        Some(begun + secs(30) + IDLE_TIMEOUT)
    );
}

#[test]
fn a_peer_that_is_not_a_contact_has_to_send_its_first_message_in_time() {
    // Bob holds no record of Alice. She is authenticated and sends nothing.
    let pair = connect(Standing::None, PeerRecord::None);
    let mut bob = pair.responder;
    assert_eq!(bob.state(), SessionState::AuthenticatedUnknown);
    assert_eq!(bob.deadline(), Some(after(UNKNOWN_FIRST_MESSAGE_TIMEOUT)));
    let Expiry::Close(Some(close)) = bob.expire(after(UNKNOWN_FIRST_MESSAGE_TIMEOUT)) else {
        panic!("no Close");
    };
    assert_over(&mut bob);
    // Alice sees an ordinary Close.
    let mut alice = pair.initiator;
    let received = deliver(&mut alice, &close).unwrap().unwrap();
    assert_eq!(received.message, Message::Close);
}

#[test]
fn the_close_after_a_strangers_first_message_is_due_at_once() {
    // Bob holds no record of Alice. Her request gets the Close the session
    // logic decides; if the caller does not close, the session ends with
    // that Close at its next deadline, which is the moment of the request.
    let mut pair = connect(Standing::Requested, PeerRecord::None);
    let request = crate::testing::request_with(card(ALICE), None);
    let frame = pair.initiator.send(&request, start()).unwrap();
    let (_, received) = pair.responder.receive(&frame, after(secs(3))).unwrap();
    assert!(received.unwrap().actions.contains(&Action::SendClose));
    assert_eq!(pair.responder.state(), SessionState::Closing);
    assert_eq!(pair.responder.deadline(), Some(after(secs(3))));
    let Expiry::Close(Some(close)) = pair.responder.expire(after(secs(3))) else {
        panic!("no Close");
    };
    assert_over(&mut pair.responder);
    let received = deliver(&mut pair.initiator, &close).unwrap().unwrap();
    assert_eq!(received.message, Message::Close);
}

#[test]
fn a_session_leaves_authenticated_unknown_in_time() {
    // Bob holds Alice as accepted; she never sends her ContactAccept, only
    // a request, which keeps the session unconfirmed.
    let mut held = Credentials::new(card(ALICE));
    let mut pair = connect(Standing::Requested, PeerRecord::Accepted(&mut held));
    assert_eq!(
        pair.responder.deadline(),
        Some(after(UNKNOWN_SESSION_TIMEOUT))
    );
    let request = crate::testing::request_with(card(ALICE), None);
    let frame = pair.initiator.send(&request, start()).unwrap();
    deliver(&mut pair.responder, &frame).unwrap();
    assert_eq!(pair.responder.state(), SessionState::AuthenticatedUnknown);
    assert_eq!(
        pair.responder.deadline(),
        Some(after(UNKNOWN_SESSION_TIMEOUT))
    );
    let Expiry::Close(Some(_)) = pair.responder.expire(after(UNKNOWN_SESSION_TIMEOUT)) else {
        panic!("no Close");
    };
    assert_over(&mut pair.responder);
}

#[test]
fn the_age_limit_ends_a_session_that_only_receives_part_of_a_frame() {
    // Limits of a minute, so that the age limit comes before the idle one.
    let limits = SessionLimits::reduced(secs(60), secs(120), 1000, 1 << 20);
    let alice = party(ALICE).with_limits(limits);
    let bob = party(BOB).with_limits(limits);
    let (outbound, inbound, _) = handshake(&alice, &bob);
    let (mut at_alice, _, _) = outbound
        .admit(PeerRecord::Accepted(&mut Credentials::new(card(BOB))))
        .unwrap();
    let (mut at_bob, _, _) = inbound
        .admit(PeerRecord::Accepted(&mut Credentials::new(card(ALICE))))
        .unwrap();
    let accept = at_alice.send(&Message::ContactAccept, start()).unwrap();
    deliver(&mut at_bob, &accept).unwrap();
    let reply = at_bob.send(&Message::ContactAccept, start()).unwrap();
    deliver(&mut at_alice, &reply).unwrap();
    // The peer keeps a frame going, a byte every 40 seconds, each within
    // what a per-read timeout would allow.
    let frame = frame_from(&mut at_alice);
    at_bob.receive(&frame[..1], after(secs(30))).unwrap();
    at_bob.receive(&frame[1..2], after(secs(59))).unwrap();
    assert_eq!(at_bob.deadline(), Some(after(secs(60))));
    let Expiry::Close(Some(_)) = at_bob.expire(after(secs(60))) else {
        panic!("no Close");
    };
    assert_over(&mut at_bob);
}

#[test]
fn an_ended_session_has_no_deadline_and_ignores_expiry() {
    let mut pair = confirmed();
    pair.responder.stream_closed();
    assert_eq!(pair.responder.deadline(), None);
    assert_eq!(pair.responder.expire(after(secs(100_000))), Expiry::Running);
    let close = pair.initiator.close().unwrap();
    assert!(!close.is_empty());
    assert_eq!(pair.initiator.deadline(), None);
}

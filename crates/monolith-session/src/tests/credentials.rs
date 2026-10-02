//! Changes of transport key over real sessions: a successor announced
//! through the active key, its promotion when it is proven, the end of
//! the sessions of the retired key, and two parties that rotate at the
//! same time.
//!
//! Each side's view of the other is a `Credentials` value, held by the
//! test the way the contact store will hold it.

use monolith_protocol::body::Message;
use monolith_protocol::card::ContactCard;
use monolith_protocol::credential::{CredentialChange, Credentials};
use monolith_protocol::session::{Action, PeerRecord, Standing};
use monolith_protocol::{ProtocolError, SessionState};

use crate::testing::{
    ALICE, BOB, MALLORY, card, card_of, chat, deliver, handshake, handshake_with, party,
    party_with, start,
};
use crate::{AuthenticatedSession, LocalParty, SessionError};

/// Seed of Alice's successor key.
const T2: u8 = MALLORY;
/// Seed of Bob's successor key.
const U2: u8 = 0x33;

/// A confirmed session from `initiator` to `responder`, each admitting the
/// other with its own record. Returns the two sessions.
fn confirmed_session(
    initiator: &LocalParty,
    responder: &LocalParty,
    dialed: &ContactCard,
    initiator_holds: &mut Credentials,
    responder_holds: &mut Credentials,
) -> (AuthenticatedSession, AuthenticatedSession) {
    let (outbound, inbound, _) = handshake_with(initiator, responder, dialed).unwrap();
    let (mut i, admission_i, first_i) = outbound
        .admit(PeerRecord::Accepted(initiator_holds))
        .unwrap();
    let (mut r, admission_r, first_r) = inbound
        .admit(PeerRecord::Accepted(responder_holds))
        .unwrap();
    assert_eq!(admission_i.standing, Standing::Accepted, "initiator side");
    assert_eq!(admission_r.standing, Standing::Accepted, "responder side");
    assert_eq!(first_i, vec![Action::SendContactAccept]);
    assert_eq!(first_r, vec![Action::SendContactAccept]);
    let to_r = i.send(&Message::ContactAccept, start()).unwrap();
    let to_i = r.send(&Message::ContactAccept, start()).unwrap();
    deliver(&mut r, &to_r).unwrap();
    deliver(&mut i, &to_i).unwrap();
    assert_eq!(i.state(), SessionState::AuthenticatedContact);
    assert_eq!(r.state(), SessionState::AuthenticatedContact);
    (i, r)
}

/// `from` sends its card in an EndpointUpdate; `to` receives it and the
/// holder of `to`'s record announces it.
fn announce(
    from: &mut AuthenticatedSession,
    to: &mut AuthenticatedSession,
    successor: &ContactCard,
    to_holds: &mut Credentials,
) -> CredentialChange {
    let frame = from
        .send(
            &Message::EndpointUpdate(Box::new(successor.clone())),
            start(),
        )
        .unwrap();
    let received = deliver(to, &frame).unwrap().unwrap();
    assert_eq!(received.actions, vec![Action::Deliver]);
    let Message::EndpointUpdate(card) = received.message else {
        panic!("not an EndpointUpdate");
    };
    to_holds.announce(&card, to.peer_card()).unwrap()
}

/// Whether a chat message from `from` is delivered by `to`.
fn chat_is_delivered(from: &mut AuthenticatedSession, to: &mut AuthenticatedSession) -> bool {
    let Ok(frame) = from.send(&chat("hello"), start()) else {
        return false;
    };
    matches!(
        deliver(to, &frame),
        Ok(Some(received)) if received.actions == vec![Action::Deliver]
    )
}

#[test]
fn a_rotation_through_the_active_key_retires_it_and_its_sessions() {
    let mut alice_at_bob = Credentials::new(card(ALICE));
    let mut bob_at_alice = Credentials::new(card(BOB));
    let successor = card_of(ALICE, T2, 2, false);

    // A. A session made with T1 announces T2. T2 is authorized, T1 stays
    // active, and the T1 session keeps working.
    let (mut alice_t1, mut bob_t1) = confirmed_session(
        &party(ALICE),
        &party(BOB),
        &card(BOB),
        &mut bob_at_alice,
        &mut alice_at_bob,
    );
    assert_eq!(
        announce(&mut alice_t1, &mut bob_t1, &successor, &mut alice_at_bob),
        CredentialChange::Authorized
    );
    assert_eq!(alice_at_bob.authorized_successor(), Some(&successor));
    assert!(alice_at_bob.authorizes(bob_t1.peer_card()));
    assert!(chat_is_delivered(&mut alice_t1, &mut bob_t1));

    // B. Alice dials with T2. Bob's admission promotes it and retires T1.
    let (outbound, inbound, _) = handshake(&party_with(ALICE, T2, 2), &party(BOB));
    let (mut bob_t2, admission, _) = inbound
        .admit(PeerRecord::Accepted(&mut alice_at_bob))
        .unwrap();
    assert_eq!(admission.standing, Standing::Accepted);
    assert_eq!(admission.change, Some(CredentialChange::Promoted));
    assert_eq!(alice_at_bob.active(), &successor);
    assert_eq!(alice_at_bob.retired(), Some(card(ALICE).transport()));
    let (mut alice_t2, _, _) = outbound
        .admit(PeerRecord::Accepted(&mut bob_at_alice))
        .unwrap();

    // C. The T1 session no longer stands for Alice. Bob withdraws it
    // before anything else; a chat message Alice sends on it afterwards
    // is not delivered.
    assert!(!alice_at_bob.authorizes(bob_t1.peer_card()));
    let close = bob_t1.withdraw().unwrap();
    assert_eq!(bob_t1.standing(), Standing::StaleCard);
    let frame = alice_t1.send(&chat("still here"), start()).unwrap();
    assert_eq!(deliver(&mut bob_t1, &frame), Ok(None));
    assert_eq!(bob_t1.send(&chat("no"), start()), Err(SessionError::Closed));
    // Alice sees an ordinary Close.
    let received = deliver(&mut alice_t1, &close).unwrap().unwrap();
    assert_eq!(received.message, Message::Close);
    assert_eq!(received.actions, vec![Action::Disconnect]);

    // The T2 session confirms as usual.
    let to_bob = alice_t2.send(&Message::ContactAccept, start()).unwrap();
    let to_alice = bob_t2.send(&Message::ContactAccept, start()).unwrap();
    deliver(&mut bob_t2, &to_bob).unwrap();
    deliver(&mut alice_t2, &to_alice).unwrap();
    assert!(chat_is_delivered(&mut alice_t2, &mut bob_t2));

    // D. A new session made with T1 is not a contact session.
    let (_, inbound, _) = handshake(&party(ALICE), &party(BOB));
    let (_, admission, first) = inbound
        .admit(PeerRecord::Accepted(&mut alice_at_bob))
        .unwrap();
    assert_eq!(admission.standing, Standing::StaleCard);
    assert_eq!(admission.change, Some(CredentialChange::Stale));
    assert_eq!(first, Vec::new());
}

#[test]
fn an_announcement_on_a_session_of_a_retired_key_proves_nothing() {
    let mut alice_at_bob = Credentials::new(card(ALICE));
    let mut bob_at_alice = Credentials::new(card(BOB));
    let (mut alice_t1, mut bob_t1) = confirmed_session(
        &party(ALICE),
        &party(BOB),
        &card(BOB),
        &mut bob_at_alice,
        &mut alice_at_bob,
    );
    // The user confirms a new key out of band while the T1 session is
    // still open, and the session is not withdrawn yet. What it delivers
    // carries no continuity any more.
    let confirmed = card_of(ALICE, T2, 2, false);
    alice_at_bob.import(confirmed.clone()).unwrap();
    alice_at_bob.confirm(&confirmed).unwrap();
    assert_eq!(
        announce(
            &mut alice_t1,
            &mut bob_t1,
            &card_of(ALICE, 0x44, 3, false),
            &mut alice_at_bob
        ),
        CredentialChange::NoContinuity
    );
    assert!(alice_at_bob.authorized_successor().is_none());
}

#[test]
fn simultaneous_rotation_completes_while_the_old_keys_answer() {
    // Alice rotates T1 to T2 and Bob rotates U1 to U2 at the same time.
    // Each keeps answering with the old key until the successors have
    // been exchanged.
    let alice_t2_card = card_of(ALICE, T2, 2, false);
    let bob_u2_card = card_of(BOB, U2, 2, false);
    let mut alice_at_bob = Credentials::new(card(ALICE));
    let mut bob_at_alice = Credentials::new(card(BOB));

    // One session between the old keys carries both announcements.
    let (mut alice_t1, mut bob_u1) = confirmed_session(
        &party(ALICE),
        &party(BOB),
        &card(BOB),
        &mut bob_at_alice,
        &mut alice_at_bob,
    );
    assert_eq!(
        announce(
            &mut alice_t1,
            &mut bob_u1,
            &alice_t2_card,
            &mut alice_at_bob
        ),
        CredentialChange::Authorized
    );
    assert_eq!(
        announce(&mut bob_u1, &mut alice_t1, &bob_u2_card, &mut bob_at_alice),
        CredentialChange::Authorized
    );

    // Alice dials Bob's active key U1, which Bob still answers, with T2:
    // Bob promotes T2.
    let (_, _) = confirmed_session(
        &party_with(ALICE, T2, 2),
        &party(BOB),
        &card(BOB),
        &mut bob_at_alice,
        &mut alice_at_bob,
    );
    assert_eq!(alice_at_bob.active(), &alice_t2_card);
    assert_eq!(bob_at_alice.active(), &card(BOB));

    // Alice now answers with T2. Bob dials it with U2: Alice promotes U2.
    let (_, _) = confirmed_session(
        &party_with(BOB, U2, 2),
        &party_with(ALICE, T2, 2),
        &alice_t2_card,
        &mut alice_at_bob,
        &mut bob_at_alice,
    );
    assert_eq!(bob_at_alice.active(), &bob_u2_card);
    assert_eq!(bob_at_alice.retired(), Some(card(BOB).transport()));
    assert_eq!(alice_at_bob.retired(), Some(card(ALICE).transport()));
}

#[test]
fn simultaneous_rotation_without_the_old_keys_fails_safely() {
    // Both dropped the old key before either announcement went through.
    let mut alice_at_bob = Credentials::new(card(ALICE));
    let alice_now = party_with(ALICE, T2, 2);
    let bob_now = party_with(BOB, U2, 2);

    // Each dials the old key of the other and gets no answer.
    assert_eq!(
        handshake_with(&alice_now, &bob_now, &card(BOB)).err(),
        Some(SessionError::Protocol(ProtocolError::HandshakeFailed))
    );
    assert_eq!(
        handshake_with(&bob_now, &alice_now, &card(ALICE)).err(),
        Some(SessionError::Protocol(ProtocolError::HandshakeFailed))
    );

    // Had Alice learned Bob's new card, her new key would still reach him
    // only as a pending one: no standing, nothing taken over.
    let (_, inbound, _) = handshake_with(&alice_now, &bob_now, bob_now.card()).unwrap();
    let (_, admission, first) = inbound
        .admit(PeerRecord::Accepted(&mut alice_at_bob))
        .unwrap();
    assert_eq!(admission.standing, Standing::PendingSuccessor);
    assert_eq!(first, Vec::new());
    assert_eq!(alice_at_bob.active(), &card(ALICE));

    // Recovery in version 1: the card handed over out of band, imported
    // and confirmed by the user.
    assert_eq!(
        alice_at_bob.import(alice_now.card().clone()),
        Ok(CredentialChange::Pending)
    );
    assert_eq!(
        alice_at_bob.confirm(alice_now.card()),
        Ok(CredentialChange::Promoted)
    );
    let (_, inbound, _) = handshake_with(&alice_now, &bob_now, bob_now.card()).unwrap();
    let (_, admission, _) = inbound
        .admit(PeerRecord::Accepted(&mut alice_at_bob))
        .unwrap();
    assert_eq!(admission.standing, Standing::Accepted);
}

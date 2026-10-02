//! Authorization after authentication: contact confirmation, the
//! stale-card rule, and what a peer can observe about its standing, with
//! two real sessions and the bytes that cross between them.
//!
//! Handshakes in these tests use fixed ephemeral keys. Two runs with the
//! same parties therefore have the same keys, and "the peer sees the same
//! thing" can be checked byte for byte on what one side writes.

use std::collections::VecDeque;

use monolith_protocol::body::Message;
use monolith_protocol::card::{CardChange, ContactCard, InvitationCapability};
use monolith_protocol::session::{Action, Admission, PeerRecord, Standing};
use monolith_protocol::{ProtocolError, SessionState};

use crate::testing::{
    ALICE, BOB, MALLORY, admit_outbound, card, card_inviting, card_of, card_with, chat, handshake,
    handshake_with, party, request_with, start, transport_secret,
};
use crate::{AuthenticatedSession, LocalParty, SessionError};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    Alice,
    Bob,
}

/// Two parties after a handshake, with everything each of them wrote to
/// the stream and was told to do.
struct Run {
    alice: AuthenticatedSession,
    bob: AuthenticatedSession,
    /// The frames each side wrote, in order.
    from_alice: Vec<Vec<u8>>,
    from_bob: Vec<Vec<u8>>,
    alice_actions: Vec<Action>,
    bob_actions: Vec<Action>,
    /// What Bob decided about Alice from his record and her card.
    admission: Admission,
    /// The contact requests that reached each side.
    requests_at_bob: Vec<Message>,
    /// A violation that ended a session, if there was one.
    violation: Option<(Side, SessionError)>,
    /// The capability Alice puts into a request.
    invitation: Option<InvitationCapability>,
    alice_card: ContactCard,
}

impl Run {
    /// Alice dials Bob. `alice_party` is what Alice presents; she holds
    /// Bob with `alice_holds_bob`, and Bob holds `bob_record` about her.
    /// Both sides then do what their sessions tell them to, until nothing
    /// is left to send.
    fn new(
        alice_party: &LocalParty,
        alice_holds_bob: Standing,
        bob_record: PeerRecord<'_>,
    ) -> Self {
        Self::dialing(alice_party, &card(BOB), alice_holds_bob, bob_record)
    }

    /// The same, with the card of Bob that Alice dials. A request she
    /// sends carries the invitation capability of that card.
    fn dialing(
        alice_party: &LocalParty,
        dialed: &ContactCard,
        alice_holds_bob: Standing,
        bob_record: PeerRecord<'_>,
    ) -> Self {
        let invitation = dialed.invitation().cloned();
        let (outbound, inbound, _) = handshake_with(alice_party, &party(BOB), dialed).unwrap();
        let (alice, alice_first) = admit_outbound(outbound, alice_holds_bob);
        let (bob, admission, bob_first) = inbound.admit(bob_record).unwrap();
        let mut run = Self {
            alice,
            bob,
            from_alice: Vec::new(),
            from_bob: Vec::new(),
            alice_actions: Vec::new(),
            bob_actions: Vec::new(),
            admission,
            requests_at_bob: Vec::new(),
            violation: None,
            invitation,
            alice_card: alice_party.card().clone(),
        };
        let mut wire = VecDeque::new();
        run.act(Side::Alice, alice_first, &mut wire);
        run.act(Side::Bob, bob_first, &mut wire);
        run.drain(wire);
        run
    }

    /// Carries out what a side was told to do and puts what it sends on
    /// the wire.
    fn act(&mut self, side: Side, actions: Vec<Action>, wire: &mut VecDeque<(Side, Vec<u8>)>) {
        for action in &actions {
            let frame = match (action, side) {
                (Action::SendContactAccept, Side::Alice) => {
                    self.alice.send(&Message::ContactAccept, start()).unwrap()
                }
                (Action::SendContactAccept, Side::Bob) => {
                    self.bob.send(&Message::ContactAccept, start()).unwrap()
                }
                (Action::SendContactRequest, Side::Alice) => {
                    let request = request_with(self.alice_card.clone(), self.invitation.clone());
                    self.alice.send(&request, start()).unwrap()
                }
                (Action::SendContactRequest, Side::Bob) => self
                    .bob
                    .send(&request_with(card(BOB), None), start())
                    .unwrap(),
                (Action::SendClose, Side::Alice) => self.alice.close().unwrap(),
                (Action::SendClose, Side::Bob) => self.bob.close().unwrap(),
                _ => continue,
            };
            self.put(side, frame, wire);
        }
        match side {
            Side::Alice => self.alice_actions.extend(actions),
            Side::Bob => self.bob_actions.extend(actions),
        }
    }

    fn put(&mut self, from: Side, frame: Vec<u8>, wire: &mut VecDeque<(Side, Vec<u8>)>) {
        match from {
            Side::Alice => {
                self.from_alice.push(frame.clone());
                wire.push_back((Side::Bob, frame));
            }
            Side::Bob => {
                self.from_bob.push(frame.clone());
                wire.push_back((Side::Alice, frame));
            }
        }
    }

    fn drain(&mut self, mut wire: VecDeque<(Side, Vec<u8>)>) {
        while let Some((to, frame)) = wire.pop_front() {
            let session = match to {
                Side::Alice => &mut self.alice,
                Side::Bob => &mut self.bob,
            };
            match session.receive(&frame, start()) {
                Ok((used, received)) => {
                    assert_eq!(used, frame.len());
                    if let Some(received) = received {
                        if to == Side::Bob && matches!(received.message, Message::ContactRequest(_))
                        {
                            self.requests_at_bob.push(received.message.clone());
                        }
                        self.act(to, received.actions, &mut wire);
                    }
                }
                Err(error) => {
                    // A violation: the stream is closed and nothing is sent.
                    self.violation = Some((to, error));
                    break;
                }
            }
        }
    }

    /// Alice sends a message that her session would not let her send.
    fn alice_breaks_the_rules(&mut self, message: &Message) {
        let frame = self.alice.seal_unchecked(message);
        let mut wire = VecDeque::new();
        self.put(Side::Alice, frame, &mut wire);
        self.drain(wire);
    }

    fn confirmed(&self) -> bool {
        self.alice.state() == SessionState::AuthenticatedContact
            && self.bob.state() == SessionState::AuthenticatedContact
    }
}

/// Alice with her usual card.
fn usual(alice_holds_bob: Standing, bob_record: PeerRecord<'_>) -> Run {
    Run::new(&party(ALICE), alice_holds_bob, bob_record)
}

#[test]
fn accepted_contacts_confirm_each_other() {
    let alice_card = card(ALICE);
    let run = usual(Standing::Accepted, PeerRecord::Accepted(&alice_card));
    assert!(run.confirmed());
    assert_eq!(
        run.alice_actions,
        vec![Action::SendContactAccept, Action::Confirmed]
    );
    assert_eq!(run.bob_actions, run.alice_actions);
    assert_eq!(run.admission.card, Some(CardChange::Unchanged));
    // One frame of one block in each direction.
    assert_eq!(run.from_alice.len(), 1);
    assert_eq!(run.from_bob.len(), 1);
    assert_eq!(run.from_alice[0].len(), 1042);
    assert_eq!(run.from_bob[0].len(), 1042);
}

#[test]
fn crossing_requests_accept_each_other() {
    // T-CONTACT-2 over real sessions.
    let alice_card = card(ALICE);
    let run = usual(Standing::Requested, PeerRecord::Requested(&alice_card));
    assert!(run.confirmed());
    let expected = vec![
        Action::SendContactRequest,
        Action::MarkAccepted,
        Action::SendContactAccept,
        Action::Confirmed,
    ];
    assert_eq!(run.alice_actions, expected);
    assert_eq!(run.bob_actions, expected);
    assert_eq!(run.alice.standing(), Standing::Accepted);
    assert_eq!(run.bob.standing(), Standing::Accepted);
}

#[test]
fn every_pair_of_records_ends_consistently() {
    // A session is confirmed on both sides or on neither, and exactly
    // when each side holds the other as requested or accepted.
    let alice_card = card(ALICE);
    let standings = [
        Standing::None,
        Standing::Declined,
        Standing::Blocked,
        Standing::Requested,
        Standing::Accepted,
    ];
    let records = [
        PeerRecord::None,
        PeerRecord::Declined,
        PeerRecord::Blocked,
        PeerRecord::Requested(&alice_card),
        PeerRecord::Accepted(&alice_card),
    ];
    for alice_holds_bob in standings {
        for bob_record in records {
            let run = usual(alice_holds_bob, bob_record);
            let pair = format!("{alice_holds_bob:?} / {bob_record:?}");
            let expected =
                alice_holds_bob.is_contact_record() && run.admission.standing.is_contact_record();
            assert_eq!(run.confirmed(), expected, "{pair}");
            assert!(run.violation.is_none(), "{pair}");
            for (actions, session) in [
                (&run.alice_actions, &run.alice),
                (&run.bob_actions, &run.bob),
            ] {
                assert!(!actions.contains(&Action::Deliver), "{pair}");
                assert_eq!(actions.contains(&Action::Confirmed), expected, "{pair}");
                let state = session.state();
                assert!(
                    expected == (state == SessionState::AuthenticatedContact),
                    "{pair}: {state:?}"
                );
            }
        }
    }
}

#[test]
fn a_card_older_than_the_pinned_one_does_not_open_a_contact_session() {
    // The stale-card rule. Alice has issued a card with epoch 2, and Bob
    // has pinned it. Whoever presents her card of epoch 1 holds the
    // transport key it states and a valid signature, and is still not a
    // contact for this session.
    let pinned = card_of(ALICE, ALICE, 2, false);
    let run = usual(Standing::Accepted, PeerRecord::Accepted(&pinned));
    assert_eq!(
        run.admission,
        Admission {
            standing: Standing::StaleCard,
            card: Some(CardChange::Stale)
        }
    );
    assert!(!run.confirmed());
    // Bob sent nothing before Alice's ContactAccept and answered it with
    // Close. Alice was told nothing else.
    assert_eq!(run.bob_actions, vec![Action::SendClose]);
    assert_eq!(
        run.alice_actions,
        vec![Action::SendContactAccept, Action::Disconnect]
    );
    assert_eq!(run.from_bob.len(), 1);

    // The same for a pending request.
    let run = usual(Standing::Requested, PeerRecord::Requested(&pinned));
    assert_eq!(run.admission.standing, Standing::StaleCard);
    assert!(!run.confirmed());
    assert_eq!(run.bob_actions, vec![Action::SendClose]);
}

#[test]
fn a_retired_transport_key_is_useless_against_a_contact_that_knows_the_new_one() {
    // Alice replaced her transport key: card of epoch 2 with the key of
    // another seed, which Bob has pinned. Someone who obtained the old
    // private key dials Bob with the old card.
    let pinned = card_of(ALICE, MALLORY, 2, false);
    let run = usual(Standing::Accepted, PeerRecord::Accepted(&pinned));
    assert_eq!(run.admission.standing, Standing::StaleCard);
    assert!(!run.confirmed());
    assert!(!run.bob_actions.contains(&Action::Deliver));
    assert!(!run.bob_actions.contains(&Action::Confirmed));

    // A fresh attempt that goes straight to a chat message is a violation,
    // as it is for any peer that is not a contact.
    let mut run = Run::new(&party(ALICE), Standing::None, PeerRecord::Accepted(&pinned));
    run.alice_breaks_the_rules(&chat("as Alice"));
    assert_eq!(
        run.violation,
        Some((
            Side::Bob,
            SessionError::Protocol(ProtocolError::MessageNotPermitted)
        ))
    );
    assert!(run.bob_actions.is_empty());

    // The holder of the new key, with the new card, is the contact.
    let new_alice = LocalParty::new(pinned.clone(), transport_secret(MALLORY)).unwrap();
    let run = Run::new(
        &new_alice,
        Standing::Accepted,
        PeerRecord::Accepted(&pinned),
    );
    assert_eq!(run.admission.card, Some(CardChange::Unchanged));
    assert!(run.confirmed());
}

#[test]
fn a_retired_key_is_refused_as_soon_as_the_successor_card_was_shown() {
    // Bob has pinned Alice's first card. Alice replaces her transport key
    // and dials Bob with the new card. Bob's user has not confirmed the
    // change, and may never do so.
    let pinned = card(ALICE);
    let successor = card_of(ALICE, MALLORY, 2, false);
    let alice = LocalParty::new(successor.clone(), transport_secret(MALLORY)).unwrap();
    let run = Run::new(&alice, Standing::Accepted, PeerRecord::Accepted(&pinned));
    assert_eq!(run.admission.card, Some(CardChange::Newer));
    assert!(run.confirmed());

    // From that moment the newest card Bob holds of Alice is the
    // successor, and it is what later cards are compared with. Whoever
    // dials with the first card and the key it states is not a contact,
    // although that card is still the pinned one.
    let newest_held = run.bob.peer_card().clone();
    assert_eq!(newest_held, successor);
    let run = usual(Standing::Accepted, PeerRecord::Accepted(&newest_held));
    assert_eq!(
        run.admission,
        Admission {
            standing: Standing::StaleCard,
            card: Some(CardChange::Stale)
        }
    );
    assert!(!run.confirmed());
    assert_eq!(run.bob_actions, vec![Action::SendClose]);
}

#[test]
fn a_dialed_card_that_was_superseded_meanwhile_gives_no_contact_session() {
    // Alice dials Bob with the card she has pinned. While the dial is in
    // progress a newer card of Bob reaches her, with another transport
    // key. The responder has then proved a key that Alice knows to be
    // retired: the session exists, and it is not a contact session.
    let (outbound, _, _) = handshake(&party(ALICE), &party(BOB));
    let newer = card_of(BOB, MALLORY, 2, false);
    let (alice, admission, first) = outbound.admit(PeerRecord::Accepted(&newer)).unwrap();
    assert_eq!(
        admission,
        Admission {
            standing: Standing::StaleCard,
            card: Some(CardChange::Stale)
        }
    );
    assert_eq!(first, Vec::new());
    assert_eq!(alice.standing(), Standing::StaleCard);
    for message_type in monolith_protocol::MessageType::ALL {
        assert!(!alice.may_send(message_type), "{message_type:?}");
    }

    // The card that was dialed is the newest one held: the record decides.
    let (outbound, _, _) = handshake(&party(ALICE), &party(BOB));
    let (_, admission, first) = outbound.admit(PeerRecord::Accepted(&card(BOB))).unwrap();
    assert_eq!(admission.card, Some(CardChange::Unchanged));
    assert_eq!(first, vec![Action::SendContactAccept]);

    // A record without a card: no comparison.
    let (outbound, _, _) = handshake(&party(ALICE), &party(BOB));
    let (_, admission, first) = outbound.admit(PeerRecord::Blocked).unwrap();
    assert_eq!(admission.standing, Standing::Blocked);
    assert_eq!(admission.card, None);
    assert_eq!(first, Vec::new());

    // The record of another identity is refused.
    let (outbound, _, _) = handshake(&party(ALICE), &party(BOB));
    assert_eq!(
        outbound.admit(PeerRecord::Accepted(&card(MALLORY))).err(),
        Some(SessionError::Protocol(ProtocolError::IdentityMismatch))
    );
}

#[test]
fn a_card_that_contradicts_the_pinned_one_does_not_open_a_contact_session() {
    // Same epoch, another transport key: the identity signed two
    // statements for one epoch. Bob keeps what he has and is told about
    // the conflict; the peer is told nothing.
    let pinned = card_of(ALICE, MALLORY, 1, false);
    let run = usual(Standing::Accepted, PeerRecord::Accepted(&pinned));
    assert_eq!(
        run.admission,
        Admission {
            standing: Standing::StaleCard,
            card: Some(CardChange::Conflict)
        }
    );
    assert!(!run.confirmed());

    // Same epoch and transport key, another endpoint.
    let pinned = card_with(ALICE, ALICE, 1, MALLORY, false);
    let run = usual(Standing::Accepted, PeerRecord::Accepted(&pinned));
    assert_eq!(run.admission.card, Some(CardChange::Conflict));
    assert_eq!(run.admission.standing, Standing::StaleCard);
}

#[test]
fn a_newer_card_keeps_the_contact_and_is_reported_as_a_pending_change() {
    // Alice presents a card of epoch 3 with a new transport key; Bob has
    // pinned epoch 1. The handshake proves that she holds the new key, and
    // the card is signed by her identity. The session is a contact
    // session, and the change of what is pinned is left to the user.
    let pinned = card(ALICE);
    let newer = card_of(ALICE, MALLORY, 3, false);
    let alice = LocalParty::new(newer.clone(), transport_secret(MALLORY)).unwrap();
    let run = Run::new(&alice, Standing::Accepted, PeerRecord::Accepted(&pinned));
    assert_eq!(
        run.admission,
        Admission {
            standing: Standing::Accepted,
            card: Some(CardChange::Newer)
        }
    );
    assert!(run.confirmed());
    assert_eq!(run.bob.peer_card(), &newer);
}

#[test]
fn peers_that_are_not_contacts_see_the_same_bytes_whatever_the_reason() {
    // T-ORACLE-1 on the wire. Bob holds no record of Alice, declined her,
    // blocked her, or holds her as a contact and is shown a stale or
    // conflicting card. For each thing Alice can do, everything Bob
    // writes is identical in all cases, down to the bytes, and so is what
    // Alice's side is told to do.
    let newer = card_of(ALICE, ALICE, 2, false);
    let conflicting = card_of(ALICE, MALLORY, 1, false);
    let records = [
        PeerRecord::None,
        PeerRecord::Declined,
        PeerRecord::Blocked,
        PeerRecord::Accepted(&newer),
        PeerRecord::Requested(&newer),
        PeerRecord::Accepted(&conflicting),
    ];
    for alice_holds_bob in [Standing::None, Standing::Requested, Standing::Accepted] {
        let reference = usual(alice_holds_bob, PeerRecord::None);
        for record in records {
            let run = usual(alice_holds_bob, record);
            let case = format!("{alice_holds_bob:?} against {record:?}");
            assert!(!run.admission.standing.is_contact_record(), "{case}");
            assert_eq!(run.from_bob, reference.from_bob, "{case}");
            assert_eq!(run.alice_actions, reference.alice_actions, "{case}");
            assert_eq!(run.alice.state(), reference.alice.state(), "{case}");
            assert_eq!(run.bob.state(), reference.bob.state(), "{case}");
            assert!(run.violation.is_none(), "{case}");
            // The only difference is whether Bob looks at a request
            // afterwards, which is not visible.
            let visible = |actions: &[Action]| -> Vec<Action> {
                actions
                    .iter()
                    .copied()
                    .filter(|action| action.is_visible_to_peer())
                    .collect()
            };
            assert_eq!(
                visible(&run.bob_actions),
                visible(&reference.bob_actions),
                "{case}"
            );
        }
    }

    // A peer that breaks the rules gets the same treatment in every case
    // as well: the stream is closed and nothing is written.
    for record in records {
        let mut run = usual(Standing::None, record);
        run.alice_breaks_the_rules(&chat("hello"));
        assert_eq!(
            run.violation,
            Some((
                Side::Bob,
                SessionError::Protocol(ProtocolError::MessageNotPermitted)
            )),
            "{record:?}"
        );
        assert!(run.from_bob.is_empty(), "{record:?}");
    }
}

#[test]
fn a_request_is_answered_with_close_before_it_is_considered() {
    let run = usual(Standing::Requested, PeerRecord::None);
    assert_eq!(
        run.bob_actions,
        vec![Action::SendClose, Action::ConsiderRequest]
    );
    assert_eq!(
        run.alice_actions,
        vec![Action::SendContactRequest, Action::Disconnect]
    );
    assert_eq!(run.bob.state(), SessionState::Closing);
    assert_eq!(run.alice.state(), SessionState::Closed);
    // The request reached Bob complete: the card is the one Alice
    // presented in the handshake.
    assert_eq!(run.requests_at_bob.len(), 1);
    let Message::ContactRequest(request) = &run.requests_at_bob[0] else {
        panic!("not a request");
    };
    assert_eq!(&request.card, run.bob.peer_card());

    // A declined or blocked identity is not considered again.
    for record in [PeerRecord::Declined, PeerRecord::Blocked] {
        assert_eq!(
            usual(Standing::Requested, record).bob_actions,
            vec![Action::SendClose]
        );
    }
}

#[test]
fn an_invitation_capability_changes_nothing_a_requester_can_see() {
    // The capability is inside the encrypted request and is looked at
    // after the Close, by the layer that holds the valid ones. Whatever
    // it is, the session answers in the same way. Alice sends the
    // capability of the card she was given; the three cards below are
    // cards of Bob with no capability and with two different ones.
    let reference = usual(Standing::Requested, PeerRecord::None);
    assert_eq!(reference.from_alice[0].len(), 1042);
    for bytes in [[0x11_u8; 16], [0xEE; 16]] {
        let capability = InvitationCapability::from_bytes(bytes);
        let run = Run::dialing(
            &party(ALICE),
            &card_inviting(BOB, bytes),
            Standing::Requested,
            PeerRecord::None,
        );
        assert_eq!(run.from_bob, reference.from_bob);
        assert_eq!(run.bob_actions, reference.bob_actions);
        assert_eq!(run.alice_actions, reference.alice_actions);
        assert_eq!(run.bob.state(), reference.bob.state());

        // It arrived intact, and it was never on the wire in clear.
        let Message::ContactRequest(request) = &run.requests_at_bob[0] else {
            panic!("not a request");
        };
        assert_eq!(request.invitation, Some(capability));
        for frame in &run.from_alice {
            assert!(!frame.windows(16).any(|window| window == bytes));
            // With or without a capability a request is one frame of one
            // block.
            assert_eq!(frame.len(), 1042);
        }
    }
}

#[test]
fn a_request_carries_the_invitation_for_this_peer_and_no_other() {
    // PROTOCOL.md 8.3: the capability in a request is the one in the card
    // of the peer that is asked. A capability that the user was given by
    // somebody else is not sent here, and none is made up.
    let given = [0x11_u8; 16];
    let other = Some(InvitationCapability::from_bytes([0x22; 16]));
    let (outbound, _, _) =
        handshake_with(&party(ALICE), &party(BOB), &card_inviting(BOB, given)).unwrap();
    let (mut alice, first) = admit_outbound(outbound, Standing::Requested);
    assert_eq!(first, vec![Action::SendContactRequest]);
    for wrong in [other.clone(), None] {
        assert_eq!(
            alice.send(&request_with(card(ALICE), wrong), start()).err(),
            Some(SessionError::InvalidMessage)
        );
    }
    let right = Some(InvitationCapability::from_bytes(given));
    assert!(
        alice
            .send(&request_with(card(ALICE), right.clone()), start())
            .is_ok()
    );

    // A card without a capability: the request has none.
    let (outbound, _, _) = handshake(&party(ALICE), &party(BOB));
    let (mut alice, _) = admit_outbound(outbound, Standing::Requested);
    assert_eq!(
        alice.send(&request_with(card(ALICE), other), start()).err(),
        Some(SessionError::InvalidMessage)
    );
    assert!(
        alice
            .send(&request_with(card(ALICE), None), start())
            .is_ok()
    );

    // The side that was dialed asks with the capability of the card it
    // holds of the peer. Bob imported a card of Alice with a capability,
    // and Alice dialed first.
    let held = card_inviting(ALICE, given);
    let (_, inbound, _) = handshake(&party(ALICE), &party(BOB));
    let (mut bob, _, first) = inbound.admit(PeerRecord::Requested(&held)).unwrap();
    assert_eq!(first, vec![Action::SendContactRequest]);
    assert_eq!(
        bob.send(&request_with(card(BOB), None), start()).err(),
        Some(SessionError::InvalidMessage)
    );
    assert!(bob.send(&request_with(card(BOB), right), start()).is_ok());
}

#[test]
fn a_request_is_sent_once_on_a_session() {
    // PROTOCOL.md 6.4. The peer would end the session over a second one,
    // so the local side does not send it.
    let (outbound, _, _) = handshake(&party(ALICE), &party(BOB));
    let (mut alice, _) = admit_outbound(outbound, Standing::Requested);
    let request = request_with(card(ALICE), None);
    assert!(alice.may_send(monolith_protocol::MessageType::ContactRequest));
    assert!(alice.send(&request, start()).is_ok());
    assert!(!alice.may_send(monolith_protocol::MessageType::ContactRequest));
    assert_eq!(
        alice.send(&request, start()).err(),
        Some(SessionError::NotPermitted)
    );
    // A request that was refused before it was encrypted does not count.
    let (outbound, _, _) = handshake(&party(ALICE), &party(BOB));
    let (mut alice, _) = admit_outbound(outbound, Standing::Requested);
    assert!(
        alice
            .send(&request_with(card(MALLORY), None), start())
            .is_err()
    );
    assert!(alice.send(&request, start()).is_ok());
}

#[test]
fn nothing_is_sent_on_a_confirmed_session_before_the_local_accept() {
    // The peer's ContactAccept confirms the session on this side. The
    // peer confirms it when it sees ours, and takes application messages
    // only then. So ours goes first.
    let alice_card = card(ALICE);
    let (outbound, inbound, _) = handshake(&party(ALICE), &party(BOB));
    let (mut alice, _) = admit_outbound(outbound, Standing::Accepted);
    let (mut bob, _, _) = inbound.admit(PeerRecord::Accepted(&alice_card)).unwrap();

    // Bob's accept arrives before Alice has sent hers.
    let from_bob = bob.send(&Message::ContactAccept, start()).unwrap();
    let received = alice.receive(&from_bob, start()).unwrap().1.unwrap();
    assert_eq!(received.actions, vec![Action::Confirmed]);
    assert_eq!(alice.state(), SessionState::AuthenticatedContact);
    for message_type in monolith_protocol::MessageType::ALL {
        let allowed = message_type == monolith_protocol::MessageType::ContactAccept;
        assert_eq!(alice.may_send(message_type), allowed, "{message_type:?}");
    }
    assert_eq!(
        alice.send(&chat("too early"), start()).err(),
        Some(SessionError::NotPermitted)
    );

    // After her accept everything a confirmed session carries may go.
    let from_alice = alice.send(&Message::ContactAccept, start()).unwrap();
    let hello = alice.send(&chat("hello"), start()).unwrap();
    assert!(bob.receive(&from_alice, start()).unwrap().1.is_some());
    let received = bob.receive(&hello, start()).unwrap().1.unwrap();
    assert_eq!(received.actions, vec![Action::Deliver]);
}

#[test]
fn a_record_of_another_identity_is_refused() {
    let (_, inbound, _) = handshake(&party(ALICE), &party(BOB));
    assert_eq!(
        inbound.admit(PeerRecord::Accepted(&card(MALLORY))).err(),
        Some(SessionError::Protocol(ProtocolError::IdentityMismatch))
    );
}

#[test]
fn a_contact_that_is_blocked_while_connected_sees_an_ordinary_close() {
    let alice_card = card(ALICE);
    let mut run = usual(Standing::Accepted, PeerRecord::Accepted(&alice_card));
    assert!(run.confirmed());
    let close = run.bob.block_peer().unwrap();
    assert_eq!(run.bob.standing(), Standing::Blocked);
    let (_, received) = run.alice.receive(&close, start()).unwrap();
    let received = received.unwrap();
    assert_eq!(received.message, Message::Close);
    assert_eq!(received.actions, vec![Action::Disconnect]);

    // On the next session Alice is one of the peers that are not contacts.
    let run = usual(Standing::Accepted, PeerRecord::Blocked);
    assert!(!run.confirmed());
    assert_eq!(run.bob_actions, vec![Action::SendClose]);
}

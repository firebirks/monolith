//! Admission against the contact state of the moment, withdrawal of the
//! links whose standing the store revoked, and message 3 only for a key
//! that may learn the local identity: through the contact store, on the
//! in-memory Tor.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use common::{
    Released, announce, befriend, both, chat, confirm_both, connect, dial_and_answer, node,
    node_in, node_with, party, request, run, run_paused, send_first, step,
};
use monolith_core::budget::Budgets;
use monolith_core::contacts::ImportOutcome;
use monolith_core::identity::{Installation, RotationState};
use monolith_core::link::{LinkError, dial};
use monolith_protocol::body::Message;
use monolith_protocol::contact::RecordKind;
use monolith_protocol::credential::{CardRelation, CredentialChange};
use monolith_protocol::limits::{
    HANDSHAKE_MSG1_LEN, MAX_CONCURRENT_DIALS, MAX_INBOUND_HANDSHAKES, MAX_UNKNOWN_SESSIONS,
};
use monolith_protocol::session::{Action, Admission, Standing};
use monolith_session::HandshakeResponder;
use monolith_storage::dir::{CrashOutcome, MemoryDir};
use monolith_storage::vault::{KdfParams, Passphrase};
use monolith_tor::{MockNetwork, OnionService};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn refused(standing: Standing, change: Option<CredentialChange>) -> Option<LinkError> {
    Some(LinkError::Refused(Admission { standing, change }))
}

fn persistent() -> Installation {
    Installation::create(
        Box::new(MemoryDir::new()),
        &Passphrase::new("test passphrase").unwrap(),
        KdfParams::FLOOR,
    )
    .unwrap()
}

#[test]
fn a_dial_is_admitted_against_the_contact_state_after_the_handshake() {
    // Alice holds Bob with his card of key T1 and dials it. While the dial
    // is in progress, before Bob has answered, Alice's user imports Bob's
    // card of key T2, which for a requested contact is the card to use.
    // The responder then proves T1: the session gets no standing from
    // what Alice held when she began, and Bob gets no message 3.
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        alice.identity.import(&bob.card()).await.unwrap();
        let successor = party(2, 22, 2, bob.identity.endpoint()).card().clone();
        let card = bob.card();
        let bob_identity = bob.identity.clone();
        let alice_identity = alice.identity.clone();
        let budgets = bob.budgets.clone();
        let service = &mut bob.service;
        let answering = async {
            let stream = service.accept().await.unwrap();
            assert_eq!(
                alice_identity.import(&successor).await,
                Ok(ImportOutcome::Evaluated {
                    relation: Some(CardRelation::NewKey),
                    change: CredentialChange::Promoted
                })
            );
            monolith_core::link::answer(stream, &budgets, &bob_identity).await
        };
        let (dialed, answered) = both(
            dial(&alice.tor, &alice.budgets, &alice.identity, &card),
            answering,
        )
        .await;
        assert_eq!(
            dialed.err(),
            refused(Standing::StaleCard, Some(CredentialChange::Stale))
        );
        // Bob never got message 3.
        assert_eq!(answered.err(), Some(LinkError::Stream));
        let view = alice.identity.contact(bob.identity.identity()).unwrap();
        assert_eq!(view.credentials.unwrap().active(), &successor);
    });
}

#[test]
fn a_rotation_withdraws_the_link_of_the_retired_key() {
    // Alice talks to Bob with T1. She begins a rotation and announces T2
    // on that session; Bob authorizes it. Alice switches and dials with
    // T2: Bob's admission promotes it and withdraws the T1 link in the
    // same step. A message Alice sent on T1 before is not delivered, and
    // the T1 link ends with a Close.
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        befriend(&alice, &mut bob).await;
        let (alice_end, bob_end) = connect(&alice, &mut bob).await;
        let (mut alice_end, mut bob_end) = (alice_end.unwrap(), bob_end.unwrap());
        send_first(&mut alice_end, &alice.identity).await;
        send_first(&mut bob_end, &bob.identity).await;
        let (mut alice_t1, mut bob_t1) = (alice_end.link, bob_end.link);
        step(&mut bob_t1, &bob.identity).await.unwrap();
        step(&mut alice_t1, &alice.identity).await.unwrap();

        // T2 is announced on the T1 session and authorized.
        let successor = alice.identity.begin_rotation().await.unwrap();
        let announcement = alice
            .identity
            .announcement_for(alice_t1.session_ref())
            .unwrap();
        assert_eq!(announcement.card(), &successor);
        alice_t1
            .send(&Message::EndpointUpdate(Box::new(successor.clone())))
            .await
            .unwrap();
        assert!(alice.identity.mark_announced(&announcement).await.unwrap());
        let received = bob_t1.receive().await.unwrap();
        let applied = bob
            .identity
            .apply(bob_t1.session_ref(), &received)
            .await
            .unwrap();
        assert_eq!(applied.announced, Some(CredentialChange::Authorized));
        let view = bob.identity.contact(alice.identity.identity()).unwrap();
        assert_eq!(
            view.credentials.unwrap().authorized_successor(),
            Some(&successor)
        );

        // Two messages that arrive together. Bob takes the first; the
        // second waits in his buffer. A third is still on the stream.
        alice_t1.send(&chat("first")).await.unwrap();
        alice_t1.send(&chat("buffered")).await.unwrap();
        assert_eq!(bob_t1.receive().await.unwrap().message, chat("first"));
        alice_t1.send(&chat("in flight")).await.unwrap();

        // Alice switches to T2, which every accepted contact was sent, and
        // dials Bob with it.
        assert!(alice.identity.switch_rotation(false).await.unwrap());
        let plan = alice.identity.dial_plan(bob.identity.identity()).unwrap();
        assert_eq!(plan.local.card(), &successor);
        let (alice_t2, bob_t2) = connect(&alice, &mut bob).await;
        let (mut alice_t2, mut bob_t2) = (alice_t2.unwrap(), bob_t2.unwrap());
        assert_eq!(bob_t2.admission.change, Some(CredentialChange::Promoted));
        assert_eq!(bob_t2.admission.standing, Standing::Accepted);
        assert!(bob_t1.is_withdrawn());

        // Neither the buffered message nor the one in flight is delivered.
        assert_eq!(bob_t1.receive().await.err(), Some(LinkError::Withdrawn));
        assert_eq!(
            bob_t1.send(&chat("no")).await.err(),
            Some(LinkError::Withdrawn)
        );
        // Alice's T1 link sees an ordinary Close.
        assert_eq!(alice_t1.receive().await.unwrap().message, Message::Close);

        // The T2 session works, and its confirmation tells Alice that Bob
        // promoted the new key.
        send_first(&mut alice_t2, &alice.identity).await;
        send_first(&mut bob_t2, &bob.identity).await;
        let received = alice_t2.link.receive().await.unwrap();
        let applied = alice
            .identity
            .apply(alice_t2.link.session_ref(), &received)
            .await
            .unwrap();
        assert!(applied.promoted_successor);
        assert!(alice.identity.finish_rotation(false).await.unwrap());
        step(&mut bob_t2.link, &bob.identity).await.unwrap();
        alice_t2.link.send(&chat("from T2")).await.unwrap();
        let received = bob_t2.link.receive().await.unwrap();
        assert_eq!(received.message, chat("from T2"));
        assert_eq!(received.actions, vec![Action::Deliver]);

        // A new session with T1 is not a contact session. It is left on
        // the path of a stranger: not kept for withdrawal, not ended early.
        let old_alice = node_with(&network, 1, 1, 1).await;
        let kept = bob
            .identity
            .contact(alice.identity.identity())
            .unwrap()
            .sessions;
        let card = bob.card();
        old_alice.identity.import(&card).await.unwrap();
        let (_, answered) = dial_and_answer(&old_alice, &card, &mut bob).await;
        let answered = answered.unwrap();
        assert_eq!(answered.admission.standing, Standing::StaleCard);
        assert!(!answered.link.is_withdrawn());
        assert_eq!(
            bob.identity
                .contact(alice.identity.identity())
                .unwrap()
                .sessions,
            kept
        );
    });
}

#[test]
fn a_link_that_waits_for_the_peer_wakes_up_when_it_is_withdrawn() {
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        befriend(&alice, &mut bob).await;
        let (mut alice_link, mut bob_link) = confirm_both(&alice, &mut bob).await;

        // Bob's link waits. Meanwhile Bob's user confirms a new key of
        // Alice that was shown to him out of band.
        let successor = party(1, 11, 2, alice.identity.endpoint()).card().clone();
        let identity = bob.identity.clone();
        let promote = async {
            tokio::task::yield_now().await;
            identity.import(&successor).await.unwrap();
            identity.confirm_pending(&successor).await.unwrap();
        };
        let (waited, ()) = both(bob_link.receive(), promote).await;
        assert_eq!(waited.err(), Some(LinkError::Withdrawn));
        let closed = alice_link.receive().await.unwrap();
        assert_eq!(closed.message, Message::Close);
        assert_eq!(closed.actions, vec![Action::Disconnect]);
    });
}

#[test]
fn blocking_or_deleting_a_contact_withdraws_its_open_links() {
    // WP4: a durable transition that leaves a session without standing
    // ends it in the same step, whatever the transition.
    for deleting in [false, true] {
        run(async {
            let network = MockNetwork::new();
            let alice = node(&network, 1).await;
            let mut bob = node(&network, 2).await;
            befriend(&alice, &mut bob).await;
            let (mut alice_link, mut bob_link) = confirm_both(&alice, &mut bob).await;
            alice_link.send(&chat("before")).await.unwrap();
            assert_eq!(bob_link.receive().await.unwrap().message, chat("before"));
            assert_eq!(
                bob.identity
                    .contact(alice.identity.identity())
                    .unwrap()
                    .sessions,
                1
            );
            alice_link.send(&chat("buffered")).await.unwrap();
            if deleting {
                bob.identity
                    .delete(alice.identity.identity())
                    .await
                    .unwrap();
                assert_eq!(
                    bob.identity.kind(alice.identity.identity()),
                    RecordKind::None
                );
            } else {
                bob.identity.block(alice.identity.identity()).await.unwrap();
                assert_eq!(
                    bob.identity.kind(alice.identity.identity()),
                    RecordKind::Blocked
                );
            }
            assert!(bob_link.is_withdrawn());
            assert_eq!(bob_link.receive().await.err(), Some(LinkError::Withdrawn));
            assert_eq!(
                bob_link.send(&chat("no")).await.err(),
                Some(LinkError::Withdrawn)
            );
            // Alice sees an ordinary Close, and her next session with Bob is
            // the one of a stranger.
            assert_eq!(alice_link.receive().await.unwrap().message, Message::Close);
            let (alice_end, bob_end) = connect(&alice, &mut bob).await;
            assert!(alice_end.is_ok());
            let bob_end = bob_end.unwrap();
            assert!(!bob_end.admission.standing.is_contact_record());
            assert!(bob_end.first.is_empty());
        });
    }
}

#[test]
fn a_withdrawn_link_fails_at_once_every_time() {
    // The contact is blocked before the link was ever polled. Every later
    // call fails at once, the first one writes the Close, and nothing
    // waits for the idle limit.
    run_paused(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        befriend(&alice, &mut bob).await;
        let (alice_end, bob_end) = connect(&alice, &mut bob).await;
        let (mut alice_end, mut bob_end) = (alice_end.unwrap(), bob_end.unwrap());
        bob.identity.block(alice.identity.identity()).await.unwrap();
        let started = tokio::time::Instant::now();
        assert_eq!(
            bob_end.link.receive().await.err(),
            Some(LinkError::Withdrawn)
        );
        assert_eq!(
            bob_end.link.receive().await.err(),
            Some(LinkError::Withdrawn)
        );
        assert_eq!(
            bob_end.link.send(&chat("x")).await.err(),
            Some(LinkError::Withdrawn)
        );
        bob_end.link.close().await.unwrap();
        assert_eq!(started.elapsed(), core::time::Duration::ZERO);
        assert_eq!(
            alice_end.link.receive().await.unwrap().message,
            Message::Close
        );
    });
}

#[test]
fn a_withdrawal_first_seen_by_send_writes_the_close() {
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        befriend(&alice, &mut bob).await;
        let (mut alice_link, mut bob_link) = confirm_both(&alice, &mut bob).await;
        bob.identity
            .delete(alice.identity.identity())
            .await
            .unwrap();
        assert_eq!(
            bob_link.send(&chat("x")).await.err(),
            Some(LinkError::Withdrawn)
        );
        let closed = alice_link.receive().await.unwrap();
        assert_eq!(closed.message, Message::Close);
    });
}

#[test]
fn the_store_forgets_the_links_that_are_gone() {
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        befriend(&alice, &mut bob).await;
        let (alice_link, bob_link) = confirm_both(&alice, &mut bob).await;
        assert_eq!(
            bob.identity
                .contact(alice.identity.identity())
                .unwrap()
                .sessions,
            1
        );
        drop(bob_link);
        drop(alice_link);
        assert_eq!(
            bob.identity
                .contact(alice.identity.identity())
                .unwrap()
                .sessions,
            0
        );
    });
}

#[test]
fn a_key_retired_while_the_admission_is_made_durable_gets_no_message_3() {
    // Alice holds Bob's key T1 active and his announced successor T2, in
    // a vault, and dials T2. The admission promotes T2, a change that the
    // dial makes durable before it writes message 3. While it waits, the
    // user blocks Bob: the session is withdrawn, and message 3 is not
    // written.
    run(async {
        let network = MockNetwork::new();
        let alice = node_in(persistent(), &network, 1, 1, 1).await;
        let mut bob = node(&network, 2).await;
        befriend(&alice, &mut bob).await;
        let (mut alice_link, mut bob_link) = confirm_both(&alice, &mut bob).await;
        let successor = bob.identity.begin_rotation().await.unwrap();
        let announcement = bob
            .identity
            .announcement_for(bob_link.session_ref())
            .unwrap();
        bob_link
            .send(&Message::EndpointUpdate(Box::new(successor.clone())))
            .await
            .unwrap();
        assert!(bob.identity.mark_announced(&announcement).await.unwrap());
        step(&mut alice_link, &alice.identity).await.unwrap();
        bob.identity.switch_rotation(false).await.unwrap();
        drop((alice_link, bob_link));

        let identity = alice.identity.clone();
        let bob_id = *bob.identity.identity();
        let blocking = async {
            // Wait until the admission tracked the session, then block.
            loop {
                if identity
                    .contact(&bob_id)
                    .is_some_and(|view| view.sessions > 0)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
            identity.block(&bob_id).await.unwrap();
        };
        let ((dialed, answered), ()) =
            both(dial_and_answer(&alice, &successor, &mut bob), blocking).await;
        assert_eq!(dialed.err(), Some(LinkError::Withdrawn));
        // Bob never got message 3.
        assert!(answered.is_err());
        assert_eq!(alice.identity.kind(&bob_id), RecordKind::Blocked);
    });
}

#[test]
fn an_older_card_of_the_active_key_still_receives_message_3() {
    // Alice holds Bob's card of epoch 2, with the same key and another
    // endpoint, and dials his card of epoch 1. The responder proves the
    // active key: it is the contact, the card is not taken.
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        let first = bob.card();
        alice.identity.import(&first).await.unwrap();
        let moved = party(2, 2, 2, common::elsewhere()).card().clone();
        alice.identity.import(&moved).await.unwrap();
        let (dialed, answered) = dial_and_answer(&alice, &first, &mut bob).await;
        let dialed = dialed.unwrap();
        assert_eq!(dialed.admission.standing, Standing::Requested);
        assert_eq!(dialed.admission.change, Some(CredentialChange::Superseded));
        assert!(answered.is_ok());
        let view = alice.identity.contact(bob.identity.identity()).unwrap();
        assert_eq!(view.credentials.unwrap().active(), &moved);
    });
}

#[test]
fn a_card_that_conflicts_at_the_same_epoch_does_not_receive_message_3() {
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        // Bob's key at epoch 1 with another endpoint: two statements for
        // one epoch.
        let other = party(2, 2, 1, common::elsewhere()).card().clone();
        alice.identity.import(&other).await.unwrap();
        let card = bob.card();
        let (dialed, answered) = dial_and_answer(&alice, &card, &mut bob).await;
        assert_eq!(
            dialed.err(),
            refused(Standing::StaleCard, Some(CredentialChange::Conflict))
        );
        assert_eq!(answered.err(), Some(LinkError::Stream));
    });
}

#[test]
fn a_contact_deleted_or_blocked_during_the_dial_gets_no_message_3() {
    for blocking in [false, true] {
        run(async {
            let network = MockNetwork::new();
            let alice = node(&network, 1).await;
            let mut bob = node(&network, 2).await;
            alice.identity.import(&bob.card()).await.unwrap();
            let card = bob.card();
            let bob_id = *bob.identity.identity();
            let alice_identity = alice.identity.clone();
            let bob_identity = bob.identity.clone();
            let budgets = bob.budgets.clone();
            let service = &mut bob.service;
            let answering = async {
                let stream = service.accept().await.unwrap();
                if blocking {
                    alice_identity.block(&bob_id).await.unwrap();
                } else {
                    alice_identity.delete(&bob_id).await.unwrap();
                }
                monolith_core::link::answer(stream, &budgets, &bob_identity).await
            };
            let (dialed, answered) = both(
                dial(&alice.tor, &alice.budgets, &alice.identity, &card),
                answering,
            )
            .await;
            let standing = if blocking {
                Standing::Blocked
            } else {
                Standing::None
            };
            assert_eq!(dialed.err(), refused(standing, None));
            assert_eq!(answered.err(), Some(LinkError::Stream));
        });
    }
}

#[test]
fn a_pending_key_does_not_receive_message_3() {
    // Alice holds Bob's key T1. Bob now answers with T22 at epoch 2,
    // announced to nobody. Alice dials that card: the key is pending, it
    // gets no message 3, and the card is held for the user.
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node_with(&network, 2, 22, 2).await;
        let old = party(2, 2, 1, bob.identity.endpoint()).card().clone();
        alice.identity.import(&old).await.unwrap();
        // For an accepted contact a new key is never taken by an import, so
        // make Bob accepted the way a request does: the user accepted it.
        let new = bob.card();
        let (dialed, answered) = dial_and_answer(&alice, &new, &mut bob).await;
        assert_eq!(
            dialed.err(),
            refused(Standing::PendingSuccessor, Some(CredentialChange::Pending))
        );
        assert_eq!(answered.err(), Some(LinkError::Stream));
        let view = alice.identity.contact(bob.identity.identity()).unwrap();
        let held = view.credentials.unwrap();
        assert_eq!(held.active(), &old);
        assert_eq!(held.pending_successor(), Some(&new));
        assert!(!held.pending_was_imported());
    });
}

#[test]
fn a_key_older_than_the_announced_successor_gets_nothing() {
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        befriend(&alice, &mut bob).await;
        // Bob announces a successor at epoch 3 on a session of T1.
        let (mut alice_link, mut bob_link) = confirm_both(&alice, &mut bob).await;
        let announced = party(2, 33, 3, bob.identity.endpoint()).card().clone();
        bob_link
            .send(&Message::EndpointUpdate(Box::new(announced.clone())))
            .await
            .unwrap();
        let received = alice_link.receive().await.unwrap();
        let applied = alice
            .identity
            .apply(alice_link.session_ref(), &received)
            .await
            .unwrap();
        assert_eq!(applied.announced, Some(CredentialChange::Authorized));
        // A responder with another key of epoch 2, which the identity
        // superseded through its active key.
        let mut older = node_with(&network, 2, 22, 2).await;
        let card = older.card();
        let before = alice.identity.contact(bob.identity.identity()).unwrap();
        let (dialed, answered) = dial_and_answer(&alice, &card, &mut older).await;
        assert_eq!(
            dialed.err(),
            refused(Standing::StaleCard, Some(CredentialChange::Stale))
        );
        assert_eq!(answered.err(), Some(LinkError::Stream));
        assert_eq!(
            alice
                .identity
                .contact(bob.identity.identity())
                .unwrap()
                .credentials,
            before.credentials
        );
    });
}

#[test]
fn a_promotion_stands_when_message_3_cannot_be_written() {
    // Alice holds Bob's key T1 active and his announced successor T2, and
    // dials T2. The responder proves T2 in message 2, which promotes it at
    // Alice; then the stream is gone before message 3 is written. The
    // promotion stands: it rests on the proof in message 2 alone.
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        befriend(&alice, &mut bob).await;
        let (mut alice_link, mut bob_link) = confirm_both(&alice, &mut bob).await;
        let successor = bob.identity.begin_rotation().await.unwrap();
        bob_link
            .send(&Message::EndpointUpdate(Box::new(successor.clone())))
            .await
            .unwrap();
        step(&mut alice_link, &alice.identity).await.unwrap();
        bob.identity.switch_rotation(true).await.unwrap();
        let party = bob.identity.answering_party().unwrap();
        assert_eq!(party.card(), &successor);
        let service = &mut bob.service;
        // A responder that answers message 1 and then breaks the stream.
        let answering = async {
            let mut stream = service.accept().await.unwrap();
            let responder = HandshakeResponder::new(&party, std::time::Instant::now()).unwrap();
            let mut message_1 = [0_u8; HANDSHAKE_MSG1_LEN];
            stream.read_exact(&mut message_1).await.unwrap();
            let (_, message_2) = responder
                .read_message_1(&message_1, std::time::Instant::now())
                .unwrap();
            stream.write_all(&message_2).await.unwrap();
            drop(stream);
        };
        let (dialed, ()) = both(
            dial(&alice.tor, &alice.budgets, &alice.identity, &successor),
            answering,
        )
        .await;
        assert!(dialed.is_err());
        let held = alice
            .identity
            .contact(bob.identity.identity())
            .unwrap()
            .credentials
            .unwrap();
        assert_eq!(held.active(), &successor);
        assert!(held.retired().is_some());
    });
}

#[test]
fn a_failed_installation_admits_nobody() {
    // A vault write fails: the installation refuses every admission and
    // every change afterwards, and nothing of the local identity is sent.
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation = Installation::create(
            Box::new(dir.clone()),
            &Passphrase::new("test passphrase").unwrap(),
            KdfParams::FLOOR,
        )
        .unwrap();
        let alice = node_in(installation.clone(), &network, 1, 1, 1).await;
        let mut bob = node(&network, 2).await;
        let card = bob.card();
        dir.crash_at(dir.steps() + 1);
        assert!(alice.identity.import(&card).await.is_err());
        assert!(installation.is_failed());
        let (dialed, answered) = dial_and_answer(&alice, &card, &mut bob).await;
        assert!(dialed.is_err());
        assert!(answered.is_err());
        assert!(alice.identity.block(bob.identity.identity()).await.is_err());
    });
}

#[test]
fn a_stranger_request_reaches_the_queue_and_nothing_durable() {
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        bob.identity
            .set_request_mode(monolith_protocol::contact::RequestMode::Open)
            .await
            .unwrap();
        alice.identity.import(&bob.card()).await.unwrap();
        let (alice_end, bob_end) = connect(&alice, &mut bob).await;
        let (mut alice_end, mut bob_end) = (alice_end.unwrap(), bob_end.unwrap());
        alice_end
            .link
            .send(&request(&alice.card(), None))
            .await
            .unwrap();
        let received = bob_end.link.receive().await.unwrap();
        let applied = bob
            .identity
            .apply(bob_end.link.session_ref(), &received)
            .await
            .unwrap();
        assert_eq!(applied.request, Some(Ok(())));
        assert_eq!(bob.identity.requests().len(), 1);
        assert_eq!(
            bob.identity.kind(alice.identity.identity()),
            RecordKind::None
        );
        // Accepting it makes an accepted contact.
        bob.identity
            .accept_request(common::request_of(&bob.identity, alice.identity.identity()))
            .await
            .unwrap();
        assert_eq!(
            bob.identity.kind(alice.identity.identity()),
            RecordKind::Accepted
        );
    });
}

#[test]
fn a_rotation_switches_and_finishes_only_when_due() {
    // The policy of DESIGN_QUESTIONS.md section 11: the identity answers
    // with the new key once every accepted contact was sent the successor,
    // and drops the old key once every accepted contact confirmed a
    // session with the new one, unless the user says so earlier.
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        let mut carol = node(&network, 3).await;
        befriend(&alice, &mut bob).await;
        befriend(&alice, &mut carol).await;
        let old_card = alice.card();
        let successor = alice.identity.begin_rotation().await.unwrap();
        // A second rotation cannot begin while one is in progress.
        assert!(alice.identity.begin_rotation().await.is_err());
        // Nobody was sent the successor: no switch, no finish.
        assert!(!alice.identity.switch_rotation(false).await.unwrap());
        assert!(!alice.identity.finish_rotation(false).await.unwrap());
        assert_eq!(alice.card(), old_card);
        // Bob only: still not due.
        assert!(announce(&alice, &mut bob).await);
        assert!(!alice.identity.switch_rotation(false).await.unwrap());
        // Bob is dialed with the old key until the switch.
        let plan = alice.identity.dial_plan(bob.identity.identity()).unwrap();
        assert_eq!(plan.local.card(), &old_card);
        // Carol too: due.
        assert!(announce(&alice, &mut carol).await);
        assert!(alice.identity.switch_rotation(false).await.unwrap());
        assert_eq!(alice.card(), successor);
        // Nobody confirmed the new key yet: the old key stays.
        assert!(!alice.identity.finish_rotation(false).await.unwrap());
        // The user may end it anyway.
        assert!(alice.identity.finish_rotation(true).await.unwrap());
        assert_eq!(alice.identity.successor_card(), None);
        assert_eq!(alice.card(), successor);
        let _ = (&mut bob, &mut carol);
    });
}

#[test]
fn the_new_key_reaches_no_peer_before_it_is_durable() {
    // A step of a rotation is used towards a peer only once its write is
    // done: the successor is announced only once it is in the vault, and
    // the identity answers, dials and hands out its card with the new key
    // only once the switch is. A crash in between leaves no contact with a
    // key the vault does not hold. (The rotation as decided is what the
    // local side reports meanwhile.)
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation = Installation::create(
            Box::new(dir.clone()),
            &Passphrase::new("test passphrase").unwrap(),
            KdfParams::FLOOR,
        )
        .unwrap();
        let alice = node_in(installation, &network, 1, 1, 1).await;
        let mut bob = node(&network, 2).await;
        befriend(&alice, &mut bob).await;
        let (alice_link, _bob_link) = confirm_both(&alice, &mut bob).await;
        let old_card = alice.card();
        let held = || async {
            while !dir.write_held() {
                tokio::time::sleep(core::time::Duration::from_millis(1)).await;
            }
        };

        dir.hold_writes();
        let _released = Released(&dir);
        let identity = alice.identity.clone();
        let begin = tokio::spawn(async move { identity.begin_rotation().await });
        held().await;
        // Decided, and handed out nowhere: not even as the successor card.
        assert_ne!(alice.identity.rotation(), RotationState::None);
        assert_eq!(alice.identity.successor_card(), None);
        assert!(
            alice
                .identity
                .announcement_for(alice_link.session_ref())
                .is_none()
        );
        dir.release_writes();
        let successor = begin.await.unwrap().unwrap();
        let announcement = alice
            .identity
            .announcement_for(alice_link.session_ref())
            .unwrap();
        assert_eq!(announcement.card(), &successor);
        assert!(alice.identity.mark_announced(&announcement).await.unwrap());

        dir.hold_writes();
        let identity = alice.identity.clone();
        let switch = tokio::spawn(async move { identity.switch_rotation(false).await });
        held().await;
        assert_eq!(alice.card(), old_card);
        assert_eq!(alice.identity.answering_party().unwrap().card(), &old_card);
        let plan = alice.identity.dial_plan(bob.identity.identity()).unwrap();
        assert_eq!(plan.local.card(), &old_card);
        dir.release_writes();
        assert!(switch.await.unwrap().unwrap());
        assert_eq!(alice.card(), successor);
        let plan = alice.identity.dial_plan(bob.identity.identity()).unwrap();
        assert_eq!(plan.local.card(), &successor);
    });
}

#[test]
fn a_dial_cancelled_while_its_admission_is_made_durable_leaves_no_session() {
    // The admission of Bob is tracked by Alice's store; the dial is then
    // dropped (its deadline, an aborted task) while it waits for a write.
    // The store forgets the session: nothing stays tracked for a link that
    // was never made.
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation = Installation::create(
            Box::new(dir.clone()),
            &Passphrase::new("test passphrase").unwrap(),
            KdfParams::FLOOR,
        )
        .unwrap();
        let alice = node_in(installation, &network, 1, 1, 1).await;
        let mut bob = node(&network, 2).await;
        befriend(&alice, &mut bob).await;
        // A write is under way and held: whatever Alice decides now waits.
        dir.hold_writes();
        let _released = Released(&dir);
        let identity = alice.identity.clone();
        let pending = tokio::spawn(async move {
            identity
                .set_request_mode(monolith_protocol::contact::RequestMode::Open)
                .await
        });
        while !dir.write_held() {
            tokio::time::sleep(core::time::Duration::from_millis(1)).await;
        }
        let sessions = |node: &common::Node, peer: &common::Node| {
            node.identity
                .contact(peer.identity.identity())
                .unwrap()
                .sessions
        };
        // The dial runs until Alice's store has admitted Bob, and is then
        // dropped: it waits for the held write, and nothing else.
        let bob_id = *bob.identity.identity();
        {
            let mut dialing = core::pin::pin!(connect(&alice, &mut bob));
            loop {
                let ended = core::future::poll_fn(|cx| {
                    core::task::Poll::Ready(dialing.as_mut().poll(cx).is_ready())
                })
                .await;
                assert!(!ended, "the dial ended while the write was held");
                tokio::time::sleep(core::time::Duration::from_millis(1)).await;
                if alice.identity.contact(&bob_id).unwrap().sessions == 1 {
                    break;
                }
            }
        }
        assert_eq!(sessions(&alice, &bob), 0);
        dir.release_writes();
        pending.await.unwrap().unwrap();
        assert_eq!(sessions(&alice, &bob), 0);
    });
}

#[test]
fn a_failed_write_withdraws_every_session_even_when_nobody_waits() {
    // The task that waited for a write is gone when the write fails. The
    // installation fails closed all the same: the open session of a
    // contact is withdrawn.
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation = Installation::create(
            Box::new(dir.clone()),
            &Passphrase::new("test passphrase").unwrap(),
            KdfParams::FLOOR,
        )
        .unwrap();
        let alice = node_in(installation.clone(), &network, 1, 1, 1).await;
        let mut bob = node(&network, 2).await;
        befriend(&alice, &mut bob).await;
        let (alice_link, _bob_link) = confirm_both(&alice, &mut bob).await;
        dir.hold_writes();
        let _released = Released(&dir);
        let identity = alice.identity.clone();
        let waiter = tokio::spawn(async move {
            identity
                .set_request_mode(monolith_protocol::contact::RequestMode::Open)
                .await
        });
        while !dir.write_held() {
            tokio::time::sleep(core::time::Duration::from_millis(1)).await;
        }
        waiter.abort();
        assert!(waiter.await.unwrap_err().is_cancelled());
        assert!(!alice_link.is_withdrawn());
        dir.crash_at(dir.steps() + 1);
        dir.release_writes();
        let withdrawn = tokio::time::timeout(core::time::Duration::from_secs(10), async {
            while !alice_link.is_withdrawn() {
                tokio::time::sleep(core::time::Duration::from_millis(1)).await;
            }
        })
        .await;
        assert!(withdrawn.is_ok());
        assert!(installation.is_failed());
    });
}

#[test]
fn a_rotation_ends_only_after_a_durable_switch() {
    // Ending a rotation makes the new key the only one; it is used only
    // once the switch is durable, even when the user forces the end.
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation = Installation::create(
            Box::new(dir.clone()),
            &Passphrase::new("test passphrase").unwrap(),
            KdfParams::FLOOR,
        )
        .unwrap();
        let alice = node_in(installation, &network, 1, 1, 1).await;
        let old_card = alice.card();
        let successor = alice.identity.begin_rotation().await.unwrap();
        // Not switched: not due, forced or not.
        assert!(!alice.identity.finish_rotation(true).await.unwrap());
        assert_eq!(alice.card(), old_card);

        dir.hold_writes();
        let _released = Released(&dir);
        let identity = alice.identity.clone();
        let switch = tokio::spawn(async move { identity.switch_rotation(true).await });
        while !dir.write_held() {
            tokio::time::sleep(core::time::Duration::from_millis(1)).await;
        }
        // Switched in memory, not yet durable: still not due. (A finish
        // that went ahead would wait for the held write: the deadline ends
        // the test then instead of letting it hang.)
        let finished = tokio::time::timeout(
            core::time::Duration::from_secs(10),
            alice.identity.finish_rotation(true),
        )
        .await;
        assert!(matches!(finished, Ok(Ok(false))), "{finished:?}");
        assert_eq!(alice.card(), old_card);
        dir.release_writes();
        assert!(switch.await.unwrap().unwrap());
        assert!(alice.identity.finish_rotation(true).await.unwrap());
        assert_eq!(alice.card(), successor);
    });
}

#[test]
fn what_a_refused_dial_recorded_survives_a_restart() {
    // Alice dials Bob's new key, which she holds as nothing yet: it is
    // pending, the dial is refused, and the card is held for the user.
    // That is made durable like any change, though no session uses it.
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let passphrase = Passphrase::new("test passphrase").unwrap();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase, KdfParams::FLOOR).unwrap();
        let alice = node_in(installation, &network, 1, 1, 1).await;
        let mut bob = node_with(&network, 2, 22, 2).await;
        let old = party(2, 2, 1, bob.identity.endpoint()).card().clone();
        alice.identity.import(&old).await.unwrap();
        let new = bob.card();
        let (dialed, _) = dial_and_answer(&alice, &new, &mut bob).await;
        assert_eq!(
            dialed.err(),
            refused(Standing::PendingSuccessor, Some(CredentialChange::Pending))
        );
        let pending = |installation: &Installation| {
            installation.identities()[0]
                .contact(bob.identity.identity())
                .unwrap()
                .credentials
                .unwrap()
                .pending_successor()
                .cloned()
        };
        assert_eq!(pending(&alice.installation), Some(new.clone()));
        let (reopened, _) =
            Installation::open(Box::new(dir.restart(CrashOutcome::ALL[0])), &passphrase).unwrap();
        assert_eq!(pending(&reopened), Some(new));
    });
}

#[test]
fn what_an_answer_without_a_slot_recorded_survives_a_restart() {
    // Bob holds Alice's card of epoch 1. She dials him with her card of
    // epoch 2, the same key: his admission takes the newer card, and then
    // finds no slot for a contact session. The link is refused, the newer
    // card stays, and it is made durable.
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let passphrase = Passphrase::new("test passphrase").unwrap();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase, KdfParams::FLOOR).unwrap();
        let alice = node_with(&network, 1, 1, 2).await;
        let mut bob = node_in(installation, &network, 2, 2, 1).await;
        bob.budgets = Budgets::with_limits(MAX_INBOUND_HANDSHAKES, 0, MAX_CONCURRENT_DIALS);
        let first = party(1, 1, 1, alice.identity.endpoint()).card().clone();
        bob.identity.import(&first).await.unwrap();
        alice.identity.import(&bob.card()).await.unwrap();
        let (_, answered) = connect(&alice, &mut bob).await;
        assert_eq!(answered.err(), Some(LinkError::Budget));
        let active = |installation: &Installation| {
            installation.identities()[0]
                .contact(alice.identity.identity())
                .unwrap()
                .credentials
                .unwrap()
                .active()
                .clone()
        };
        assert_eq!(active(&bob.installation), alice.card());
        let (reopened, _) =
            Installation::open(Box::new(dir.restart(CrashOutcome::ALL[0])), &passphrase).unwrap();
        assert_eq!(active(&reopened), alice.card());
    });
}

#[test]
fn what_a_dial_without_a_slot_recorded_survives_a_restart() {
    // Alice holds Bob's card of epoch 1 and dials his card of epoch 2, the
    // same key. Her admission takes the newer card, and then she has no
    // slot for a contact session.
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let passphrase = Passphrase::new("test passphrase").unwrap();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase, KdfParams::FLOOR).unwrap();
        let mut alice = node_in(installation, &network, 1, 1, 1).await;
        alice.budgets = Budgets::with_limits(MAX_INBOUND_HANDSHAKES, 0, MAX_CONCURRENT_DIALS);
        let mut bob = node_with(&network, 2, 2, 2).await;
        let first = party(2, 2, 1, bob.identity.endpoint()).card().clone();
        alice.identity.import(&first).await.unwrap();
        let newer = bob.card();
        let (dialed, _) = dial_and_answer(&alice, &newer, &mut bob).await;
        assert_eq!(dialed.err(), Some(LinkError::Budget));
        let active = |installation: &Installation| {
            installation.identities()[0]
                .contact(bob.identity.identity())
                .unwrap()
                .credentials
                .unwrap()
                .active()
                .clone()
        };
        assert_eq!(active(&alice.installation), bob.card());
        let (reopened, _) =
            Installation::open(Box::new(dir.restart(CrashOutcome::ALL[0])), &passphrase).unwrap();
        assert_eq!(active(&reopened), bob.card());
    });
}

#[test]
fn what_an_answer_without_a_slot_for_strangers_recorded_survives_a_restart() {
    // Bob holds Alice with her key T1. Every slot for strangers is taken,
    // and as many evicted strangers are still draining. Alice dials him
    // with T22, announced to nobody: the key is pending, so her session is
    // not a contact's and needs a slot for strangers, which there is not.
    // The pending card stays, and is made durable.
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let passphrase = Passphrase::new("test passphrase").unwrap();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase, KdfParams::FLOOR).unwrap();
        let mut bob = node_in(installation, &network, 2, 2, 1).await;
        let mut held = Vec::new();
        for seed in 0..2 * MAX_UNKNOWN_SESSIONS {
            let stranger = node(&network, 40 + u8::try_from(seed).unwrap()).await;
            stranger.identity.import(&bob.card()).await.unwrap();
            let (asking, answering) = connect(&stranger, &mut bob).await;
            held.push((stranger, asking.unwrap(), answering.unwrap()));
        }
        let alice = node_with(&network, 1, 22, 2).await;
        let old = party(1, 1, 1, alice.identity.endpoint()).card().clone();
        bob.identity.import(&old).await.unwrap();
        alice.identity.import(&bob.card()).await.unwrap();
        let (_, answered) = connect(&alice, &mut bob).await;
        assert_eq!(answered.err(), Some(LinkError::Budget));
        let pending = |installation: &Installation| {
            installation.identities()[0]
                .contact(alice.identity.identity())
                .unwrap()
                .credentials
                .unwrap()
                .pending_successor()
                .cloned()
        };
        assert_eq!(pending(&bob.installation), Some(alice.card()));
        let (reopened, _) =
            Installation::open(Box::new(dir.restart(CrashOutcome::ALL[0])), &passphrase).unwrap();
        assert_eq!(pending(&reopened), Some(alice.card()));
        drop(held);
    });
}

#[test]
fn the_progress_of_one_rotation_never_counts_for_the_next() {
    // Alice announces the successor of rotation R1 to Bob, and records
    // that only later, when R1 was ended and R2 begun. Bob never got the
    // successor of R2: R2 must not switch on his account.
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        befriend(&alice, &mut bob).await;
        let (alice_link, _bob_link) = confirm_both(&alice, &mut bob).await;
        alice.identity.begin_rotation().await.unwrap();
        let first = alice
            .identity
            .announcement_for(alice_link.session_ref())
            .unwrap();
        assert!(alice.identity.switch_rotation(true).await.unwrap());
        assert!(alice.identity.finish_rotation(true).await.unwrap());
        let second = alice.identity.begin_rotation().await.unwrap();
        assert_ne!(first.card(), &second);
        // The late record of what was sent in R1: it is for R1, which is
        // over, and counts for nothing. Bob is the only contact, so the
        // switch would be due on his account alone.
        assert!(!alice.identity.mark_announced(&first).await.unwrap());
        assert!(!alice.identity.switch_rotation(false).await.unwrap());
        // Dave becomes a contact on a session of the old key, which he
        // confirms: that is no confirmation of the new key.
        let mut dave = node(&network, 4).await;
        befriend(&alice, &mut dave).await;
        let view = alice.identity.contact(dave.identity.identity()).unwrap();
        assert!(!view.successor_promoted);
        assert!(alice.identity.switch_rotation(true).await.unwrap());
        assert!(!alice.identity.finish_rotation(false).await.unwrap());
    });
}

#[test]
fn the_progress_of_a_rotation_survives_a_restart_with_it_and_no_further() {
    // Bob and Carol were sent the successor of R1, and the vault holds
    // that with R1: after a restart R1 may switch. R1 then ends and R2
    // begins: after another restart, nothing of R1's progress counts.
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let passphrase = Passphrase::new("test passphrase").unwrap();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase, KdfParams::FLOOR).unwrap();
        let alice = node_in(installation, &network, 1, 1, 1).await;
        let mut bob = node(&network, 2).await;
        let mut carol = node(&network, 3).await;
        befriend(&alice, &mut bob).await;
        befriend(&alice, &mut carol).await;
        alice.identity.begin_rotation().await.unwrap();
        assert!(announce(&alice, &mut bob).await);
        assert!(announce(&alice, &mut carol).await);
        let open = |dir: &MemoryDir| {
            Installation::open(Box::new(dir.clone()), &passphrase)
                .unwrap()
                .0
        };
        let second_run = dir.restart(CrashOutcome::ALL[0]);
        let restarted = open(&second_run);
        let local = restarted.identities()[0].clone();
        assert!(
            local
                .contact(bob.identity.identity())
                .unwrap()
                .successor_announced
        );
        assert!(local.switch_rotation(false).await.unwrap());
        assert!(local.finish_rotation(true).await.unwrap());
        local.begin_rotation().await.unwrap();
        drop(local);
        drop(restarted);
        let restarted = open(&second_run.restart(CrashOutcome::ALL[0]));
        let local = restarted.identities()[0].clone();
        assert!(local.rotation_id().is_some());
        assert!(
            !local
                .contact(bob.identity.identity())
                .unwrap()
                .successor_announced
        );
        assert!(!local.switch_rotation(false).await.unwrap());
        let _ = (&mut bob, &mut carol);
    });
}

#[test]
fn a_late_announcement_counts_only_for_the_contact_it_was_made_to() {
    // Alice announces the successor to Bob on a session of the old key and
    // records it late. Meanwhile Bob was deleted and became a contact
    // again: a new contact, that never got the successor. The late record
    // counts for nobody, and the switch is not due on its account.
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        befriend(&alice, &mut bob).await;
        let (alice_link, _bob_link) = confirm_both(&alice, &mut bob).await;
        alice.identity.begin_rotation().await.unwrap();
        let announcement = alice
            .identity
            .announcement_for(alice_link.session_ref())
            .unwrap();
        alice
            .identity
            .delete(bob.identity.identity())
            .await
            .unwrap();
        befriend(&alice, &mut bob).await;
        assert!(!alice.identity.mark_announced(&announcement).await.unwrap());
        assert!(
            !alice
                .identity
                .contact(bob.identity.identity())
                .unwrap()
                .successor_announced
        );
        assert!(!alice.identity.switch_rotation(false).await.unwrap());
    });
}

/// What happens to Bob, or to the session Alice announced on, between the
/// announcement and its late completion.
#[derive(Clone, Copy, Debug)]
enum Meanwhile {
    /// Deleted, and his card imported again: a requested contact anew.
    ImportedAgain,
    /// Blocked.
    Blocked,
    /// He changed his own key, and Alice promoted it: the session Alice
    /// announced on was withdrawn.
    Promoted,
    /// Deleted, and a contact again on another transport key.
    OtherKey,
    /// Deleted and a contact again, and Alice's rotation ended and another
    /// began.
    NextRotation,
}

#[test]
fn a_late_announcement_counts_for_nothing_once_its_contact_or_session_changed() {
    for meanwhile in [
        Meanwhile::ImportedAgain,
        Meanwhile::Blocked,
        Meanwhile::Promoted,
        Meanwhile::OtherKey,
        Meanwhile::NextRotation,
    ] {
        run(async {
            let network = MockNetwork::new();
            let mut alice = node(&network, 1).await;
            let mut bob = node(&network, 2).await;
            befriend(&alice, &mut bob).await;
            let (mut alice_link, mut bob_link) = confirm_both(&alice, &mut bob).await;
            alice.identity.begin_rotation().await.unwrap();
            let announcement = alice
                .identity
                .announcement_for(alice_link.session_ref())
                .unwrap();
            let bob_id = *bob.identity.identity();
            match meanwhile {
                Meanwhile::ImportedAgain => {
                    alice.identity.delete(&bob_id).await.unwrap();
                    alice.identity.import(&bob.card()).await.unwrap();
                }
                Meanwhile::Blocked => alice.identity.block(&bob_id).await.unwrap(),
                Meanwhile::Promoted => {
                    let successor = bob.identity.begin_rotation().await.unwrap();
                    let of_bob = bob
                        .identity
                        .announcement_for(bob_link.session_ref())
                        .unwrap();
                    bob_link
                        .send(&Message::EndpointUpdate(Box::new(successor)))
                        .await
                        .unwrap();
                    assert!(bob.identity.mark_announced(&of_bob).await.unwrap());
                    step(&mut alice_link, &alice.identity).await.unwrap();
                    assert!(bob.identity.switch_rotation(false).await.unwrap());
                    let (dialed, answered) = connect(&bob, &mut alice).await;
                    assert_eq!(
                        answered.unwrap().admission.change,
                        Some(CredentialChange::Promoted)
                    );
                    drop(dialed);
                    assert!(alice_link.is_withdrawn());
                }
                Meanwhile::OtherKey => {
                    alice.identity.delete(&bob_id).await.unwrap();
                    let mut moved = node_with(&network, 2, 22, 1).await;
                    befriend(&alice, &mut moved).await;
                }
                Meanwhile::NextRotation => {
                    alice.identity.delete(&bob_id).await.unwrap();
                    befriend(&alice, &mut bob).await;
                    assert!(alice.identity.switch_rotation(true).await.unwrap());
                    assert!(alice.identity.finish_rotation(true).await.unwrap());
                    alice.identity.begin_rotation().await.unwrap();
                }
            }
            assert!(
                !alice.identity.mark_announced(&announcement).await.unwrap(),
                "{meanwhile:?}"
            );
            if let Some(view) = alice.identity.contact(&bob_id) {
                assert!(!view.successor_announced, "{meanwhile:?}");
            }
        });
    }
}

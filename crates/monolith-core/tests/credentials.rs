//! Admission against the contact state of the moment, and withdrawal of
//! a link whose transport key is retired, on the in-memory Tor.
//!
//! The contact state is the smallest thing that does what the contact
//! store of Phase 4 has to do: credentials and the withdrawals of the
//! sessions admitted for them, behind one lock, changed in one step with
//! each admission.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use core::future::Future;
use std::sync::Mutex;

use monolith_core::budget::Budgets;
use monolith_core::link::{LinkError, Withdrawal, answer, dial};
use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::body::{Message, MessageId};
use monolith_protocol::card::{ContactCard, EndpointSet};
use monolith_protocol::credential::{CredentialChange, Credentials};
use monolith_protocol::session::{Action, PeerRecord, Standing};
use monolith_protocol::text::ChatText;
use monolith_session::{Admitted, LocalParty, SessionError, TransportSecretKey};
use monolith_tor::{KeySource, MockNetwork, OnionService, TorBackend};

fn run<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// Runs two futures to completion on the current task.
async fn both<A: Future, B: Future>(a: A, b: B) -> (A::Output, B::Output) {
    let mut a = core::pin::pin!(a);
    let mut b = core::pin::pin!(b);
    let (mut out_a, mut out_b) = (None, None);
    core::future::poll_fn(|cx| {
        if out_a.is_none() {
            if let core::task::Poll::Ready(value) = a.as_mut().poll(cx) {
                out_a = Some(value);
            }
        }
        if out_b.is_none() {
            if let core::task::Poll::Ready(value) = b.as_mut().poll(cx) {
                out_b = Some(value);
            }
        }
        if out_a.is_some() && out_b.is_some() {
            core::task::Poll::Ready(())
        } else {
            core::task::Poll::Pending
        }
    })
    .await;
    (out_a.unwrap(), out_b.unwrap())
}

fn identity(seed: u8) -> IdentitySecretKey {
    IdentitySecretKey::from_seed(&[seed; 32])
}

/// The party of identity `seed` with a fresh transport key, at `epoch`,
/// reachable at `endpoint`.
fn party(seed: u8, epoch: u64, endpoint: OnionServiceKey) -> LocalParty {
    LocalParty::issue(
        &identity(seed),
        TransportSecretKey::generate().unwrap(),
        EndpointEpoch::new(epoch).unwrap(),
        EndpointSet::single(endpoint),
    )
    .unwrap()
}

fn chat(text: &str) -> Message {
    Message::ChatMessage {
        id: MessageId::from_bytes([1; 16]),
        text: ChatText::new(text).unwrap(),
    }
}

/// What the local side holds of one contact: its credentials and the
/// withdrawal of every session admitted for it, with the card of that
/// session.
struct Contact {
    held: Credentials,
    sessions: Vec<(ContactCard, Withdrawal)>,
}

impl Contact {
    fn new(card: ContactCard) -> Mutex<Self> {
        Mutex::new(Self {
            held: Credentials::new(card),
            sessions: Vec::new(),
        })
    }

    /// Keeps the withdrawal of a session that was just admitted, and
    /// withdraws every session whose key no longer stands for the
    /// contact. Called in the same step as the admission.
    fn admitted(
        &mut self,
        admitted: Result<Admitted, SessionError>,
        withdrawal: &Withdrawal,
    ) -> Result<Admitted, SessionError> {
        let admitted = admitted?;
        self.sessions
            .push((admitted.0.peer_card().clone(), withdrawal.clone()));
        self.withdraw_retired();
        Ok(admitted)
    }

    fn withdraw_retired(&mut self) {
        let held = &self.held;
        self.sessions.retain(|(card, withdrawal)| {
            let stands = held.authorizes(card);
            if !stands {
                withdrawal.withdraw();
            }
            stands
        });
    }
}

/// Bob's side of a confirmed session: sends its ContactAccept and waits
/// for Alice's.
async fn confirm<S>(end: &mut monolith_core::link::Established<S>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    assert_eq!(end.first, vec![Action::SendContactAccept]);
    end.link.send(&Message::ContactAccept).await.unwrap();
    assert_eq!(
        end.link.receive().await.unwrap().actions,
        vec![Action::Confirmed]
    );
}

#[test]
fn a_dial_is_admitted_against_the_contact_state_after_the_handshake() {
    // Alice holds Bob with his card of epoch 1 and key T1 and dials it.
    // While the dial is in progress, before Bob has answered, Alice's
    // state changes: the user confirmed Bob's card of epoch 2 with key
    // T2. The responder then proves T1. The session must not get the
    // standing of an accepted contact from what Alice held when she began
    // to dial.
    run(async {
        let network = MockNetwork::new();
        let budgets = Budgets::new();
        let (alice_tor, bob_tor) = (network.backend(), network.backend());
        let mut bob_service = bob_tor.publish_onion(KeySource::Generate).await.unwrap();
        let alice_service = alice_tor.publish_onion(KeySource::Generate).await.unwrap();
        let bob = party(2, 1, *bob_service.service_key());
        let alice = party(1, 1, *alice_service.service_key());
        let bob_card = bob.card().clone();
        let successor = party(2, 2, *bob_service.service_key()).card().clone();
        let alice_holds_bob = Contact::new(bob_card.clone());
        let isolation = alice_tor.isolation_group().unwrap();

        let bob_side = async {
            let stream = bob_service.accept().await.unwrap();
            {
                let mut contact = alice_holds_bob.lock().unwrap();
                assert_eq!(
                    contact.held.import(successor.clone()),
                    Ok(CredentialChange::Pending)
                );
                assert_eq!(
                    contact.held.confirm(&successor),
                    Ok(CredentialChange::Promoted)
                );
            }
            answer(stream, &budgets, &bob, |peer, _| {
                peer.admit(PeerRecord::Accepted(&mut Credentials::new(
                    alice.card().clone(),
                )))
            })
            .await
            .unwrap()
        };
        let alice_side = dial(
            &alice_tor,
            &budgets,
            &alice,
            &bob_card,
            &isolation,
            // Only the admission, so that what it decides is seen alone.
            |peer, _| {
                let mut contact = alice_holds_bob.lock().unwrap();
                peer.admit(PeerRecord::Accepted(&mut contact.held))
            },
        );
        let (alice_end, mut bob_end) = both(alice_side, bob_side).await;
        let mut alice_end = alice_end.unwrap();
        assert_eq!(alice_end.admission.standing, Standing::StaleCard);
        assert_eq!(alice_end.admission.change, Some(CredentialChange::Stale));
        assert!(alice_end.first.is_empty());
        assert_eq!(alice_holds_bob.lock().unwrap().held.active(), &successor);

        // The holder of T1 offers its ContactAccept and gets a Close; Alice
        // delivers nothing.
        bob_end.link.send(&Message::ContactAccept).await.unwrap();
        let received = alice_end.link.receive().await.unwrap();
        assert_eq!(received.actions, vec![Action::SendClose]);
        assert!(alice_end.link.send(&chat("x")).await.is_err());
    });
}

#[test]
fn a_rotation_withdraws_the_link_of_the_retired_key() {
    // Alice talks to Bob with T1. She announces T2 on that session, and
    // later dials with T2. Bob's admission of the T2 session promotes it,
    // and in the same step the T1 link is withdrawn: a message Alice sent
    // on it before is not delivered, and the link ends with a Close.
    run(async {
        let network = MockNetwork::new();
        let budgets = Budgets::new();
        let (alice_tor, bob_tor) = (network.backend(), network.backend());
        let mut bob_service = bob_tor.publish_onion(KeySource::Generate).await.unwrap();
        let alice_service = alice_tor.publish_onion(KeySource::Generate).await.unwrap();
        let bob = party(2, 1, *bob_service.service_key());
        let alice_t1 = party(1, 1, *alice_service.service_key());
        let alice_t2 = party(1, 2, *alice_service.service_key());
        let bob_card = bob.card().clone();
        let bob_holds_alice = Contact::new(alice_t1.card().clone());
        let alice_holds_bob = Contact::new(bob_card.clone());

        let bob_answers = |stream| {
            answer(stream, &budgets, &bob, |peer, withdrawal| {
                let mut contact = bob_holds_alice.lock().unwrap();
                let admitted = peer.admit(PeerRecord::Accepted(&mut contact.held));
                contact.admitted(admitted, withdrawal)
            })
        };
        let alice_dials = |local| {
            let isolation = alice_tor.isolation_group().unwrap();
            let alice_tor = &alice_tor;
            let bob_card = &bob_card;
            let alice_holds_bob = &alice_holds_bob;
            let budgets = &budgets;
            async move {
                dial(
                    alice_tor,
                    budgets,
                    local,
                    bob_card,
                    &isolation,
                    |peer, withdrawal| {
                        let mut contact = alice_holds_bob.lock().unwrap();
                        let admitted = peer.admit(PeerRecord::Accepted(&mut contact.held));
                        contact.admitted(admitted, withdrawal)
                    },
                )
                .await
            }
        };

        // The T1 session, confirmed.
        let bob_side = async { bob_answers(bob_service.accept().await.unwrap()).await };
        let (alice_end, bob_end) = both(alice_dials(&alice_t1), bob_side).await;
        let (mut alice_link, mut bob_link) = (alice_end.unwrap(), bob_end.unwrap());
        alice_link.link.send(&Message::ContactAccept).await.unwrap();
        confirm(&mut bob_link).await;
        alice_link.link.receive().await.unwrap();

        // T2 is announced on it and authorized.
        let announcement = Message::EndpointUpdate(Box::new(alice_t2.card().clone()));
        alice_link.link.send(&announcement).await.unwrap();
        let received = bob_link.link.receive().await.unwrap();
        let Message::EndpointUpdate(card) = received.message else {
            panic!("not an EndpointUpdate");
        };
        {
            let mut contact = bob_holds_alice.lock().unwrap();
            let session = bob_link.link.session().peer_card().clone();
            assert_eq!(
                contact.held.announce(&card, &session),
                Ok(CredentialChange::Authorized)
            );
        }

        // A message on the T1 session that Bob has not read yet.
        alice_link.link.send(&chat("in flight")).await.unwrap();

        // Alice dials with T2. Bob's admission promotes T2 and withdraws
        // the T1 link in the same step.
        let bob_side = async { bob_answers(bob_service.accept().await.unwrap()).await };
        let (alice_end, bob_end) = both(alice_dials(&alice_t2), bob_side).await;
        let bob_t2 = bob_end.unwrap();
        assert_eq!(bob_t2.admission.change, Some(CredentialChange::Promoted));
        assert_eq!(bob_t2.admission.standing, Standing::Accepted);
        assert!(bob_link.link.withdrawal().is_withdrawn());
        let mut alice_t2_link = alice_end.unwrap();

        // The message in flight is not delivered.
        assert_eq!(
            bob_link.link.receive().await.err(),
            Some(LinkError::Withdrawn)
        );
        assert_eq!(
            bob_link.link.send(&chat("no")).await.err(),
            Some(LinkError::Withdrawn)
        );
        // Alice's T1 link sees an ordinary Close.
        let closed = alice_link.link.receive().await.unwrap();
        assert_eq!(closed.message, Message::Close);

        // The T2 session works.
        let mut bob_t2 = bob_t2;
        alice_t2_link
            .link
            .send(&Message::ContactAccept)
            .await
            .unwrap();
        confirm(&mut bob_t2).await;
        alice_t2_link.link.receive().await.unwrap();
        alice_t2_link.link.send(&chat("from T2")).await.unwrap();
        let received = bob_t2.link.receive().await.unwrap();
        assert_eq!(received.message, chat("from T2"));
        assert_eq!(received.actions, vec![Action::Deliver]);

        // A new session with T1 is not a contact session.
        let bob_side = async { bob_answers(bob_service.accept().await.unwrap()).await };
        let (_, bob_end) = both(alice_dials(&alice_t1), bob_side).await;
        assert_eq!(bob_end.unwrap().admission.standing, Standing::StaleCard);
    });
}

#[test]
fn a_link_that_waits_for_the_peer_wakes_up_when_it_is_withdrawn() {
    run(async {
        let network = MockNetwork::new();
        let budgets = Budgets::new();
        let (alice_tor, bob_tor) = (network.backend(), network.backend());
        let mut bob_service = bob_tor.publish_onion(KeySource::Generate).await.unwrap();
        let alice_service = alice_tor.publish_onion(KeySource::Generate).await.unwrap();
        let bob = party(2, 1, *bob_service.service_key());
        let alice = party(1, 1, *alice_service.service_key());
        let bob_holds_alice = Contact::new(alice.card().clone());
        let isolation = alice_tor.isolation_group().unwrap();

        let bob_side = async {
            let stream = bob_service.accept().await.unwrap();
            answer(stream, &budgets, &bob, |peer, withdrawal| {
                let mut contact = bob_holds_alice.lock().unwrap();
                let admitted = peer.admit(PeerRecord::Accepted(&mut contact.held));
                contact.admitted(admitted, withdrawal)
            })
            .await
            .unwrap()
        };
        let alice_side = dial(
            &alice_tor,
            &budgets,
            &alice,
            bob.card(),
            &isolation,
            |peer, _| {
                peer.admit(PeerRecord::Accepted(&mut Credentials::new(
                    bob.card().clone(),
                )))
            },
        );
        let (alice_end, mut bob_end) = both(alice_side, bob_side).await;
        let mut alice_end = alice_end.unwrap();
        alice_end.link.send(&Message::ContactAccept).await.unwrap();
        confirm(&mut bob_end).await;
        alice_end.link.receive().await.unwrap();

        // Bob's link waits. Meanwhile Bob's user confirms a new key of
        // Alice that was shown to him out of band.
        let successor = party(1, 2, *alice_service.service_key()).card().clone();
        let promote = async {
            tokio::task::yield_now().await;
            let mut contact = bob_holds_alice.lock().unwrap();
            contact.held.import(successor.clone()).unwrap();
            contact.held.confirm(&successor).unwrap();
            contact.withdraw_retired();
        };
        let (waited, ()) = both(bob_end.link.receive(), promote).await;
        assert_eq!(waited.err(), Some(LinkError::Withdrawn));
        let closed = alice_end.link.receive().await.unwrap();
        assert_eq!(closed.message, Message::Close);
        assert_eq!(closed.actions, vec![Action::Disconnect]);
    });
}

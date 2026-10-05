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
use monolith_protocol::ProtocolError;
use monolith_protocol::body::{Message, MessageId};
use monolith_protocol::card::{ContactCard, EndpointSet};
use monolith_protocol::credential::{CredentialChange, Credentials};
use monolith_protocol::session::{Action, Admission, PeerRecord, Standing};
use monolith_protocol::text::ChatText;
use monolith_session::{
    Admitted, LocalParty, OutboundAdmission, OutboundPeer, SessionError, TransportSecretKey,
};
use monolith_tor::{KeySource, MockNetwork, OnionService, TorBackend};

fn run<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// As `run`, with time that advances only when every task waits, so that a
/// wait for the idle limit would end at once instead of after minutes.
fn run_paused<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
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

    /// Keeps the withdrawal of a session that was just admitted as a
    /// contact's, and withdraws every session whose key no longer stands
    /// for the contact. Called in the same step as the admission. A
    /// session admitted with any other standing is left on the path of a
    /// stranger: ending it here would show the peer that it is held as a
    /// contact.
    fn admitted(
        &mut self,
        admitted: Result<Admitted, SessionError>,
        withdrawal: &Withdrawal,
    ) -> Result<Admitted, SessionError> {
        let admitted = admitted?;
        if admitted.1.standing.is_contact_record() {
            self.sessions
                .push((admitted.0.peer_card().clone(), withdrawal.clone()));
        }
        self.withdraw_retired();
        Ok(admitted)
    }

    /// The same for an outbound admission, which makes a session only
    /// when the responder may learn the local identity.
    fn admitted_outbound(
        &mut self,
        admitted: Result<OutboundAdmission, SessionError>,
        withdrawal: &Withdrawal,
    ) -> Result<OutboundAdmission, SessionError> {
        let admitted = admitted?;
        if let OutboundAdmission::Granted { session, .. } = &admitted {
            self.sessions
                .push((session.peer_card().clone(), withdrawal.clone()));
        }
        self.withdraw_retired();
        Ok(admitted)
    }

    /// Withdraws the sessions whose key no longer stands for the contact
    /// and forgets those and the links that are gone.
    fn withdraw_retired(&mut self) {
        let held = &self.held;
        self.sessions.retain(|(card, withdrawal)| {
            if withdrawal.is_ended() {
                return false;
            }
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
    // to dial, and Alice does not send her identity in message 3 to the
    // holder of a key Bob has left.
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
            answer(
                stream,
                &budgets,
                &bob,
                |_, _| -> Result<Admitted, SessionError> { panic!("message 3 arrived") },
            )
            .await
        };
        let decided = core::cell::Cell::new(None);
        let alice_side = dial(
            &alice_tor,
            &budgets,
            &alice,
            &bob_card,
            &isolation,
            // Only the admission, so that what it decides is seen alone.
            |peer, _| {
                let mut contact = alice_holds_bob.lock().unwrap();
                let admitted = peer.admit(PeerRecord::Accepted(&mut contact.held));
                decided.set(admitted.as_ref().ok().map(admission));
                admitted
            },
        );
        let (alice_end, bob_end) = both(alice_side, bob_side).await;
        let admission = decided.get().unwrap();
        assert_eq!(admission.standing, Standing::StaleCard);
        assert_eq!(admission.change, Some(CredentialChange::Stale));
        assert_eq!(alice_holds_bob.lock().unwrap().held.active(), &successor);
        assert_eq!(alice_end.err(), Some(LinkError::Refused(admission)));
        // Bob never got message 3: his side ended without an admission.
        assert_eq!(bob_end.err(), Some(LinkError::Stream));
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
                        contact.admitted_outbound(admitted, withdrawal)
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

        // Two messages that arrive together. Bob takes the first; the
        // second waits in his buffer. A third is still on the stream.
        alice_link.link.send(&chat("first")).await.unwrap();
        alice_link.link.send(&chat("buffered")).await.unwrap();
        assert_eq!(
            bob_link.link.receive().await.unwrap().message,
            chat("first")
        );
        alice_link.link.send(&chat("in flight")).await.unwrap();

        // Alice dials with T2. Bob's admission promotes T2 and withdraws
        // the T1 link in the same step.
        let bob_side = async { bob_answers(bob_service.accept().await.unwrap()).await };
        let (alice_end, bob_end) = both(alice_dials(&alice_t2), bob_side).await;
        let bob_t2 = bob_end.unwrap();
        assert_eq!(bob_t2.admission.change, Some(CredentialChange::Promoted));
        assert_eq!(bob_t2.admission.standing, Standing::Accepted);
        assert!(bob_link.link.is_withdrawn());
        let mut alice_t2_link = alice_end.unwrap();

        // Neither the buffered message nor the one in flight is delivered.
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

        // A new session with T1 is not a contact session. It is left on
        // the path of a stranger: not kept for withdrawal, not ended early.
        let kept = bob_holds_alice.lock().unwrap().sessions.len();
        let bob_side = async { bob_answers(bob_service.accept().await.unwrap()).await };
        let (_, bob_end) = both(alice_dials(&alice_t1), bob_side).await;
        let bob_end = bob_end.unwrap();
        assert_eq!(bob_end.admission.standing, Standing::StaleCard);
        assert!(!bob_end.link.is_withdrawn());
        assert_eq!(bob_holds_alice.lock().unwrap().sessions.len(), kept);
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

#[test]
fn a_withdrawn_link_fails_at_once_every_time() {
    // The key of the session is retired before the link was ever polled.
    // Every later call fails at once, the first one writes the Close, and
    // nothing waits for the idle limit.
    run_paused(async {
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
        let successor = party(1, 2, *alice_service.service_key()).card().clone();
        {
            let mut contact = bob_holds_alice.lock().unwrap();
            contact.held.import(successor.clone()).unwrap();
            contact.held.confirm(&successor).unwrap();
            contact.withdraw_retired();
        }
        assert!(bob_end.link.is_withdrawn());

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
            bob_end.link.send(&chat("no")).await.err(),
            Some(LinkError::Withdrawn)
        );
        assert_eq!(
            bob_end.link.receive().await.err(),
            Some(LinkError::Withdrawn)
        );
        assert_eq!(started.elapsed(), core::time::Duration::ZERO);
        // One Close reached Alice, then the stream ended.
        let closed = alice_end.link.receive().await.unwrap();
        assert_eq!(closed.message, Message::Close);
        assert!(alice_end.link.receive().await.is_err());
    });
}

#[test]
fn a_withdrawal_first_seen_by_send_writes_the_close() {
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

        // The key Alice used is retired by a confirmation of the user.
        let successor = party(1, 2, *alice_service.service_key()).card().clone();
        {
            let mut contact = bob_holds_alice.lock().unwrap();
            contact.held.import(successor.clone()).unwrap();
            contact.held.confirm(&successor).unwrap();
            contact.withdraw_retired();
        }
        assert_eq!(
            bob_end.link.send(&chat("after")).await.err(),
            Some(LinkError::Withdrawn)
        );
        let closed = alice_end.link.receive().await.unwrap();
        assert_eq!(closed.message, Message::Close);
        assert_eq!(closed.actions, vec![Action::Disconnect]);
    });
}

#[test]
fn the_withdrawal_of_a_link_that_is_gone_says_so() {
    // A dropped link, and a link that `answer` never made because the
    // budget for strangers was full after the admission, both report that
    // they ended, so that the contact state can forget them.
    run(async {
        let network = MockNetwork::new();
        let budgets = Budgets::new();
        let (alice_tor, bob_tor) = (network.backend(), network.backend());
        let mut bob_service = bob_tor.publish_onion(KeySource::Generate).await.unwrap();
        let alice_service = alice_tor.publish_onion(KeySource::Generate).await.unwrap();
        let bob = party(2, 1, *bob_service.service_key());
        let alice = party(1, 1, *alice_service.service_key());
        let kept: Mutex<Vec<Withdrawal>> = Mutex::new(Vec::new());

        // A contact session, dropped.
        let isolation = alice_tor.isolation_group().unwrap();
        let bob_side = async {
            let stream = bob_service.accept().await.unwrap();
            answer(stream, &budgets, &bob, |peer, withdrawal| {
                kept.lock().unwrap().push(withdrawal.clone());
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
            bob.card(),
            &isolation,
            |peer, _| {
                peer.admit(PeerRecord::Accepted(&mut Credentials::new(
                    bob.card().clone(),
                )))
            },
        );
        let (_, bob_end) = both(alice_side, bob_side).await;
        let first = kept.lock().unwrap()[0].clone();
        assert!(!first.is_ended());
        drop(bob_end);
        assert!(first.is_ended());

        // A stranger when no slot for strangers is left.
        let held: Vec<_> = core::iter::from_fn(|| budgets.unknown_session()).collect();
        assert!(!held.is_empty());
        let isolation = alice_tor.isolation_group().unwrap();
        let bob_side = async {
            let stream = bob_service.accept().await.unwrap();
            answer(stream, &budgets, &bob, |peer, withdrawal| {
                kept.lock().unwrap().push(withdrawal.clone());
                peer.admit(PeerRecord::None)
            })
            .await
        };
        let alice_side = dial(
            &alice_tor,
            &budgets,
            &alice,
            bob.card(),
            &isolation,
            |peer, _| {
                // A stranger that dials asks to become a contact.
                let mut held = Credentials::new(peer.card().clone());
                peer.admit(PeerRecord::Requested(&mut held))
            },
        );
        let (_, refused) = both(alice_side, bob_side).await;
        assert_eq!(refused.err(), Some(LinkError::Budget));
        let second = kept.lock().unwrap()[1].clone();
        assert!(second.is_ended());
        assert!(!second.same_link(&first));
        assert!(first.same_link(&first.clone()));
    });
}

/// Bob's transport key in the tests below, the same for each of his cards.
const BOB_KEY: u8 = 0x52;

/// The party of Bob with his transport key, at `epoch`, reachable at
/// `endpoint`.
fn bob_at(epoch: u64, endpoint: OnionServiceKey) -> LocalParty {
    LocalParty::issue(
        &identity(2),
        TransportSecretKey::from_bytes(&[BOB_KEY; 32]).unwrap(),
        EndpointEpoch::new(epoch).unwrap(),
        EndpointSet::single(endpoint),
    )
    .unwrap()
}

/// An endpoint nobody publishes.
fn elsewhere() -> OnionServiceKey {
    OnionServiceKey::from_bytes(identity(0x66).public_key().as_bytes()).unwrap()
}

/// Alice dials Bob's card of epoch 1 and admits him with `alice_admits`,
/// which is given that card; Bob answers. Returns what the dial returned,
/// and whether message 3, with Alice's identity, reached Bob. Both sides
/// use `budgets`.
async fn dial_bob<F>(
    budgets: &Budgets,
    alice_admits: F,
) -> (
    Result<monolith_core::link::Established<tokio::io::DuplexStream>, LinkError>,
    bool,
)
where
    F: FnOnce(&ContactCard, OutboundPeer, &Withdrawal) -> Result<OutboundAdmission, SessionError>,
{
    dial_bob_at(1, budgets, alice_admits).await
}

/// As [`dial_bob`], with Bob's card of `epoch`.
async fn dial_bob_at<F>(
    epoch: u64,
    budgets: &Budgets,
    alice_admits: F,
) -> (
    Result<monolith_core::link::Established<tokio::io::DuplexStream>, LinkError>,
    bool,
)
where
    F: FnOnce(&ContactCard, OutboundPeer, &Withdrawal) -> Result<OutboundAdmission, SessionError>,
{
    let network = MockNetwork::new();
    let (alice_tor, bob_tor) = (network.backend(), network.backend());
    let mut bob_service = bob_tor.publish_onion(KeySource::Generate).await.unwrap();
    let alice_service = alice_tor.publish_onion(KeySource::Generate).await.unwrap();
    let alice = party(1, 1, *alice_service.service_key());
    let bob = bob_at(epoch, *bob_service.service_key());
    let dialed = bob.card().clone();
    let isolation = alice_tor.isolation_group().unwrap();
    let reached = core::cell::Cell::new(false);
    let bob_side = async {
        let stream = bob_service.accept().await.unwrap();
        answer(stream, budgets, &bob, |peer, _| {
            reached.set(true);
            peer.admit(PeerRecord::Accepted(&mut Credentials::new(
                alice.card().clone(),
            )))
        })
        .await
    };
    let (dialed_result, _) = both(
        dial(
            &alice_tor,
            budgets,
            &alice,
            &dialed,
            &isolation,
            |peer, withdrawal| alice_admits(&dialed, peer, withdrawal),
        ),
        bob_side,
    )
    .await;
    (dialed_result, reached.get())
}

/// The error of a dial refused with `standing` and `change`.
/// The admission inside the result of an outbound admission.
fn admission(admitted: &OutboundAdmission) -> Admission {
    match admitted {
        OutboundAdmission::Granted { admission, .. } | OutboundAdmission::Refused(admission) => {
            *admission
        }
    }
}

fn admission_of(admitted: &Result<OutboundAdmission, SessionError>) -> Admission {
    admission(admitted.as_ref().unwrap())
}

fn refused(standing: Standing, change: Option<CredentialChange>) -> Option<LinkError> {
    Some(LinkError::Refused(Admission { standing, change }))
}

fn identity_mismatch() -> Option<LinkError> {
    Some(LinkError::Session(SessionError::Protocol(
        ProtocolError::IdentityMismatch,
    )))
}

#[test]
fn a_key_retired_right_after_the_admission_does_not_receive_message_3() {
    // Alice admits Bob, then, before message 3 is written, Bob's key is
    // retired and the session withdrawn. Message 3 is not sent.
    run(async {
        let (result, reached) = dial_bob(&Budgets::new(), |dialed, peer, withdrawal| {
            let admitted = peer.admit(PeerRecord::Accepted(&mut Credentials::new(dialed.clone())));
            assert!(admission_of(&admitted).may_learn_local_identity());
            withdrawal.withdraw();
            admitted
        })
        .await;
        assert_eq!(result.err(), Some(LinkError::Withdrawn));
        assert!(!reached);
    });
}

#[test]
fn an_older_card_of_the_active_key_still_receives_message_3() {
    // Alice holds Bob's card of epoch 2, with the same transport key and
    // another endpoint she has not confirmed for dialing, and dials his
    // card of epoch 1. Bob proves the active key: he is the contact, and
    // the older card does not keep him from learning who dials.
    run(async {
        let (result, reached) = dial_bob(&Budgets::new(), |dialed, peer, _| {
            let mut held = Credentials::new(bob_at(2, elsewhere()).card().clone());
            assert_eq!(held.active().transport(), dialed.transport());
            let admitted = peer.admit(PeerRecord::Accepted(&mut held));
            // The card of epoch 2 stays the active one: no rollback.
            assert_eq!(held.active(), bob_at(2, elsewhere()).card());
            admitted
        })
        .await;
        let established = result.unwrap();
        assert_eq!(established.admission.standing, Standing::Accepted);
        assert_eq!(
            established.admission.change,
            Some(CredentialChange::Superseded)
        );
        assert!(reached);
    });
}

#[test]
fn a_card_that_conflicts_at_the_same_epoch_does_not_receive_message_3() {
    // Alice holds a card of Bob of epoch 1 with another transport key. The
    // responder proves the key of a second statement for that epoch.
    run(async {
        let (result, reached) = dial_bob(&Budgets::new(), |_, peer, _| {
            let other = LocalParty::issue(
                &identity(2),
                TransportSecretKey::from_bytes(&[0x53; 32]).unwrap(),
                EndpointEpoch::FIRST,
                EndpointSet::single(elsewhere()),
            )
            .unwrap();
            peer.admit(PeerRecord::Accepted(&mut Credentials::new(
                other.card().clone(),
            )))
        })
        .await;
        assert_eq!(
            result.err(),
            refused(Standing::StaleCard, Some(CredentialChange::Conflict))
        );
        assert!(!reached);
    });
}

#[test]
fn a_contact_deleted_declined_or_blocked_during_the_dial_gets_no_message_3() {
    // The record of Bob changed while Alice was dialing. No message 3, no
    // session, and no slot for strangers is taken.
    for record in [0_u8, 1, 2] {
        run(async {
            let budgets = Budgets::new();
            let (result, reached) = dial_bob(&budgets, |_, peer, _| {
                peer.admit(match record {
                    0 => PeerRecord::None,
                    1 => PeerRecord::Declined,
                    _ => PeerRecord::Blocked,
                })
            })
            .await;
            let standing = match record {
                0 => Standing::None,
                1 => Standing::Declined,
                _ => Standing::Blocked,
            };
            assert_eq!(result.err(), refused(standing, None), "{record}");
            assert!(!reached, "{record}");
            assert_eq!(
                budgets.free_unknown_sessions(),
                monolith_protocol::limits::MAX_UNKNOWN_SESSIONS
            );
        });
    }
}

#[test]
fn a_dial_that_loses_its_contact_takes_no_slot_when_none_is_left() {
    // All slots for strangers are taken. A dial whose contact is deleted
    // meanwhile makes no session and takes no slot beyond the limit.
    run(async {
        let budgets = Budgets::new();
        let held: Vec<_> = core::iter::from_fn(|| budgets.unknown_session()).collect();
        assert_eq!(held.len(), monolith_protocol::limits::MAX_UNKNOWN_SESSIONS);
        let (result, reached) = dial_bob(&budgets, |_, peer, _| peer.admit(PeerRecord::None)).await;
        assert_eq!(result.err(), refused(Standing::None, None));
        assert!(!reached);
        assert_eq!(budgets.free_unknown_sessions(), 0);
        drop(held);
        assert_eq!(
            budgets.free_unknown_sessions(),
            monolith_protocol::limits::MAX_UNKNOWN_SESSIONS
        );
    });
}

#[test]
fn a_pending_key_does_not_receive_message_3() {
    // Alice holds Bob with another transport key of epoch 1 and nothing
    // announced. The responder proves Bob's key of epoch 2, which did not
    // come through the active key: it is pending. No message 3 and no
    // session. The card is held as the pending successor, a candidate for
    // the user only, as for an inbound handshake.
    run(async {
        let active = party_of_key(2, 0x53, 1, elsewhere()).card().clone();
        let held = Mutex::new(Credentials::new(active.clone()));
        let (result, reached) = dial_bob_at(2, &Budgets::new(), |_, peer, _| {
            let admitted = peer.admit(PeerRecord::Accepted(&mut held.lock().unwrap()));
            assert_eq!(admission_of(&admitted).standing, Standing::PendingSuccessor);
            admitted
        })
        .await;
        assert_eq!(
            result.err(),
            refused(Standing::PendingSuccessor, Some(CredentialChange::Pending))
        );
        assert!(!reached);
        let held = held.lock().unwrap();
        assert_eq!(held.active(), &active);
        assert_eq!(
            held.pending_successor().map(|card| card.epoch()),
            Some(EndpointEpoch::new(2).unwrap())
        );
    });
}

#[test]
fn a_key_older_than_the_announced_successor_gets_nothing() {
    // Alice holds Bob's key 0x53 active and had his key of epoch 2
    // announced, then a newer key of epoch 3. She dials the card of epoch
    // 2. Its key is one the identity has superseded: no message 3, no
    // session, and nothing is recorded, not even a pending card.
    run(async {
        let active = party_of_key(2, 0x53, 1, elsewhere());
        let mut credentials = Credentials::new(active.card().clone());
        credentials
            .announce(bob_at(2, elsewhere()).card(), active.card())
            .unwrap();
        credentials
            .announce(party_of_key(2, 0x54, 3, elsewhere()).card(), active.card())
            .unwrap();
        let before = credentials.clone();
        let held = Mutex::new(credentials);
        let (result, reached) = dial_bob_at(2, &Budgets::new(), |_, peer, _| {
            let admitted = peer.admit(PeerRecord::Accepted(&mut held.lock().unwrap()));
            assert_eq!(
                admission_of(&admitted).change,
                Some(CredentialChange::Stale)
            );
            admitted
        })
        .await;
        assert_eq!(
            result.err(),
            refused(Standing::StaleCard, Some(CredentialChange::Stale))
        );
        assert!(!reached);
        assert_eq!(*held.lock().unwrap(), before);
    });
}

#[test]
fn a_failed_admission_ends_its_withdrawal() {
    // The admission function keeps the withdrawal and then fails. The
    // contact state learns that the link is gone.
    run(async {
        let kept = Mutex::new(None);
        let (result, reached) = dial_bob(&Budgets::new(), |_, _, withdrawal| {
            *kept.lock().unwrap() = Some(withdrawal.clone());
            Err(SessionError::Protocol(ProtocolError::IdentityMismatch))
        })
        .await;
        assert_eq!(result.err(), identity_mismatch());
        assert!(!reached);
        assert!(kept.lock().unwrap().as_ref().unwrap().is_ended());
    });
}

#[test]
fn a_promotion_stands_when_message_3_cannot_be_written() {
    // Alice holds Bob's key T1 active and his announced successor T2, and
    // dials T2. The responder proves T2 in message 2, which promotes it at
    // Alice; then the stream breaks before message 3 is written. The
    // promotion stands: it rests on the proof in message 2 alone, the
    // peer that gave it holds T2 and answers with it, and nobody gained a
    // standing it should not have. The dial fails.
    run(async {
        let network = MockNetwork::new();
        let budgets = Budgets::new();
        let (alice_tor, bob_tor) = (network.backend(), network.backend());
        let mut bob_service = bob_tor.publish_onion(KeySource::Generate).await.unwrap();
        let alice_service = alice_tor.publish_onion(KeySource::Generate).await.unwrap();
        let alice = party(1, 1, *alice_service.service_key());
        let bob_t1 = bob_at(1, *bob_service.service_key());
        let bob_t2 = party_of_key(2, 0x54, 2, *bob_service.service_key());
        let mut held = Credentials::new(bob_t1.card().clone());
        assert_eq!(
            held.announce(bob_t2.card(), bob_t1.card()),
            Ok(CredentialChange::Authorized)
        );
        let held = Mutex::new(held);
        let isolation = alice_tor.isolation_group().unwrap();

        // Bob answers message 1 with T2 and drops the stream at once.
        let bob_side = async {
            let mut stream = bob_service.accept().await.unwrap();
            let mut message_1 = [0_u8; 48];
            tokio::io::AsyncReadExt::read_exact(&mut stream, &mut message_1)
                .await
                .unwrap();
            let responder =
                monolith_session::HandshakeResponder::new(&bob_t2, std::time::Instant::now())
                    .unwrap();
            let (_, message_2) = responder
                .read_message_1(&message_1, std::time::Instant::now())
                .unwrap();
            tokio::io::AsyncWriteExt::write_all(&mut stream, &message_2)
                .await
                .unwrap();
            drop(stream);
        };
        let (result, ()) = both(
            dial(
                &alice_tor,
                &budgets,
                &alice,
                bob_t2.card(),
                &isolation,
                |peer, _| {
                    let mut held = held.lock().unwrap();
                    let admitted = peer.admit(PeerRecord::Accepted(&mut held));
                    assert_eq!(
                        admission_of(&admitted).change,
                        Some(CredentialChange::Promoted)
                    );
                    admitted
                },
            ),
            bob_side,
        )
        .await;
        assert!(result.is_err());
        let held = held.lock().unwrap();
        assert_eq!(held.active(), bob_t2.card());
        assert_eq!(held.retired(), Some(bob_t1.card().transport()));
    });
}

/// The party of identity `seed` with the transport key made from `key`.
fn party_of_key(seed: u8, key: u8, epoch: u64, endpoint: OnionServiceKey) -> LocalParty {
    LocalParty::issue(
        &identity(seed),
        TransportSecretKey::from_bytes(&[key; 32]).unwrap(),
        EndpointEpoch::new(epoch).unwrap(),
        EndpointSet::single(endpoint),
    )
    .unwrap()
}

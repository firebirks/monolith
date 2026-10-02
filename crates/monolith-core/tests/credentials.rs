//! Admission against the contact state of the moment, on the in-memory
//! Tor.
//!
//! The contact state is the smallest thing that does what the contact
//! store of Phase 4 has to do: credentials behind one lock, read and
//! changed in one step with each admission.

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
use monolith_core::link::{answer, dial};
use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::body::{Message, MessageId};
use monolith_protocol::card::EndpointSet;
use monolith_protocol::credential::{CredentialChange, Credentials};
use monolith_protocol::session::{Action, PeerRecord, Standing};
use monolith_protocol::text::ChatText;
use monolith_session::{LocalParty, TransportSecretKey};
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
        let alice_holds_bob = Mutex::new(Credentials::new(bob_card.clone()));
        let isolation = alice_tor.isolation_group().unwrap();

        let bob_side = async {
            let stream = bob_service.accept().await.unwrap();
            {
                let mut held = alice_holds_bob.lock().unwrap();
                assert_eq!(
                    held.import(successor.clone()),
                    Ok(CredentialChange::Pending)
                );
                assert_eq!(held.confirm(&successor), Ok(CredentialChange::Promoted));
            }
            answer(stream, &budgets, &bob, |peer| {
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
            |peer| {
                let mut held = alice_holds_bob.lock().unwrap();
                peer.admit(PeerRecord::Accepted(&mut held))
            },
        );
        let (alice_end, mut bob_end) = both(alice_side, bob_side).await;
        let mut alice_end = alice_end.unwrap();
        assert_eq!(alice_end.admission.standing, Standing::StaleCard);
        assert_eq!(alice_end.admission.change, Some(CredentialChange::Stale));
        assert!(alice_end.first.is_empty());
        assert_eq!(alice_holds_bob.lock().unwrap().active(), &successor);

        // The holder of T1 offers its ContactAccept and gets a Close; Alice
        // delivers nothing.
        bob_end.link.send(&Message::ContactAccept).await.unwrap();
        let received = alice_end.link.receive().await.unwrap();
        assert_eq!(received.actions, vec![Action::SendClose]);
        assert!(alice_end.link.send(&chat("x")).await.is_err());
    });
}

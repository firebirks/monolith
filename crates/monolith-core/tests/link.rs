//! Tor streams carrying the Phase 2 session, on the in-memory Tor, with
//! the contact store of each local identity deciding every admission.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::{
    both, chat, confirm_both, connect, dial_and_answer, node, request, run, run_paused, send_first,
};
use monolith_core::budget::{Budgets, ServeEnd, serve};
use monolith_core::identity::Installation;
use monolith_core::link::{LinkError, answer, dial};
use monolith_identity::{EndpointEpoch, IdentitySecretKey};
use monolith_protocol::SessionState;
use monolith_protocol::body::Message;
use monolith_protocol::card::{ContactCard, EndpointSet};
use monolith_protocol::limits::{
    MAX_CONCURRENT_DIALS, MAX_INBOUND_HANDSHAKES, MAX_UNKNOWN_SESSIONS,
};
use monolith_protocol::session::{Action, Standing};
use monolith_tor::{MockNetwork, OnionService, TorBackend, TorError};
use tokio::io::AsyncReadExt;
use tokio::sync::watch;

#[test]
fn a_tor_stream_carries_an_authenticated_exchange_both_ways() {
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        alice.identity.import(&bob.card()).await.unwrap();
        bob.identity.import(&alice.card()).await.unwrap();

        // Both asked: the first session makes both accepted and confirms.
        let (mut alice_link, mut bob_link) = confirm_both(&alice, &mut bob).await;
        assert_eq!(bob_link.session().peer(), alice.identity.identity());
        assert_eq!(
            alice
                .identity
                .contact(bob.identity.identity())
                .unwrap()
                .kind,
            monolith_protocol::contact::RecordKind::Accepted
        );

        alice_link.send(&chat("hello over Tor")).await.unwrap();
        let received = bob_link.receive().await.unwrap();
        assert_eq!(received.message, chat("hello over Tor"));
        assert_eq!(received.actions, vec![Action::Deliver]);
        bob_link.send(&chat("hello back")).await.unwrap();
        assert_eq!(
            alice_link.receive().await.unwrap().message,
            chat("hello back")
        );

        alice_link.close().await.unwrap();
        let closed = bob_link.receive().await.unwrap();
        assert_eq!(closed.message, Message::Close);
        assert_eq!(bob_link.session().state(), SessionState::Closed);

        // The next session starts accepted on both sides.
        let (alice_end, bob_end) = connect(&alice, &mut bob).await;
        assert_eq!(alice_end.unwrap().first, vec![Action::SendContactAccept]);
        assert_eq!(bob_end.unwrap().first, vec![Action::SendContactAccept]);
    });
}

#[test]
fn reaching_the_onion_service_does_not_authenticate_another_identity() {
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        // A card of another identity that names Bob's endpoint: Tor reaches
        // Bob, the handshake does not accept the identity.
        let mallory_at_bob = ContactCard::sign(
            &IdentitySecretKey::from_seed(&[3; 32]),
            *bob.card().transport(),
            EndpointEpoch::FIRST,
            EndpointSet::single(bob.identity.endpoint()),
            None,
        )
        .unwrap();
        alice.identity.import(&mallory_at_bob).await.unwrap();
        let (alice_result, bob_result) = dial_and_answer(&alice, &mallory_at_bob, &mut bob).await;
        assert!(alice_result.is_err());
        assert!(matches!(
            bob_result,
            Err(LinkError::Session(_) | LinkError::Stream)
        ));
    });
}

#[test]
fn a_stranger_through_a_valid_tor_stream_gets_only_a_close() {
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        alice.identity.import(&bob.card()).await.unwrap();
        let (alice_end, bob_end) = connect(&alice, &mut bob).await;
        let (mut alice_end, mut bob_end) = (alice_end.unwrap(), bob_end.unwrap());
        assert_eq!(bob_end.admission.standing, Standing::None);
        assert!(bob_end.first.is_empty());
        assert!(bob_end.link.holds_unknown_slot());
        assert_eq!(alice_end.first, vec![Action::SendContactRequest]);
        alice_end
            .link
            .send(&request(&alice.card(), None))
            .await
            .unwrap();
        let received = bob_end.link.receive().await.unwrap();
        assert_eq!(
            received.actions,
            vec![Action::SendClose, Action::ConsiderRequest]
        );
        // The Close went out and the slot is back before the message is
        // returned.
        assert!(!bob_end.link.holds_unknown_slot());
        // A chat message cannot even be sent before confirmation.
        assert!(alice_end.link.send(&chat("x")).await.is_err());
    });
}

#[test]
fn without_socks_or_a_service_a_dial_fails_and_tries_nothing_else() {
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let bob = node(&network, 2).await;
        alice.identity.import(&bob.card()).await.unwrap();

        network.set_socks_available(false);
        let result = dial(&alice.tor, &alice.budgets, &alice.identity, &bob.card()).await;
        assert_eq!(
            result.err(),
            Some(LinkError::Tor(TorError::SocksUnavailable))
        );
        assert_eq!(network.dials(), 0);

        network.set_socks_available(true);
        network.lose_service(&bob.identity.endpoint());
        let result = dial(&alice.tor, &alice.budgets, &alice.identity, &bob.card()).await;
        assert_eq!(
            result.err(),
            Some(LinkError::Tor(TorError::OnionUnreachable))
        );
        assert_eq!(network.dials(), 1);
    });
}

#[test]
fn the_accept_loop_bounds_handshakes_owns_its_tasks_and_ends_with_the_service() {
    run(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        let key = bob.identity.endpoint();
        let budgets = Budgets::new();
        let (stop, shutdown) = watch::channel(false);
        let running = Arc::new(AtomicUsize::new(0));
        let counter = running.clone();
        let identity = bob.identity.clone();

        // Handlers that hold their permit until they are aborted.
        let serving = async {
            serve(
                &mut bob.service,
                &budgets,
                &identity,
                shutdown,
                move |stream, permit| {
                    let counter = counter.clone();
                    async move {
                        let _stream = stream;
                        struct Guard(Arc<AtomicUsize>);
                        impl Drop for Guard {
                            fn drop(&mut self) {
                                self.0.fetch_sub(1, Ordering::SeqCst);
                            }
                        }
                        counter.fetch_add(1, Ordering::SeqCst);
                        let _guard = Guard(counter);
                        let _permit = permit;
                        core::future::pending::<()>().await;
                    }
                },
            )
            .await
        };
        let flooding = async {
            let isolation = bob.tor.isolation_group().unwrap();
            let mut streams = Vec::new();
            for _ in 0..MAX_INBOUND_HANDSHAKES + 4 {
                streams.push(bob.tor.connect_onion(&key, &isolation).await.unwrap());
                // Let the loop take it, as a real listener would.
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            // Every refusal costs the loop ACCEPT_BACKOFF; wait for all.
            tokio::time::sleep(Duration::from_millis(1500)).await;
            let mut refused = 0;
            let mut held = Vec::new();
            for mut stream in streams {
                let mut byte = [0_u8; 1];
                match tokio::time::timeout(Duration::from_millis(50), stream.read(&mut byte)).await
                {
                    // Closed without a handler: no permit was free.
                    Ok(Ok(0)) => refused += 1,
                    _ => held.push(stream),
                }
            }
            assert_eq!(refused, 4);
            assert_eq!(held.len(), MAX_INBOUND_HANDSHAKES);
            assert_eq!(running.load(Ordering::SeqCst), MAX_INBOUND_HANDSHAKES);
            stop.send(true).unwrap();
            held
        };
        let (end, _held) = both(serving, flooding).await;
        assert_eq!(end, ServeEnd::Shutdown);
        // Every handler was aborted and gave its permits back.
        assert_eq!(running.load(Ordering::SeqCst), 0);
        assert_eq!(
            budgets.free_inbound_handshakes(),
            monolith_protocol::limits::MAX_PROCESS_INBOUND_HANDSHAKES
        );
        drop(bob);
        assert!(!network.is_published(&key));
    });
}

#[test]
fn losing_tor_ends_the_accept_loop_and_the_publication() {
    run(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        let key = bob.identity.endpoint();
        let identity = bob.identity.clone();
        let (_stop, shutdown) = watch::channel(false);
        let losing = async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            network.lose_service(&key);
        };
        let serving = serve(
            &mut bob.service,
            &bob.budgets,
            &identity,
            shutdown,
            |_, _| async {},
        );
        let (end, ()) = both(serving, losing).await;
        assert_eq!(end, ServeEnd::Service(TorError::ControlLost));
        assert!(!bob.service.is_published());
        assert_eq!(
            bob.service.accept().await.err(),
            Some(TorError::ControlLost)
        );
    });
}

#[test]
fn a_dropped_shutdown_sender_stops_the_loop() {
    run(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        let identity = bob.identity.clone();
        let (stop, shutdown) = watch::channel(false);
        drop(stop);
        let end = tokio::time::timeout(
            Duration::from_secs(5),
            serve(
                &mut bob.service,
                &bob.budgets,
                &identity,
                shutdown,
                |_, _| async {},
            ),
        )
        .await
        .unwrap();
        assert_eq!(end, ServeEnd::Shutdown);
    });
}

#[test]
fn several_peers_handshake_at_the_same_time() {
    run(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        let bob_card = bob.card();
        let identity = bob.identity.clone();
        let budgets = bob.budgets.clone();
        let (stop, shutdown) = watch::channel(false);
        let authenticated = Arc::new(AtomicUsize::new(0));
        let count = authenticated.clone();
        let responder = identity.clone();
        let serving = serve(
            &mut bob.service,
            &budgets,
            &identity,
            shutdown,
            move |stream, permit| {
                let responder = responder.clone();
                let count = count.clone();
                let budgets = Budgets::new();
                async move {
                    if answer(stream, &budgets, &responder).await.is_ok() {
                        count.fetch_add(1, Ordering::SeqCst);
                    }
                    drop(permit);
                }
            },
        );
        let dialing = async {
            let mut nodes = Vec::new();
            for seed in 10..14_u8 {
                let peer = node(&network, seed).await;
                peer.identity.import(&bob_card).await.unwrap();
                nodes.push(peer);
            }
            let mut ok = 0;
            let dials: Vec<_> = nodes
                .iter()
                .map(|peer| dial(&peer.tor, &peer.budgets, &peer.identity, &bob_card))
                .collect();
            for dial in dials {
                if dial.await.is_ok() {
                    ok += 1;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            stop.send(true).unwrap();
            ok
        };
        let (end, ok) = both(serving, dialing).await;
        assert_eq!(end, ServeEnd::Shutdown);
        assert_eq!(ok, 4);
        assert_eq!(authenticated.load(Ordering::SeqCst), 4);
    });
}

#[test]
fn local_identities_are_reached_through_their_own_services() {
    // One process and one Tor backend serve two local identities of one
    // installation. Each answers the streams of its own service with its
    // own party and its own contact store; a stream is never answered by
    // the other. Each dials with isolation groups of its own, also towards
    // the same remote contact (T-MI-5, T-MI-6, T-MI-8).
    run(async {
        let network = MockNetwork::new();
        let installation = Installation::ephemeral();
        let mut a = common::node_in(installation.clone(), &network, 1, 1, 1).await;
        let mut b = common::node_in(installation.clone(), &network, 2, 2, 1).await;
        assert_eq!(installation.identities().len(), 2);
        let mut carol = node(&network, 3).await;
        carol.identity.import(&a.card()).await.unwrap();
        carol.identity.import(&b.card()).await.unwrap();
        a.identity.import(&carol.card()).await.unwrap();
        b.identity.import(&carol.card()).await.unwrap();

        // Carol reaches each local identity at its own service, and each
        // decides with its own record of her.
        for local in [&mut a, &mut b] {
            let (dialed, answered) = connect(&carol, local).await;
            assert_eq!(
                dialed.unwrap().link.session().peer(),
                local.identity.identity()
            );
            let answered = answered.unwrap();
            assert_eq!(answered.link.session().peer(), carol.identity.identity());
            assert_eq!(answered.admission.standing, Standing::Requested);
        }

        // A card of A that names the service of B reaches B, and B does
        // not answer as A.
        let a_at_b = ContactCard::sign(
            &IdentitySecretKey::from_seed(&[1; 32]),
            *a.card().transport(),
            EndpointEpoch::new(2).unwrap(),
            EndpointSet::single(b.identity.endpoint()),
            None,
        )
        .unwrap();
        let (dialed, answered) = dial_and_answer(&carol, &a_at_b, &mut b).await;
        assert!(dialed.is_err());
        assert!(answered.is_err());

        // A and B both dial Carol, each with a group of its own, kept per
        // pair of local identity and contact.
        for local in [&a, &b] {
            let (dialed, answered) = connect(local, &mut carol).await;
            assert_eq!(
                dialed.unwrap().link.session().peer(),
                carol.identity.identity()
            );
            assert_eq!(
                answered.unwrap().link.session().peer(),
                local.identity.identity()
            );
        }
        let group_a = a.identity.isolation(carol.identity.identity()).unwrap();
        let group_b = b.identity.isolation(carol.identity.identity()).unwrap();
        assert!(!group_a.same_as(&group_b));
        assert!(group_a.same_as(&a.identity.isolation(carol.identity.identity()).unwrap()));
        let group_a_other = a.identity.isolation(b.identity.identity()).unwrap();
        assert!(!group_a.same_as(&group_a_other));
    });
}

#[test]
fn a_silent_peer_is_dropped_after_the_handshake_timeout() {
    run_paused(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        let isolation = bob.tor.isolation_group().unwrap();
        // Connects and then sends nothing at all.
        let _silent = bob
            .tor
            .connect_onion(&bob.identity.endpoint(), &isolation)
            .await
            .unwrap();
        let stream = bob.service.accept().await.unwrap();
        let started = tokio::time::Instant::now();
        let result = answer(stream, &bob.budgets, &bob.identity).await;
        assert_eq!(result.err(), Some(LinkError::TimedOut));
        let timeout = monolith_protocol::limits::HANDSHAKE_TIMEOUT;
        assert!(started.elapsed() >= timeout);
        assert!(started.elapsed() <= timeout + Duration::from_secs(1));
    });
}

#[test]
fn a_full_stranger_budget_evicts_the_oldest_silent_stranger() {
    // WP7. Bob holds no record of anybody. Strangers dial him and stay
    // silent; when the budget is full, the oldest silent one is closed
    // with a Close and the newcomer gets its slot, in the order the slots
    // were taken. A stranger whose message has been taken is not silent;
    // that race is decided on one state (`strangers::tests`).
    run(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        let bob_card = bob.card();
        let mut strangers = Vec::new();
        for seed in 20..20 + u8::try_from(MAX_UNKNOWN_SESSIONS).unwrap() + 2 {
            let stranger = node(&network, seed).await;
            stranger.identity.import(&bob_card).await.unwrap();
            strangers.push(stranger);
        }
        let mut held = Vec::new();
        for stranger in strangers.iter().take(MAX_UNKNOWN_SESSIONS) {
            let (dialed, answered) = dial_and_answer(stranger, &bob_card, &mut bob).await;
            held.push((dialed.unwrap(), answered.unwrap()));
        }
        assert_eq!(bob.identity.strangers().free(), 0);
        // Two newcomers evict the two oldest, in order.
        let mut newcomers = Vec::new();
        for (index, stranger) in strangers.iter().skip(MAX_UNKNOWN_SESSIONS).enumerate() {
            let (_, answered) = dial_and_answer(stranger, &bob_card, &mut bob).await;
            let newcomer = answered.unwrap();
            assert!(newcomer.link.holds_unknown_slot());
            newcomers.push(newcomer);
            let (dialed, answered) = &mut held[index];
            assert_eq!(
                answered.link.receive().await.err(),
                Some(LinkError::Evicted)
            );
            // The evicted stranger sees an ordinary Close.
            assert_eq!(dialed.link.receive().await.unwrap().message, Message::Close);
            assert_eq!(bob.identity.strangers().held(), MAX_UNKNOWN_SESSIONS);
        }
        // The younger ones are untouched and still answered as usual.
        let (dialed, answered) = &mut held[2];
        dialed
            .link
            .send(&request(&strangers[2].card(), None))
            .await
            .unwrap();
        let received = answered.link.receive().await.unwrap();
        assert!(received.actions.contains(&Action::ConsiderRequest));
        assert_eq!(bob.identity.strangers().held(), MAX_UNKNOWN_SESSIONS - 1);
    });
}

#[test]
fn a_contact_is_never_evicted_for_a_stranger() {
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        common::befriend(&alice, &mut bob).await;
        let (alice_end, bob_end) = connect(&alice, &mut bob).await;
        let (mut alice_end, mut bob_end) = (alice_end.unwrap(), bob_end.unwrap());
        assert!(bob_end.link.holds_contact_slot());
        assert!(!bob_end.link.holds_unknown_slot());
        // Strangers fill and churn the budget.
        let bob_card = bob.card();
        let mut kept = Vec::new();
        for seed in 40..40 + u8::try_from(MAX_UNKNOWN_SESSIONS).unwrap() * 3 {
            let stranger = node(&network, seed).await;
            stranger.identity.import(&bob_card).await.unwrap();
            kept.push(dial_and_answer(&stranger, &bob_card, &mut bob).await);
            kept.push((Err(LinkError::Budget), Err(LinkError::Budget)));
            assert!(bob.identity.strangers().held() <= MAX_UNKNOWN_SESSIONS);
            assert!(bob.identity.strangers().draining() <= MAX_UNKNOWN_SESSIONS);
        }
        // The contact session still works.
        send_first(&mut alice_end, &alice.identity).await;
        send_first(&mut bob_end, &bob.identity).await;
        assert!(
            bob_end
                .link
                .receive()
                .await
                .unwrap()
                .actions
                .contains(&Action::Confirmed)
        );
        assert!(
            alice_end
                .link
                .receive()
                .await
                .unwrap()
                .actions
                .contains(&Action::Confirmed)
        );
        alice_end.link.send(&chat("still here")).await.unwrap();
        assert_eq!(
            bob_end.link.receive().await.unwrap().message,
            chat("still here")
        );
    });
}

#[test]
fn listener_errors_do_not_end_the_accept_loop() {
    run(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        let key = bob.identity.endpoint();
        network.fail_accepts(&key, 3);
        let identity = bob.identity.clone();
        let (stop, shutdown) = watch::channel(false);
        let accepted = Arc::new(AtomicUsize::new(0));
        let count = accepted.clone();
        let tor = network.backend();
        let serving = serve(
            &mut bob.service,
            &bob.budgets,
            &identity,
            shutdown,
            move |_, _| {
                count.fetch_add(1, Ordering::SeqCst);
                async {}
            },
        );
        let dialing = async {
            let isolation = tor.isolation_group().unwrap();
            // Three errors, each followed by ACCEPT_BACKOFF, then a stream.
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _stream = tor.connect_onion(&key, &isolation).await.unwrap();
            tokio::time::sleep(Duration::from_millis(1500)).await;
            stop.send(true).unwrap();
        };
        let (end, ()) = both(serving, dialing).await;
        assert_eq!(end, ServeEnd::Shutdown);
        assert_eq!(accepted.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn dials_beyond_the_dial_budget_wait_for_a_slot() {
    run_paused(async {
        let network = MockNetwork::new();
        // A service that never answers: each dial holds its slot until
        // the handshake deadline.
        let bob = node(&network, 2).await;
        let card = bob.card();
        let budgets = Budgets::new();
        let mut tasks = tokio::task::JoinSet::new();
        for seed in 30..30 + 5_u8 {
            let peer = node(&network, seed).await;
            peer.identity.import(&card).await.unwrap();
            let budgets = budgets.clone();
            let card = card.clone();
            // The whole node goes into the task: its installation, too,
            // which would close the identity if it were dropped.
            tasks.spawn(async move {
                let peer = peer;
                dial(&peer.tor, &budgets, &peer.identity, &card).await.err()
            });
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(network.dials(), MAX_CONCURRENT_DIALS);
        // When the first four give up at the handshake deadline, the
        // fifth gets its slot.
        tokio::time::sleep(monolith_protocol::limits::HANDSHAKE_TIMEOUT).await;
        assert_eq!(network.dials(), MAX_CONCURRENT_DIALS + 1);
        while let Some(result) = tasks.join_next().await {
            assert_eq!(result.unwrap(), Some(LinkError::TimedOut));
        }
        drop(bob);
    });
}

#[test]
fn a_dial_to_an_identity_without_a_record_sends_nothing_of_the_local_identity() {
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        // Alice holds no record of Bob: the dial is refused after message
        // 2, and Bob never gets message 3.
        let card = bob.card();
        let (dialed, answered) = dial_and_answer(&alice, &card, &mut bob).await;
        assert_eq!(
            dialed.err(),
            Some(LinkError::Refused(monolith_protocol::session::Admission {
                standing: Standing::None,
                change: None
            }))
        );
        assert_eq!(answered.err(), Some(LinkError::Stream));
    });
}

#[test]
fn a_contact_refused_for_the_contact_budget_leaves_no_session_behind() {
    // Bob's process has no room for another contact session. Alice's dial
    // is admitted as a contact's and then refused for the budget: the
    // stream is closed, and the store forgets the session it had tracked.
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        common::befriend(&alice, &mut bob).await;
        bob.budgets = Budgets::with_limits(MAX_INBOUND_HANDSHAKES, 0, MAX_CONCURRENT_DIALS);
        let (_, answered) = connect(&alice, &mut bob).await;
        assert_eq!(answered.err(), Some(LinkError::Budget));
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
fn a_dial_without_a_contact_slot_writes_no_message_3() {
    // Alice's process has no room for another contact session. Her dial
    // authenticates Bob and is admitted as a contact's, and is then refused
    // for the budget: message 3, which carries her identity, is never
    // written, so Bob sees the stream end where it was due, and neither
    // store keeps a session.
    run(async {
        let network = MockNetwork::new();
        let mut alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        common::befriend(&alice, &mut bob).await;
        alice.budgets = Budgets::with_limits(MAX_INBOUND_HANDSHAKES, 0, MAX_CONCURRENT_DIALS);
        let (dialed, answered) = connect(&alice, &mut bob).await;
        assert_eq!(dialed.err(), Some(LinkError::Budget));
        assert_eq!(answered.err(), Some(LinkError::Stream));
        let held = |node: &common::Node, peer: &common::Node| {
            node.identity
                .contact(peer.identity.identity())
                .unwrap()
                .sessions
        };
        assert_eq!(held(&alice, &bob), 0);
        assert_eq!(held(&bob, &alice), 0);
    });
}

#[test]
fn a_stranger_finds_no_slot_while_evicted_strangers_drain() {
    // Twice the budget of strangers dial Bob and stay silent: the second
    // half evicts the first, whose links have not ended, as nobody has
    // polled them. Evicted links that have not ended count against the
    // same bound, so the next stranger is closed without a reply. Once an
    // evicted link ends, a newcomer gets in again.
    run(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        let bob_card = bob.card();
        let budget = u8::try_from(MAX_UNKNOWN_SESSIONS).unwrap();
        let mut ends = Vec::new();
        for seed in 60..60 + 2 * budget {
            let stranger = node(&network, seed).await;
            stranger.identity.import(&bob_card).await.unwrap();
            let (dialed, answered) = dial_and_answer(&stranger, &bob_card, &mut bob).await;
            ends.push((dialed.unwrap(), answered.unwrap()));
        }
        assert_eq!(bob.identity.strangers().held(), MAX_UNKNOWN_SESSIONS);
        assert_eq!(bob.identity.strangers().draining(), MAX_UNKNOWN_SESSIONS);

        let late = node(&network, 90).await;
        late.identity.import(&bob_card).await.unwrap();
        let (dialed, answered) = dial_and_answer(&late, &bob_card, &mut bob).await;
        assert_eq!(answered.err(), Some(LinkError::Budget));
        assert!(dialed.unwrap().link.receive().await.is_err());
        assert_eq!(bob.identity.strangers().held(), MAX_UNKNOWN_SESSIONS);

        let (_, evicted) = &mut ends[0];
        assert_eq!(evicted.link.receive().await.err(), Some(LinkError::Evicted));
        drop(ends.remove(0));
        assert_eq!(
            bob.identity.strangers().draining(),
            MAX_UNKNOWN_SESSIONS - 1
        );
        let again = node(&network, 91).await;
        again.identity.import(&bob_card).await.unwrap();
        let (_, answered) = dial_and_answer(&again, &bob_card, &mut bob).await;
        assert!(answered.unwrap().link.holds_unknown_slot());
    });
}

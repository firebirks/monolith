//! Tor streams carrying the Phase 2 session, on the in-memory Tor.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use core::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use monolith_core::budget::{Budgets, ServeEnd, serve};
use monolith_core::link::{LinkError, answer, dial};
use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::SessionState;
use monolith_protocol::body::{Message, MessageId};
use monolith_protocol::card::{ContactCard, EndpointSet};
use monolith_protocol::limits::MAX_INBOUND_HANDSHAKES;
use monolith_protocol::session::{Action, PeerRecord, Standing};
use monolith_protocol::text::ChatText;
use monolith_session::{LocalParty, TransportSecretKey};
use monolith_tor::{KeySource, MockNetwork, OnionService, TorBackend, TorError};
use tokio::io::AsyncReadExt;
use tokio::sync::watch;

fn run<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// A party whose card names `endpoint`.
fn party(seed: u8, endpoint: OnionServiceKey) -> LocalParty {
    let identity = IdentitySecretKey::from_seed(&[seed; 32]);
    let transport = TransportSecretKey::generate().unwrap();
    LocalParty::issue(
        &identity,
        transport,
        EndpointEpoch::FIRST,
        EndpointSet::single(endpoint),
    )
    .unwrap()
}

/// Budgets of their own for one call, for tests that do not exercise them.
fn fresh() -> &'static Budgets {
    Box::leak(Box::new(Budgets::new()))
}

fn chat(text: &str) -> Message {
    Message::ChatMessage {
        id: MessageId::from_bytes([1; 16]),
        text: ChatText::new(text).unwrap(),
    }
}

#[test]
fn a_tor_stream_carries_an_authenticated_exchange_both_ways() {
    run(async {
        let network = MockNetwork::new();
        let (alice_tor, bob_tor) = (network.backend(), network.backend());
        let mut bob_service = bob_tor.publish_onion(KeySource::Generate).await.unwrap();
        let alice_service = alice_tor.publish_onion(KeySource::Generate).await.unwrap();
        let bob = party(2, *bob_service.service_key());
        let alice = party(1, *alice_service.service_key());
        let (alice_card, bob_card) = (alice.card().clone(), bob.card().clone());

        let isolation = alice_tor.isolation_group().unwrap();
        let bob_side = async {
            let stream = bob_service.accept().await.unwrap();
            answer(stream, fresh(), &bob, |_| PeerRecord::Accepted(&alice_card))
                .await
                .unwrap()
        };
        let alice_side = dial(
            &alice_tor,
            fresh(),
            &alice,
            &bob_card,
            PeerRecord::Accepted(&bob_card),
            &isolation,
        );
        let (alice_end, bob_end) = futures_join(alice_side, bob_side).await;
        let (mut alice_link, mut bob_link) = (alice_end.unwrap(), bob_end);
        assert_eq!(alice_link.first, vec![Action::SendContactAccept]);
        assert_eq!(bob_link.first, vec![Action::SendContactAccept]);
        assert_eq!(bob_link.link.session().peer(), alice_card.identity());

        alice_link.link.send(&Message::ContactAccept).await.unwrap();
        bob_link.link.send(&Message::ContactAccept).await.unwrap();
        assert_eq!(
            bob_link.link.receive().await.unwrap().actions,
            vec![Action::Confirmed]
        );
        assert_eq!(
            alice_link.link.receive().await.unwrap().actions,
            vec![Action::Confirmed]
        );

        alice_link.link.send(&chat("hello over Tor")).await.unwrap();
        let received = bob_link.link.receive().await.unwrap();
        assert_eq!(received.message, chat("hello over Tor"));
        assert_eq!(received.actions, vec![Action::Deliver]);
        bob_link.link.send(&chat("hello back")).await.unwrap();
        assert_eq!(
            alice_link.link.receive().await.unwrap().message,
            chat("hello back")
        );

        alice_link.link.close().await.unwrap();
        let closed = bob_link.link.receive().await.unwrap();
        assert_eq!(closed.message, Message::Close);
        assert_eq!(bob_link.link.session().state(), SessionState::Closed);
    });
}

/// Runs two futures to completion on the current task.
async fn futures_join<A: Future, B: Future>(a: A, b: B) -> (A::Output, B::Output) {
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

#[test]
fn reaching_the_onion_service_does_not_authenticate_another_identity() {
    run(async {
        let network = MockNetwork::new();
        let backend = network.backend();
        let mut bob_service = backend.publish_onion(KeySource::Generate).await.unwrap();
        let bob = party(2, *bob_service.service_key());
        // A card of another identity that names Bob's endpoint: Tor reaches
        // Bob, the handshake does not accept the identity.
        let mallory = party(3, *bob_service.service_key());
        let alice = party(1, *bob_service.service_key());
        let isolation = backend.isolation_group().unwrap();
        let bob_side = async {
            let stream = bob_service.accept().await.unwrap();
            answer(stream, fresh(), &bob, |_| PeerRecord::None).await
        };
        let alice_side = dial(
            &backend,
            fresh(),
            &alice,
            mallory.card(),
            PeerRecord::None,
            &isolation,
        );
        let (alice_result, bob_result) = futures_join(alice_side, bob_side).await;
        assert!(alice_result.is_err());
        assert!(matches!(bob_result, Err(LinkError::Session(_))));
    });
}

#[test]
fn a_stranger_through_a_valid_tor_stream_gets_only_a_close() {
    run(async {
        let network = MockNetwork::new();
        let backend = network.backend();
        let mut bob_service = backend.publish_onion(KeySource::Generate).await.unwrap();
        let alice_service = backend.publish_onion(KeySource::Generate).await.unwrap();
        let bob = party(2, *bob_service.service_key());
        let alice = party(1, *alice_service.service_key());
        let bob_card = bob.card().clone();
        let isolation = backend.isolation_group().unwrap();
        let bob_side = async {
            let stream = bob_service.accept().await.unwrap();
            answer(stream, fresh(), &bob, |_| PeerRecord::None)
                .await
                .unwrap()
        };
        let alice_side = dial(
            &backend,
            fresh(),
            &alice,
            &bob_card,
            PeerRecord::Requested(&bob_card),
            &isolation,
        );
        let (alice_end, mut bob_end) = futures_join(alice_side, bob_side).await;
        let mut alice_end = alice_end.unwrap();
        assert_eq!(bob_end.admission.standing, Standing::None);
        assert!(bob_end.first.is_empty());
        assert_eq!(alice_end.first, vec![Action::SendContactRequest]);
        let request = Message::ContactRequest(Box::new(monolith_protocol::body::ContactRequest {
            card: alice.card().clone(),
            invitation: None,
            display_name: monolith_protocol::text::DisplayName::new("Alice").unwrap(),
            introduction: monolith_protocol::text::IntroductionText::new("hi").unwrap(),
        }));
        alice_end.link.send(&request).await.unwrap();
        let received = bob_end.link.receive().await.unwrap();
        assert_eq!(
            received.actions,
            vec![Action::SendClose, Action::ConsiderRequest]
        );
        // A chat message cannot even be sent before confirmation.
        assert!(alice_end.link.send(&chat("x")).await.is_err());
    });
}

#[test]
fn without_socks_or_a_service_a_dial_fails_and_tries_nothing_else() {
    run(async {
        let network = MockNetwork::new();
        let backend = network.backend();
        let service = backend.publish_onion(KeySource::Generate).await.unwrap();
        let bob = party(2, *service.service_key());
        let alice = party(1, *service.service_key());
        let isolation = backend.isolation_group().unwrap();

        network.set_socks_available(false);
        let result = dial(
            &backend,
            fresh(),
            &alice,
            bob.card(),
            PeerRecord::None,
            &isolation,
        )
        .await;
        assert_eq!(
            result.err(),
            Some(LinkError::Tor(TorError::SocksUnavailable))
        );
        assert_eq!(network.dials(), 0);

        network.set_socks_available(true);
        network.lose_service(service.service_key());
        let result = dial(
            &backend,
            fresh(),
            &alice,
            bob.card(),
            PeerRecord::None,
            &isolation,
        )
        .await;
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
        let backend = network.backend();
        let mut service = backend.publish_onion(KeySource::Generate).await.unwrap();
        let key = *service.service_key();
        let budgets = Budgets::new();
        let (stop, shutdown) = watch::channel(false);
        let running = Arc::new(AtomicUsize::new(0));
        let counter = running.clone();

        // Handlers that hold their permit until they are aborted.
        let serving = async {
            serve(&mut service, &budgets, shutdown, move |stream, permit| {
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
            })
            .await
        };
        let flooding = async {
            let isolation = backend.isolation_group().unwrap();
            let mut streams = Vec::new();
            for _ in 0..MAX_INBOUND_HANDSHAKES + 4 {
                streams.push(backend.connect_onion(&key, &isolation).await.unwrap());
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
            assert_eq!(budgets.free_inbound_handshakes(), 0);
            stop.send(true).unwrap();
            held
        };
        let (end, _held) = futures_join(serving, flooding).await;
        assert_eq!(end, ServeEnd::Shutdown);
        // Every handler was aborted and gave its permit back.
        assert_eq!(running.load(Ordering::SeqCst), 0);
        assert_eq!(budgets.free_inbound_handshakes(), MAX_INBOUND_HANDSHAKES);
        drop(service);
        assert!(!network.is_published(&key));
    });
}

#[test]
fn losing_tor_ends_the_accept_loop_and_the_publication() {
    run(async {
        let network = MockNetwork::new();
        let backend = network.backend();
        let mut service = backend.publish_onion(KeySource::Generate).await.unwrap();
        let key = *service.service_key();
        let budgets = Budgets::new();
        let (_stop, shutdown) = watch::channel(false);
        let losing = async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            network.lose_service(&key);
        };
        let serving = serve(&mut service, &budgets, shutdown, |_stream, _permit| async {
        });
        let (end, ()) = futures_join(serving, losing).await;
        assert_eq!(end, ServeEnd::Service(TorError::ControlLost));
        assert!(!service.is_published());
        assert_eq!(service.accept().await.err(), Some(TorError::ControlLost));
    });
}

#[test]
fn a_dropped_shutdown_sender_stops_the_loop() {
    run(async {
        let network = MockNetwork::new();
        let backend = network.backend();
        let mut service = backend.publish_onion(KeySource::Generate).await.unwrap();
        let budgets = Budgets::new();
        let (stop, shutdown) = watch::channel(false);
        drop(stop);
        let end = tokio::time::timeout(
            Duration::from_secs(5),
            serve(&mut service, &budgets, shutdown, |_stream, _permit| async {
            }),
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
        let backend = network.backend();
        let mut service = backend.publish_onion(KeySource::Generate).await.unwrap();
        let bob = Arc::new(party(2, *service.service_key()));
        let bob_card = bob.card().clone();
        let budgets = Budgets::new();
        let (stop, shutdown) = watch::channel(false);
        let authenticated = Arc::new(AtomicUsize::new(0));
        let count = authenticated.clone();
        let responder = bob.clone();
        let serving = serve(&mut service, &budgets, shutdown, move |stream, permit| {
            let responder = responder.clone();
            let count = count.clone();
            async move {
                if answer(stream, fresh(), &responder, |_| PeerRecord::None)
                    .await
                    .is_ok()
                {
                    count.fetch_add(1, Ordering::SeqCst);
                }
                drop(permit);
            }
        });
        let dialing = async {
            let mut dials = Vec::new();
            for seed in 10..18_u8 {
                let backend = &backend;
                let bob_card = &bob_card;
                dials.push(async move {
                    let alice = party(
                        seed,
                        OnionServiceKey::from_bytes(
                            IdentitySecretKey::from_seed(&[seed.wrapping_add(100); 32])
                                .public_key()
                                .as_bytes(),
                        )
                        .unwrap(),
                    );
                    let isolation = backend.isolation_group().unwrap();
                    dial(
                        backend,
                        fresh(),
                        &alice,
                        bob_card,
                        PeerRecord::None,
                        &isolation,
                    )
                    .await
                    .map(|_| ())
                });
            }
            let mut ok = 0;
            for dial in dials {
                if dial.await.is_ok() {
                    ok += 1;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            stop.send(true).unwrap();
            ok
        };
        let (end, ok) = futures_join(serving, dialing).await;
        assert_eq!(end, ServeEnd::Shutdown);
        assert_eq!(ok, 8);
        assert_eq!(authenticated.load(Ordering::SeqCst), 8);
    });
}

#[test]
fn local_identities_are_reached_through_their_own_services() {
    // One process and one Tor backend can serve several local identities.
    // Each identity has its own service and answers the streams of that
    // service with its own party and its own contact lookup; a stream is
    // never answered by another local identity. Each identity dials with
    // isolation groups of its own, also towards the same remote contact.
    run(async {
        let network = MockNetwork::new();
        let backend = network.backend();
        let carol_tor = network.backend();
        let mut service_a = backend.publish_onion(KeySource::Generate).await.unwrap();
        let mut service_b = backend.publish_onion(KeySource::Generate).await.unwrap();
        let mut service_carol = carol_tor.publish_onion(KeySource::Generate).await.unwrap();
        let a = party(1, *service_a.service_key());
        let b = party(2, *service_b.service_key());
        let carol = party(3, *service_carol.service_key());
        let carol_card = carol.card().clone();

        // Carol reaches each local identity at its own service.
        for (service, local) in [(&mut service_a, &a), (&mut service_b, &b)] {
            let isolation = carol_tor.isolation_group().unwrap();
            let answering = async {
                let stream = service.accept().await.unwrap();
                answer(stream, fresh(), local, |_| {
                    PeerRecord::Accepted(&carol_card)
                })
                .await
                .unwrap()
            };
            let dialing = dial(
                &carol_tor,
                fresh(),
                &carol,
                local.card(),
                PeerRecord::Accepted(local.card()),
                &isolation,
            );
            let (dialed, answered) = futures_join(dialing, answering).await;
            assert_eq!(
                dialed.unwrap().link.session().peer(),
                local.card().identity()
            );
            assert_eq!(answered.link.session().peer(), carol_card.identity());
        }

        // A card of A that names the service of B reaches B, and B does
        // not answer as A.
        let a_at_b = ContactCard::sign(
            &IdentitySecretKey::from_seed(&[1; 32]),
            *a.card().transport(),
            EndpointEpoch::FIRST,
            EndpointSet::single(*service_b.service_key()),
            None,
        )
        .unwrap();
        let isolation = carol_tor.isolation_group().unwrap();
        let answering = async {
            let stream = service_b.accept().await.unwrap();
            answer(stream, fresh(), &b, |_| PeerRecord::None).await
        };
        let dialing = dial(
            &carol_tor,
            fresh(),
            &carol,
            &a_at_b,
            PeerRecord::None,
            &isolation,
        );
        let (dialed, answered) = futures_join(dialing, answering).await;
        assert!(dialed.is_err());
        assert!(matches!(answered, Err(LinkError::Session(_))));

        // A and B both dial Carol, each with a group of its own.
        let mut groups = Vec::new();
        for local in [&a, &b] {
            let isolation = backend.isolation_group().unwrap();
            let answering = async {
                let stream = service_carol.accept().await.unwrap();
                answer(stream, fresh(), &carol, |_| {
                    PeerRecord::Accepted(local.card())
                })
                .await
                .unwrap()
            };
            let dialing = dial(
                &backend,
                fresh(),
                local,
                &carol_card,
                PeerRecord::Accepted(&carol_card),
                &isolation,
            );
            let (dialed, answered) = futures_join(dialing, answering).await;
            assert_eq!(dialed.unwrap().link.session().peer(), carol_card.identity());
            assert_eq!(answered.link.session().peer(), local.card().identity());
            groups.push(isolation);
        }
        assert!(!groups[0].same_as(&groups[1]));
    });
}

#[test]
fn a_silent_peer_is_dropped_after_the_handshake_timeout() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap()
        .block_on(async {
            let network = MockNetwork::new();
            let backend = network.backend();
            let mut service = backend.publish_onion(KeySource::Generate).await.unwrap();
            let bob = party(2, *service.service_key());
            let isolation = backend.isolation_group().unwrap();
            // Connects and then sends nothing at all.
            let _silent = backend
                .connect_onion(service.service_key(), &isolation)
                .await
                .unwrap();
            let stream = service.accept().await.unwrap();
            let started = tokio::time::Instant::now();
            let result = answer(stream, fresh(), &bob, |_| PeerRecord::None).await;
            assert_eq!(result.err(), Some(LinkError::TimedOut));
            let timeout = monolith_protocol::limits::HANDSHAKE_TIMEOUT;
            assert!(started.elapsed() >= timeout);
            assert!(started.elapsed() <= timeout + Duration::from_secs(1));
        });
}

#[test]
fn strangers_beyond_the_unknown_session_budget_are_closed() {
    run(async {
        let network = MockNetwork::new();
        let backend = network.backend();
        let mut service = backend.publish_onion(KeySource::Generate).await.unwrap();
        let bob = party(2, *service.service_key());
        let bob_card = bob.card().clone();
        let budgets = Budgets::new();
        let mut held = Vec::new();
        for seed in
            20..20 + u8::try_from(monolith_protocol::limits::MAX_UNKNOWN_SESSIONS).unwrap() + 1
        {
            let stranger = party(
                seed,
                OnionServiceKey::from_bytes(
                    IdentitySecretKey::from_seed(&[seed.wrapping_add(100); 32])
                        .public_key()
                        .as_bytes(),
                )
                .unwrap(),
            );
            let isolation = backend.isolation_group().unwrap();
            let bob_side = async {
                let stream = service.accept().await.unwrap();
                answer(stream, &budgets, &bob, |_| PeerRecord::None).await
            };
            let stranger_side = dial(
                &backend,
                fresh(),
                &stranger,
                &bob_card,
                PeerRecord::None,
                &isolation,
            );
            let (_, answered) = futures_join(stranger_side, bob_side).await;
            if usize::from(seed - 20) < monolith_protocol::limits::MAX_UNKNOWN_SESSIONS {
                let established = answered.unwrap();
                assert!(established.unknown_slot.is_some());
                held.push(established);
            } else {
                assert_eq!(answered.err(), Some(LinkError::Budget));
            }
        }
        // A slot comes back when its session goes.
        held.pop();
        let isolation = backend.isolation_group().unwrap();
        let late = party(99, *service.service_key());
        let bob_side = async {
            let stream = service.accept().await.unwrap();
            answer(stream, &budgets, &bob, |_| PeerRecord::None).await
        };
        let late_side = dial(
            &backend,
            fresh(),
            &late,
            &bob_card,
            PeerRecord::None,
            &isolation,
        );
        let (_, answered) = futures_join(late_side, bob_side).await;
        assert!(answered.is_ok());
    });
}

#[test]
fn listener_errors_do_not_end_the_accept_loop() {
    run(async {
        let network = MockNetwork::new();
        let backend = network.backend();
        let mut service = backend.publish_onion(KeySource::Generate).await.unwrap();
        let key = *service.service_key();
        network.fail_accepts(&key, 3);
        let budgets = Budgets::new();
        let (stop, shutdown) = watch::channel(false);
        let accepted = Arc::new(AtomicUsize::new(0));
        let count = accepted.clone();
        let serving = serve(&mut service, &budgets, shutdown, move |_stream, _permit| {
            count.fetch_add(1, Ordering::SeqCst);
            async {}
        });
        let dialing = async {
            let isolation = backend.isolation_group().unwrap();
            // Three errors, each followed by ACCEPT_BACKOFF, then a stream.
            tokio::time::sleep(Duration::from_millis(100)).await;
            let _stream = backend.connect_onion(&key, &isolation).await.unwrap();
            tokio::time::sleep(Duration::from_millis(1500)).await;
            stop.send(true).unwrap();
        };
        let (end, ()) = futures_join(serving, dialing).await;
        assert_eq!(end, ServeEnd::Shutdown);
        assert_eq!(accepted.load(Ordering::SeqCst), 1);
    });
}

#[test]
fn dials_beyond_the_dial_budget_wait_for_a_slot() {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap()
        .block_on(async {
            let network = MockNetwork::new();
            let backend = network.backend();
            // A service that never answers: each dial holds its slot until
            // the handshake deadline.
            let service = backend.publish_onion(KeySource::Generate).await.unwrap();
            let card = party(2, *service.service_key()).card().clone();
            let budgets = Budgets::new();
            let mut tasks = tokio::task::JoinSet::new();
            for seed in 30..30 + 5_u8 {
                let backend = network.backend();
                let budgets = budgets.clone();
                let card = card.clone();
                let local = party(seed, *service.service_key());
                tasks.spawn(async move {
                    let isolation = backend.isolation_group().unwrap();
                    dial(
                        &backend,
                        &budgets,
                        &local,
                        &card,
                        PeerRecord::None,
                        &isolation,
                    )
                    .await
                    .err()
                });
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            assert_eq!(
                network.dials(),
                monolith_protocol::limits::MAX_CONCURRENT_DIALS
            );
            // When the first four give up at the handshake deadline, the
            // fifth gets its slot.
            tokio::time::sleep(monolith_protocol::limits::HANDSHAKE_TIMEOUT).await;
            assert_eq!(
                network.dials(),
                monolith_protocol::limits::MAX_CONCURRENT_DIALS + 1
            );
            while let Some(result) = tasks.join_next().await {
                assert_eq!(result.unwrap(), Some(LinkError::TimedOut));
            }
            drop(service);
        });
}

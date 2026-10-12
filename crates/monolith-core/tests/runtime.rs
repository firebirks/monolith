//! The messaging runtime over the in-memory Tor
//! (`docs/adr/0007-delivery-semantics.md`,
//! `docs/adr/0009-session-manager.md`): queueing while the peer or Tor is
//! away, delivery once it is back, one session when both sides dial,
//! retries after a lost acknowledgement, and work that stops when a contact
//! or an identity is gone.
//!
//! Time is paused and advances only when every task waits, so the
//! reconnect delays pass at once and every run takes the same steps.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

mod common;

use core::time::Duration;
use std::sync::Arc;

use common::{Released, fixed_endpoint, keys, run, run_paused};
use monolith_core::api::{ContactId, DeferReason, Event, IdentityId, MessageStatus, SendError};
use monolith_core::budget::Budgets;
use monolith_core::identity::{Installation, LocalIdentity};
use monolith_core::runtime::{Runtime, RuntimeOptions};
use monolith_protocol::limits::{
    HANDSHAKE_TIMEOUT, MAX_CHAT_TEXT_LEN, MESSAGE_ACK_TIMEOUT, SESSION_LINGER,
};
use monolith_storage::dir::{CrashOutcome, MemoryDir};
use monolith_storage::vault::{KdfParams, Passphrase};
use monolith_tor::{KeySource, MockNetwork, MockTorBackend, OnionServiceSecret, TorBackend};
use tokio::sync::broadcast;

fn passphrase() -> Passphrase {
    Passphrase::new("correct horse battery staple").unwrap()
}

/// One installation with one identity of fixed keys, and its runtime once
/// it is started.
struct Peer {
    seed: u8,
    installation: Installation,
    identity: Arc<LocalIdentity>,
    runtime: Option<Runtime<MockTorBackend>>,
}

impl Peer {
    async fn new(seed: u8) -> Self {
        Self::in_installation(Installation::ephemeral(), seed).await
    }

    async fn in_installation(installation: Installation, seed: u8) -> Self {
        let identity = installation
            .restore_identity(keys(seed, seed, 1, fixed_endpoint(seed)), None)
            .await
            .unwrap();
        Self {
            seed,
            installation,
            identity,
            runtime: None,
        }
    }

    fn id(&self) -> IdentityId {
        self.identity.id()
    }

    fn runtime(&self) -> &Runtime<MockTorBackend> {
        self.runtime.as_ref().expect("started")
    }

    /// Starts the runtime and waits until its service is published.
    async fn start(&mut self, network: &MockNetwork, options: RuntimeOptions) {
        self.runtime = Some(Runtime::start(
            self.installation.clone(),
            network.backend(),
            Budgets::new(),
            options,
        ));
        until(|| network.is_published(&fixed_endpoint(self.seed))).await;
    }

    /// Stops the runtime as a crash of the process would: every task ends
    /// where it is.
    async fn stop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown(Duration::ZERO).await;
        }
    }

    /// Makes `other` a contact of this identity by its card. Returns its
    /// record.
    async fn add(&self, other: &Peer) -> ContactId {
        self.identity.import(&other.identity.card()).await.unwrap();
        self.identity
            .contact_id(other.identity.identity())
            .expect("a record")
    }

    fn statuses(&self, contact: &ContactId) -> Vec<MessageStatus> {
        self.runtime()
            .messages(&self.id(), contact, None, 1000)
            .unwrap()
            .into_iter()
            .map(|message| message.status)
            .collect()
    }
}

fn options() -> RuntimeOptions {
    RuntimeOptions {
        startup_spread: Duration::ZERO,
        ..RuntimeOptions::default()
    }
}

/// Waits, with paused time, until `condition` holds, for up to a day of
/// test time.
async fn until(condition: impl Fn() -> bool) {
    for _ in 0..(24 * 60 * 60 * 10) {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the condition did not come about");
}

/// Waits for an event that `matches`, for up to a day of test time.
async fn event(events: &mut broadcast::Receiver<Event>, matches: impl Fn(&Event) -> bool) -> Event {
    let wait = async {
        loop {
            match events.recv().await {
                Ok(event) if matches(&event) => return event,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => panic!("no more events"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(24 * 60 * 60), wait)
        .await
        .expect("the event did not come")
}

/// Two peers that hold each other's cards, both requested contacts.
async fn pair(network: &MockNetwork) -> (Peer, Peer, ContactId, ContactId) {
    let a = Peer::new(1).await;
    let b = Peer::new(2).await;
    let at_a = a.add(&b).await;
    let at_b = b.add(&a).await;
    let _ = network;
    (a, b, at_a, at_b)
}

#[test]
fn a_message_queued_while_the_peer_is_away_is_delivered_once_it_is_back() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = pair(&network).await;
        a.start(&network, options()).await;
        let mut events = a.runtime().subscribe();
        let sent = a
            .runtime()
            .send_message(&a.id(), &at_a, "hello")
            .await
            .unwrap();
        assert_eq!(a.statuses(&at_a), vec![MessageStatus::Queued]);
        // B is not online: the attempt is deferred, and repeated later.
        event(&mut events, |event| {
            matches!(
                event,
                Event::DeliveryDeferred {
                    reason: DeferReason::Tor,
                    ..
                }
            )
        })
        .await;
        b.start(&network, options()).await;
        let delivered = event(&mut events, |event| {
            matches!(event, Event::MessageDelivered(_))
        })
        .await;
        assert_eq!(delivered, Event::MessageDelivered(sent));
        assert_eq!(a.statuses(&at_a), vec![MessageStatus::Delivered]);
        let received = b.runtime().messages(&b.id(), &at_b, None, 10).unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].status, MessageStatus::Received);
        assert_eq!(received[0].text, "hello");
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn delivery_resumes_through_tor_once_it_is_back() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = pair(&network).await;
        a.start(&network, options()).await;
        b.start(&network, options()).await;
        let mut events = a.runtime().subscribe();
        network.set_socks_available(false);
        a.runtime()
            .send_message(&a.id(), &at_a, "through tor")
            .await
            .unwrap();
        event(&mut events, |event| {
            matches!(
                event,
                Event::DeliveryDeferred {
                    reason: DeferReason::Tor,
                    ..
                }
            )
        })
        .await;
        assert_eq!(a.statuses(&at_a), vec![MessageStatus::Queued]);
        network.set_socks_available(true);
        event(&mut events, |event| {
            matches!(event, Event::MessageDelivered(_))
        })
        .await;
        assert_eq!(b.statuses(&at_b), vec![MessageStatus::Received]);
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn both_sides_sending_at_once_end_with_one_session_and_every_message_once() {
    for gap in [0_u64, 1, 5, 50] {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut a, mut b, at_a, at_b) = pair(&network).await;
            a.start(&network, options()).await;
            b.start(&network, options()).await;
            let mut at_a_events = a.runtime().subscribe();
            let mut at_b_events = b.runtime().subscribe();
            a.runtime()
                .send_message(&a.id(), &at_a, "from a")
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(gap)).await;
            b.runtime()
                .send_message(&b.id(), &at_b, "from b")
                .await
                .unwrap();
            until(|| {
                a.statuses(&at_a).contains(&MessageStatus::Delivered)
                    && b.statuses(&at_b).contains(&MessageStatus::Delivered)
            })
            .await;
            // Each side holds its own message, delivered, and the other's,
            // received, once.
            for statuses in [a.statuses(&at_a), b.statuses(&at_b)] {
                assert_eq!(statuses.len(), 2, "{gap} {statuses:?}");
                assert!(statuses.contains(&MessageStatus::Delivered), "{gap}");
                assert!(statuses.contains(&MessageStatus::Received), "{gap}");
            }
            // One session stays on each side, the same one.
            until(|| {
                a.runtime().sessions(&a.id(), &at_a).unwrap() <= 1
                    && b.runtime().sessions(&b.id(), &at_b).unwrap() <= 1
            })
            .await;
            // Whatever sessions came and went, the record was reported
            // available and unavailable in turn, starting with available.
            tokio::time::sleep(SESSION_LINGER * 2).await;
            for events in [&mut at_a_events, &mut at_b_events] {
                let mut available = false;
                while let Ok(event) = events.try_recv() {
                    match event {
                        Event::SessionAvailable { .. } => {
                            assert!(!available, "{gap}");
                            available = true;
                        }
                        Event::SessionUnavailable { .. } => {
                            assert!(available, "{gap}");
                            available = false;
                        }
                        _ => {}
                    }
                }
                assert!(!available, "{gap}");
            }
            a.stop().await;
            b.stop().await;
        });
    }
}

#[test]
fn a_lost_acknowledgement_is_followed_by_a_retry_the_receiver_drops() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = pair(&network).await;
        a.start(&network, options()).await;
        // B takes the message in and crashes before it acknowledges it.
        b.start(
            &network,
            RuntimeOptions {
                acknowledgement_delay: Duration::from_secs(3600),
                ..options()
            },
        )
        .await;
        let mut at_b_events = b.runtime().subscribe();
        let mut at_a_events = a.runtime().subscribe();
        let sent = a
            .runtime()
            .send_message(&a.id(), &at_a, "once")
            .await
            .unwrap();
        event(&mut at_b_events, |event| {
            matches!(event, Event::MessageReceived(_))
        })
        .await;
        b.stop().await;
        // A had no acknowledgement in time.
        assert_eq!(a.statuses(&at_a), vec![MessageStatus::Queued]);
        b.start(&network, options()).await;
        let mut at_b_events = b.runtime().subscribe();
        let delivered = event(&mut at_a_events, |event| {
            matches!(event, Event::MessageDelivered(_))
        })
        .await;
        assert_eq!(delivered, Event::MessageDelivered(sent));
        // B holds it once and raised no second event for it.
        assert_eq!(b.statuses(&at_b), vec![MessageStatus::Received]);
        while let Ok(event) = at_b_events.try_recv() {
            assert!(!matches!(event, Event::MessageReceived(_)), "{event:?}");
        }
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_session_that_ends_with_a_message_in_flight_reports_nothing_delivered() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = pair(&network).await;
        a.start(&network, options()).await;
        b.start(
            &network,
            RuntimeOptions {
                acknowledgement_delay: Duration::from_secs(3600),
                ..options()
            },
        )
        .await;
        let mut at_a_events = a.runtime().subscribe();
        let mut at_b_events = b.runtime().subscribe();
        let sent = a
            .runtime()
            .send_message(&a.id(), &at_a, "in flight")
            .await
            .unwrap();
        event(&mut at_b_events, |event| {
            matches!(event, Event::MessageReceived(_))
        })
        .await;
        // B goes away without acknowledging: A's session ends with the
        // message in flight, and A tries again while B is away. Nothing up
        // to the end of the session says delivered.
        b.stop().await;
        event(&mut at_a_events, |event| {
            assert!(!matches!(event, Event::MessageDelivered(_)), "{event:?}");
            matches!(event, Event::SessionUnavailable { .. })
        })
        .await;
        tokio::time::sleep(Duration::from_secs(600)).await;
        while let Ok(event) = at_a_events.try_recv() {
            assert!(!matches!(event, Event::MessageDelivered(_)), "{event:?}");
        }
        assert_eq!(a.statuses(&at_a), vec![MessageStatus::Queued]);
        // Delivered once B is back, and reported once.
        b.start(&network, options()).await;
        let delivered = event(&mut at_a_events, |event| {
            matches!(event, Event::MessageDelivered(_))
        })
        .await;
        assert_eq!(delivered, Event::MessageDelivered(sent));
        assert_eq!(a.statuses(&at_a), vec![MessageStatus::Delivered]);
        assert_eq!(b.statuses(&at_b), vec![MessageStatus::Received]);
        tokio::time::sleep(Duration::from_secs(600)).await;
        while let Ok(event) = at_a_events.try_recv() {
            assert!(!matches!(event, Event::MessageDelivered(_)), "{event:?}");
        }
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn no_acknowledgement_in_time_closes_the_session_and_tries_again_later() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, _) = pair(&network).await;
        a.start(&network, options()).await;
        b.start(
            &network,
            RuntimeOptions {
                acknowledgement_delay: MESSAGE_ACK_TIMEOUT * 10,
                ..options()
            },
        )
        .await;
        let mut events = a.runtime().subscribe();
        a.runtime()
            .send_message(&a.id(), &at_a, "slow")
            .await
            .unwrap();
        event(&mut events, |event| {
            matches!(
                event,
                Event::DeliveryDeferred {
                    reason: DeferReason::AckTimeout,
                    ..
                }
            )
        })
        .await;
        assert_eq!(a.statuses(&at_a), vec![MessageStatus::Queued]);
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_message_in_flight_is_sent_once_and_waited_for_from_then() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, _) = pair(&network).await;
        a.start(&network, options()).await;
        b.start(
            &network,
            RuntimeOptions {
                acknowledgement_delay: MESSAGE_ACK_TIMEOUT * 10,
                ..options()
            },
        )
        .await;
        let mut at_a_events = a.runtime().subscribe();
        let mut at_b_events = b.runtime().subscribe();
        a.runtime()
            .send_message(&a.id(), &at_a, "first")
            .await
            .unwrap();
        event(&mut at_b_events, |event| {
            matches!(event, Event::MessageReceived(_))
        })
        .await;
        let sent = tokio::time::Instant::now();
        // More is queued behind it while its acknowledgement does not come.
        // A send of the session starts the wait for an acknowledgement
        // again, so the session ends `MESSAGE_ACK_TIMEOUT` after the one
        // send only if it sends neither this message again nor anything
        // behind it.
        for text in ["second", "third"] {
            tokio::time::sleep(MESSAGE_ACK_TIMEOUT / 3).await;
            a.runtime()
                .send_message(&a.id(), &at_a, text)
                .await
                .unwrap();
        }
        event(&mut at_a_events, |event| {
            matches!(
                event,
                Event::DeliveryDeferred {
                    reason: DeferReason::AckTimeout,
                    ..
                }
            )
        })
        .await;
        assert!(
            sent.elapsed() <= MESSAGE_ACK_TIMEOUT,
            "{:?}",
            sent.elapsed()
        );
        assert_eq!(a.statuses(&at_a), vec![MessageStatus::Queued; 3]);
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_blocked_contact_gets_nothing_of_its_queue() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = pair(&network).await;
        a.start(&network, options()).await;
        let mut events = a.runtime().subscribe();
        a.runtime()
            .send_message(&a.id(), &at_a, "never")
            .await
            .unwrap();
        event(&mut events, |event| {
            matches!(event, Event::DeliveryDeferred { .. })
        })
        .await;
        a.runtime().block_contact(&a.id(), &at_a).await.unwrap();
        b.start(&network, options()).await;
        let dials = network.dials();
        // A day passes: A dials nobody for the ended record.
        tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        assert_eq!(network.dials(), dials);
        assert_eq!(a.statuses(&at_a), vec![MessageStatus::Queued]);
        assert!(b.statuses(&at_b).is_empty());
        let conversation = &a.runtime().conversations(&a.id()).unwrap()[0];
        assert!(!conversation.active);
        assert_eq!(
            a.runtime().send_message(&a.id(), &at_a, "more").await,
            Err(SendError::NotAContact)
        );
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn blocking_or_deleting_a_contact_ends_the_attempt_in_progress() {
    for block in [true, false] {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut a, _b, at_a, _) = pair(&network).await;
            // B's service takes streams and nothing answers on them: a dial
            // waits for the handshake until HANDSHAKE_TIMEOUT.
            let _silent = network
                .backend()
                .publish_onion(KeySource::Existing {
                    secret: OnionServiceSecret::from_bytes(&[2; 64]),
                    expected: fixed_endpoint(2),
                })
                .await
                .unwrap();
            a.start(&network, options()).await;
            let mut events = a.runtime().subscribe();
            a.runtime().send_message(&a.id(), &at_a, "x").await.unwrap();
            until(|| network.dials() == 1).await;
            if block {
                a.runtime().block_contact(&a.id(), &at_a).await.unwrap();
            } else {
                a.runtime().delete_contact(&a.id(), &at_a).await.unwrap();
            }
            // The attempt ended with the record: it reports nothing, and
            // nothing follows it.
            tokio::time::sleep(HANDSHAKE_TIMEOUT * 100).await;
            while let Ok(event) = events.try_recv() {
                assert!(
                    !matches!(event, Event::DeliveryDeferred { .. }),
                    "{block} {event:?}"
                );
            }
            assert_eq!(network.dials(), 1);
            assert_eq!(a.statuses(&at_a), vec![MessageStatus::Queued]);
            a.stop().await;
        });
    }
}

#[test]
fn a_contact_made_again_is_not_dialed_for_the_queue_of_its_old_record() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, b, old, _) = pair(&network).await;
        a.start(&network, options()).await;
        let mut events = a.runtime().subscribe();
        a.runtime()
            .send_message(&a.id(), &old, "old")
            .await
            .unwrap();
        event(&mut events, |event| {
            matches!(event, Event::DeliveryDeferred { .. })
        })
        .await;
        a.runtime().delete_contact(&a.id(), &old).await.unwrap();
        let new = a.add(&b).await;
        assert_ne!(new, old);
        // Nothing is queued for the new record: nothing is dialed for it,
        // and the old record's queue stays where it is.
        let dials = network.dials();
        tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        assert_eq!(network.dials(), dials);
        assert_eq!(a.statuses(&old), vec![MessageStatus::Queued]);
        assert!(a.statuses(&new).is_empty());
        a.stop().await;
    });
}

#[test]
fn a_failed_attempt_is_not_repeated_at_once() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, _) = pair(&network).await;
        a.start(&network, options()).await;
        // B is not online: every attempt fails at once, after Tor counted
        // it.
        a.runtime().send_message(&a.id(), &at_a, "x").await.unwrap();
        // Time stands still while this task yields: a scheduler that waits
        // between attempts makes one; one that does not, many.
        for _ in 0..10_000 {
            tokio::task::yield_now().await;
        }
        assert_eq!(network.dials(), 1);
        a.stop().await;
    });
}

#[test]
fn a_deleted_identity_stops_dialing_and_takes_no_command() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, _) = pair(&network).await;
        a.start(&network, options()).await;
        let mut events = a.runtime().subscribe();
        a.runtime().send_message(&a.id(), &at_a, "x").await.unwrap();
        event(&mut events, |event| {
            matches!(event, Event::DeliveryDeferred { .. })
        })
        .await;
        let id = a.id();
        a.runtime().delete_identity(&id).await.unwrap();
        event(&mut events, |event| {
            matches!(event, Event::IdentityClosed { .. })
        })
        .await;
        let dials = network.dials();
        tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        assert_eq!(network.dials(), dials);
        assert_eq!(
            a.runtime().send_message(&id, &at_a, "y").await,
            Err(SendError::UnknownIdentity)
        );
        assert!(a.runtime().conversations(&id).is_err());
        a.stop().await;
    });
}

#[test]
fn nothing_is_dialed_without_a_queued_message_and_an_idle_session_ends() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = pair(&network).await;
        a.start(&network, options()).await;
        b.start(&network, options()).await;
        tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        assert_eq!(network.dials(), 0);
        a.runtime()
            .send_message(&a.id(), &at_a, "hello")
            .await
            .unwrap();
        until(|| a.statuses(&at_a) == vec![MessageStatus::Delivered]).await;
        assert_eq!(network.dials(), 1);
        // Nothing left to say: the session lingers, then ends.
        tokio::time::sleep(SESSION_LINGER * 2).await;
        until(|| {
            a.runtime().sessions(&a.id(), &at_a).unwrap() == 0
                && b.runtime().sessions(&b.id(), &at_b).unwrap() == 0
        })
        .await;
        tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        assert_eq!(network.dials(), 1);
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn text_is_checked_before_anything_is_stored() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, _) = pair(&network).await;
        a.start(&network, options()).await;
        let runtime = a.runtime();
        for len in [MAX_CHAT_TEXT_LEN - 1, MAX_CHAT_TEXT_LEN] {
            runtime
                .send_message(&a.id(), &at_a, &"x".repeat(len))
                .await
                .unwrap();
        }
        assert_eq!(
            runtime
                .send_message(&a.id(), &at_a, &"x".repeat(MAX_CHAT_TEXT_LEN + 1))
                .await,
            Err(SendError::TooLong)
        );
        assert_eq!(
            runtime.send_message(&a.id(), &at_a, "").await,
            Err(SendError::InvalidText)
        );
        assert_eq!(
            runtime.send_message(&a.id(), &at_a, "a\rb").await,
            Err(SendError::InvalidText)
        );
        assert_eq!(a.statuses(&at_a).len(), 2);
        a.stop().await;
    });
}

#[test]
fn identities_of_one_runtime_keep_their_queues_and_sessions_apart() {
    run_paused(async {
        let network = MockNetwork::new();
        let installation = Installation::ephemeral();
        let x = Peer::in_installation(installation.clone(), 1).await;
        let mut y = Peer::in_installation(installation.clone(), 3).await;
        let mut p = Peer::new(2).await;
        let x_to_p = x.add(&p).await;
        let y_to_p = y.add(&p).await;
        let p_to_x = p.add(&x).await;
        let _p_to_y = p.add(&y).await;
        y.start(&network, options()).await;
        until(|| network.is_published(&fixed_endpoint(1))).await;
        p.start(&network, options()).await;
        let runtime = y.runtime();
        runtime
            .send_message(&y.id(), &y_to_p, "from y")
            .await
            .unwrap();
        p.runtime()
            .send_message(&p.id(), &p_to_x, "to x")
            .await
            .unwrap();
        until(|| {
            runtime.messages(&x.id(), &x_to_p, None, 10).unwrap().len() == 1
                && runtime.messages(&y.id(), &y_to_p, None, 10).unwrap()[0].status
                    == MessageStatus::Delivered
        })
        .await;
        let at_x = runtime.messages(&x.id(), &x_to_p, None, 10).unwrap();
        assert_eq!(at_x[0].text, "to x");
        assert_eq!(
            runtime.messages(&y.id(), &y_to_p, None, 10).unwrap().len(),
            1
        );
        y.stop().await;
        p.stop().await;
    });
}

#[test]
fn a_failed_message_store_ends_every_session_and_attempt() {
    run_paused(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let mut a = Peer::in_installation(installation, 1).await;
        let mut b = Peer::new(2).await;
        let at_a = a.add(&b).await;
        let at_b = b.add(&a).await;
        a.start(&network, options()).await;
        // B acknowledges five seconds after it stored a message.
        b.start(
            &network,
            RuntimeOptions {
                acknowledgement_delay: Duration::from_secs(5),
                ..options()
            },
        )
        .await;
        let mut events = a.runtime().subscribe();
        // A first delivery: the session is up, and both accepted each
        // other durably.
        a.runtime()
            .send_message(&a.id(), &at_a, "zero")
            .await
            .unwrap();
        until(|| a.statuses(&at_a) == vec![MessageStatus::Delivered]).await;
        for text in ["one", "two"] {
            a.runtime()
                .send_message(&a.id(), &at_a, text)
                .await
                .unwrap();
        }
        // B stores "one"; A's message store fails before it records the
        // delivery.
        until(|| b.statuses(&at_b).len() == 2).await;
        dir.crash_at(dir.steps() + 1);
        event(&mut events, |event| matches!(event, Event::StorageFailed)).await;
        // Nothing more is recorded, so nothing more is sent: the session
        // ends at once, "two" is not sent again and again, and no attempt
        // follows.
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(a.runtime().sessions(&a.id(), &at_a).unwrap(), 0);
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, Event::StorageFailed), "{event:?}");
        }
        let dials = network.dials();
        tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        assert_eq!(network.dials(), dials);
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_session_that_ends_with_its_service_leaves_no_registration_behind() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = pair(&network).await;
        a.start(&network, options()).await;
        b.start(&network, options()).await;
        a.runtime()
            .send_message(&a.id(), &at_a, "one")
            .await
            .unwrap();
        until(|| a.statuses(&at_a) == vec![MessageStatus::Delivered]).await;
        // A dialed: B's session runs under the accept loop of its service.
        assert_eq!(b.runtime().sessions(&b.id(), &at_b).unwrap(), 1);
        // The service goes away, as when Tor restarts: the accept loop
        // ends and its sessions with it.
        network.lose_service(&fixed_endpoint(2));
        until(|| b.runtime().sessions(&b.id(), &at_b).unwrap() == 0).await;
        // B's queue is dialed for and delivered.
        b.runtime()
            .send_message(&b.id(), &at_b, "two")
            .await
            .unwrap();
        until(|| b.statuses(&at_b).contains(&MessageStatus::Delivered)).await;
        assert!(a.statuses(&at_a).contains(&MessageStatus::Received));
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_peer_that_never_acknowledges_is_dialed_with_growing_delays() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, _) = pair(&network).await;
        a.start(&network, options()).await;
        b.start(
            &network,
            RuntimeOptions {
                acknowledgement_delay: MESSAGE_ACK_TIMEOUT * 1000,
                ..options()
            },
        )
        .await;
        let mut events = a.runtime().subscribe();
        a.runtime()
            .send_message(&a.id(), &at_a, "never acknowledged")
            .await
            .unwrap();
        let mut delays = Vec::new();
        while delays.len() < 4 {
            if let Event::DeliveryDeferred {
                reason: DeferReason::AckTimeout,
                retry_in,
                ..
            } = event(&mut events, |event| {
                matches!(
                    event,
                    Event::DeliveryDeferred {
                        reason: DeferReason::AckTimeout,
                        ..
                    }
                )
            })
            .await
            {
                delays.push(retry_in);
            }
        }
        // The delays double, with their jitter: the fourth is at least
        // half of 80 seconds.
        assert!(delays[3] >= Duration::from_secs(40), "{delays:?}");
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_send_its_caller_stops_waiting_for_completes() {
    // In real time: paused time does not move while a write is held on a
    // thread of its own.
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let mut a = Peer::in_installation(installation, 1).await;
        let mut b = Peer::new(2).await;
        let at_a = a.add(&b).await;
        let at_b = b.add(&a).await;
        a.start(&network, options()).await;
        b.start(&network, options()).await;
        let mut events = a.runtime().subscribe();
        // The write of the message is held, and the caller gives up.
        dir.hold_writes();
        let released = Released(&dir);
        let sending = tokio::time::timeout(
            Duration::from_millis(200),
            a.runtime().send_message(&a.id(), &at_a, "half"),
        )
        .await;
        assert!(sending.is_err());
        drop(released);
        // The command completed all the same: it was reported, and the
        // message is delivered.
        tokio::time::timeout(
            Duration::from_secs(30),
            event(&mut events, |event| {
                matches!(event, Event::MessageQueued(_))
            }),
        )
        .await
        .expect("the queueing was reported");
        tokio::time::timeout(
            Duration::from_secs(30),
            until(|| a.statuses(&at_a) == vec![MessageStatus::Delivered]),
        )
        .await
        .expect("the message was delivered");
        assert_eq!(b.statuses(&at_b), vec![MessageStatus::Received]);
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_runtime_dropped_without_a_shutdown_stops() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, _) = pair(&network).await;
        a.start(&network, options()).await;
        a.runtime().send_message(&a.id(), &at_a, "x").await.unwrap();
        let runtime = a.runtime.take().unwrap();
        let copy = runtime.clone();
        drop(runtime);
        // A copy keeps it running.
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert!(network.is_published(&fixed_endpoint(1)));
        drop(copy);
        // The last handle is gone: its service goes at once, and it dials
        // nobody.
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(!network.is_published(&fixed_endpoint(1)));
        let dials = network.dials();
        tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        assert_eq!(network.dials(), dials);
    });
}

#[test]
fn a_failed_vault_ends_every_attempt() {
    run_paused(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let mut a = Peer::in_installation(installation, 1).await;
        let b = Peer::new(2).await;
        let at_a = a.add(&b).await;
        a.start(&network, options()).await;
        let mut events = a.runtime().subscribe();
        // B is away: A dials with the reconnect delays.
        a.runtime().send_message(&a.id(), &at_a, "x").await.unwrap();
        event(&mut events, |event| {
            matches!(event, Event::DeliveryDeferred { .. })
        })
        .await;
        // A change of the vault fails.
        dir.crash_at(dir.steps() + 1);
        assert!(
            a.identity
                .set_verified(b.identity.identity(), true)
                .await
                .is_err()
        );
        assert!(a.identity.vault_failed());
        event(&mut events, |event| matches!(event, Event::StorageFailed)).await;
        // Nothing is made durable any more: the scheduler stops dialing.
        tokio::time::sleep(Duration::from_secs(60 * 60)).await;
        let dials = network.dials();
        tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
        assert_eq!(network.dials(), dials);
        a.stop().await;
    });
}

#[test]
fn a_record_with_two_sessions_is_reported_available_once() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = pair(&network).await;
        a.start(&network, options()).await;
        b.start(&network, options()).await;
        let mut events = a.runtime().subscribe();
        // Both sides ask for a session at the same moment: two sessions of
        // the record register at each side, and the duplicate rule keeps
        // one.
        a.runtime().connect(&a.id(), &at_a).unwrap();
        b.runtime().connect(&b.id(), &at_b).unwrap();
        until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 1).await;
        // The idle session lingers and ends.
        tokio::time::sleep(SESSION_LINGER * 2).await;
        let mut reported = Vec::new();
        while let Ok(event) = events.try_recv() {
            match event {
                Event::SessionAvailable { .. } => reported.push(true),
                Event::SessionUnavailable { .. } => reported.push(false),
                _ => {}
            }
        }
        assert_eq!(reported, vec![true, false]);
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_failed_vault_is_reported_at_once_and_refuses_a_send() {
    run_paused(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let mut a = Peer::in_installation(installation, 1).await;
        let b = Peer::new(2).await;
        let at_a = a.add(&b).await;
        // Nothing runs: no queue, no session, no attempt.
        a.start(&network, options()).await;
        let mut events = a.runtime().subscribe();
        dir.crash_at(dir.steps() + 1);
        assert!(
            a.identity
                .set_verified(b.identity.identity(), true)
                .await
                .is_err()
        );
        // Reported at once, by the runtime itself.
        tokio::time::timeout(
            Duration::from_secs(1),
            event(&mut events, |event| matches!(event, Event::StorageFailed)),
        )
        .await
        .expect("the failure was reported");
        // A send is refused as a store that failed, not as an identity
        // that is gone.
        assert_eq!(
            a.runtime().send_message(&a.id(), &at_a, "x").await,
            Err(SendError::Storage)
        );
        a.stop().await;
    });
}

#[test]
fn an_identity_deleted_while_a_send_waits_for_the_vault_fails_nothing() {
    // In real time: writes are held on a thread of their own.
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let mut a = Peer::in_installation(installation, 1).await;
        let b = Peer::new(2).await;
        let at_a = a.add(&b).await;
        a.start(&network, options()).await;
        let mut events = a.runtime().subscribe();
        // A change of the vault waits for its write.
        dir.hold_writes();
        let released = Released(&dir);
        let identity = a.identity.clone();
        let peer = *b.identity.identity();
        let verifying = tokio::spawn(async move { identity.set_verified(&peer, true).await });
        common::held(&dir).await;
        // A send waits for the vault, and the identity is deleted.
        let runtime = a.runtime().clone();
        let id = a.id();
        let sending = tokio::spawn(async move { runtime.send_message(&id, &at_a, "x").await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let runtime = a.runtime().clone();
        let deleting = tokio::spawn(async move { runtime.delete_identity(&id).await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(released);
        assert_eq!(sending.await.unwrap(), Err(SendError::UnknownIdentity));
        deleting.await.unwrap().unwrap();
        let _ = verifying.await.unwrap();
        // Nothing failed, and nothing says so.
        assert!(!a.installation.is_failed());
        assert!(!a.installation.messages_failed());
        tokio::time::sleep(Duration::from_millis(200)).await;
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, Event::StorageFailed), "{event:?}");
        }
        a.stop().await;
    });
}

#[test]
fn a_peer_that_dials_in_and_never_acknowledges_does_not_reset_the_delays() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = pair(&network).await;
        a.start(&network, options()).await;
        let mut events = a.runtime().subscribe();
        a.runtime()
            .send_message(&a.id(), &at_a, "from a")
            .await
            .unwrap();
        // B is away: three attempts fail, and the delays grow.
        for _ in 0..3 {
            event(&mut events, |event| {
                matches!(
                    event,
                    Event::DeliveryDeferred {
                        reason: DeferReason::Tor,
                        ..
                    }
                )
            })
            .await;
        }
        // B comes, dials A for a message of its own and never acknowledges
        // what A sends on that session.
        b.start(
            &network,
            RuntimeOptions {
                acknowledgement_delay: MESSAGE_ACK_TIMEOUT * 1000,
                ..options()
            },
        )
        .await;
        b.runtime()
            .send_message(&b.id(), &at_b, "from b")
            .await
            .unwrap();
        event(&mut events, |event| {
            matches!(event, Event::SessionUnavailable { .. })
        })
        .await;
        // The session ended for want of an acknowledgement: the next
        // attempt follows the grown delay, not the first one again.
        let ended = tokio::time::Instant::now();
        let dials = network.dials();
        until(|| network.dials() > dials).await;
        assert!(
            ended.elapsed() >= Duration::from_secs(20),
            "{:?}",
            ended.elapsed()
        );
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_shutdown_waits_for_a_command_under_way() {
    // In real time: the write is held on a thread of its own.
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let mut a = Peer::in_installation(installation, 1).await;
        let b = Peer::new(2).await;
        let at_a = a.add(&b).await;
        a.start(&network, options()).await;
        let mut events = a.runtime().subscribe();
        dir.hold_writes();
        // Released also if an assertion fails: the runtime of the test
        // would otherwise wait for ever for the held write.
        let _released = Released(&dir);
        let runtime = a.runtime().clone();
        let id = a.id();
        let sending = tokio::spawn(async move { runtime.send_message(&id, &at_a, "x").await });
        common::held(&dir).await;
        let releasing = dir.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            releasing.release_writes();
        });
        // The shutdown waits for the send, which ends once its write is
        // done: its event is out when the shutdown returns.
        a.runtime().shutdown(Duration::from_secs(30)).await;
        let mut queued = false;
        while let Ok(event) = events.try_recv() {
            queued |= matches!(event, Event::MessageQueued(_));
        }
        assert!(queued);
        assert!(sending.await.unwrap().is_ok());
    });
}

#[test]
fn a_restarted_sender_delivers_what_it_had_queued() {
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let mut a = Peer::in_installation(installation, 1).await;
        let mut b = Peer::new(2).await;
        let at_a = a.add(&b).await;
        let at_b = b.add(&a).await;
        a.start(&network, options()).await;
        let sent = a
            .runtime()
            .send_message(&a.id(), &at_a, "kept")
            .await
            .unwrap();
        a.stop().await;
        let identity = a.identity.clone();
        drop(a);
        drop(identity);
        let (installation, _) =
            Installation::open(Box::new(dir.restart(CrashOutcome::ALL[0])), &passphrase()).unwrap();
        let identity = installation.identities().into_iter().next().unwrap();
        let mut a = Peer {
            seed: 1,
            installation,
            identity,
            runtime: None,
        };
        b.start(&network, options()).await;
        a.start(&network, options()).await;
        let mut events = a.runtime().subscribe();
        if a.statuses(&at_a) != vec![MessageStatus::Delivered] {
            // In real time: bounded, so that a fault fails it at once.
            let delivered = tokio::time::timeout(
                Duration::from_secs(60),
                event(&mut events, |event| {
                    matches!(event, Event::MessageDelivered(_))
                }),
            )
            .await
            .expect("the message was delivered");
            assert_eq!(delivered, Event::MessageDelivered(sent));
        }
        assert_eq!(b.statuses(&at_b), vec![MessageStatus::Received]);
        a.stop().await;
        b.stop().await;
    });
}

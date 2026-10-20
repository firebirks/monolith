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

use core::future::Future;
use core::pin::{Pin, pin};
use core::task::{Context, Poll, Waker};
use core::time::Duration;
use std::sync::Arc;

use common::{Released, fixed_endpoint, keys, run, run_paused};
use monolith_core::api::{
    CommandError, ContactId, DeferReason, Event, IdentityId, MessageStatus, SendError, ServiceState,
};
use monolith_core::budget::Budgets;
use monolith_core::identity::{Installation, LocalIdentity};
use monolith_core::link::{Link, dial};
use monolith_core::runtime::{Runtime, RuntimeOptions};
use monolith_protocol::body::{FileChunk, Message, MessageId, TransferId};
use monolith_protocol::card::ContactCard;
use monolith_protocol::limits::{
    CHAT_MESSAGE_RATE, CONTACT_SESSION_RATE, CONTROL_MESSAGE_RATE, DUPLICATE_PROBE_TIMEOUT,
    FILE_OFFER_RATE, HANDSHAKE_TIMEOUT, MAX_CHAT_TEXT_LEN, MESSAGE_ACK_TIMEOUT, PONG_TIMEOUT,
    PROFILE_UPDATE_RATE, RateLimit, SESSION_LINGER,
};
use monolith_protocol::session::Action;
use monolith_protocol::text::{ChatText, DisplayName, Filename, ProfileText};
use monolith_storage::dir::{CrashOutcome, MemoryDir};
use monolith_storage::vault::{KdfParams, Passphrase};
use monolith_tor::{
    Bootstrap, KeySource, MockNetwork, MockTorBackend, OnionService, OnionServiceSecret, TorBackend,
};
use tokio::io::{AsyncReadExt, DuplexStream};
use tokio::sync::broadcast;
use tokio::time::Instant;

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

    /// Starts the runtime and waits until its service is published: a
    /// minute of test time, far more than it takes, so that a service that
    /// is never published fails the test at once instead of after a day.
    async fn start(&mut self, network: &MockNetwork, options: RuntimeOptions) {
        self.runtime = Some(Runtime::start(
            self.installation.clone(),
            network.backend(),
            Budgets::new(),
            options,
        ));
        let endpoint = fixed_endpoint(self.seed);
        tokio::time::timeout(
            Duration::from_secs(60),
            until(|| network.is_published(&endpoint)),
        )
        .await
        .expect("the service was published");
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
fn a_session_the_scheduler_waited_on_announces_the_retry_when_it_ends() {
    // A queues while B is away, and its scheduler waits to dial again. B
    // dials in, takes the message and never acknowledges it: the session
    // ends for want of an acknowledgement while the scheduler runs, and the
    // retry that follows is announced (P7-23).
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = pair(&network).await;
        a.start(&network, options()).await;
        let mut events = a.runtime().subscribe();
        a.runtime()
            .send_message(&a.id(), &at_a, "while away")
            .await
            .unwrap();
        event(&mut events, |event| {
            matches!(event, Event::DeliveryDeferred { .. })
        })
        .await;
        b.start(
            &network,
            RuntimeOptions {
                acknowledgement_delay: MESSAGE_ACK_TIMEOUT * 10,
                ..options()
            },
        )
        .await;
        let dialed_in = tokio::time::Instant::now();
        b.runtime()
            .send_message(&b.id(), &at_b, "dialing in")
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
        // At the end of that session, not after a dial of A's own and the
        // wait for its acknowledgement.
        assert!(
            dialed_in.elapsed() < MESSAGE_ACK_TIMEOUT * 2,
            "{:?}",
            dialed_in.elapsed()
        );
        assert_eq!(
            a.statuses(&at_a),
            vec![MessageStatus::Queued, MessageStatus::Received]
        );
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
            let mut silent = network
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
            let mut stream = silent.accept().await.unwrap();
            if block {
                a.runtime().block_contact(&a.id(), &at_a).await.unwrap();
            } else {
                a.runtime().delete_contact(&a.id(), &at_a).await.unwrap();
            }
            // The attempt is dropped at once, not when its handshake times
            // out: the stream it opened closes.
            let closed = tokio::time::timeout(HANDSHAKE_TIMEOUT / 4, async {
                let mut buffer = [0_u8; 1024];
                while stream.read(&mut buffer).await.is_ok_and(|read| read > 0) {}
            })
            .await;
            assert!(closed.is_ok(), "{block}");
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

/// The cards A dials for B, in order.
fn plan(a: &Peer, b: &Peer) -> Vec<ContactCard> {
    a.identity
        .dial_plan(b.identity.identity())
        .map(|plan| plan.cards)
        .unwrap_or_default()
}

/// A and B are accepted contacts. B has begun a rotation of its transport
/// key and announced the successor to A on a session that has ended
/// since: A's dial plan of B holds the successor, then the active key.
/// B goes on answering with its old key until it switches. Both run.
async fn announced_successor(network: &MockNetwork) -> (Peer, Peer, ContactId, ContactId) {
    let (mut a, mut b, at_a, at_b) = pair(network).await;
    b.identity.begin_rotation().await.unwrap();
    a.start(network, options()).await;
    b.start(network, options()).await;
    a.runtime()
        .send_message(&a.id(), &at_a, "before")
        .await
        .unwrap();
    until(|| a.statuses(&at_a) == vec![MessageStatus::Delivered]).await;
    until(|| plan(&a, &b).len() == 2).await;
    assert_eq!(plan(&a, &b)[1], b.identity.card());
    until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 0).await;
    (a, b, at_a, at_b)
}

/// Fails the test on a `DeliveryDeferred` among the events received.
fn no_deferral(events: &mut broadcast::Receiver<Event>) {
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(event, Event::DeliveryDeferred { .. }),
            "{event:?}"
        );
    }
}

#[test]
fn a_successor_that_does_not_answer_is_followed_by_the_active_key() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = announced_successor(&network).await;
        let mut events = a.runtime().subscribe();
        let dials = network.dials();
        a.runtime()
            .send_message(&a.id(), &at_a, "after")
            .await
            .unwrap();
        until(|| a.statuses(&at_a) == vec![MessageStatus::Delivered; 2]).await;
        // One attempt: the successor, which B does not answer with yet,
        // then the active key. The first is not reported.
        assert_eq!(network.dials(), dials + 2);
        no_deferral(&mut events);
        assert_eq!(a.runtime().sessions(&a.id(), &at_a).unwrap(), 1);
        assert_eq!(b.statuses(&at_b), vec![MessageStatus::Received; 2]);
        // B has not taken the successor into use: it stays announced.
        assert_eq!(plan(&a, &b).len(), 2);
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_successor_in_use_is_dialed_first_and_promoted() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, _) = announced_successor(&network).await;
        let successor = plan(&a, &b)[0].clone();
        let rotation = b.identity.rotation_id().unwrap();
        assert!(b.identity.switch_rotation(rotation, true).await.unwrap());
        let dials = network.dials();
        a.runtime()
            .send_message(&a.id(), &at_a, "after")
            .await
            .unwrap();
        until(|| a.statuses(&at_a) == vec![MessageStatus::Delivered; 2]).await;
        // The successor answered: one dial, and it is the active key now.
        assert_eq!(network.dials(), dials + 1);
        assert_eq!(plan(&a, &b), vec![successor]);
        assert_eq!(a.runtime().sessions(&a.id(), &at_a).unwrap(), 1);
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_failure_of_tor_is_not_followed_by_the_active_key() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, _) = announced_successor(&network).await;
        let mut events = a.runtime().subscribe();
        // Tor counts the dial and reaches no service: the same for every
        // card, which all state the same endpoints.
        network.set_readiness(false, Bootstrap::InProgress(50));
        let dials = network.dials();
        a.runtime()
            .send_message(&a.id(), &at_a, "after")
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
        assert_eq!(network.dials(), dials + 1);
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_plan_whose_cards_both_fail_is_reported_once() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, _) = announced_successor(&network).await;
        // B is gone; its service takes every stream and drops it, so
        // neither card answers.
        b.stop().await;
        let mut service = network
            .backend()
            .publish_onion(KeySource::Existing {
                secret: OnionServiceSecret::from_bytes(&[2; 64]),
                expected: fixed_endpoint(2),
            })
            .await
            .unwrap();
        let dropping = tokio::spawn(async move {
            while let Ok(stream) = service.accept().await {
                drop(stream);
            }
        });
        let mut events = a.runtime().subscribe();
        let dials = network.dials();
        a.runtime()
            .send_message(&a.id(), &at_a, "after")
            .await
            .unwrap();
        event(&mut events, |event| {
            matches!(
                event,
                Event::DeliveryDeferred {
                    reason: DeferReason::Handshake,
                    ..
                }
            )
        })
        .await;
        // Both cards in one attempt, one report for it.
        assert_eq!(network.dials(), dials + 2);
        no_deferral(&mut events);
        assert_eq!(
            a.statuses(&at_a),
            vec![MessageStatus::Delivered, MessageStatus::Queued]
        );
        dropping.abort();
        a.stop().await;
    });
}

/// What ends an attempt while its first dial, of the successor, is held.
#[derive(Clone, Copy, Debug)]
enum Ending {
    Block,
    DeleteAndAddAgain,
    DeleteIdentity,
}

#[test]
fn an_attempt_that_ends_between_its_cards_dials_no_further() {
    for ending in [
        Ending::Block,
        Ending::DeleteAndAddAgain,
        Ending::DeleteIdentity,
    ] {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut a, mut b, at_a, _) = announced_successor(&network).await;
            let mut events = a.runtime().subscribe();
            let dials = network.dials();
            let hold = network.hold_dial(dials + 1);
            a.runtime()
                .send_message(&a.id(), &at_a, "after")
                .await
                .unwrap();
            until(|| network.dials() == dials + 1).await;
            let again = match ending {
                Ending::Block => {
                    a.runtime().block_contact(&a.id(), &at_a).await.unwrap();
                    None
                }
                Ending::DeleteAndAddAgain => {
                    a.runtime().delete_contact(&a.id(), &at_a).await.unwrap();
                    Some(a.add(&b).await)
                }
                Ending::DeleteIdentity => {
                    a.runtime().delete_identity(&a.id()).await.unwrap();
                    None
                }
            };
            hold.release();
            tokio::time::sleep(HANDSHAKE_TIMEOUT * 10).await;
            // The successor did not answer, and the active key is not
            // dialed for a record or an identity that is gone: no stream,
            // no session, nothing reported.
            assert_eq!(network.dials(), dials + 1, "{ending:?}");
            no_deferral(&mut events);
            if let Some(again) = again {
                assert_ne!(again, at_a);
                assert_eq!(a.runtime().sessions(&a.id(), &again).unwrap(), 0);
            }
            a.stop().await;
            b.stop().await;
        });
    }
}

#[test]
fn a_connect_once_the_runtime_shuts_down_dials_no_further() {
    // The successor does not answer after the shutdown began: the active
    // key is not dialed (P7-23).
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, _) = announced_successor(&network).await;
        let dials = network.dials();
        let hold = network.hold_dial(dials + 1);
        a.runtime().connect(&a.id(), &at_a).unwrap();
        until(|| network.dials() == dials + 1).await;
        let runtime = a.runtime().clone();
        let stopping = tokio::spawn(async move { runtime.shutdown(HANDSHAKE_TIMEOUT * 20).await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        hold.release();
        tokio::time::sleep(HANDSHAKE_TIMEOUT * 10).await;
        assert_eq!(network.dials(), dials + 1);
        stopping.await.unwrap();
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_connect_for_a_record_made_again_between_its_cards_dials_no_further() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, _) = announced_successor(&network).await;
        let dials = network.dials();
        let hold = network.hold_dial(dials + 1);
        a.runtime().connect(&a.id(), &at_a).unwrap();
        until(|| network.dials() == dials + 1).await;
        a.runtime().delete_contact(&a.id(), &at_a).await.unwrap();
        let again = a.add(&b).await;
        hold.release();
        tokio::time::sleep(HANDSHAKE_TIMEOUT * 10).await;
        // The connect was for the record that is gone: the active key is
        // not dialed for the one made again.
        assert_eq!(network.dials(), dials + 1);
        assert_eq!(a.runtime().sessions(&a.id(), &again).unwrap(), 0);
        a.stop().await;
        b.stop().await;
    });
}

/// A and B are accepted contacts. A runs; B's runtime is stopped, and B
/// dials A by hand: the link of B, on which the test sends what it wants
/// and reads what A answers.
async fn dialed_by_hand(network: &MockNetwork) -> (Peer, Peer, ContactId, Link<DuplexStream>) {
    let (mut a, mut b, at_a, _) = pair(network).await;
    a.start(network, options()).await;
    b.start(network, options()).await;
    a.runtime()
        .send_message(&a.id(), &at_a, "before")
        .await
        .unwrap();
    until(|| a.statuses(&at_a) == vec![MessageStatus::Delivered]).await;
    until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 0).await;
    b.stop().await;
    let link = hand_link(network, &a, &b).await;
    (a, b, at_a, link)
}

/// A session from B to A, which B drives by hand.
async fn hand_link(network: &MockNetwork, a: &Peer, b: &Peer) -> Link<DuplexStream> {
    let established = dial(
        &network.backend(),
        &Budgets::new(),
        &b.identity,
        &a.identity.card(),
    )
    .await
    .unwrap();
    let mut link = established.link;
    // What a session of two accepted contacts begins with.
    for action in established.first {
        assert_eq!(action, Action::SendContactAccept);
        link.send(&Message::ContactAccept).await.unwrap();
    }
    // A's accept confirms the session at B.
    loop {
        let arrived = link.receive().await.unwrap();
        if arrived.received().actions.contains(&Action::Confirmed) {
            return link;
        }
    }
}

/// Sends a Ping on `link` and returns the messages that arrive before its
/// Pong, which ends them: what A sent in answer to what came before.
async fn before_pong(link: &mut Link<DuplexStream>, nonce: u8) -> Vec<Message> {
    link.send(&Message::Ping([nonce; 8])).await.unwrap();
    let mut before = Vec::new();
    loop {
        let arrived = link.receive().await.unwrap();
        match &arrived.received().message {
            Message::Pong(answer) if *answer == [nonce; 8] => return before,
            other => before.push(other.clone()),
        }
    }
}

fn transfer(byte: u8) -> TransferId {
    TransferId::from_bytes([byte; 16])
}

fn offer(byte: u8) -> Message {
    Message::FileOffer {
        transfer: transfer(byte),
        size: 100,
        filename: Filename::new("photo.jpg").unwrap(),
    }
}

/// Every file message but an offer, for the transfer `byte`.
fn file_messages(byte: u8) -> Vec<Message> {
    vec![
        Message::FileAccept(transfer(byte)),
        Message::FileReject(transfer(byte)),
        Message::FileChunk(FileChunk::new(transfer(byte), vec![7; 10]).unwrap()),
        Message::FileComplete {
            transfer: transfer(byte),
            digest: [0; 32],
        },
        Message::FileAbort(transfer(byte)),
    ]
}

#[test]
fn a_file_offer_is_rejected_at_once_and_what_crosses_it_is_discarded() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, mut link) = dialed_by_hand(&network).await;
        link.send(&offer(1)).await.unwrap();
        assert_eq!(
            before_pong(&mut link, 1).await,
            vec![Message::FileReject(transfer(1))]
        );
        // The same offer again, and an abort that crossed the rejection:
        // discarded, the session goes on, nothing is answered.
        link.send(&offer(1)).await.unwrap();
        link.send(&Message::FileAbort(transfer(1))).await.unwrap();
        assert_eq!(before_pong(&mut link, 2).await, Vec::<Message>::new());
        // A profile is taken and dropped.
        link.send(&Message::Profile {
            display_name: DisplayName::new("Sam").unwrap(),
            profile_text: ProfileText::new("Hello").unwrap(),
        })
        .await
        .unwrap();
        assert_eq!(before_pong(&mut link, 3).await, Vec::<Message>::new());
        // Nothing of it reached the conversation.
        assert_eq!(a.statuses(&at_a), vec![MessageStatus::Delivered]);
        a.stop().await;
    });
}

#[test]
fn what_cannot_follow_a_rejected_offer_ends_the_session() {
    // A chunk or a completion of an offer never accepted, or an answer from
    // the side that made it (P7-23).
    for message in file_messages(1) {
        if matches!(message, Message::FileAbort(_)) {
            continue;
        }
        run_paused(async {
            let network = MockNetwork::new();
            let (mut a, _b, at_a, mut link) = dialed_by_hand(&network).await;
            link.send(&offer(1)).await.unwrap();
            assert_eq!(
                before_pong(&mut link, 1).await,
                vec![Message::FileReject(transfer(1))]
            );
            link.send(&message).await.unwrap();
            assert!(link.receive().await.is_err(), "{message:?}");
            assert_eq!(a.statuses(&at_a), vec![MessageStatus::Delivered]);
            a.stop().await;
        });
    }
}

#[test]
fn a_file_message_for_a_transfer_never_offered_ends_the_session() {
    for message in file_messages(9) {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut a, b, at_a, mut link) = dialed_by_hand(&network).await;
            assert_eq!(before_pong(&mut link, 1).await, Vec::<Message>::new());
            link.send(&message).await.unwrap();
            // A ends the session without a Close: the link ends.
            assert!(link.receive().await.is_err(), "{message:?}");
            // Nothing else changed: the contact and the conversation stay.
            assert_eq!(a.statuses(&at_a), vec![MessageStatus::Delivered]);
            assert!(
                a.runtime()
                    .contacts(&a.id())
                    .unwrap()
                    .iter()
                    .any(|contact| contact.contact == at_a)
            );
            // And another session goes on as before.
            let mut again = hand_link(&network, &a, &b).await;
            assert_eq!(before_pong(&mut again, 2).await, Vec::<Message>::new());
            a.stop().await;
        });
    }
}

#[test]
fn what_one_session_rejected_is_unknown_to_the_next() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, b, _, mut link) = dialed_by_hand(&network).await;
        link.send(&offer(5)).await.unwrap();
        assert_eq!(
            before_pong(&mut link, 1).await,
            vec![Message::FileReject(transfer(5))]
        );
        link.close().await.unwrap();
        // A later session that names it, a replay of the first: a
        // violation.
        let mut later = hand_link(&network, &a, &b).await;
        later.send(&Message::FileAbort(transfer(5))).await.unwrap();
        assert!(later.receive().await.is_err());
        a.stop().await;
    });
}

/// Like [`before_pong`], but `None` if the session ended instead.
async fn answered(link: &mut Link<DuplexStream>, nonce: u8) -> Option<Vec<Message>> {
    link.send(&Message::Ping([nonce; 8])).await.ok()?;
    let mut before = Vec::new();
    loop {
        let arrived = link.receive().await.ok()?;
        match &arrived.received().message {
            Message::Pong(answer) if *answer == [nonce; 8] => return Some(before),
            other => before.push(other.clone()),
        }
    }
}

fn chat(byte: u8) -> Message {
    Message::ChatMessage {
        id: MessageId::from_bytes([byte; 16]),
        text: ChatText::new("hi").unwrap(),
    }
}

#[test]
fn a_contact_keeps_no_idle_session_open_with_messages_that_carry_nothing() {
    // A message A holds already, or an acknowledgement of nothing A sent,
    // every little while: neither is activity, and the session lingers
    // from the last message taken in, and closes.
    let interval = SESSION_LINGER * 2 / 5;
    for repeated in [chat(9), Message::MessageAck(MessageId::from_bytes([7; 16]))] {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut a, _b, _, mut link) = dialed_by_hand(&network).await;
            link.send(&chat(9)).await.unwrap();
            assert_eq!(
                answered(&mut link, 1).await,
                Some(vec![Message::MessageAck(MessageId::from_bytes([9; 16]))])
            );
            let taken = Instant::now();
            let mut nonce = 2;
            let ended = loop {
                tokio::time::sleep(interval).await;
                if link.send(&repeated).await.is_err() || answered(&mut link, nonce).await.is_none()
                {
                    break taken.elapsed();
                }
                nonce += 1;
                assert!(taken.elapsed() < SESSION_LINGER * 4, "{repeated:?}");
            };
            assert!(ended >= SESSION_LINGER, "{ended:?}");
            assert!(ended <= SESSION_LINGER + interval, "{ended:?}");
            a.stop().await;
        });
    }
}

/// A and B, accepted contacts, both running; B's identity key is the
/// smaller, so a session B initiated is the preferred one
/// (`PROTOCOL.md` section 14).
async fn b_preferred(
    network: &MockNetwork,
    a_options: RuntimeOptions,
) -> (Peer, Peer, ContactId, ContactId) {
    let (mut a, mut b) = (Peer::new(1).await, Peer::new(2).await);
    if a.identity.identity() < b.identity.identity() {
        core::mem::swap(&mut a, &mut b);
    }
    let at_a = a.add(&b).await;
    let at_b = b.add(&a).await;
    a.start(network, a_options).await;
    b.start(network, options()).await;
    a.runtime()
        .send_message(&a.id(), &at_a, "before")
        .await
        .unwrap();
    until(|| a.statuses(&at_a) == vec![MessageStatus::Delivered]).await;
    until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 0).await;
    until(|| b.runtime().sessions(&b.id(), &at_b).unwrap() == 0).await;
    (a, b, at_a, at_b)
}

/// B dials A by hand and A registers that session; then a session A
/// dialed, which B's runtime answers, registers after it. The one of B is
/// the preferred one, and older: A probes it. Returns its link and the
/// nonce of the Ping.
async fn probed(
    network: &MockNetwork,
    a: &Peer,
    b: &Peer,
    at_a: ContactId,
) -> (Link<DuplexStream>, [u8; 8]) {
    let dials = network.dials();
    let hold = network.hold_dial(dials + 1);
    a.runtime().connect(&a.id(), &at_a).unwrap();
    until(|| network.dials() == dials + 1).await;
    let mut link = hand_link(network, a, b).await;
    until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 1).await;
    hold.release();
    until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 2).await;
    let arrived = link.receive().await.unwrap();
    let Message::Ping(nonce) = arrived.received().message else {
        panic!("{:?}", arrived.received().message);
    };
    (link, nonce)
}

#[test]
fn a_probed_session_that_answers_while_it_is_not_read_is_kept() {
    // A takes the message B sends before its Pong, and waits a long time
    // before it acknowledges it: the time the session is not read does not
    // count against the probe (BL4).
    run_paused(async {
        let network = MockNetwork::new();
        let slow = RuntimeOptions {
            acknowledgement_delay: DUPLICATE_PROBE_TIMEOUT * 3,
            ..options()
        };
        let (mut a, mut b, at_a, at_b) = b_preferred(&network, slow).await;
        let (mut link, nonce) = probed(&network, &a, &b, at_a).await;
        link.send(&chat(4)).await.unwrap();
        link.send(&Message::Pong(nonce)).await.unwrap();
        // The session of B stays, the newer one is closed.
        assert_eq!(
            answered(&mut link, 1).await,
            Some(vec![Message::MessageAck(MessageId::from_bytes([4; 16]))])
        );
        until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 1).await;
        until(|| b.runtime().sessions(&b.id(), &at_b).unwrap() == 0).await;
        assert_eq!(answered(&mut link, 2).await, Some(Vec::new()));
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_pong_after_the_probe_was_decided_keeps_the_session() {
    // The newer session ends while the probe runs, and the probe is not
    // answered in time: the older one stays, the only one. Its Pong comes
    // late; it answers a Ping A sent, and is no violation (BL30).
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = b_preferred(&network, options()).await;
        let (mut link, nonce) = probed(&network, &a, &b, at_a).await;
        b.runtime().close_sessions(&b.id(), &at_b).unwrap();
        until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 1).await;
        tokio::time::sleep(DUPLICATE_PROBE_TIMEOUT * 2).await;
        link.send(&Message::Pong(nonce)).await.unwrap();
        assert_eq!(answered(&mut link, 1).await, Some(Vec::new()));
        // A second one answers nothing.
        link.send(&Message::Pong(nonce)).await.unwrap();
        assert_eq!(answered(&mut link, 2).await, None);
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_ping_not_answered_within_its_timeout_ends_the_session() {
    // The probe is not answered and the session stays, the only one; B
    // keeps it in use, and never answers the Ping. A ends it once the Ping
    // waited `PONG_TIMEOUT` (`PROTOCOL.md` section 8.2).
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = b_preferred(&network, options()).await;
        let (mut link, _) = probed(&network, &a, &b, at_a).await;
        let pinged = Instant::now();
        b.runtime().close_sessions(&b.id(), &at_b).unwrap();
        until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 1).await;
        let interval = PONG_TIMEOUT / 6;
        let mut byte = 10;
        let ended = loop {
            if link.send(&chat(byte)).await.is_err() || answered(&mut link, byte).await.is_none() {
                break pinged.elapsed();
            }
            byte += 1;
            assert!(pinged.elapsed() < PONG_TIMEOUT * 4, "the session was kept");
            tokio::time::sleep(interval).await;
        };
        assert!(ended >= PONG_TIMEOUT, "{ended:?}");
        assert!(ended <= PONG_TIMEOUT + interval, "{ended:?}");
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_probed_session_that_does_not_answer_loses_to_the_newer_one() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = b_preferred(&network, options()).await;
        let (mut link, _) = probed(&network, &a, &b, at_a).await;
        tokio::time::sleep(DUPLICATE_PROBE_TIMEOUT * 2).await;
        assert_eq!(answered(&mut link, 1).await, None);
        assert_eq!(a.runtime().sessions(&a.id(), &at_a).unwrap(), 1);
        assert_eq!(b.runtime().sessions(&b.id(), &at_b).unwrap(), 1);
        // The newer one carries the queue now.
        a.runtime()
            .send_message(&a.id(), &at_a, "after")
            .await
            .unwrap();
        until(|| a.statuses(&at_a) == vec![MessageStatus::Delivered; 2]).await;
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_probe_asked_while_a_ping_is_outstanding_waits_for_its_pong() {
    // The first probe is not answered in time, and the session stays, the
    // only one. A newer session makes A probe it again: no second Ping is
    // sent, and the late Pong of the first answers the second probe.
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = b_preferred(&network, options()).await;
        let dials = network.dials();
        let first = network.hold_dial(dials + 1);
        let second = network.hold_dial(dials + 2);
        a.runtime().connect(&a.id(), &at_a).unwrap();
        a.runtime().connect(&a.id(), &at_a).unwrap();
        until(|| network.dials() == dials + 2).await;
        let mut link = hand_link(&network, &a, &b).await;
        until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 1).await;
        first.release();
        until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 2).await;
        let arrived = link.receive().await.unwrap();
        let Message::Ping(nonce) = arrived.received().message else {
            panic!("{:?}", arrived.received().message);
        };
        b.runtime().close_sessions(&b.id(), &at_b).unwrap();
        until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 1).await;
        tokio::time::sleep(DUPLICATE_PROBE_TIMEOUT * 2).await;
        second.release();
        until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 2).await;
        link.send(&Message::Pong(nonce)).await.unwrap();
        assert_eq!(answered(&mut link, 1).await, Some(Vec::new()));
        until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 1).await;
        until(|| b.runtime().sessions(&b.id(), &at_b).unwrap() == 0).await;
        assert_eq!(answered(&mut link, 2).await, Some(Vec::new()));
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_session_that_ends_with_its_record_announces_no_retry() {
    // B takes A's message in and waits before it acknowledges it; the
    // record ends while the session runs. The message stays queued for a
    // record that is gone: no retry is announced, since none comes (BL29).
    for ending in [
        Ending::Block,
        Ending::DeleteAndAddAgain,
        Ending::DeleteIdentity,
    ] {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut a, mut b, at_a, at_b) = pair(&network).await;
            a.start(&network, options()).await;
            let slow = RuntimeOptions {
                acknowledgement_delay: MESSAGE_ACK_TIMEOUT / 2,
                ..options()
            };
            b.start(&network, slow).await;
            a.runtime()
                .send_message(&a.id(), &at_a, "unanswered")
                .await
                .unwrap();
            until(|| b.statuses(&at_b) == vec![MessageStatus::Received]).await;
            let mut events = a.runtime().subscribe();
            match ending {
                Ending::Block => {
                    a.runtime().block_contact(&a.id(), &at_a).await.unwrap();
                }
                Ending::DeleteAndAddAgain => {
                    a.runtime().delete_contact(&a.id(), &at_a).await.unwrap();
                    a.add(&b).await;
                }
                Ending::DeleteIdentity => {
                    a.runtime().delete_identity(&a.id()).await.unwrap();
                }
            }
            tokio::time::sleep(MESSAGE_ACK_TIMEOUT * 4).await;
            no_deferral(&mut events);
            a.stop().await;
            b.stop().await;
        });
    }
}

/// Polls `future` once, letting no other task run.
fn poll_now<F: Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

#[test]
fn a_block_for_a_record_made_again_before_it_runs_ends_nothing() {
    // The command for the record names its contact once its work runs, not
    // before: the record deleted and made again in between, it finds the
    // record gone and ends nothing of the new one (BL28).
    for block in [true, false] {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut a, b, at_a, _) = pair(&network).await;
            a.start(&network, options()).await;
            let runtime = a.runtime().clone();
            let id = a.id();
            let mut ending = pin!(async move {
                if block {
                    runtime.block_contact(&id, &at_a).await
                } else {
                    runtime.delete_contact(&id, &at_a).await
                }
            });
            assert!(poll_now(ending.as_mut()).is_pending());
            let peer = *b.identity.identity();
            assert!(matches!(
                poll_now(pin!(a.identity.delete(&peer))),
                Poll::Ready(Ok(()))
            ));
            assert!(matches!(
                poll_now(pin!(a.identity.import(&b.identity.card()))),
                Poll::Ready(Ok(_))
            ));
            let again = a.identity.contact_id(&peer).unwrap();
            assert_ne!(again, at_a);
            assert_eq!(ending.await, Err(CommandError::UnknownContact));
            assert_eq!(a.identity.current_record(&again), Some(peer));
            assert!(a.identity.blocked().is_empty());
            a.stop().await;
        });
    }
}

#[test]
fn a_send_for_a_record_deleted_while_it_waits_for_the_vault_queues_nothing() {
    // In real time: writes are held on a thread of their own. The record
    // is looked at again once the vault is durable (BL28).
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let mut a = Peer::in_installation(installation, 1).await;
        let b = Peer::new(2).await;
        let at_a = a.add(&b).await;
        a.start(&network, options()).await;
        // A change of the vault waits for its write.
        dir.hold_writes();
        let released = Released(&dir);
        let identity = a.identity.clone();
        let peer = *b.identity.identity();
        let verifying = tokio::spawn(async move { identity.set_verified(&peer, true).await });
        common::held(&dir).await;
        // A send waits for the vault, and the record is deleted.
        let runtime = a.runtime().clone();
        let id = a.id();
        let sending = tokio::spawn(async move { runtime.send_message(&id, &at_a, "x").await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let identity = a.identity.clone();
        let deleting = tokio::spawn(async move { identity.delete(&peer).await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(released);
        assert_eq!(sending.await.unwrap(), Err(SendError::NotAContact));
        deleting.await.unwrap().unwrap();
        verifying.await.unwrap().unwrap();
        // Nothing was queued for the record that is gone.
        assert!(a.statuses(&at_a).is_empty());
        a.stop().await;
    });
}

#[test]
fn a_dial_that_fails_once_the_vault_failed_announces_no_retry() {
    run_paused(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let mut a = Peer::in_installation(installation, 1).await;
        let b = Peer::new(2).await;
        let at_a = a.add(&b).await;
        a.start(&network, options()).await;
        // B is away. The dial is held while a change of the vault fails.
        let dials = network.dials();
        let hold = network.hold_dial(dials + 1);
        a.runtime().send_message(&a.id(), &at_a, "x").await.unwrap();
        until(|| network.dials() == dials + 1).await;
        let mut events = a.runtime().subscribe();
        dir.crash_at(dir.steps() + 1);
        assert!(
            a.identity
                .set_verified(b.identity.identity(), true)
                .await
                .is_err()
        );
        event(&mut events, |event| matches!(event, Event::StorageFailed)).await;
        hold.release();
        tokio::time::sleep(HANDSHAKE_TIMEOUT * 10).await;
        // The dial failed, and no attempt follows: none is announced.
        no_deferral(&mut events);
        assert_eq!(network.dials(), dials + 1);
        a.stop().await;
    });
}

/// A, with two accepted contacts B and C that dial it by hand, as
/// [`dialed_by_hand`] makes B: their links.
async fn two_by_hand(
    network: &MockNetwork,
) -> (Peer, Peer, Peer, Link<DuplexStream>, Link<DuplexStream>) {
    let (mut a, mut b, at_a, _) = pair(network).await;
    let mut c = Peer::new(3).await;
    let c_at_a = a.add(&c).await;
    c.add(&a).await;
    a.start(network, options()).await;
    b.start(network, options()).await;
    c.start(network, options()).await;
    for contact in [at_a, c_at_a] {
        a.runtime()
            .send_message(&a.id(), &contact, "before")
            .await
            .unwrap();
    }
    for contact in [at_a, c_at_a] {
        until(|| a.statuses(&contact) == vec![MessageStatus::Delivered]).await;
        until(|| a.runtime().sessions(&a.id(), &contact).unwrap() == 0).await;
    }
    b.stop().await;
    c.stop().await;
    let b_link = hand_link(network, &a, &b).await;
    let c_link = hand_link(network, &a, &c).await;
    (a, b, c, b_link, c_link)
}

/// The time one token of `limit` takes to come back.
fn interval(limit: RateLimit) -> Duration {
    Duration::from_secs(60) / limit.per_minute
}

fn profile() -> Message {
    Message::Profile {
        display_name: DisplayName::new("Sam").unwrap(),
        profile_text: ProfileText::new("Hello").unwrap(),
    }
}

/// Sends `messages` on `link`, then a Ping, and reads until its Pong: how
/// long A took to handle them, and what it answered.
async fn flood(link: &mut Link<DuplexStream>, messages: &[Message]) -> (Duration, Vec<Message>) {
    let start = Instant::now();
    for message in messages {
        link.send(message).await.unwrap();
    }
    let answers = before_pong(link, 255).await;
    (start.elapsed(), answers)
}

/// A rate, and how to make the n-th message it counts.
type Counted = (RateLimit, fn(u8) -> Message);

#[test]
fn a_contact_above_a_rate_is_slowed_and_no_other_contact_is() {
    // For each rate of what a contact sends: its burst at once, then one
    // per interval. Another contact sending the same meanwhile is not
    // slowed (P7-8).
    let cases: [Counted; 4] = [
        (CHAT_MESSAGE_RATE, chat),
        (CONTROL_MESSAGE_RATE, |n| Message::Ping([n; 8])),
        (FILE_OFFER_RATE, offer),
        (PROFILE_UPDATE_RATE, |_| profile()),
    ];
    for (limit, make) in cases {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut a, _b, _c, mut link, mut other) = two_by_hand(&network).await;
            let over = 3;
            let messages: Vec<Message> = (0..limit.burst + over)
                .map(|n| make(u8::try_from(n).unwrap()))
                .collect();
            let flooding = tokio::spawn(async move { flood(&mut link, &messages).await });
            let (took, _) = flood(&mut other, &[make(250)]).await;
            assert!(took < interval(limit), "{limit:?} {took:?}");
            let (took, answers) = flooding.await.unwrap();
            // The Ping that ends the flood is a control message too.
            let waited = if limit == CONTROL_MESSAGE_RATE {
                over + 1
            } else {
                over
            };
            assert!(took >= interval(limit) * waited, "{limit:?} {took:?}");
            assert!(took < interval(limit) * (waited + 1), "{limit:?} {took:?}");
            let answered = if limit == PROFILE_UPDATE_RATE {
                0
            } else {
                limit.burst + over
            };
            assert_eq!(
                answers.len(),
                usize::try_from(answered).unwrap(),
                "{limit:?}"
            );
            a.stop().await;
        });
    }
}

#[test]
fn the_rate_of_a_contact_holds_across_its_sessions() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, b, _, mut link) = dialed_by_hand(&network).await;
        let burst: Vec<Message> = (0..CHAT_MESSAGE_RATE.burst)
            .map(|n| chat(u8::try_from(n).unwrap()))
            .collect();
        let (took, _) = flood(&mut link, &burst).await;
        assert!(took < interval(CHAT_MESSAGE_RATE));
        link.close().await.unwrap();
        // A new session of B: what the first took is not given back.
        let mut again = hand_link(&network, &a, &b).await;
        let (took, answers) = flood(&mut again, &[chat(200)]).await;
        assert!(took >= interval(CHAT_MESSAGE_RATE), "{took:?}");
        assert_eq!(
            answers,
            vec![Message::MessageAck(MessageId::from_bytes([200; 16]))]
        );
        a.stop().await;
    });
}

#[test]
fn a_contact_that_opens_sessions_above_its_rate_has_them_closed() {
    run_paused(async {
        let network = MockNetwork::new();
        // `dialed_by_hand` opens one; the rest of the burst follows.
        let (mut a, b, _, link) = dialed_by_hand(&network).await;
        drop(link);
        for _ in 1..CONTACT_SESSION_RATE.burst {
            let mut link = hand_link(&network, &a, &b).await;
            assert_eq!(answered(&mut link, 1).await, Some(Vec::new()));
        }
        // The next one is closed once it is admitted: never confirmed.
        let established = dial(
            &network.backend(),
            &Budgets::new(),
            &b.identity,
            &a.identity.card(),
        )
        .await
        .unwrap();
        let mut link = established.link;
        for action in established.first {
            assert_eq!(action, Action::SendContactAccept);
            link.send(&Message::ContactAccept).await.unwrap();
        }
        let mut confirmed = false;
        while let Ok(arrived) = link.receive().await {
            confirmed |= arrived.received().actions.contains(&Action::Confirmed);
        }
        assert!(!confirmed);
        // One interval later, one more is taken.
        tokio::time::sleep(interval(CONTACT_SESSION_RATE)).await;
        let mut link = hand_link(&network, &a, &b).await;
        assert_eq!(answered(&mut link, 2).await, Some(Vec::new()));
        a.stop().await;
    });
}

#[test]
fn a_session_that_waits_for_a_token_ends_when_it_is_closed() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, mut link) = dialed_by_hand(&network).await;
        for n in 0..=FILE_OFFER_RATE.burst {
            link.send(&offer(u8::try_from(n).unwrap())).await.unwrap();
        }
        // The last offer waits for its token; the session is closed
        // meanwhile, and ends at once.
        tokio::time::sleep(interval(FILE_OFFER_RATE) / 10).await;
        let start = Instant::now();
        a.runtime().close_sessions(&a.id(), &at_a).unwrap();
        while link.receive().await.is_ok() {}
        assert!(start.elapsed() < interval(FILE_OFFER_RATE) / 2);
        a.stop().await;
    });
}

#[test]
fn an_identity_without_a_key_is_reported_so_and_stays_so() {
    // Its service cannot be published: the runtime says so, and that
    // stays what it says (BL31).
    run_paused(async {
        let network = MockNetwork::new();
        let installation = Installation::ephemeral();
        let mut held = keys(5, 5, 1, fixed_endpoint(5));
        held.onion = None;
        let identity = installation.restore_identity(held, None).await.unwrap();
        let runtime = Runtime::start(installation, network.backend(), Budgets::new(), options());
        let mut events = runtime.subscribe();
        let id = identity.id();
        event(&mut events, |event| {
            matches!(
                event,
                Event::Service {
                    state: ServiceState::NoKey,
                    ..
                }
            )
        })
        .await;
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert_eq!(runtime.service_state(&id), Ok(ServiceState::NoKey));
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, Event::Service { .. }), "{event:?}");
        }
        assert!(!network.is_published(&fixed_endpoint(5)));
        runtime.shutdown(Duration::ZERO).await;
    });
}

#[test]
fn a_send_once_the_runtime_shuts_down_says_so() {
    // Nothing was stored; it is neither a store that failed nor an
    // identity that is gone (BL27).
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, _) = pair(&network).await;
        a.start(&network, options()).await;
        let runtime = a.runtime().clone();
        a.stop().await;
        assert_eq!(
            runtime.send_message(&a.id(), &at_a, "late").await,
            Err(SendError::ShuttingDown)
        );
    });
}

#[test]
fn a_send_a_shutdown_cuts_short_says_so() {
    // In real time: writes are held on a thread of their own. The send
    // waits for the vault when the runtime shuts down (BL27).
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let mut a = Peer::in_installation(installation, 1).await;
        let b = Peer::new(2).await;
        let at_a = a.add(&b).await;
        a.start(&network, options()).await;
        dir.hold_writes();
        let released = Released(&dir);
        let identity = a.identity.clone();
        let peer = *b.identity.identity();
        let verifying = tokio::spawn(async move { identity.set_verified(&peer, true).await });
        common::held(&dir).await;
        let runtime = a.runtime().clone();
        let id = a.id();
        let sending = tokio::spawn(async move { runtime.send_message(&id, &at_a, "x").await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        a.stop().await;
        assert_eq!(sending.await.unwrap(), Err(SendError::ShuttingDown));
        drop(released);
        let _ = verifying.await.unwrap();
    });
}

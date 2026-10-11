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

use common::{fixed_endpoint, keys, run, run_paused};
use monolith_core::api::{ContactId, DeferReason, Event, IdentityId, MessageStatus, SendError};
use monolith_core::budget::Budgets;
use monolith_core::identity::{Installation, LocalIdentity};
use monolith_core::runtime::{Runtime, RuntimeOptions};
use monolith_protocol::limits::{MAX_CHAT_TEXT_LEN, MESSAGE_ACK_TIMEOUT, SESSION_LINGER};
use monolith_storage::dir::{CrashOutcome, MemoryDir};
use monolith_storage::vault::{KdfParams, Passphrase};
use monolith_tor::{MockNetwork, MockTorBackend};
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
            let delivered = event(&mut events, |event| {
                matches!(event, Event::MessageDelivered(_))
            })
            .await;
            assert_eq!(delivered, Event::MessageDelivered(sent));
        }
        assert_eq!(b.statuses(&at_b), vec![MessageStatus::Received]);
        a.stop().await;
        b.stop().await;
    });
}

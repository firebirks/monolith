//! The publication supervisor on the in-memory Tor: the service of an
//! identity is published from its key, published again under the same name
//! when Tor loses it, with growing delays while Tor cannot publish, and
//! removed at shutdown.

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
use std::time::Duration;

use common::{both, chat, run_paused, send_first, step};
use monolith_core::budget::Budgets;
use monolith_core::identity::{IdentityKeys, Installation, LocalIdentity};
use monolith_core::link::{answer, dial};
use monolith_core::supervisor::{Publication, supervise};
use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::session::Action;
use monolith_tor::{
    IsolationGroup, KeySource, MockNetwork, MockOnionService, MockTorBackend, OnionService,
    OnionServiceSecret, TorBackend, TorError, TorStatus,
};
use tokio::sync::watch;
use zeroize::Zeroizing;

fn endpoint(seed: u8) -> OnionServiceKey {
    OnionServiceKey::from_bytes(
        IdentitySecretKey::from_seed(&[seed.wrapping_add(200); 32])
            .public_key()
            .as_bytes(),
    )
    .unwrap()
}

/// An identity of `seed` whose service key, and its secret, it holds. Its
/// installation is kept for the rest of the test: an identity whose
/// installation is gone refuses every change.
async fn identity(seed: u8, onion: bool) -> Arc<LocalIdentity> {
    let installation = Box::leak(Box::new(Installation::ephemeral()));
    installation
        .restore_identity(
            IdentityKeys {
                seed: Zeroizing::new([seed; 32]),
                transport: Zeroizing::new([seed ^ 0xA5; 32]),
                onion: onion.then(|| Zeroizing::new([seed; 64])),
                epoch: EndpointEpoch::FIRST,
                endpoint: endpoint(seed),
            },
            None,
        )
        .await
        .unwrap()
}

/// Runs the supervisor of `local` on `backend` and answers every stream
/// with `local`'s store, until `shutdown`.
fn supervised(
    backend: MockTorBackend,
    local: Arc<LocalIdentity>,
    shutdown: watch::Receiver<bool>,
) -> (
    tokio::task::JoinHandle<Publication>,
    watch::Receiver<Publication>,
) {
    let (state, states) = watch::channel(Publication::Publishing);
    let task = tokio::spawn(async move {
        let budgets = Budgets::new();
        let answering = local.clone();
        supervise(
            &backend,
            &budgets,
            &local,
            shutdown,
            &state,
            move |stream, permit| {
                let local = answering.clone();
                async move {
                    let budgets = Budgets::new();
                    let _permit = permit;
                    if let Ok(mut established) = answer(stream, &budgets, &local).await {
                        send_first(&mut established, &local).await;
                        while let Ok(received) = step(&mut established.link, &local).await {
                            if let monolith_protocol::body::Message::ChatMessage { .. } =
                                received.message
                            {
                                let _ = established.link.send(&chat("hello back")).await;
                            }
                        }
                    }
                }
            },
        )
        .await
    });
    (task, states)
}

async fn wait_for(states: &mut watch::Receiver<Publication>, wanted: Publication) {
    tokio::time::timeout(Duration::from_secs(24 * 60 * 60), async {
        loop {
            if *states.borrow_and_update() == wanted {
                return;
            }
            states.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
}

/// Alice, a contact of `bob`, dials him, confirms and exchanges a chat
/// message. Returns true if it worked.
async fn reach(network: &MockNetwork, alice: &LocalIdentity, bob: &LocalIdentity) -> bool {
    let tor = network.backend();
    let Ok(mut established) = dial(&tor, &Budgets::new(), alice, &bob.card()).await else {
        return false;
    };
    send_first(&mut established, alice).await;
    loop {
        let Ok(received) = step(&mut established.link, alice).await else {
            return false;
        };
        if received.actions.contains(&Action::Confirmed) {
            break;
        }
    }
    established.link.send(&chat("hello")).await.unwrap();
    loop {
        let Ok(received) = established.link.receive().await else {
            return false;
        };
        if received.message == chat("hello back") {
            return true;
        }
    }
}

#[test]
fn a_lost_service_is_published_again_under_the_same_name() {
    run_paused(async {
        let network = MockNetwork::new();
        let bob = identity(2, true).await;
        let alice = identity(1, true).await;
        alice.import(&bob.card()).await.unwrap();
        bob.import(&alice.card()).await.unwrap();
        let (stop, shutdown) = watch::channel(false);
        let (task, mut states) = supervised(network.backend(), bob.clone(), shutdown);
        wait_for(&mut states, Publication::Available).await;
        assert!(network.is_published(&bob.endpoint()));
        assert!(reach(&network, &alice, &bob).await);

        // Tor loses the service: it is unavailable, then published again
        // from the same key, and reached again with the same handshake.
        network.lose_service(&bob.endpoint());
        wait_for(&mut states, Publication::Unavailable(TorError::ControlLost)).await;
        assert!(!network.is_published(&bob.endpoint()));
        wait_for(&mut states, Publication::Available).await;
        assert!(network.is_published(&bob.endpoint()));
        assert!(reach(&network, &alice, &bob).await);

        stop.send(true).unwrap();
        assert_eq!(task.await.unwrap(), Publication::Stopped);
        assert!(!network.is_published(&bob.endpoint()));
    });
}

#[test]
fn while_tor_cannot_publish_the_attempts_slow_down() {
    run_paused(async {
        let network = MockNetwork::new();
        let bob = identity(2, true).await;
        network.set_control_available(false);
        let (stop, shutdown) = watch::channel(false);
        let started = tokio::time::Instant::now();
        let (task, mut states) = supervised(network.backend(), bob.clone(), shutdown);
        // Count the failed attempts during an hour without Tor.
        let mut failures = 0;
        let mut times = Vec::new();
        while started.elapsed() < Duration::from_secs(3600) {
            states.changed().await.unwrap();
            if *states.borrow_and_update() == Publication::Unavailable(TorError::ControlUnavailable)
            {
                failures += 1;
                times.push(started.elapsed());
            }
        }
        // 10, 20, 40, ... seconds with jitter: far fewer than one attempt
        // a second, and more than a handful.
        assert!((5..=12).contains(&failures), "{failures}");
        for pair in times.windows(2) {
            assert!(pair[1] - pair[0] >= Duration::from_secs(5));
        }
        // Tor comes back: the service is published.
        network.set_control_available(true);
        wait_for(&mut states, Publication::Available).await;
        stop.send(true).unwrap();
        assert_eq!(task.await.unwrap(), Publication::Stopped);
    });
}

#[test]
fn shutdown_stops_the_supervisor_while_it_waits_or_serves() {
    run_paused(async {
        let network = MockNetwork::new();
        let bob = identity(2, true).await;
        // While it waits between attempts.
        network.set_control_available(false);
        let (stop, shutdown) = watch::channel(false);
        let (task, mut states) = supervised(network.backend(), bob.clone(), shutdown);
        wait_for(
            &mut states,
            Publication::Unavailable(TorError::ControlUnavailable),
        )
        .await;
        let asked = tokio::time::Instant::now();
        stop.send(true).unwrap();
        assert_eq!(task.await.unwrap(), Publication::Stopped);
        assert!(asked.elapsed() < Duration::from_secs(1));

        // While it serves.
        network.set_control_available(true);
        let (stop, shutdown) = watch::channel(false);
        let (task, mut states) = supervised(network.backend(), bob.clone(), shutdown);
        wait_for(&mut states, Publication::Available).await;
        stop.send(true).unwrap();
        assert_eq!(task.await.unwrap(), Publication::Stopped);
        assert!(!network.is_published(&bob.endpoint()));

        // When the sender is gone.
        let (stop, shutdown) = watch::channel(false);
        let (task, mut states) = supervised(network.backend(), bob.clone(), shutdown);
        wait_for(&mut states, Publication::Available).await;
        drop(stop);
        assert_eq!(task.await.unwrap(), Publication::Stopped);
    });
}

#[test]
fn the_supervisors_of_two_identities_do_not_touch_each_other() {
    run_paused(async {
        let network = MockNetwork::new();
        let a = identity(1, true).await;
        let b = identity(2, true).await;
        let (stop, shutdown) = watch::channel(false);
        let (task_a, mut states_a) = supervised(network.backend(), a.clone(), shutdown.clone());
        let (task_b, mut states_b) = supervised(network.backend(), b.clone(), shutdown);
        wait_for(&mut states_a, Publication::Available).await;
        wait_for(&mut states_b, Publication::Available).await;
        network.lose_service(&a.endpoint());
        wait_for(
            &mut states_a,
            Publication::Unavailable(TorError::ControlLost),
        )
        .await;
        // B is untouched while A is down, and A comes back with its own key.
        assert_eq!(*states_b.borrow(), Publication::Available);
        assert!(network.is_published(&b.endpoint()));
        wait_for(&mut states_a, Publication::Available).await;
        assert!(network.is_published(&a.endpoint()));
        assert_ne!(a.endpoint(), b.endpoint());
        stop.send(true).unwrap();
        let (end_a, end_b) = both(task_a, task_b).await;
        assert_eq!(end_a.unwrap(), Publication::Stopped);
        assert_eq!(end_b.unwrap(), Publication::Stopped);
    });
}

#[test]
fn an_identity_without_a_key_is_not_published() {
    run_paused(async {
        let network = MockNetwork::new();
        let bob = identity(2, false).await;
        let (_stop, shutdown) = watch::channel(false);
        let (task, _) = supervised(network.backend(), bob.clone(), shutdown);
        assert_eq!(task.await.unwrap(), Publication::NoKey);
        assert!(!network.is_published(&bob.endpoint()));
        assert_eq!(network.dials(), 0);
    });
}

/// How a supervisor ended, within an hour of paused time: one that does
/// not end fails the test instead of letting it hang.
async fn ended(task: tokio::task::JoinHandle<Publication>) -> Publication {
    tokio::time::timeout(Duration::from_secs(3600), task)
        .await
        .expect("the supervisor did not end")
        .unwrap()
}

#[test]
fn a_deleted_identity_is_taken_down_and_not_published_again() {
    run_paused(async {
        let network = MockNetwork::new();
        let installation = Box::leak(Box::new(Installation::ephemeral()));
        let keys = |seed: u8| IdentityKeys {
            seed: Zeroizing::new([seed; 32]),
            transport: Zeroizing::new([seed ^ 0xA5; 32]),
            onion: Some(Zeroizing::new([seed; 64])),
            epoch: EndpointEpoch::FIRST,
            endpoint: endpoint(seed),
        };
        let bob = installation.restore_identity(keys(2), None).await.unwrap();
        let alice = identity(1, true).await;
        alice.import(&bob.card()).await.unwrap();
        bob.import(&alice.card()).await.unwrap();

        // While it serves: the supervisor ends and the service is removed.
        let (_stop, shutdown) = watch::channel(false);
        let (task, mut states) = supervised(network.backend(), bob.clone(), shutdown);
        wait_for(&mut states, Publication::Available).await;
        assert!(reach(&network, &alice, &bob).await);
        installation.delete_identity(bob.identity()).await.unwrap();
        assert_eq!(ended(task).await, Publication::Stopped);
        assert!(!network.is_published(&bob.endpoint()));
        assert!(bob.onion_secret().is_none());

        // A supervisor started for it later publishes nothing.
        let (_stop, shutdown) = watch::channel(false);
        let (task, _states) = supervised(network.backend(), bob.clone(), shutdown);
        assert_eq!(ended(task).await, Publication::Stopped);
        assert!(!network.is_published(&bob.endpoint()));

        // While it waits between attempts: it ends at once.
        let carol = installation.restore_identity(keys(3), None).await.unwrap();
        network.set_control_available(false);
        let (_stop, shutdown) = watch::channel(false);
        let (task, mut states) = supervised(network.backend(), carol.clone(), shutdown);
        wait_for(
            &mut states,
            Publication::Unavailable(TorError::ControlUnavailable),
        )
        .await;
        let asked = tokio::time::Instant::now();
        installation
            .delete_identity(carol.identity())
            .await
            .unwrap();
        assert_eq!(ended(task).await, Publication::Stopped);
        assert!(asked.elapsed() < Duration::from_secs(1));
    });
}

/// A backend whose publications go through `gate`: each waits there for
/// `release`, if one is set, and then runs `during` before it returns. The
/// services it returns note the state of the publication when they are
/// removed.
struct Gated {
    inner: MockTorBackend,
    entered: Arc<tokio::sync::Notify>,
    release: Option<Arc<tokio::sync::Notify>>,
    during: std::sync::Mutex<Option<(&'static Installation, monolith_identity::IdentityPublicKey)>>,
    states: watch::Receiver<Publication>,
    removed_while_available: Arc<std::sync::atomic::AtomicBool>,
}

struct Watched {
    inner: MockOnionService,
    states: watch::Receiver<Publication>,
    removed_while_available: Arc<std::sync::atomic::AtomicBool>,
}

impl OnionService for Watched {
    type Stream = <MockOnionService as OnionService>::Stream;

    fn service_key(&self) -> &OnionServiceKey {
        self.inner.service_key()
    }

    fn take_generated_secret(&mut self) -> Option<OnionServiceSecret> {
        self.inner.take_generated_secret()
    }

    fn is_published(&mut self) -> bool {
        self.inner.is_published()
    }

    fn accept(
        &mut self,
    ) -> impl core::future::Future<Output = Result<Self::Stream, TorError>> + Send {
        self.inner.accept()
    }

    fn close(self) -> impl core::future::Future<Output = Result<(), TorError>> + Send {
        if *self.states.borrow() == Publication::Available {
            self.removed_while_available
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        self.inner.close()
    }
}

impl TorBackend for Gated {
    type Stream = <MockTorBackend as TorBackend>::Stream;
    type Service = Watched;

    fn status(&self) -> impl core::future::Future<Output = TorStatus> + Send {
        self.inner.status()
    }

    fn connect_onion(
        &self,
        target: &OnionServiceKey,
        isolation: &IsolationGroup,
    ) -> impl core::future::Future<Output = Result<Self::Stream, TorError>> + Send {
        self.inner.connect_onion(target, isolation)
    }

    async fn publish_onion(&self, key: KeySource) -> Result<Self::Service, TorError> {
        self.entered.notify_one();
        if let Some(release) = &self.release {
            release.notified().await;
        }
        let inner = self.inner.publish_onion(key).await?;
        let during = self.during.lock().unwrap().take();
        if let Some((installation, identity)) = during {
            installation.delete_identity(&identity).await.unwrap();
        }
        Ok(Watched {
            inner,
            states: self.states.clone(),
            removed_while_available: self.removed_while_available.clone(),
        })
    }
}

/// Runs the supervisor of `local` on `backend` until it ends, with the
/// state sender whose receiver `backend` holds.
fn supervised_on(
    backend: Gated,
    local: Arc<LocalIdentity>,
    state: watch::Sender<Publication>,
) -> (tokio::task::JoinHandle<Publication>, watch::Sender<bool>) {
    let (stop, shutdown) = watch::channel(false);
    let task = tokio::spawn(async move {
        let budgets = Budgets::new();
        supervise(&backend, &budgets, &local, shutdown, &state, |_, _| async {
        })
        .await
    });
    (task, stop)
}

fn gated(
    network: &MockNetwork,
    release: Option<Arc<tokio::sync::Notify>>,
    during: Option<(&'static Installation, monolith_identity::IdentityPublicKey)>,
) -> (
    Gated,
    watch::Sender<Publication>,
    Arc<std::sync::atomic::AtomicBool>,
) {
    let (state, states) = watch::channel(Publication::Publishing);
    let removed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    (
        Gated {
            inner: network.backend(),
            entered: Arc::new(tokio::sync::Notify::new()),
            release,
            during: std::sync::Mutex::new(during),
            states,
            removed_while_available: removed.clone(),
        },
        state,
        removed,
    )
}

#[test]
fn a_publication_under_way_ends_when_the_identity_is_deleted() {
    // Tor has not answered the publication yet when the identity is
    // deleted. The supervisor does not wait for it: it ends at once, and
    // the publication is dropped, which removes whatever Tor would have
    // published with its control connection. The service never appears.
    run_paused(async {
        let network = MockNetwork::new();
        let installation = Box::leak(Box::new(Installation::ephemeral()));
        let bob = installation
            .restore_identity(
                IdentityKeys {
                    seed: Zeroizing::new([2; 32]),
                    transport: Zeroizing::new([2 ^ 0xA5; 32]),
                    onion: Some(Zeroizing::new([2; 64])),
                    epoch: EndpointEpoch::FIRST,
                    endpoint: endpoint(2),
                },
                None,
            )
            .await
            .unwrap();
        let release = Arc::new(tokio::sync::Notify::new());
        let (backend, state, _) = gated(&network, Some(release.clone()), None);
        let entered = backend.entered.clone();
        let (task, _stop) = supervised_on(backend, bob.clone(), state);
        entered.notified().await;
        let asked = tokio::time::Instant::now();
        installation.delete_identity(bob.identity()).await.unwrap();
        assert_eq!(ended(task).await, Publication::Stopped);
        assert!(asked.elapsed() < Duration::from_secs(1));
        release.notify_waiters();
        tokio::task::yield_now().await;
        assert!(!network.is_published(&bob.endpoint()));
    });
}

#[test]
fn a_service_published_for_a_deleted_identity_is_never_reported_available() {
    // The identity is deleted while Tor publishes its service, and Tor
    // answers after that. The service is removed at once, and the
    // publication is never reported available.
    run_paused(async {
        let network = MockNetwork::new();
        let installation: &'static Installation = Box::leak(Box::new(Installation::ephemeral()));
        let bob = installation
            .restore_identity(
                IdentityKeys {
                    seed: Zeroizing::new([2; 32]),
                    transport: Zeroizing::new([2 ^ 0xA5; 32]),
                    onion: Some(Zeroizing::new([2; 64])),
                    epoch: EndpointEpoch::FIRST,
                    endpoint: endpoint(2),
                },
                None,
            )
            .await
            .unwrap();
        let (backend, state, removed_while_available) =
            gated(&network, None, Some((installation, *bob.identity())));
        let (task, _stop) = supervised_on(backend, bob.clone(), state);
        assert_eq!(ended(task).await, Publication::Stopped);
        assert!(!removed_while_available.load(std::sync::atomic::Ordering::SeqCst));
        assert!(!network.is_published(&bob.endpoint()));
    });
}

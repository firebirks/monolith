//! The runtime against a model written here (`docs/TEST_PLAN.md`
//! T-MSG-11): two peers on persistent installations over the in-memory Tor,
//! and any sequence of sending from either side, crashing and restarting
//! either side, Tor going away and coming back, closing the sessions,
//! letting time pass, and blocking. The model is the list of texts each
//! side had accepted for sending, nothing else.
//!
//! After every step: what one side holds of the other's messages is, in
//! order, the start of what the other sent, each once; a message recorded
//! delivered is held by its receiver. At the end, with Tor back and both
//! sides running, everything sent before a block is delivered, and after a
//! block nothing more moves.
//!
//! Time is paused, so the reconnect delays pass at once.

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

use common::{fixed_endpoint, keys, run_paused};
use monolith_core::api::{ContactId, MessageStatus, SendError};
use monolith_core::budget::Budgets;
use monolith_core::identity::{Installation, LocalIdentity};
use monolith_core::runtime::{Runtime, RuntimeOptions};
use monolith_storage::dir::{CrashOutcome, MemoryDir};
use monolith_storage::vault::{KdfParams, Passphrase};
use monolith_tor::{MockNetwork, MockTorBackend};
use proptest::prelude::*;

fn passphrase() -> Passphrase {
    Passphrase::new("correct horse battery staple").unwrap()
}

fn options() -> RuntimeOptions {
    RuntimeOptions {
        startup_spread: Duration::ZERO,
        ..RuntimeOptions::default()
    }
}

/// Waits, with paused time, until `condition` holds, for up to `limit` of
/// test time. Returns whether it did.
async fn within(limit: Duration, condition: impl Fn() -> bool) -> bool {
    let step = Duration::from_millis(250);
    let mut waited = Duration::ZERO;
    while waited < limit {
        if condition() {
            return true;
        }
        tokio::time::sleep(step).await;
        waited += step;
    }
    condition()
}

/// One side: its directory, its installation, its identity, its runtime.
struct Side {
    seed: u8,
    dir: MemoryDir,
    installation: Installation,
    identity: Arc<LocalIdentity>,
    runtime: Option<Runtime<MockTorBackend>>,
    /// The record of the other side.
    other: Option<ContactId>,
}

impl Side {
    async fn new(seed: u8) -> Self {
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let identity = installation
            .restore_identity(keys(seed, seed, 1, fixed_endpoint(seed)), None)
            .await
            .unwrap();
        Self {
            seed,
            dir,
            installation,
            identity,
            runtime: None,
            other: None,
        }
    }

    fn runtime(&self) -> &Runtime<MockTorBackend> {
        self.runtime.as_ref().expect("started")
    }

    async fn start(&mut self, network: &MockNetwork) {
        self.runtime = Some(Runtime::start(
            self.installation.clone(),
            network.backend(),
            Budgets::new(),
            options(),
        ));
        let endpoint = fixed_endpoint(self.seed);
        assert!(within(Duration::from_secs(600), || network.is_published(&endpoint)).await);
    }

    /// A crash: every task ends where it is, and the installation is
    /// opened again from what the directory holds durably.
    async fn crash(&mut self, network: &MockNetwork) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown(Duration::ZERO).await;
        }
        let dir = self.dir.restart(CrashOutcome::ALL[0]);
        let (installation, _) = Installation::open(Box::new(dir.clone()), &passphrase()).unwrap();
        let identity = installation.identities().into_iter().next().unwrap();
        self.dir = dir;
        self.installation = installation;
        self.identity = identity;
        self.start(network).await;
    }

    /// The messages of the conversation with the other side, by status.
    fn texts(&self, wanted: &[MessageStatus]) -> Vec<String> {
        let Some(other) = self.other else {
            return Vec::new();
        };
        self.runtime()
            .messages(&self.identity.id(), &other, None, 1000)
            .unwrap()
            .into_iter()
            .filter(|message| wanted.contains(&message.status))
            .map(|message| message.text)
            .collect()
    }

    fn received(&self) -> Vec<String> {
        self.texts(&[MessageStatus::Received])
    }

    fn delivered(&self) -> Vec<String> {
        self.texts(&[MessageStatus::Delivered])
    }

    fn outbound(&self) -> Vec<String> {
        self.texts(&[MessageStatus::Queued, MessageStatus::Delivered])
    }
}

#[derive(Clone, Copy, Debug)]
enum Step {
    /// The side sends a message.
    Send(bool),
    /// The side crashes and starts again.
    Crash(bool),
    /// Tor's SOCKS endpoint goes away, or comes back.
    Tor(bool),
    /// The side closes its sessions.
    Close(bool),
    /// Time passes, in seconds.
    Wait(u16),
    /// The first side blocks the second.
    Block,
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        6 => any::<bool>().prop_map(Step::Send),
        2 => any::<bool>().prop_map(Step::Crash),
        2 => any::<bool>().prop_map(Step::Tor),
        2 => any::<bool>().prop_map(Step::Close),
        3 => (1..600_u16).prop_map(Step::Wait),
        1 => Just(Step::Block),
    ]
}

/// What one side holds of the other's messages is, in order, the start of
/// what the other sent, each once; and what the sender records delivered
/// the receiver holds.
fn check(sender: &[String], sent: &[String], receiver: &Side, delivered: &[String]) {
    let received = receiver.received();
    assert!(received.len() <= sent.len(), "{received:?} {sent:?}");
    assert_eq!(received, sent[..received.len()], "{sent:?}");
    for text in delivered {
        assert!(received.contains(text), "{text} delivered, not received");
    }
    // The sender holds exactly what it sent, in order.
    assert_eq!(sender, sent);
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 24,
        max_shrink_iters: 64,
        ..ProptestConfig::default()
    })]

    #[test]
    fn the_runtime_delivers_what_the_model_says(steps in prop::collection::vec(step(), 1..24)) {
        run_paused(async {
            let network = MockNetwork::new();
            let mut sides = [Side::new(1).await, Side::new(2).await];
            let card_0 = sides[0].identity.card();
            let card_1 = sides[1].identity.card();
            sides[0].identity.import(&card_1).await.unwrap();
            sides[1].identity.import(&card_0).await.unwrap();
            sides[0].other = sides[0].identity.contact_id(sides[1].identity.identity());
            sides[1].other = sides[1].identity.contact_id(sides[0].identity.identity());
            for side in &mut sides {
                side.start(&network).await;
            }
            // The model: what each side accepted for sending.
            let mut sent: [Vec<String>; 2] = [Vec::new(), Vec::new()];
            let mut blocked = false;
            for (number, step) in steps.iter().enumerate() {
                match *step {
                    Step::Send(first) => {
                        let index = usize::from(!first);
                        let text = format!("m{number}");
                        let side = &sides[index];
                        let result = side
                            .runtime()
                            .send_message(&side.identity.id(), &side.other.unwrap(), &text)
                            .await;
                        if blocked && index == 0 {
                            assert_eq!(result.err(), Some(SendError::NotAContact));
                        } else {
                            result.unwrap();
                            sent[index].push(text);
                        }
                    }
                    Step::Crash(first) => sides[usize::from(!first)].crash(&network).await,
                    Step::Tor(up) => network.set_socks_available(up),
                    Step::Close(first) => {
                        let side = &sides[usize::from(!first)];
                        let _ = side
                            .runtime()
                            .close_sessions(&side.identity.id(), &side.other.unwrap());
                    }
                    Step::Wait(seconds) => {
                        tokio::time::sleep(Duration::from_secs(u64::from(seconds))).await;
                    }
                    Step::Block => {
                        if !blocked {
                            let side = &sides[0];
                            side.runtime()
                                .block_contact(&side.identity.id(), &side.other.unwrap())
                                .await
                                .unwrap();
                            blocked = true;
                        }
                    }
                }
                for index in 0..2 {
                    let other = 1 - index;
                    check(
                        &sides[index].outbound(),
                        &sent[index],
                        &sides[other],
                        &sides[index].delivered(),
                    );
                }
            }
            // The end: Tor back, both running, a day of time.
            network.set_socks_available(true);
            let done = within(Duration::from_secs(24 * 60 * 60), || {
                blocked
                    || (sides[0].delivered() == sent[0] && sides[1].delivered() == sent[1])
            })
            .await;
            assert!(done, "not delivered: {sent:?}");
            if blocked {
                // Nothing more moves: once what was under way at the block
                // has settled, a day passes and nothing changes.
                tokio::time::sleep(Duration::from_secs(60 * 60)).await;
                let before = [sides[0].received(), sides[1].received()];
                let delivered = [sides[0].delivered(), sides[1].delivered()];
                tokio::time::sleep(Duration::from_secs(24 * 60 * 60)).await;
                assert_eq!([sides[0].received(), sides[1].received()], before);
                assert_eq!([sides[0].delivered(), sides[1].delivered()], delivered);
            } else {
                assert_eq!(sides[1].received(), sent[0]);
                assert_eq!(sides[0].received(), sent[1]);
            }
            for index in 0..2 {
                let other = 1 - index;
                check(
                    &sides[index].outbound(),
                    &sent[index],
                    &sides[other],
                    &sides[index].delivered(),
                );
            }
            for side in &mut sides {
                if let Some(runtime) = side.runtime.take() {
                    runtime.shutdown(Duration::ZERO).await;
                }
            }
        });
    }
}

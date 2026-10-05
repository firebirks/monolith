//! The contact store as a durable, serialized authority: restart, crash at
//! every step of a durable transition, and concurrent operations on one
//! contact and on many.

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

use common::{State, befriend, confirm_both, party, run, state, step};
use monolith_core::contacts::{ContactView, ImportOutcome, StoreError};
use monolith_core::identity::{Installation, LocalIdentity};
use monolith_identity::{IdentityPublicKey, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::body::Message;
use monolith_protocol::card::ContactCard;
use monolith_protocol::contact::{RecordKind, RequestMode};
use monolith_protocol::text::DisplayName;
use monolith_storage::dir::{CrashOutcome, MemoryDir};
use monolith_storage::vault::{KdfParams, Passphrase};
use monolith_tor::MockNetwork;

fn passphrase() -> Passphrase {
    Passphrase::new("store test passphrase").unwrap()
}

fn create(dir: &MemoryDir) -> Installation {
    Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap()
}

fn open(dir: &MemoryDir) -> Installation {
    Installation::open(Box::new(dir.clone()), &passphrase())
        .unwrap()
        .0
}

fn place(seed: u8) -> OnionServiceKey {
    OnionServiceKey::from_bytes(
        IdentitySecretKey::from_seed(&[seed.wrapping_add(100); 32])
            .public_key()
            .as_bytes(),
    )
    .unwrap()
}

/// A card of remote identity `seed` with key `key` at `epoch`.
fn card(seed: u8, key: u8, epoch: u64) -> ContactCard {
    party(seed, key, epoch, place(seed)).card().clone()
}

fn id(seed: u8) -> IdentityPublicKey {
    IdentitySecretKey::from_seed(&[seed; 32]).public_key()
}

/// An installation in a vault with one identity and some of everything:
/// a requested contact, an accepted one with a pending successor and a
/// retired key, a blocked identity, invitations, a request mode, a label.
/// Every card is the same in every run.
async fn rich(dir: &MemoryDir) -> (Installation, Arc<LocalIdentity>) {
    let network = MockNetwork::new();
    let installation = create(dir);
    let mut node = common::node_fixed(
        installation.clone(),
        &network,
        1,
        1,
        1,
        common::fixed_endpoint(1),
    )
    .await;
    let local = node.identity.clone();
    // Contact 11 accepts through a session, the way a contact does.
    let peer = common::node_fixed(
        Installation::ephemeral(),
        &network,
        11,
        11,
        1,
        common::fixed_endpoint(11),
    )
    .await;
    befriend(&peer, &mut node).await;
    local.import(&card(10, 10, 1)).await.unwrap();
    local.import(&card(11, 12, 2)).await.unwrap();
    local.confirm_pending(&card(11, 12, 2)).await.unwrap();
    local.import(&card(11, 13, 5)).await.unwrap();
    local.block(&id(20)).await.unwrap();
    local.set_request_mode(RequestMode::Open).await.unwrap();
    local
        .create_invitation(Some(DisplayName::new("Website").unwrap()))
        .await
        .unwrap();
    local.create_invitation(None).await.unwrap();
    local.set_verified(&id(10), true).await.unwrap();
    (installation, local)
}

#[test]
fn a_restart_reproduces_the_durable_state_exactly() {
    run(async {
        let dir = MemoryDir::new();
        let (installation, local) = rich(&dir).await;
        local.begin_rotation().await.unwrap();
        let before = state(&installation);
        assert_eq!(before.identities.len(), 1);
        assert_eq!(before.identities[0].known.len(), 3);
        drop(local);
        drop(installation);
        let reopened = open(&dir);
        assert_eq!(state(&reopened), before);
        // And it goes on from there: a change after the restart is durable
        // in turn.
        let local = reopened.identities()[0].clone();
        local.delete(&id(10)).await.unwrap();
        let after = state(&reopened);
        drop(local);
        drop(reopened);
        assert_eq!(state(&open(&dir)), after);
    });
}

#[test]
fn a_restart_on_disk_reproduces_the_durable_state() {
    run(async {
        let path = std::env::temp_dir().join(format!("monolith-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let dir = monolith_storage::dir::DiskDir::open(&path, true).unwrap();
        let installation =
            Installation::create(Box::new(dir), &passphrase(), KdfParams::FLOOR).unwrap();
        let local = installation
            .restore_identity(common::keys(1, 1, 1, place(1)), None)
            .await
            .unwrap();
        local.import(&card(10, 10, 1)).await.unwrap();
        local.block(&id(20)).await.unwrap();
        let before = state(&installation);
        drop(local);
        // The lock is held while the installation lives.
        assert!(matches!(
            monolith_storage::dir::DiskDir::open(&path, false),
            Err(monolith_storage::StorageError::Locked)
        ));
        drop(installation);
        let dir = monolith_storage::dir::DiskDir::open(&path, false).unwrap();
        let (reopened, _) = Installation::open(Box::new(dir), &passphrase()).unwrap();
        assert_eq!(state(&reopened), before);
        drop(reopened);
        std::fs::remove_dir_all(&path).unwrap();
    });
}

/// One durable transition, stopped at every step of its vault write under
/// every crash outcome. A new process must find the state before or the
/// state after, and nothing else.
fn crash_everywhere<S, O>(setup: S, operation: O)
where
    S: Fn(MemoryDir) -> std::pin::Pin<Box<dyn core::future::Future<Output = ()>>>,
    O: Fn(Installation) -> std::pin::Pin<Box<dyn core::future::Future<Output = bool>>>,
{
    run(async {
        // The states before and after, and how many steps the write takes.
        let base = MemoryDir::new();
        setup(base.clone()).await;
        let before = state(&open(&base));
        let probe = base.restart(CrashOutcome::ALL[0]);
        let installation = open(&probe);
        let start = probe.steps();
        assert!(operation(installation.clone()).await);
        let total = probe.steps() - start;
        let after = state(&installation);
        assert_ne!(before, after);
        drop(installation);
        assert!(total >= 6, "{total}");
        for step in 1..=total {
            for outcome in CrashOutcome::ALL {
                let dir = base.restart(CrashOutcome::ALL[0]);
                let installation = open(&dir);
                dir.crash_at(dir.steps() + step);
                let succeeded = operation(installation.clone()).await;
                // What this run would have made durable. The same as the
                // state after the operation in the first run, but for
                // values drawn from the random source, such as a new key or
                // a capability.
                let after_here = state(&installation);
                assert_eq!(
                    after_here == after,
                    !draws_randomness(&before, &after),
                    "{step}"
                );
                drop(installation);
                let found = state(&open(&dir.restart(outcome)));
                if succeeded {
                    assert_eq!(found, after_here, "step {step} {outcome:?}");
                } else {
                    assert!(
                        found == before || found == after_here,
                        "step {step} {outcome:?}"
                    );
                }
                // Never a lower epoch, never a retired key active again.
                for (identity_before, identity_found) in
                    before.identities.iter().zip(&found.identities)
                {
                    for (remote, view) in &identity_found.known {
                        let Some(held) = &view.credentials else {
                            continue;
                        };
                        if let Some((_, old)) =
                            identity_before.known.iter().find(|(r, _)| r == remote)
                        {
                            if let Some(old_held) = &old.credentials {
                                assert!(held.active().epoch() >= old_held.active().epoch());
                                if let Some(retired) = old_held.retired() {
                                    assert_ne!(held.active().transport(), retired);
                                }
                            }
                        }
                    }
                }
            }
        }
    });
}

/// True if the operation that turned `before` into `after` drew from the
/// random source: a rotation key or an invitation capability.
fn draws_randomness(before: &State, after: &State) -> bool {
    before
        .identities
        .iter()
        .zip(&after.identities)
        .any(|(b, a)| b.successor != a.successor || b.invitations.len() < a.invitations.len())
}

fn setup_rich(dir: MemoryDir) -> std::pin::Pin<Box<dyn core::future::Future<Output = ()>>> {
    Box::pin(async move {
        let _ = rich(&dir).await;
    })
}

fn first(installation: &Installation) -> Arc<LocalIdentity> {
    installation.identities()[0].clone()
}

#[test]
fn creating_a_contact_is_atomic() {
    crash_everywhere(setup_rich, |installation| {
        Box::pin(async move { first(&installation).import(&card(30, 30, 1)).await.is_ok() })
    });
}

#[test]
fn updating_a_card_is_atomic() {
    crash_everywhere(setup_rich, |installation| {
        Box::pin(async move {
            first(&installation).import(&card(10, 10, 4)).await
                == Ok(ImportOutcome::Evaluated {
                    relation: Some(monolith_protocol::credential::CardRelation::NewerActiveKey),
                    change: monolith_protocol::credential::CredentialChange::Advanced,
                })
        })
    });
}

#[test]
fn promoting_a_successor_is_atomic() {
    crash_everywhere(setup_rich, |installation| {
        Box::pin(async move {
            first(&installation)
                .confirm_pending(&card(11, 13, 5))
                .await
                .is_ok()
        })
    });
}

#[test]
fn blocking_is_atomic() {
    crash_everywhere(setup_rich, |installation| {
        Box::pin(async move { first(&installation).block(&id(11)).await.is_ok() })
    });
}

#[test]
fn deleting_is_atomic() {
    crash_everywhere(setup_rich, |installation| {
        Box::pin(async move { first(&installation).delete(&id(11)).await.is_ok() })
    });
}

#[test]
fn creating_and_revoking_an_invitation_are_atomic() {
    crash_everywhere(setup_rich, |installation| {
        Box::pin(async move { first(&installation).create_invitation(None).await.is_ok() })
    });
    crash_everywhere(setup_rich, |installation| {
        Box::pin(async move {
            let local = first(&installation);
            let invitation = local.invitations()[0].0;
            local.revoke_invitation(invitation).await.is_ok()
        })
    });
}

#[test]
fn beginning_a_rotation_is_atomic() {
    crash_everywhere(setup_rich, |installation| {
        Box::pin(async move { first(&installation).begin_rotation().await.is_ok() })
    });
}

#[test]
fn creating_an_identity_is_atomic() {
    crash_everywhere(
        |dir| {
            Box::pin(async move {
                let _ = create(&dir);
            })
        },
        |installation| {
            Box::pin(async move {
                installation
                    .restore_identity(common::keys(5, 5, 1, place(5)), None)
                    .await
                    .is_ok()
            })
        },
    );
}

#[test]
fn an_announced_successor_is_atomic() {
    // The EndpointUpdate arrives on a real session, and the write of what
    // it changes is stopped at every step. The history before it is built
    // again for every stop.
    async fn up_to_the_announcement(
        network: &MockNetwork,
        base: &MemoryDir,
    ) -> (
        common::Node,
        common::Node,
        monolith_core::link::Link<tokio::io::DuplexStream>,
        monolith_core::link::Link<tokio::io::DuplexStream>,
        ContactCard,
    ) {
        let mut alice =
            common::node_fixed(create(base), network, 1, 1, 1, common::fixed_endpoint(1)).await;
        let bob = common::node_fixed(
            Installation::ephemeral(),
            network,
            2,
            2,
            1,
            common::fixed_endpoint(2),
        )
        .await;
        befriend(&bob, &mut alice).await;
        let successor = bob.identity.begin_rotation().await.unwrap();
        let (bob_link, alice_link) = confirm_both(&bob, &mut alice).await;
        (alice, bob, alice_link, bob_link, successor)
    }
    run(async {
        let network = MockNetwork::new();
        let base = MemoryDir::new();
        let (alice, _bob, mut alice_link, mut bob_link, successor) =
            up_to_the_announcement(&network, &base).await;
        let before = state(&alice.installation);
        let start = base.steps();
        bob_link
            .send(&Message::EndpointUpdate(Box::new(successor.clone())))
            .await
            .unwrap();
        step(&mut alice_link, &alice.identity).await.unwrap();
        let total = base.steps() - start;
        assert!(total >= 6, "{total}");
        assert_ne!(before, state(&alice.installation));
        for step_number in 1..=total {
            for outcome in CrashOutcome::ALL {
                let network = MockNetwork::new();
                let dir = MemoryDir::new();
                let (alice, _bob, mut alice_link, mut bob_link, successor) =
                    up_to_the_announcement(&network, &dir).await;
                assert_eq!(state(&alice.installation), before);
                dir.crash_at(dir.steps() + step_number);
                bob_link
                    .send(&Message::EndpointUpdate(Box::new(successor)))
                    .await
                    .unwrap();
                let received = alice_link.receive().await.unwrap();
                let applied = alice
                    .identity
                    .apply(alice_link.session_ref(), &received)
                    .await;
                // Bob's successor key is new in every run.
                let after_here = state(&alice.installation);
                assert_ne!(after_here, before);
                let found = state(&open(&dir.restart(outcome)));
                if applied.is_ok() {
                    assert_eq!(found, after_here, "step {step_number} {outcome:?}");
                } else {
                    assert!(
                        found == before || found == after_here,
                        "step {step_number} {outcome:?}"
                    );
                }
            }
        }
    });
}

#[test]
fn concurrent_operations_on_one_contact_are_one_of_their_serial_orders() {
    // Four operations on the same remote identity race on a runtime with
    // several threads. Their results and the final state must be those of
    // one of the 24 orders in which they could run one after the other.
    #[derive(Clone, Copy, Debug)]
    enum Op {
        ImportNewKey,
        ConfirmNewKey,
        Block,
        ImportNewerSameKey,
    }
    const OPS: [Op; 4] = [
        Op::ImportNewKey,
        Op::ConfirmNewKey,
        Op::Block,
        Op::ImportNewerSameKey,
    ];
    type Outcome = (Vec<Result<(), StoreError>>, Option<ContactView>);

    async fn apply(local: &LocalIdentity, op: Op) -> Result<(), StoreError> {
        match op {
            Op::ImportNewKey => local.import(&card(10, 12, 3)).await.map(|_| ()),
            Op::ConfirmNewKey => local.confirm_pending(&card(10, 12, 3)).await,
            Op::Block => local.block(&id(10)).await,
            Op::ImportNewerSameKey => local.import(&card(10, 10, 2)).await.map(|_| ()),
        }
    }

    async fn fresh() -> (Installation, Arc<LocalIdentity>) {
        let installation = Installation::ephemeral();
        let local = installation
            .restore_identity(common::keys(1, 1, 1, place(1)), None)
            .await
            .unwrap();
        // An accepted contact: the request was accepted.
        local.import(&card(10, 10, 1)).await.unwrap();
        (installation, local)
    }

    fn normalize(view: Option<ContactView>) -> Option<ContactView> {
        view.map(|mut view| {
            view.sessions = 0;
            view
        })
    }

    // Every serial order.
    let mut serial: Vec<Outcome> = Vec::new();
    let mut orders = Vec::new();
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    let order = [a, b, c, d];
                    let mut seen = [false; 4];
                    if order
                        .iter()
                        .all(|index| !std::mem::replace(&mut seen[*index], true))
                    {
                        orders.push(order);
                    }
                }
            }
        }
    }
    assert_eq!(orders.len(), 24);
    run(async {
        for order in &orders {
            let (_installation, local) = fresh().await;
            let mut results = vec![Ok(()); 4];
            for index in order {
                results[*index] = apply(&local, OPS[*index]).await;
            }
            serial.push((results, normalize(local.contact(&id(10)))));
        }
    });

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .build()
        .unwrap();
    for _ in 0..300 {
        let outcome: Outcome = runtime.block_on(async {
            let (_installation, local) = fresh().await;
            let tasks: Vec<_> = OPS
                .iter()
                .map(|op| {
                    let local = local.clone();
                    let op = *op;
                    tokio::spawn(async move { apply(&local, op).await })
                })
                .collect();
            let mut results = Vec::new();
            for task in tasks {
                results.push(task.await.unwrap());
            }
            (results, normalize(local.contact(&id(10))))
        });
        assert!(serial.contains(&outcome), "{outcome:?}");
    }
}

#[test]
fn concurrent_operations_on_different_contacts_lose_nothing() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(8)
        .build()
        .unwrap();
    runtime.block_on(async {
        let installation = Installation::ephemeral();
        let local = installation
            .restore_identity(common::keys(1, 1, 1, place(1)), None)
            .await
            .unwrap();
        let tasks: Vec<_> = (30..130_u8)
            .map(|seed| {
                let local = local.clone();
                tokio::spawn(async move {
                    if seed % 3 == 0 {
                        local.block(&id(seed)).await
                    } else {
                        local.import(&card(seed, seed, 1)).await.map(|_| ())
                    }
                })
            })
            .collect();
        for task in tasks {
            task.await.unwrap().unwrap();
        }
        for seed in 30..130_u8 {
            let expected = if seed % 3 == 0 {
                RecordKind::Blocked
            } else {
                RecordKind::Requested
            };
            assert_eq!(local.kind(&id(seed)), expected, "{seed}");
        }
        assert_eq!(local.known().len(), 100);
        drop(installation);
    });
}

#[test]
fn a_block_that_races_an_admission_leaves_no_contact_session_standing() {
    // Bob admits Alice's dial while his user blocks her, on several
    // threads. Whichever came first, afterwards no link of Bob's that was
    // admitted as Alice's still stands.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    for _ in 0..40 {
        runtime.block_on(async {
            let network = MockNetwork::new();
            let alice = common::node(&network, 1).await;
            let mut bob = common::node(&network, 2).await;
            befriend(&alice, &mut bob).await;
            let bob_identity = bob.identity.clone();
            let alice_id = *alice.identity.identity();
            let blocking = tokio::spawn(async move {
                tokio::task::yield_now().await;
                bob_identity.block(&alice_id).await.unwrap();
            });
            let (_, answered) = common::connect(&alice, &mut bob).await;
            blocking.await.unwrap();
            assert_eq!(bob.identity.kind(&alice_id), RecordKind::Blocked);
            if let Ok(answered) = answered {
                // Admitted before the block: withdrawn by it. Admitted after:
                // not a contact's at all.
                assert!(
                    answered.link.is_withdrawn()
                        || !answered.admission.standing.is_contact_record()
                );
            }
            assert_eq!(bob.identity.contact(&alice_id).unwrap().sessions, 0);
        });
    }
}

#[test]
fn identities_restored_or_created_never_share_a_key() {
    run(async {
        let installation = Installation::ephemeral();
        installation
            .restore_identity(common::keys(1, 1, 1, place(1)), None)
            .await
            .unwrap();
        // The same identity key, another transport key and endpoint.
        assert!(
            installation
                .restore_identity(common::keys(1, 2, 1, place(2)), None)
                .await
                .is_err()
        );
        // Another identity with the same transport key.
        assert!(
            installation
                .restore_identity(common::keys(3, 1, 1, place(3)), None)
                .await
                .is_err()
        );
        // Another identity with the same endpoint.
        assert!(
            installation
                .restore_identity(common::keys(4, 4, 1, place(1)), None)
                .await
                .is_err()
        );
        assert_eq!(installation.identities().len(), 1);
        // Up to the limit, and not beyond.
        for seed in
            10..10 + u8::try_from(monolith_protocol::limits::MAX_LOCAL_IDENTITIES).unwrap() - 1
        {
            installation
                .restore_identity(common::keys(seed, seed, 1, place(seed)), None)
                .await
                .unwrap();
        }
        assert!(
            installation
                .restore_identity(common::keys(50, 50, 1, place(50)), None)
                .await
                .is_err()
        );
    });
}

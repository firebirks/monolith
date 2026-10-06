//! Several local identities in one installation: T-MI-1 to T-MI-10 of
//! `docs/TEST_PLAN.md`, invariants S39 to S46.

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

use common::{
    Node, befriend, confirm_both, connect, dial_and_answer, keys, node, node_in, request, run,
    send_first, state,
};
use monolith_core::contacts::StoreError;
use monolith_core::identity::{Installation, InstallationError};
use monolith_core::link::{LinkError, dial};
use monolith_core::persist::CommitError;
use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::contact::{RecordKind, RequestMode};
use monolith_protocol::limits::MAX_UNKNOWN_SESSIONS;
use monolith_protocol::session::Standing;
use monolith_storage::dir::{CrashOutcome, MemoryDir, VAULT, VaultDir};
use monolith_storage::record::{Contents, StoredIdentity};
use monolith_storage::vault::{KdfParams, Passphrase, Vault};
use monolith_tor::{MockNetwork, OnionService};
use zeroize::Zeroizing;

fn passphrase() -> Passphrase {
    Passphrase::new("identities test").unwrap()
}

fn place(seed: u8) -> OnionServiceKey {
    OnionServiceKey::from_bytes(
        IdentitySecretKey::from_seed(&[seed.wrapping_add(77); 32])
            .public_key()
            .as_bytes(),
    )
    .unwrap()
}

/// Two local identities, A and B, in one installation, each with its
/// service on the in-memory Tor.
async fn two(network: &MockNetwork, installation: &Installation) -> (Node, Node) {
    (
        node_in(installation.clone(), network, 1, 1, 1).await,
        node_in(installation.clone(), network, 2, 2, 1).await,
    )
}

#[test]
fn t_mi_1_no_two_local_identities_share_a_key() {
    run(async {
        let installation = Installation::ephemeral();
        installation
            .restore_identity(keys(1, 1, 1, place(1)), None)
            .await
            .unwrap();
        for (seed, transport, endpoint) in [(1, 9, 9), (9, 1, 9), (9, 9, 1)] {
            assert_eq!(
                installation
                    .restore_identity(keys(seed, transport, 1, place(endpoint)), None)
                    .await
                    .err(),
                Some(InstallationError::Invalid)
            );
        }
        // A vault that holds two identities with one key is refused when
        // it is opened.
        let dir = MemoryDir::new();
        let identity = |seed: u8, transport: u8, endpoint: u8| StoredIdentity {
            identity_seed: Zeroizing::new([seed; 32]),
            transport_secret: Zeroizing::new([transport; 32]),
            onion_secret: None,
            epoch: EndpointEpoch::FIRST,
            endpoint: place(endpoint),
            rotation: None,
            request_mode: RequestMode::Invitation,
            label: None,
            invitations: Vec::new(),
            contacts: Vec::new(),
            blocked: Vec::new(),
            declined: Vec::new(),
        };
        let contents = Contents {
            identities: vec![identity(1, 1, 1), identity(2, 1, 2)],
        };
        Vault::create(
            dir.clone(),
            &passphrase(),
            KdfParams::FLOOR,
            &contents.encode().unwrap(),
        )
        .unwrap();
        assert_eq!(
            Installation::open(Box::new(dir), &passphrase()).err(),
            Some(InstallationError::Invalid)
        );
    });
}

#[test]
fn an_identity_is_restored_only_if_the_vault_can_hold_it() {
    // What `restore_identity` accepts is written to the vault, and has to
    // be read back: keys the vault would refuse when it is opened are
    // refused here first. Every identity that was accepted is there after
    // a restart.
    run(async {
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let mut zero_seed = keys(1, 1, 1, place(1));
        zero_seed.seed = Zeroizing::new([0; 32]);
        let mut zero_onion = keys(2, 2, 1, place(2));
        zero_onion.onion = Some(Zeroizing::new([0; 64]));
        let mut one_bit = keys(3, 3, 1, place(3));
        one_bit.seed = Zeroizing::new([0; 32]);
        one_bit.seed[31] = 1;
        let mut no_onion = keys(4, 4, 1, place(4));
        no_onion.onion = None;
        let mut accepted = Vec::new();
        for (keys, valid) in [
            (zero_seed, false),
            (zero_onion, false),
            (one_bit, true),
            (no_onion, true),
            (keys(5, 5, 7, place(5)), true),
        ] {
            let restored = installation.restore_identity(keys, None).await;
            if valid {
                accepted.push(*restored.unwrap().identity());
            } else {
                assert_eq!(restored.err(), Some(InstallationError::Invalid));
            }
        }
        drop(installation);
        let (reopened, _) =
            Installation::open(Box::new(dir.restart(CrashOutcome::ALL[0])), &passphrase()).unwrap();
        let mut found: Vec<_> = reopened
            .identities()
            .iter()
            .map(|identity| *identity.identity())
            .collect();
        found.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        accepted.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        assert_eq!(found, accepted);
    });
}

#[test]
fn a_rotation_to_the_same_public_key_is_refused_when_the_vault_is_opened() {
    // X25519 clamps the scalar: two secrets that differ only in the bits
    // it clears are one key. A rotation whose new secret is the old one
    // with such a bit changed would announce the active key as its own
    // successor; the vault that holds it is refused.
    let active = [0xC2_u8; 32];
    let mut low = active;
    low[0] ^= 0x01;
    let mut high = active;
    high[31] &= 0x7F;
    for next in [low, high] {
        let dir = MemoryDir::new();
        let identity = StoredIdentity {
            identity_seed: Zeroizing::new([1; 32]),
            transport_secret: Zeroizing::new(active),
            onion_secret: None,
            epoch: EndpointEpoch::FIRST,
            endpoint: place(1),
            rotation: Some(monolith_storage::record::StoredRotation {
                transport_secret: Zeroizing::new(next),
                epoch: EndpointEpoch::FIRST.next().unwrap(),
                switched: false,
            }),
            request_mode: RequestMode::Invitation,
            label: None,
            invitations: Vec::new(),
            contacts: Vec::new(),
            blocked: Vec::new(),
            declined: Vec::new(),
        };
        let contents = Contents {
            identities: vec![identity],
        };
        Vault::create(
            dir.clone(),
            &passphrase(),
            KdfParams::FLOOR,
            &contents.encode().unwrap(),
        )
        .unwrap();
        assert_eq!(
            Installation::open(Box::new(dir), &passphrase()).err(),
            Some(InstallationError::Invalid)
        );
    }
}

#[test]
fn t_mi_2_and_3_contact_state_belongs_to_one_identity() {
    run(async {
        let network = MockNetwork::new();
        let installation = Installation::ephemeral();
        let (mut a, mut b) = two(&network, &installation).await;
        let carol = node(&network, 3).await;
        // Carol is an accepted, verified contact of A.
        befriend(&carol, &mut a).await;
        a.identity
            .set_verified(carol.identity.identity(), true)
            .await
            .unwrap();
        assert_eq!(
            a.identity.kind(carol.identity.identity()),
            RecordKind::Accepted
        );
        // For B she is a stranger.
        assert_eq!(b.identity.kind(carol.identity.identity()), RecordKind::None);
        assert!(b.identity.contact(carol.identity.identity()).is_none());
        carol.identity.import(&b.card()).await.unwrap();
        let (_, answered) = connect(&carol, &mut b).await;
        assert_eq!(answered.unwrap().admission.standing, Standing::None);
        // A pending request at B is not one at A, and a decline at B
        // changes nothing at A.
        b.identity
            .set_request_mode(RequestMode::Open)
            .await
            .unwrap();
        let dave = node(&network, 4).await;
        dave.identity.import(&b.card()).await.unwrap();
        let (asking, answering) = connect(&dave, &mut b).await;
        let (mut asking, mut answering) = (asking.unwrap(), answering.unwrap());
        send_first(&mut asking, &dave.identity).await;
        let received = answering.link.receive().await.unwrap();
        b.identity
            .apply(answering.link.session_ref(), &received)
            .await
            .unwrap();
        assert_eq!(b.identity.requests().len(), 1);
        assert!(a.identity.requests().is_empty());

        // T-MI-3: blocking Carol at B leaves A's contact with her as it was,
        // and a session of A with her still confirms.
        let before = a.identity.contact(carol.identity.identity()).unwrap();
        b.identity.block(carol.identity.identity()).await.unwrap();
        assert_eq!(
            a.identity.contact(carol.identity.identity()).unwrap(),
            before
        );
        let (carol_link, a_link) = confirm_both(&carol, &mut a).await;
        assert!(!carol_link.is_over() && !a_link.is_over());
    });
}

#[test]
fn t_mi_4_a_capability_of_one_identity_admits_nothing_at_another() {
    run(async {
        let network = MockNetwork::new();
        let installation = Installation::ephemeral();
        let (a, mut b) = two(&network, &installation).await;
        let (id_a, card_a) = a.identity.create_invitation(None).await.unwrap();
        // A card of B that carries A's capability: B does not know it.
        let carrying = monolith_protocol::card::ContactCard::sign(
            &IdentitySecretKey::from_seed(&[2; 32]),
            *b.card().transport(),
            b.card().epoch(),
            b.card().endpoints().clone(),
            card_a.invitation().cloned(),
        )
        .unwrap();
        let carol = node(&network, 3).await;
        carol.identity.import(&carrying).await.unwrap();
        let (asking, answering) = dial_and_answer(&carol, &carrying, &mut b).await;
        let (mut asking, mut answering) = (asking.unwrap(), answering.unwrap());
        send_first(&mut asking, &carol.identity).await;
        let received = answering.link.receive().await.unwrap();
        let applied = b
            .identity
            .apply(answering.link.session_ref(), &received)
            .await
            .unwrap();
        assert_eq!(
            applied.request,
            Some(Err(monolith_core::requests::Dropped::Capability))
        );
        // Revoking it at A changes nothing at B.
        let (_, card_b) = b.identity.create_invitation(None).await.unwrap();
        a.identity.revoke_invitation(id_a).await.unwrap();
        assert_eq!(b.identity.invitations().len(), 1);
        let dave = node(&network, 4).await;
        dave.identity.import(&card_b).await.unwrap();
        let (asking, answering) = dial_and_answer(&dave, &card_b, &mut b).await;
        let (mut asking, mut answering) = (asking.unwrap(), answering.unwrap());
        send_first(&mut asking, &dave.identity).await;
        let received = answering.link.receive().await.unwrap();
        let applied = b
            .identity
            .apply(answering.link.session_ref(), &received)
            .await
            .unwrap();
        assert_eq!(applied.request, Some(Ok(())));
    });
}

#[test]
fn t_mi_10_a_session_of_one_identity_changes_nothing_at_another() {
    // Carol asks A for contact on a session of A. What she sent is applied
    // to A's state, never to B's, even if the caller hands B the session:
    // a session belongs to the identity whose store admitted it.
    run(async {
        let network = MockNetwork::new();
        let installation = Installation::ephemeral();
        let (mut a, b) = two(&network, &installation).await;
        a.identity
            .set_request_mode(RequestMode::Open)
            .await
            .unwrap();
        b.identity
            .set_request_mode(RequestMode::Open)
            .await
            .unwrap();
        let carol = node(&network, 3).await;
        carol.identity.import(&a.card()).await.unwrap();
        let (asking, answering) = connect(&carol, &mut a).await;
        let (mut asking, mut answering) = (asking.unwrap(), answering.unwrap());
        send_first(&mut asking, &carol.identity).await;
        let received = answering.link.receive().await.unwrap();
        assert!(
            b.identity
                .apply(answering.link.session_ref(), &received)
                .await
                .is_err()
        );
        assert!(b.identity.requests().is_empty());
        assert_eq!(b.identity.kind(carol.identity.identity()), RecordKind::None);
        let applied = a
            .identity
            .apply(answering.link.session_ref(), &received)
            .await
            .unwrap();
        assert_eq!(applied.request, Some(Ok(())));
        assert_eq!(a.identity.requests().len(), 1);
        // Nor at an identity made again from A's keys: the session belongs
        // to the store that admitted it, which is gone.
        let endpoint = a.identity.endpoint();
        installation
            .delete_identity(a.identity.identity())
            .await
            .unwrap();
        let again = installation
            .restore_identity(keys(1, 1, 1, endpoint), None)
            .await
            .unwrap();
        again.set_request_mode(RequestMode::Open).await.unwrap();
        assert_eq!(
            again
                .apply(answering.link.session_ref(), &received)
                .await
                .err(),
            Some(StoreError::OtherIdentity)
        );
        assert!(again.requests().is_empty());
    });
}

#[test]
fn t_mi_7_strangers_at_one_identity_change_nothing_at_another() {
    // A flood of strangers at B fills B's budget; A's budget and what A
    // answers do not change.
    run(async {
        let network = MockNetwork::new();
        let installation = Installation::ephemeral();
        let (mut a, mut b) = two(&network, &installation).await;
        let mut kept = Vec::new();
        for seed in 30..30 + u8::try_from(MAX_UNKNOWN_SESSIONS).unwrap() + 3 {
            let stranger = node(&network, seed).await;
            stranger.identity.import(&b.card()).await.unwrap();
            kept.push(connect(&stranger, &mut b).await);
        }
        assert_eq!(b.identity.strangers().free(), 0);
        assert_eq!(a.identity.strangers().free(), MAX_UNKNOWN_SESSIONS);
        // A stranger at A is answered exactly as before.
        let stranger = node(&network, 50).await;
        stranger.identity.import(&a.card()).await.unwrap();
        let (asking, answering) = connect(&stranger, &mut a).await;
        let (mut asking, mut answering) = (asking.unwrap(), answering.unwrap());
        assert!(answering.link.holds_unknown_slot());
        asking
            .link
            .send(&request(&stranger.card(), None))
            .await
            .unwrap();
        let received = answering.link.receive().await.unwrap();
        assert_eq!(
            received.actions,
            vec![
                monolith_protocol::session::Action::SendClose,
                monolith_protocol::session::Action::ConsiderRequest
            ]
        );
        drop(kept);
    });
}

#[test]
fn t_mi_8_every_dial_names_its_identity() {
    run(async {
        let network = MockNetwork::new();
        let installation = Installation::ephemeral();
        let (a, b) = two(&network, &installation).await;
        let mut carol = node(&network, 3).await;
        // Only A holds Carol: A's dial reaches her with A's party; B's dial
        // of the same card finds no record of B's and sends nothing.
        a.identity.import(&carol.card()).await.unwrap();
        let card = carol.card();
        let (dialed, answered) = dial_and_answer(&a, &card, &mut carol).await;
        assert_eq!(
            answered.unwrap().link.session().peer(),
            a.identity.identity()
        );
        assert_eq!(dialed.unwrap().link.session().local_card(), &a.card());
        let (dialed, answered) = dial_and_answer(&b, &card, &mut carol).await;
        assert!(matches!(dialed.err(), Some(LinkError::Refused(_))));
        assert!(answered.is_err());
    });
}

#[test]
fn a_deleted_identity_admits_nobody_and_changes_nothing() {
    // B is deleted while Carol still holds it as a contact. B dials
    // nobody and answers nobody, refuses every change, and nothing of it
    // is in the vault: a deletion is not undone by a write that no longer
    // holds the identity.
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let (a, mut b) = two(&network, &installation).await;
        let mut carol = node(&network, 3).await;
        befriend(&carol, &mut b).await;
        installation
            .delete_identity(b.identity.identity())
            .await
            .unwrap();
        assert!(b.identity.is_deleted());
        assert!(b.identity.onion_secret().is_none());

        // B dials nobody: it has no party to dial with, and no stream is
        // opened.
        assert_eq!(
            dial(&b.tor, &b.budgets, &b.identity, &carol.card())
                .await
                .err(),
            Some(LinkError::Storage(CommitError::Failed))
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), carol.service.accept())
                .await
                .is_err()
        );
        // Carol dials B: B answers nobody, and Carol's link ends.
        let (dialed, answered) = connect(&carol, &mut b).await;
        assert!(answered.is_err());
        if let Ok(mut dialed) = dialed {
            assert!(dialed.link.receive().await.is_err());
        }

        let other = node(&network, 4).await;
        assert_eq!(
            b.identity.create_invitation(None).await.err(),
            Some(StoreError::Failed)
        );
        assert_eq!(
            b.identity.import(&other.card()).await.err(),
            Some(StoreError::Failed)
        );
        assert_eq!(
            b.identity.block(other.identity.identity()).await.err(),
            Some(StoreError::Failed)
        );

        let after = state(&installation);
        assert_eq!(after.identities.len(), 1);
        assert_eq!(&after.identities[0].identity, a.identity.identity());
        drop((a, b, installation));
        let reopened =
            Installation::open(Box::new(dir.restart(CrashOutcome::ALL[0])), &passphrase())
                .unwrap()
                .0;
        assert_eq!(state(&reopened), after);
    });
}

#[test]
fn a_deleted_identity_hands_out_no_secret() {
    // Once B is deleted, nothing that holds or uses one of its secrets is
    // handed out any more: no card signed with its identity key for a
    // capability, no capability, no party with its transport key to dial
    // or answer with, no Onion Service secret. A stays as it was.
    run(async {
        let network = MockNetwork::new();
        let installation = Installation::ephemeral();
        let (a, b) = two(&network, &installation).await;
        let carol = node(&network, 3).await;
        b.identity.import(&carol.card()).await.unwrap();
        a.identity.import(&carol.card()).await.unwrap();
        let (invitation, _) = b.identity.create_invitation(None).await.unwrap();
        let (kept, _) = a.identity.create_invitation(None).await.unwrap();
        installation
            .delete_identity(b.identity.identity())
            .await
            .unwrap();
        assert_eq!(
            b.identity.invitation_card(invitation).err(),
            Some(StoreError::Failed)
        );
        assert!(b.identity.invitations().is_empty());
        assert!(b.identity.dial_plan(carol.identity.identity()).is_none());
        assert!(b.identity.answering_party().is_err());
        assert!(b.identity.onion_secret().is_none());
        assert!(a.identity.invitation_card(kept).is_ok());
        assert!(a.identity.dial_plan(carol.identity.identity()).is_some());
        assert!(a.identity.answering_party().is_ok());
        assert!(a.identity.onion_secret().is_some());
    });
}

#[test]
fn a_session_that_outlives_its_deleted_identity_changes_nothing() {
    // Carol, a stranger, has a session with B and has not spoken yet when
    // B is deleted. Her request then arrives: B queues nothing.
    run(async {
        let network = MockNetwork::new();
        let installation = Installation::ephemeral();
        let (_a, mut b) = two(&network, &installation).await;
        b.identity
            .set_request_mode(RequestMode::Open)
            .await
            .unwrap();
        let carol = node(&network, 3).await;
        carol.identity.import(&b.card()).await.unwrap();
        let (asking, answering) = connect(&carol, &mut b).await;
        let (mut asking, mut answering) = (asking.unwrap(), answering.unwrap());
        installation
            .delete_identity(b.identity.identity())
            .await
            .unwrap();
        send_first(&mut asking, &carol.identity).await;
        let received = answering.link.receive().await.unwrap();
        assert_eq!(
            b.identity
                .apply(answering.link.session_ref(), &received)
                .await
                .err(),
            Some(StoreError::Failed)
        );
        assert!(b.identity.requests().is_empty());
    });
}

#[test]
fn a_dial_that_waited_for_its_slot_while_the_identity_was_deleted_opens_no_stream() {
    // B's dial waits for the only dial slot. B is deleted meanwhile; when
    // the slot frees, the dial ends without opening a stream.
    run(async {
        let network = MockNetwork::new();
        let installation = Installation::ephemeral();
        let (_a, mut b) = two(&network, &installation).await;
        let mut carol = node(&network, 3).await;
        b.identity.import(&carol.card()).await.unwrap();
        b.budgets = monolith_core::budget::Budgets::with_limits(16, 256, 1);
        let held = b.budgets.dial().await.unwrap();
        let card = carol.card();
        let dialing = {
            let identity = b.identity.clone();
            let budgets = b.budgets.clone();
            let tor = network.backend();
            tokio::spawn(async move { dial(&tor, &budgets, &identity, &card).await.err() })
        };
        tokio::task::yield_now().await;
        installation
            .delete_identity(b.identity.identity())
            .await
            .unwrap();
        drop(held);
        assert_eq!(
            dialing.await.unwrap(),
            Some(LinkError::Storage(CommitError::Failed))
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), carol.service.accept())
                .await
                .is_err()
        );
    });
}

#[test]
fn t_mi_9_deleting_one_identity_leaves_the_other_as_it_was() {
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let (mut a, mut b) = two(&network, &installation).await;
        let carol = node(&network, 3).await;
        befriend(&carol, &mut a).await;
        befriend(&carol, &mut b).await;
        a.identity.create_invitation(None).await.unwrap();
        b.identity.create_invitation(None).await.unwrap();
        b.identity
            .block(&IdentitySecretKey::from_seed(&[9; 32]).public_key())
            .await
            .unwrap();
        // B has a session with Carol when it is deleted: it is withdrawn.
        let (_carol_link, b_link) = confirm_both(&carol, &mut b).await;
        let a_before = state(&installation)
            .identities
            .into_iter()
            .find(|held| &held.identity == a.identity.identity())
            .unwrap();
        installation
            .delete_identity(b.identity.identity())
            .await
            .unwrap();
        assert!(b_link.is_withdrawn());
        let after = state(&installation);
        assert_eq!(after.identities, vec![a_before]);
        // Durable: a new process finds A alone, as it was.
        drop((a, b, installation));
        let reopened =
            Installation::open(Box::new(dir.restart(CrashOutcome::ALL[0])), &passphrase())
                .unwrap()
                .0;
        assert_eq!(state(&reopened), after);
        // A copy of the vault is a backup of every identity in it, with its
        // keys, capabilities and contacts together.
        let copy = MemoryDir::new();
        let bytes = dir.read(VAULT, u64::MAX).unwrap().unwrap();
        copy.write_new(VAULT, &bytes).unwrap();
        let restored = Installation::open(Box::new(copy), &passphrase()).unwrap().0;
        assert_eq!(state(&restored), after);
    });
}

#[test]
fn a_new_identity_is_listed_only_once_it_is_durable() {
    // An identity being created is listed nowhere until its write is done,
    // also when the creation is cancelled; after a crash before that, it
    // does not exist.
    run(async {
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        dir.hold_writes();
        let released = common::Released(&dir);
        let creating = installation.clone();
        let create = tokio::spawn(async move {
            creating
                .restore_identity(keys(1, 1, 1, place(1)), None)
                .await
        });
        while !dir.write_held() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        let id = *keys_identity(1);
        assert!(installation.identities().is_empty());
        assert!(installation.identity(&id).is_none());
        let crashed = dir.restart(CrashOutcome::ALL[0]);
        create.abort();
        assert!(create.await.unwrap_err().is_cancelled());
        assert!(installation.identities().is_empty());
        drop(released);
        tokio::time::timeout(Duration::from_secs(10), async {
            while installation.identity(&id).is_none() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        let (reopened, _) = Installation::open(Box::new(crashed), &passphrase()).unwrap();
        assert!(reopened.identities().is_empty());
    });
}

fn keys_identity(seed: u8) -> Box<monolith_identity::IdentityPublicKey> {
    Box::new(IdentitySecretKey::from_seed(&[seed; 32]).public_key())
}

#[test]
fn the_rotation_of_one_identity_counts_for_no_other() {
    // A and B both rotate and hold Carol as accepted. A rotation number of
    // A handed to B records nothing at B.
    run(async {
        let network = MockNetwork::new();
        let installation = Installation::ephemeral();
        let (mut a, mut b) = two(&network, &installation).await;
        let mut carol = node(&network, 3).await;
        common::befriend(&a, &mut carol).await;
        common::befriend(&b, &mut carol).await;
        a.identity.begin_rotation().await.unwrap();
        b.identity.begin_rotation().await.unwrap();
        let of_a = a.identity.rotation_id().unwrap();
        assert_ne!(Some(of_a), b.identity.rotation_id());
        assert!(
            !b.identity
                .mark_announced(carol.identity.identity(), of_a)
                .await
                .unwrap()
        );
        assert!(!b.identity.switch_rotation(false).await.unwrap());
        let _ = (&mut a, &mut b);
    });
}

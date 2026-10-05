//! Several local identities in one installation: T-MI-1 to T-MI-9 of
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

use common::{
    Node, befriend, confirm_both, connect, dial_and_answer, keys, node, node_in, request, run,
    send_first, state,
};
use monolith_core::contacts::StoreError;
use monolith_core::identity::{Installation, InstallationError};
use monolith_core::link::LinkError;
use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::contact::{RecordKind, RequestMode};
use monolith_protocol::limits::MAX_UNKNOWN_SESSIONS;
use monolith_protocol::session::Standing;
use monolith_storage::dir::{CrashOutcome, MemoryDir, VAULT, VaultDir};
use monolith_storage::record::{Contents, StoredIdentity};
use monolith_storage::vault::{KdfParams, Passphrase, Vault};
use monolith_tor::MockNetwork;
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

        // B dials Carol: no message 3, so Carol sees the stream end where
        // it was due.
        let (dialed, answered) = connect(&b, &mut carol).await;
        assert!(dialed.is_err());
        assert_eq!(answered.err(), Some(LinkError::Stream));
        // Carol dials B: B admits nobody, and Carol's link ends.
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

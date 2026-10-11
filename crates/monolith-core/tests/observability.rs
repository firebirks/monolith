//! What a peer can tell about how it is held, through the contact store on
//! the in-memory Tor: T-ORACLE-6, the later sessions of T-ORACLE-7,
//! T-CONFIRM-1 with every fate of a request, T-CONTACT-1, T-CONTACT-3 and
//! T-INJ for the text the vault stores, of `docs/TEST_PLAN.md`.
//!
//! A peer's view of a session is the messages it receives, in order, and
//! whether its link ends after them. That the same messages are the same
//! bytes on the wire is T-ORACLE-8 in the session layer.

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
    Node, befriend, both, confirm_both, dial_and_answer, node, node_in, node_with, run, send_first,
};
use monolith_core::contacts::StoreError;
use monolith_core::identity::Installation;
use monolith_core::link::{Established, LinkError};
use monolith_core::requests::Dropped;
use monolith_protocol::body::Message;
use monolith_protocol::card::ContactCard;
use monolith_protocol::contact::{RecordKind, RequestMode};
use monolith_protocol::limits::{
    MAX_PENDING_CONTACT_REQUESTS, MAX_PENDING_REQUESTS_PER_INVITATION, MAX_UNKNOWN_SESSIONS,
};
use monolith_protocol::text::DisplayName;
use monolith_storage::dir::{MemoryDir, VAULT, VaultDir};
use monolith_storage::vault::{KdfParams, Passphrase};
use monolith_tor::MockNetwork;
use tokio::io::DuplexStream;

/// What a peer saw on one session.
#[derive(Debug, PartialEq, Eq)]
struct View {
    /// The messages it received, in order.
    seen: Vec<Message>,
    /// Whether its link ended after them.
    ended: bool,
}

async fn watch(end: &mut Established<DuplexStream>) -> View {
    let mut seen = Vec::new();
    let ended = loop {
        match end.link.receive().await {
            Ok(received) => seen.push(received.message.clone()),
            Err(_) => break true,
        }
    };
    View { seen, ended }
}

/// What a non-contact sees from Bob, whatever it sent first.
fn closed() -> View {
    View {
        seen: vec![Message::Close],
        ended: true,
    }
}

/// `from` dials `card` of `to` and sends what its record of `to` makes it
/// send first. Bob takes that message and applies it. Returns what `from`
/// saw and what Bob decided about a request, which `from` cannot see.
async fn session(
    from: &Node,
    to: &mut Node,
    card: &ContactCard,
) -> (View, Option<Result<(), Dropped>>) {
    let (asking, answering) = dial_and_answer(from, card, to).await;
    let (mut asking, mut answering) = (asking.unwrap(), answering.unwrap());
    send_first(&mut asking, &from.identity).await;
    let received = answering.link.receive().await.unwrap();
    let applied = to.identity.apply(&received).await.unwrap();
    (watch(&mut asking).await, applied.request)
}

/// `from` imports `card` of `to` and asks with it.
async fn ask(
    from: &Node,
    to: &mut Node,
    card: &ContactCard,
) -> (View, Option<Result<(), Dropped>>) {
    from.identity.import(card).await.unwrap();
    session(from, to, card).await
}

/// The identities that are not Bob's contacts, by `PROTOCOL.md` section
/// 12.1.
#[derive(Clone, Copy, Debug)]
enum Class {
    NoRecord,
    Blocked,
    Declined,
    Deleted,
    StaleCard,
    PendingKey,
}

const CLASSES: [Class; 6] = [
    Class::NoRecord,
    Class::Blocked,
    Class::Declined,
    Class::Deleted,
    Class::StaleCard,
    Class::PendingKey,
];

/// Makes the identity of `seed` a member of `class` at Bob, and returns a
/// node of that identity that holds Bob as a contact and so dials him.
async fn member(class: Class, network: &MockNetwork, bob: &mut Node, seed: u8) -> Node {
    let card = bob.card();
    let peer = match class {
        Class::NoRecord => node(network, seed).await,
        Class::Blocked => {
            let peer = node(network, seed).await;
            bob.identity.block(peer.identity.identity()).await.unwrap();
            peer
        }
        Class::Declined => {
            let peer = node(network, seed).await;
            bob.identity
                .set_request_mode(RequestMode::Open)
                .await
                .unwrap();
            assert_eq!(ask(&peer, bob, &card).await.1, Some(Ok(())));
            bob.identity
                .decline_request(common::request_of(&bob.identity, peer.identity.identity()))
                .await
                .unwrap();
            bob.identity
                .set_request_mode(RequestMode::Invitation)
                .await
                .unwrap();
            peer
        }
        Class::Deleted => {
            let peer = node(network, seed).await;
            befriend(&peer, bob).await;
            bob.identity.delete(peer.identity.identity()).await.unwrap();
            peer
        }
        Class::StaleCard => {
            // Bob holds the identity at epoch 2; this node presents
            // another key at epoch 1.
            let newer = node_with(network, seed, seed, 2).await;
            befriend(&newer, bob).await;
            node_with(network, seed, seed.wrapping_add(100), 1).await
        }
        Class::PendingKey => {
            // A newer key that the identity never announced.
            let older = node_with(network, seed, seed, 1).await;
            befriend(&older, bob).await;
            node_with(network, seed, seed.wrapping_add(100), 2).await
        }
    };
    if peer.identity.contact(bob.identity.identity()).is_none() {
        peer.identity.import(&card).await.unwrap();
    }
    peer
}

#[test]
fn t_oracle_6_with_the_stranger_budget_full_every_non_contact_is_treated_alike() {
    // For each class, Bob's budget for strangers is full of silent
    // strangers when a member of the class arrives: the oldest silent
    // stranger sees an ordinary Close, the newcomer takes its slot and
    // sees what any stranger sees.
    run(async {
        let mut outcomes = Vec::new();
        for class in CLASSES {
            let network = MockNetwork::new();
            let mut bob = node(&network, 2).await;
            let peer = member(class, &network, &mut bob, 7).await;
            let bob_card = bob.card();
            let mut held = Vec::new();
            for seed in 20..20 + u8::try_from(MAX_UNKNOWN_SESSIONS).unwrap() {
                let stranger = node(&network, seed).await;
                stranger.identity.import(&bob_card).await.unwrap();
                let (dialed, answered) = dial_and_answer(&stranger, &bob_card, &mut bob).await;
                // The node is kept with its link: closing its installation
                // would withdraw the link.
                held.push((dialed.unwrap(), answered.unwrap(), stranger));
            }
            assert_eq!(bob.identity.strangers().free(), 0, "{class:?}");

            let (dialed, answered) = dial_and_answer(&peer, &bob_card, &mut bob).await;
            let (mut dialed, mut answered) = (dialed.unwrap(), answered.unwrap());
            let took_a_slot = answered.link.holds_unknown_slot();
            let (oldest_dialed, oldest_answered, _) = &mut held[0];
            let evicted = oldest_answered.link.receive().await.err();
            let oldest_saw = watch(oldest_dialed).await;
            send_first(&mut dialed, &peer.identity).await;
            let (_, saw) = both(answered.link.receive(), watch(&mut dialed)).await;
            outcomes.push((took_a_slot, evicted, oldest_saw, saw));
            assert_eq!(
                bob.identity.strangers().held(),
                MAX_UNKNOWN_SESSIONS - 1,
                "{class:?}"
            );
        }
        let first = (true, Some(LinkError::Evicted), closed(), closed());
        for (class, outcome) in CLASSES.iter().zip(&outcomes) {
            assert_eq!(*outcome, first, "{class:?}");
        }
    });
}

#[test]
fn t_oracle_7_a_contact_blocked_or_deleted_in_a_session_sees_a_close_then_a_stranger() {
    run(async {
        for block in [true, false] {
            let network = MockNetwork::new();
            let alice = node(&network, 1).await;
            let mut bob = node(&network, 2).await;
            befriend(&alice, &mut bob).await;
            let (mut alice_link, mut bob_link) = confirm_both(&alice, &mut bob).await;
            if block {
                bob.identity.block(alice.identity.identity()).await.unwrap();
            } else {
                bob.identity
                    .delete(alice.identity.identity())
                    .await
                    .unwrap();
            }
            // The open session ends with a Close, as any session ends.
            let (bob_end, alice_saw) = both(bob_link.receive(), alice_link.receive()).await;
            assert_eq!(bob_end.err(), Some(LinkError::Withdrawn));
            assert_eq!(alice_saw.unwrap().message, Message::Close);
            assert!(alice_link.receive().await.is_err());

            // Alice still holds Bob as accepted and dials him: what she
            // sees is what an identity Bob never heard of sees.
            let card = bob.card();
            let (alice_view, decided) = session(&alice, &mut bob, &card).await;
            assert_eq!(alice_view, closed(), "block {block}");
            assert_eq!(decided, None);
            let carol = node(&network, 3).await;
            let (carol_view, _) = ask(&carol, &mut bob, &card).await;
            assert_eq!(alice_view, carol_view, "block {block}");
        }
    });
}

#[test]
fn t_confirm_1_every_fate_of_a_request_looks_the_same() {
    run(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        let mut ids = Vec::new();
        let mut cards = Vec::new();
        for _ in 0..5 {
            let (id, card) = bob.identity.create_invitation(None).await.unwrap();
            ids.push(id);
            cards.push(card);
        }
        let (revoked, revoked_card) = bob.identity.create_invitation(None).await.unwrap();
        bob.identity.revoke_invitation(revoked).await.unwrap();
        let mut seed = 10_u8;
        let mut next = || {
            seed += 1;
            seed
        };

        // Queued, with no contact session open at Bob.
        let first = node(&network, next()).await;
        let (queued, decided) = ask(&first, &mut bob, &cards[0]).await;
        assert_eq!(decided, Some(Ok(())));
        assert_eq!(queued, closed());
        let mut views = Vec::new();

        // Queued while a contact session is open at Bob.
        let carol = node(&network, next()).await;
        befriend(&carol, &mut bob).await;
        let (carol_link, bob_link) = confirm_both(&carol, &mut bob).await;
        let peer = node(&network, next()).await;
        let (view, decided) = ask(&peer, &mut bob, &cards[0]).await;
        assert_eq!(decided, Some(Ok(())));
        views.push(("queued beside a contact session", view));
        drop((carol_link, bob_link));

        // A capability that was revoked.
        let peer = node(&network, next()).await;
        let (view, decided) = ask(&peer, &mut bob, &revoked_card).await;
        assert_eq!(decided, Some(Err(Dropped::Capability)));
        views.push(("revoked capability", view));

        // The same identity again while its request is pending.
        let (view, decided) = ask(&first, &mut bob, &cards[0]).await;
        assert_eq!(decided, Some(Err(Dropped::AlreadyPending)));
        views.push(("already pending", view));

        // A blocked sender, and a declined one: the request is not even
        // looked at.
        let peer = node(&network, next()).await;
        bob.identity.block(peer.identity.identity()).await.unwrap();
        let (view, decided) = ask(&peer, &mut bob, &cards[0]).await;
        assert_eq!(decided, None);
        views.push(("blocked", view));
        let peer = node(&network, next()).await;
        assert_eq!(ask(&peer, &mut bob, &cards[1]).await.1, Some(Ok(())));
        bob.identity
            .decline_request(common::request_of(&bob.identity, peer.identity.identity()))
            .await
            .unwrap();
        let (view, decided) = ask(&peer, &mut bob, &cards[1]).await;
        assert_eq!(decided, None);
        views.push(("declined", view));

        // The quota of a capability, then the whole queue.
        while bob
            .identity
            .requests()
            .iter()
            .filter(|request| request.admitted_by == Some(ids[0]))
            .count()
            < MAX_PENDING_REQUESTS_PER_INVITATION
        {
            let peer = node(&network, next()).await;
            assert_eq!(ask(&peer, &mut bob, &cards[0]).await.1, Some(Ok(())));
        }
        let peer = node(&network, next()).await;
        let (view, decided) = ask(&peer, &mut bob, &cards[0]).await;
        assert_eq!(decided, Some(Err(Dropped::Quota)));
        views.push(("quota of the capability", view));
        let mut card = 1;
        while bob.identity.requests().len() < MAX_PENDING_CONTACT_REQUESTS {
            let peer = node(&network, next()).await;
            match ask(&peer, &mut bob, &cards[card]).await.1 {
                Some(Ok(())) => {}
                Some(Err(Dropped::Quota)) => card += 1,
                other => panic!("{other:?}"),
            }
        }
        let peer = node(&network, next()).await;
        let (view, decided) = ask(&peer, &mut bob, &cards[4]).await;
        assert_eq!(decided, Some(Err(Dropped::QueueFull)));
        views.push(("queue full", view));

        // A request mode that takes none.
        bob.identity
            .set_request_mode(RequestMode::Closed)
            .await
            .unwrap();
        let peer = node(&network, next()).await;
        let (view, decided) = ask(&peer, &mut bob, &cards[4]).await;
        assert_eq!(decided, Some(Err(Dropped::Mode)));
        views.push(("mode", view));

        for (fate, view) in views {
            assert_eq!(view, queued, "{fate}");
        }
    });
}

fn passphrase() -> Passphrase {
    Passphrase::new("observability test passphrase").unwrap()
}

#[test]
fn t_contact_1_a_flood_of_requests_leaves_the_vault_as_it_was() {
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let mut bob = node_in(installation, &network, 2, 2, 1).await;
        let mut cards = Vec::new();
        for _ in 0..MAX_PENDING_CONTACT_REQUESTS / MAX_PENDING_REQUESTS_PER_INVITATION + 1 {
            cards.push(bob.identity.create_invitation(None).await.unwrap().1);
        }
        let before = dir.read(VAULT, u64::MAX).unwrap().unwrap();
        let state = common::state(&bob.installation);

        let mut seed = 10_u8;
        for card in &cards {
            for _ in 0..MAX_PENDING_REQUESTS_PER_INVITATION + 1 {
                let peer = node(&network, seed).await;
                seed += 1;
                let (view, _) = ask(&peer, &mut bob, card).await;
                assert_eq!(view, closed());
            }
        }
        // The queue is at its bound, and nothing was written: the vault
        // is byte for byte the file it was, nonces included.
        assert_eq!(bob.identity.requests().len(), MAX_PENDING_CONTACT_REQUESTS);
        assert_eq!(dir.read(VAULT, u64::MAX).unwrap().unwrap(), before);
        assert_eq!(common::state(&bob.installation), state);
        assert!(bob.identity.known().is_empty());
    });
}

#[test]
fn t_contact_3_a_request_after_a_deletion_is_from_an_unknown_identity() {
    run(async {
        let network = MockNetwork::new();
        let alice = node(&network, 1).await;
        let mut bob = node(&network, 2).await;
        bob.identity
            .set_request_mode(RequestMode::Open)
            .await
            .unwrap();
        befriend(&alice, &mut bob).await;
        bob.identity
            .delete(alice.identity.identity())
            .await
            .unwrap();
        assert_eq!(
            bob.identity.kind(alice.identity.identity()),
            RecordKind::None
        );
        assert!(bob.identity.contact(alice.identity.identity()).is_none());

        // Another identity asks under the name Alice had: a request of its
        // own, nothing of Alice's.
        let card = bob.card();
        let mallory = node(&network, 9).await;
        assert_eq!(ask(&mallory, &mut bob, &card).await.1, Some(Ok(())));
        // Alice asks again: she is an identity Bob does not know.
        alice
            .identity
            .delete(bob.identity.identity())
            .await
            .unwrap();
        assert_eq!(ask(&alice, &mut bob, &card).await.1, Some(Ok(())));
        let mut askers: Vec<_> = bob
            .identity
            .requests()
            .iter()
            .map(|request| *request.identity())
            .collect();
        askers.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        let mut expected = vec![*alice.identity.identity(), *mallory.identity.identity()];
        expected.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));
        assert_eq!(askers, expected);
        assert!(bob.identity.known().is_empty());
        // Accepting her makes a new contact, with nothing of the old one.
        bob.identity
            .accept_request(common::request_of(&bob.identity, alice.identity.identity()))
            .await
            .unwrap();
        let view = bob.identity.contact(alice.identity.identity()).unwrap();
        assert_eq!(view.kind, RecordKind::Accepted);
        let held = view.credentials.unwrap();
        assert!(held.retired().is_none() && held.pending_successor().is_none());
        assert!(!view.verified);
    });
}

#[test]
fn t_inj_text_the_vault_stores_comes_back_byte_for_byte() {
    // The local labels of identities and invitations are the text a vault
    // stores. Text that would mean something elsewhere (a delimiter, a
    // quote, SQL, a path, a format directive, an escape sequence) is
    // either refused by its type or kept exactly as it was given, and
    // touches nothing else.
    run(async {
        let refused = [
            "line\nbreak",
            "carriage\rreturn",
            "escape \u{1b}[31m",
            "nul\0",
        ];
        for text in refused {
            assert!(DisplayName::new(text).is_err(), "{text:?}");
        }
        let kept = [
            "Robert'); DROP TABLE contacts;--",
            "../../etc/passwd",
            "C:\\Windows\\system32",
            "%s %n {} {0} $HOME `id`",
            "\"quoted\" <b>bold</b> & more",
            "a,b;c|d=e",
        ];
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let mut labels = Vec::new();
        for (index, text) in kept.iter().enumerate() {
            let label = DisplayName::new(text).unwrap();
            let seed = u8::try_from(index).unwrap() + 1;
            let keys = common::keys(seed, seed, 1, common::fixed_endpoint(seed));
            let local = installation
                .restore_identity(keys, Some(label.clone()))
                .await
                .unwrap();
            let _ = local.create_invitation(Some(label.clone())).await.unwrap();
            labels.push((*local.identity(), label));
        }
        let state = common::state(&installation);
        drop(installation);

        let (reopened, _) = Installation::open(Box::new(dir), &passphrase()).unwrap();
        assert_eq!(common::state(&reopened), state);
        for (identity, label) in labels {
            let local = reopened.identity(&identity).unwrap();
            let stored = local.label().unwrap();
            assert_eq!(stored.as_str().as_bytes(), label.as_str().as_bytes());
            let invitations: Vec<_> = local.invitations().into_iter().map(|(_, l)| l).collect();
            assert_eq!(invitations, vec![Some(label)]);
        }
    });
}

#[test]
fn a_confirmation_is_for_a_contact_only() {
    // A blocked, a declined and an unknown identity have no pending key to
    // confirm and no card to dial: both confirmations find nothing, and
    // nothing changes.
    run(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        let card = bob.card();
        let blocked = node(&network, 3).await;
        bob.identity
            .block(blocked.identity.identity())
            .await
            .unwrap();
        let declined = node(&network, 4).await;
        bob.identity
            .set_request_mode(RequestMode::Open)
            .await
            .unwrap();
        assert_eq!(ask(&declined, &mut bob, &card).await.1, Some(Ok(())));
        bob.identity
            .decline_request(common::request_of(
                &bob.identity,
                declined.identity.identity(),
            ))
            .await
            .unwrap();
        let unknown = node(&network, 5).await;
        let state = common::state(&bob.installation);
        for peer in [&blocked, &declined, &unknown] {
            let peers = peer.card();
            assert_eq!(
                bob.identity.confirm_pending(&peers).await.err(),
                Some(StoreError::NotFound)
            );
            assert_eq!(
                bob.identity.confirm_dial(&peers).await.err(),
                Some(StoreError::NotFound)
            );
        }
        assert_eq!(common::state(&bob.installation), state);
        assert_eq!(
            bob.identity.kind(blocked.identity.identity()),
            RecordKind::Blocked
        );
        assert_eq!(
            bob.identity.kind(declined.identity.identity()),
            RecordKind::Declined
        );
    });
}

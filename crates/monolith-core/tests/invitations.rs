//! Invitation capabilities and contact requests through the contact store,
//! on the in-memory Tor: T-INV-1 to T-INV-11 of `docs/TEST_PLAN.md`.

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
    Node, Released, befriend, chat, confirm_both, connect, dial_and_answer, held, node, node_in,
    run, send_first,
};
use monolith_core::contacts::StoreError;
use monolith_core::identity::Installation;
use monolith_core::requests::Dropped;
use monolith_identity::{EndpointEpoch, IdentitySecretKey};
use monolith_protocol::body::Message;
use monolith_protocol::card::EndpointSet;
use monolith_protocol::card::{ContactCard, InvitationCapability};
use monolith_protocol::contact::{RecordKind, RequestMode};
use monolith_protocol::limits::{
    MAX_ACTIVE_INVITATIONS, MAX_CONTACTS, MAX_PENDING_REQUESTS_PER_INVITATION,
};
use monolith_protocol::session::Action;
use monolith_protocol::text::DisplayName;
use monolith_session::{LocalParty, TransportSecretKey};
use monolith_storage::dir::{CrashOutcome, MemoryDir};
use monolith_storage::vault::{KdfParams, Passphrase};
use monolith_tor::MockNetwork;

/// What the requester saw, and what became of its request at Bob.
#[derive(Debug, PartialEq, Eq)]
struct Asked {
    /// The messages the requester received, in order.
    seen: Vec<Message>,
    /// Whether its link ended after them.
    ended: bool,
    /// What Bob decided, which the requester cannot see.
    decided: Option<Result<(), Dropped>>,
}

/// `from` imports `card` of `to`, dials it, and sends its request with the
/// capability the card carries.
async fn ask(from: &Node, to: &mut Node, card: &ContactCard) -> Asked {
    from.identity.import(card).await.unwrap();
    let (asking, answering) = dial_and_answer(from, card, to).await;
    let (mut asking, mut answering) = (asking.unwrap(), answering.unwrap());
    assert_eq!(asking.first, vec![Action::SendContactRequest]);
    send_first(&mut asking, &from.identity).await;
    let received = answering.link.receive().await.unwrap();
    let applied = to
        .identity
        .apply(answering.link.session_ref(), &received)
        .await
        .unwrap();
    let mut seen = Vec::new();
    let ended = loop {
        match asking.link.receive().await {
            Ok(received) => seen.push(received.message),
            Err(_) => break true,
        }
    };
    Asked {
        seen,
        ended,
        decided: applied.request,
    }
}

/// What a requester sees whatever Bob decided: one Close, then the end.
fn closed(decided: Option<Result<(), Dropped>>) -> Asked {
    Asked {
        seen: vec![Message::Close],
        ended: true,
        decided,
    }
}

#[test]
fn t_inv_1_to_4_cards_with_capabilities_admit_requests_until_revoked() {
    run(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        // T-INV-1: two cards with capabilities A and B; both are valid,
        // both capabilities are in the set, and a request with either is
        // queued.
        let (a, card_a) = bob.identity.create_invitation(None).await.unwrap();
        let (b, card_b) = bob.identity.create_invitation(None).await.unwrap();
        assert!(card_a.invitation().is_some() && card_b.invitation().is_some());
        assert_ne!(card_a.invitation(), card_b.invitation());
        assert_eq!(card_a.identity(), card_b.identity());
        assert_eq!(bob.identity.invitations().len(), 2);
        let (alice, carol) = (node(&network, 1).await, node(&network, 3).await);
        assert_eq!(ask(&alice, &mut bob, &card_a).await, closed(Some(Ok(()))));
        assert_eq!(ask(&carol, &mut bob, &card_b).await, closed(Some(Ok(()))));

        // T-INV-2 and 3: A revoked. A request with A is dropped, one with B
        // queued.
        bob.identity.revoke_invitation(a).await.unwrap();
        let (dave, erin) = (node(&network, 4).await, node(&network, 5).await);
        assert_eq!(
            ask(&dave, &mut bob, &card_a).await,
            closed(Some(Err(Dropped::Capability)))
        );
        assert_eq!(ask(&erin, &mut bob, &card_b).await, closed(Some(Ok(()))));

        // T-INV-4: B is not used up. Requests with B from more identities
        // are queued up to the quota per capability, and B is still valid
        // after the requests were handled.
        let mut queued_with_b = 2;
        let mut seed = 10;
        while queued_with_b < MAX_PENDING_REQUESTS_PER_INVITATION {
            let peer = node(&network, seed).await;
            assert_eq!(ask(&peer, &mut bob, &card_b).await, closed(Some(Ok(()))));
            queued_with_b += 1;
            seed += 1;
        }
        let over = node(&network, seed).await;
        assert_eq!(
            ask(&over, &mut bob, &card_b).await,
            closed(Some(Err(Dropped::Quota)))
        );
        for request in bob.identity.requests() {
            bob.identity
                .decline_request(request.identity())
                .await
                .unwrap();
        }
        assert!(bob.identity.requests().is_empty());
        let late = node(&network, 40).await;
        assert_eq!(ask(&late, &mut bob, &card_b).await, closed(Some(Ok(()))));
        assert!(bob.identity.invitations().iter().any(|(id, _)| *id == b));
    });
}

#[test]
fn t_inv_5_and_6_every_dropped_request_looks_like_a_queued_one() {
    run(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        let (a, card_a) = bob.identity.create_invitation(None).await.unwrap();
        let (_, card_b) = bob.identity.create_invitation(None).await.unwrap();
        bob.identity.revoke_invitation(a).await.unwrap();
        // A capability that was never issued, and one of another identity.
        let never = ContactCard::sign(
            &IdentitySecretKey::from_seed(&[2; 32]),
            *card_b.transport(),
            card_b.epoch(),
            card_b.endpoints().clone(),
            Some(InvitationCapability::from_bytes([0x5A; 16])),
        )
        .unwrap();
        let mut carol = node(&network, 3).await;
        let (_, carols) = carol.identity.create_invitation(None).await.unwrap();
        let foreign = ContactCard::sign(
            &IdentitySecretKey::from_seed(&[2; 32]),
            *card_b.transport(),
            card_b.epoch(),
            card_b.endpoints().clone(),
            carols.invitation().cloned(),
        )
        .unwrap();
        let valid = ask(&node(&network, 10).await, &mut bob, &card_b).await;
        let revoked = ask(&node(&network, 11).await, &mut bob, &card_a).await;
        let unknown = ask(&node(&network, 12).await, &mut bob, &never).await;
        let other = ask(&node(&network, 13).await, &mut bob, &foreign).await;
        assert_eq!(valid.decided, Some(Ok(())));
        for dropped in [&revoked, &unknown, &other] {
            assert_eq!(dropped.decided, Some(Err(Dropped::Capability)));
            // The same messages, in the same order, and the same end.
            assert_eq!(dropped.seen, valid.seen);
            assert_eq!(dropped.ended, valid.ended);
        }
        let _ = &mut carol;
    });
}

#[test]
fn t_inv_7_a_contact_accepted_from_a_revoked_capability_stays_accepted() {
    run(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        let (a, card_a) = bob.identity.create_invitation(None).await.unwrap();
        let mut alice = node(&network, 1).await;
        assert_eq!(ask(&alice, &mut bob, &card_a).await.decided, Some(Ok(())));
        bob.identity
            .accept_request(alice.identity.identity())
            .await
            .unwrap();
        // The next session confirms, and stays open.
        let (mut alice_link, mut bob_link) = confirm_both(&alice, &mut bob).await;
        let before = bob.identity.contact(alice.identity.identity()).unwrap();
        bob.identity.revoke_invitation(a).await.unwrap();
        // Record and session unchanged.
        assert_eq!(
            bob.identity.contact(alice.identity.identity()).unwrap(),
            before
        );
        alice_link.send(&chat("still a contact")).await.unwrap();
        assert_eq!(
            bob_link.receive().await.unwrap().message,
            chat("still a contact")
        );
        drop((alice_link, bob_link));
        // And the next session is confirmed as well.
        let (alice_link, _) = confirm_both(&alice, &mut bob).await;
        assert!(!alice_link.is_over());
        let _ = &mut alice;
    });
}

#[test]
fn t_inv_8_a_full_active_set_refuses_and_revokes_nothing() {
    run(async {
        let network = MockNetwork::new();
        let bob = node(&network, 2).await;
        let mut cards = Vec::new();
        for _ in 0..MAX_ACTIVE_INVITATIONS {
            cards.push(bob.identity.create_invitation(None).await.unwrap());
        }
        assert_eq!(
            bob.identity.create_invitation(None).await.err(),
            Some(StoreError::Full)
        );
        assert_eq!(bob.identity.invitations().len(), MAX_ACTIVE_INVITATIONS);
        for (id, card) in &cards {
            assert_eq!(&bob.identity.invitation_card(*id).unwrap(), card);
        }
        // Room again after one is revoked.
        bob.identity.revoke_invitation(cards[0].0).await.unwrap();
        assert!(bob.identity.create_invitation(None).await.is_ok());
    });
}

#[test]
fn t_inv_9_a_label_is_never_in_a_card() {
    run(async {
        let network = MockNetwork::new();
        let bob = node(&network, 2).await;
        let label = "Private QR for Alice";
        let (id, card) = bob
            .identity
            .create_invitation(Some(DisplayName::new(label).unwrap()))
            .await
            .unwrap();
        let bytes = card.encode();
        assert!(
            !bytes
                .windows(label.len())
                .any(|window| window == label.as_bytes())
        );
        assert!(!card.to_text().contains(label));
        assert_eq!(
            bob.identity.invitations(),
            vec![(id, Some(DisplayName::new(label).unwrap()))]
        );
    });
}

#[test]
fn t_inv_10_another_capability_for_the_same_statement() {
    run(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        bob.identity
            .set_request_mode(RequestMode::Invitation)
            .await
            .unwrap();
        let (_, card_a) = bob.identity.create_invitation(None).await.unwrap();
        let (_, card_b) = bob.identity.create_invitation(None).await.unwrap();
        // A requested contact: the import of the same statement with B
        // replaces A, and the next request carries B.
        let alice = node(&network, 1).await;
        alice.identity.import(&card_a).await.unwrap();
        alice.identity.import(&card_b).await.unwrap();
        let held = alice
            .identity
            .contact(bob.identity.identity())
            .unwrap()
            .credentials
            .unwrap();
        assert_eq!(held.invitation(), card_b.invitation());
        assert_eq!(ask(&alice, &mut bob, &card_b).await.decided, Some(Ok(())));
        // Without a capability, the next request has none.
        let plain = bob.identity.card();
        alice.identity.import(&plain).await.unwrap();
        assert_eq!(
            alice
                .identity
                .contact(bob.identity.identity())
                .unwrap()
                .credentials
                .unwrap()
                .invitation(),
            None
        );

        // An accepted contact: the same import changes neither the contact
        // nor its sessions.
        let mut carol = node(&network, 3).await;
        befriend(&carol, &mut bob).await;
        let (carol_link, bob_link) = confirm_both(&carol, &mut bob).await;
        let before = carol.identity.contact(bob.identity.identity()).unwrap();
        carol.identity.import(&card_a).await.unwrap();
        assert_eq!(
            carol.identity.contact(bob.identity.identity()).unwrap(),
            before
        );
        assert!(!carol_link.is_withdrawn() && !bob_link.is_withdrawn());
        let _ = &mut carol;
    });
}

#[test]
fn t_inv_11_revoking_keeps_pending_requests_unless_asked_to_discard_them() {
    run(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        let (a, card_a) = bob.identity.create_invitation(None).await.unwrap();
        let (b, card_b) = bob.identity.create_invitation(None).await.unwrap();
        let mut by_a = Vec::new();
        for seed in 10..13 {
            let peer = node(&network, seed).await;
            assert_eq!(ask(&peer, &mut bob, &card_a).await.decided, Some(Ok(())));
            by_a.push(*peer.identity.identity());
        }
        let peer_b = node(&network, 20).await;
        assert_eq!(ask(&peer_b, &mut bob, &card_b).await.decided, Some(Ok(())));
        // Revoking alone leaves the queue as it is.
        bob.identity.revoke_invitation(a).await.unwrap();
        assert_eq!(bob.identity.requests().len(), 4);
        // Revoking and discarding removes exactly the requests B admitted.
        assert_eq!(bob.identity.revoke_and_discard(b).await.unwrap(), 1);
        let left: Vec<_> = bob
            .identity
            .requests()
            .iter()
            .map(|request| *request.identity())
            .collect();
        assert_eq!(left, by_a);
        assert!(
            bob.identity
                .requests()
                .iter()
                .all(|request| request.admitted_by.is_some())
        );
    });
}

#[test]
fn request_modes_decide_what_a_stranger_can_ask() {
    run(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        let (_, card_a) = bob.identity.create_invitation(None).await.unwrap();
        let open = bob.identity.card();
        // Invitation mode, the default: a request without a capability is
        // dropped.
        assert_eq!(bob.identity.request_mode(), RequestMode::Invitation);
        assert_eq!(
            ask(&node(&network, 10).await, &mut bob, &open).await,
            closed(Some(Err(Dropped::Mode)))
        );
        // Open mode: considered without one.
        bob.identity
            .set_request_mode(RequestMode::Open)
            .await
            .unwrap();
        assert_eq!(
            ask(&node(&network, 11).await, &mut bob, &open).await,
            closed(Some(Ok(())))
        );
        // Closed mode: nothing is considered.
        bob.identity
            .set_request_mode(RequestMode::Closed)
            .await
            .unwrap();
        assert_eq!(
            ask(&node(&network, 12).await, &mut bob, &card_a).await,
            closed(Some(Err(Dropped::Mode)))
        );
        // A declined or blocked identity is dropped in every mode.
        bob.identity
            .set_request_mode(RequestMode::Open)
            .await
            .unwrap();
        let declined = node(&network, 13).await;
        assert_eq!(ask(&declined, &mut bob, &open).await.decided, Some(Ok(())));
        bob.identity
            .decline_request(declined.identity.identity())
            .await
            .unwrap();
        assert_eq!(
            bob.identity.kind(declined.identity.identity()),
            RecordKind::Declined
        );
        // It still holds Bob as requested and asks again.
        let (asking, answering) = connect(&declined, &mut bob).await;
        let (mut asking, mut answering) = (asking.unwrap(), answering.unwrap());
        send_first(&mut asking, &declined.identity).await;
        let received = answering.link.receive().await.unwrap();
        // The session logic does not even ask the store: only a request
        // from an identity without a record is considered. The sender sees
        // the same Close.
        assert_eq!(received.actions, vec![Action::SendClose]);
        let applied = bob
            .identity
            .apply(answering.link.session_ref(), &received)
            .await
            .unwrap();
        assert_eq!(applied.request, None);
        assert_eq!(asking.link.receive().await.unwrap().message, Message::Close);
        assert_eq!(
            bob.identity.kind(declined.identity.identity()),
            RecordKind::Declined
        );
    });
}

#[test]
fn an_invitation_is_handed_out_only_once_it_is_durable() {
    // The capability is in the active set in memory before its write is
    // done. Until then it is listed nowhere, no card carries it and it
    // admits no request: a crash would lose it, and a card handed out for
    // it would admit nothing after the restart. A creation that is
    // cancelled while its write is pending hands out nothing either; the
    // capability shows up once the write is done, and never after a crash
    // before it.
    run(async {
        let network = MockNetwork::new();
        let dir = MemoryDir::new();
        let passphrase = Passphrase::new("invitations test").unwrap();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase, KdfParams::FLOOR).unwrap();
        let bob = node_in(installation, &network, 2, 2, 1).await;

        dir.hold_writes();
        let released = Released(&dir);
        let identity = bob.identity.clone();
        let create = tokio::spawn(async move { identity.create_invitation(None).await });
        held(&dir).await;
        assert!(bob.identity.invitations().is_empty());
        let crashed = dir.restart(CrashOutcome::ALL[0]);
        create.abort();
        assert!(create.await.unwrap_err().is_cancelled());
        assert!(bob.identity.invitations().is_empty());
        drop(released);
        tokio::time::timeout(core::time::Duration::from_secs(10), async {
            while bob.identity.invitations().is_empty() {
                tokio::time::sleep(core::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap();
        let (id, _) = bob.identity.invitations()[0].clone();
        assert!(bob.identity.invitation_card(id).is_ok());

        let (reopened, _) = Installation::open(Box::new(crashed), &passphrase).unwrap();
        assert!(reopened.identities()[0].invitations().is_empty());
    });
}

/// The card of contact number `n`, an identity of no node.
fn filler(n: usize) -> ContactCard {
    let mut seed = [0xF1_u8; 32];
    seed[..8].copy_from_slice(&(n as u64).to_be_bytes());
    let mut transport = [0xF2_u8; 32];
    transport[..8].copy_from_slice(&(n as u64).to_be_bytes());
    LocalParty::issue(
        &IdentitySecretKey::from_seed(&seed),
        TransportSecretKey::from_bytes(&transport).unwrap(),
        EndpointEpoch::FIRST,
        EndpointSet::single(common::elsewhere()),
    )
    .unwrap()
    .card()
    .clone()
}

#[test]
fn a_request_stays_pending_when_answering_it_fails() {
    // Accepting a request while the contact list is full fails. The
    // request is not lost: it waits as before, and the user can still
    // decline it, which then takes it out.
    run(async {
        let network = MockNetwork::new();
        let mut bob = node(&network, 2).await;
        bob.identity
            .set_request_mode(RequestMode::Open)
            .await
            .unwrap();
        let open = bob.identity.card();
        let carol = node(&network, 3).await;
        assert_eq!(ask(&carol, &mut bob, &open).await.decided, Some(Ok(())));
        for n in 0..MAX_CONTACTS {
            bob.identity.import(&filler(n)).await.unwrap();
        }
        assert_eq!(
            bob.identity.accept_request(carol.identity.identity()).await,
            Err(StoreError::Full)
        );
        assert_eq!(bob.identity.requests().len(), 1);
        assert_eq!(
            bob.identity.kind(carol.identity.identity()),
            RecordKind::None
        );
        bob.identity
            .decline_request(carol.identity.identity())
            .await
            .unwrap();
        assert!(bob.identity.requests().is_empty());
        assert_eq!(
            bob.identity.kind(carol.identity.identity()),
            RecordKind::Declined
        );
    });
}

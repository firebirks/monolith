//! What the runtime offers a front end since Phase 6
//! (`docs/DESIGN_QUESTIONS.md` P6-4), over the in-memory Tor: contact
//! requests and invitations by handles that name one request or invitation
//! of one instance of an identity, the local name and the verification of a
//! contact record, the request mode, the state of the Onion Service, a
//! check of Tor, and whether an added card carried an invitation. A handle
//! of something answered, revoked, made again or of another identity names
//! nothing.
//!
//! Time is paused and advances only when every task waits.

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
use monolith_core::api::{
    CommandError, ContactId, Event, IdentityId, MessageStatus, RequestMode, ServiceState, TorCheck,
};
use monolith_core::budget::Budgets;
use monolith_core::identity::{Installation, LocalIdentity};
use monolith_core::runtime::{Runtime, RuntimeOptions};
use monolith_protocol::card::ContactCard;
use monolith_protocol::limits::MAX_ACTIVE_INVITATIONS;
use monolith_storage::dir::{CrashOutcome, MemoryDir};
use monolith_storage::vault::{KdfParams, Passphrase};
use monolith_tor::{Bootstrap, MockNetwork, MockTorBackend};
use tokio::sync::broadcast;

/// One installation with one identity of fixed keys, and its runtime.
struct Peer {
    seed: u8,
    identity: Arc<LocalIdentity>,
    runtime: Runtime<MockTorBackend>,
}

fn options() -> RuntimeOptions {
    RuntimeOptions {
        startup_spread: Duration::ZERO,
        ..RuntimeOptions::default()
    }
}

fn passphrase() -> Passphrase {
    Passphrase::new("correct horse battery staple").unwrap()
}

impl Peer {
    async fn start(network: &MockNetwork, seed: u8) -> Self {
        Self::start_in(network, seed, Installation::ephemeral()).await
    }

    async fn start_in(network: &MockNetwork, seed: u8, installation: Installation) -> Self {
        let identity = installation
            .restore_identity(keys(seed, seed, 1, fixed_endpoint(seed)), None)
            .await
            .unwrap();
        let runtime = Runtime::start(installation, network.backend(), Budgets::new(), options());
        until(|| network.is_published(&fixed_endpoint(seed))).await;
        Self {
            seed,
            identity,
            runtime,
        }
    }

    fn id(&self) -> IdentityId {
        self.identity.id()
    }

    fn card(&self) -> String {
        self.runtime.own_card(&self.id()).unwrap()
    }
}

/// Waits, with paused time, until `condition` holds, for up to an hour of
/// test time.
async fn until(condition: impl Fn() -> bool) {
    for _ in 0..36_000 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the condition did not come about");
}

/// Waits for an event that `matches`, for up to an hour of test time.
async fn event(events: &mut broadcast::Receiver<Event>, matches: impl Fn(&Event) -> bool) {
    let wait = async {
        loop {
            match events.recv().await {
                Ok(event) if matches(&event) => return,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => panic!("the runtime stopped"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(3600), wait)
        .await
        .expect("the event came");
}

/// `a` imports `card` and sends a first message, which takes its request
/// to the identity of the card. Returns the record `a` made.
async fn request_with_card(a: &Peer, card: &str) -> ContactId {
    let contact = a.runtime.add_contact(&a.id(), card).await.unwrap().contact;
    a.runtime
        .send_message(&a.id(), &contact, "hello")
        .await
        .unwrap();
    contact
}

#[test]
fn a_request_is_listed_with_the_senders_fingerprint_and_accepted_by_its_handle() {
    run_paused(async {
        let network = MockNetwork::new();
        let a = Peer::start(&network, 1).await;
        let b = Peer::start(&network, 2).await;
        b.runtime
            .set_request_mode(&b.id(), RequestMode::Open)
            .await
            .unwrap();
        assert_eq!(b.runtime.request_mode(&b.id()).unwrap(), RequestMode::Open);
        let mut events = b.runtime.subscribe();
        let at_a = request_with_card(&a, &b.card()).await;
        event(&mut events, |event| {
            matches!(event, Event::RequestReceived { queued: true, .. })
        })
        .await;

        let requests = b.runtime.requests(&b.id()).unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        let own = a.runtime.identities();
        assert_eq!(request.fingerprint, own[0].fingerprint);
        assert_eq!(request.full_fingerprint, own[0].full_fingerprint);
        assert!(request.invitation.is_none());
        // Monolith sends its own requests with an empty name and
        // introduction (P6-5).
        assert!(request.display_name.is_empty());
        assert!(request.introduction.is_empty());

        let contact = b.runtime.accept_request(&request.request).await.unwrap();
        assert_eq!(Some(contact), b.identity.contact_id(a.identity.identity()));
        assert!(b.runtime.requests(&b.id()).unwrap().is_empty());
        // Answered: the handle names nothing any more.
        assert_eq!(
            b.runtime.accept_request(&request.request).await,
            Err(CommandError::UnknownRequest)
        );
        // The first message reaches B once A dials again as a contact.
        until(|| {
            a.runtime.messages(&a.id(), &at_a, None, 10).unwrap()[0].status
                == MessageStatus::Delivered
        })
        .await;
    });
}

#[test]
fn a_request_handle_of_one_identity_names_nothing_at_another() {
    run_paused(async {
        let network = MockNetwork::new();
        let a = Peer::start(&network, 1).await;
        let b = Peer::start(&network, 2).await;
        let c = Peer::start(&network, 3).await;
        for peer in [&b, &c] {
            peer.runtime
                .set_request_mode(&peer.id(), RequestMode::Open)
                .await
                .unwrap();
        }
        let mut events = b.runtime.subscribe();
        request_with_card(&a, &b.card()).await;
        event(&mut events, |event| {
            matches!(event, Event::RequestReceived { queued: true, .. })
        })
        .await;
        let mut request = b.runtime.requests(&b.id()).unwrap()[0].request;
        // The same request number, said to be of another identity.
        request.identity = c.id();
        // C has no request of that number; B does not hold C.
        assert_eq!(
            c.runtime.accept_request(&request).await,
            Err(CommandError::UnknownRequest)
        );
        assert_eq!(
            b.runtime.accept_request(&request).await,
            Err(CommandError::UnknownIdentity)
        );
        // And told to the runtime that holds the other identity, which has
        // a request of its own number space: nothing of B is answered.
        let mut events = c.runtime.subscribe();
        request_with_card(&a, &c.card()).await;
        event(&mut events, |event| {
            matches!(event, Event::RequestReceived { queued: true, .. })
        })
        .await;
        let at_c = c.runtime.requests(&c.id()).unwrap()[0].request;
        let mut crossed = b.runtime.requests(&b.id()).unwrap()[0].request;
        crossed.identity = c.id();
        if crossed != at_c {
            assert_eq!(
                c.runtime.decline_request(&crossed).await,
                Err(CommandError::UnknownRequest)
            );
        }
        assert_eq!(b.runtime.requests(&b.id()).unwrap().len(), 1);
        assert_eq!(c.runtime.requests(&c.id()).unwrap().len(), 1);
    });
}

#[test]
fn declining_and_blocking_answer_only_the_named_request() {
    run_paused(async {
        let network = MockNetwork::new();
        let a = Peer::start(&network, 1).await;
        let d = Peer::start(&network, 4).await;
        let b = Peer::start(&network, 2).await;
        b.runtime
            .set_request_mode(&b.id(), RequestMode::Open)
            .await
            .unwrap();
        let mut events = b.runtime.subscribe();
        request_with_card(&a, &b.card()).await;
        request_with_card(&d, &b.card()).await;
        event(&mut events, |event| {
            matches!(event, Event::RequestReceived { queued: true, .. })
        })
        .await;
        event(&mut events, |event| {
            matches!(event, Event::RequestReceived { queued: true, .. })
        })
        .await;
        let requests = b.runtime.requests(&b.id()).unwrap();
        assert_eq!(requests.len(), 2);
        let of = |peer: &Peer| {
            let fingerprint = &peer.runtime.identities()[0].fingerprint;
            requests
                .iter()
                .find(|request| &request.fingerprint == fingerprint)
                .unwrap()
                .request
        };
        b.runtime.decline_request(&of(&a)).await.unwrap();
        let left = b.runtime.requests(&b.id()).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].request, of(&d));
        b.runtime.block_request(&of(&d)).await.unwrap();
        assert!(b.runtime.requests(&b.id()).unwrap().is_empty());
        assert_eq!(
            b.runtime.decline_request(&of(&a)).await,
            Err(CommandError::UnknownRequest)
        );
        assert!(b.identity.contact_id(a.identity.identity()).is_none());
        assert!(b.identity.contact_id(d.identity.identity()).is_none());
    });
}

#[test]
fn an_invitation_is_listed_shown_again_and_revoked_by_its_handle() {
    run_paused(async {
        let network = MockNetwork::new();
        let b = Peer::start(&network, 2).await;
        let c = Peer::start(&network, 3).await;
        let (invitation, card) = b
            .runtime
            .create_invitation(&b.id(), Some("Friends"))
            .await
            .unwrap();
        let parsed = ContactCard::from_text(&card).unwrap();
        assert!(parsed.invitation().is_some());
        assert_eq!(parsed.identity(), b.identity.identity());
        let listed = b.runtime.invitations(&b.id()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].invitation, invitation);
        assert_eq!(listed[0].label.as_deref(), Some("Friends"));
        assert_eq!(b.runtime.invitation_card(&invitation).unwrap(), card);

        // Of B, said to be of C: nothing at C, which has none.
        let mut crossed = invitation;
        crossed.identity = c.id();
        assert_eq!(
            c.runtime.invitation_card(&crossed),
            Err(CommandError::UnknownInvitation)
        );
        let (own, _) = c.runtime.create_invitation(&c.id(), None).await.unwrap();
        if own != crossed {
            assert_eq!(
                c.runtime.revoke_invitation(&crossed, false).await,
                Err(CommandError::UnknownInvitation)
            );
        }
        assert_eq!(c.runtime.invitations(&c.id()).unwrap().len(), 1);

        assert_eq!(b.runtime.revoke_invitation(&invitation, false).await, Ok(0));
        assert!(b.runtime.invitations(&b.id()).unwrap().is_empty());
        assert_eq!(
            b.runtime.invitation_card(&invitation),
            Err(CommandError::UnknownInvitation)
        );
        assert_eq!(
            b.runtime.revoke_invitation(&invitation, false).await,
            Err(CommandError::UnknownInvitation)
        );
    });
}

#[test]
fn a_full_set_of_invitations_refuses_another_and_revokes_nothing() {
    run_paused(async {
        let network = MockNetwork::new();
        let b = Peer::start(&network, 2).await;
        for _ in 0..MAX_ACTIVE_INVITATIONS {
            b.runtime.create_invitation(&b.id(), None).await.unwrap();
        }
        assert_eq!(
            b.runtime.create_invitation(&b.id(), None).await,
            Err(CommandError::Full)
        );
        assert_eq!(
            b.runtime.invitations(&b.id()).unwrap().len(),
            MAX_ACTIVE_INVITATIONS
        );
    });
}

#[test]
fn revoking_and_discarding_drops_the_requests_of_that_invitation_only() {
    run_paused(async {
        let network = MockNetwork::new();
        let a = Peer::start(&network, 1).await;
        let b = Peer::start(&network, 2).await;
        // Invitation mode, the default: only a request with an invitation
        // is considered.
        assert_eq!(
            b.runtime.request_mode(&b.id()).unwrap(),
            RequestMode::Invitation
        );
        let (invitation, card) = b
            .runtime
            .create_invitation(&b.id(), Some("A"))
            .await
            .unwrap();
        let mut events = b.runtime.subscribe();
        request_with_card(&a, &card).await;
        event(&mut events, |event| {
            matches!(event, Event::RequestReceived { queued: true, .. })
        })
        .await;
        let requests = b.runtime.requests(&b.id()).unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].invitation, Some(invitation));
        assert_eq!(b.runtime.revoke_invitation(&invitation, true).await, Ok(1));
        assert!(b.runtime.requests(&b.id()).unwrap().is_empty());
        assert_eq!(
            b.runtime.accept_request(&requests[0].request).await,
            Err(CommandError::UnknownRequest)
        );
    });
}

#[test]
fn a_name_and_a_verification_go_to_the_record_they_were_set_for() {
    run_paused(async {
        let network = MockNetwork::new();
        let a = Peer::start(&network, 1).await;
        let b = Peer::start(&network, 2).await;
        let first = a
            .runtime
            .add_contact(&a.id(), &b.card())
            .await
            .unwrap()
            .contact;
        a.runtime
            .set_contact_name(&a.id(), &first, Some("Bob"))
            .await
            .unwrap();
        a.runtime
            .set_contact_verified(&a.id(), &first, true)
            .await
            .unwrap();
        let contacts = a.runtime.contacts(&a.id()).unwrap();
        assert_eq!(contacts[0].alias.as_deref(), Some("Bob"));
        assert!(contacts[0].verified);
        // 52 characters in 13 groups of 4 (PROTOCOL.md section 10).
        let groups: Vec<&str> = contacts[0].full_fingerprint.split(' ').collect();
        assert_eq!(groups.len(), 13);
        assert!(groups.iter().all(|group| group.len() == 4));

        // A name that breaks the rules of names is refused.
        assert_eq!(
            a.runtime
                .set_contact_name(&a.id(), &first, Some("Bob\nAlice"))
                .await,
            Err(CommandError::InvalidText)
        );
        // An empty name removes it.
        a.runtime
            .set_contact_name(&a.id(), &first, Some(""))
            .await
            .unwrap();
        assert_eq!(a.runtime.contacts(&a.id()).unwrap()[0].alias, None);

        // Deleted and made again: a new record, which the handle of the
        // old one does not name.
        a.runtime.delete_contact(&a.id(), &first).await.unwrap();
        let second = a
            .runtime
            .add_contact(&a.id(), &b.card())
            .await
            .unwrap()
            .contact;
        assert_ne!(first, second);
        assert_eq!(
            a.runtime
                .set_contact_name(&a.id(), &first, Some("Old"))
                .await,
            Err(CommandError::UnknownContact)
        );
        assert_eq!(
            a.runtime.set_contact_verified(&a.id(), &first, true).await,
            Err(CommandError::UnknownContact)
        );
        let contacts = a.runtime.contacts(&a.id()).unwrap();
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].contact, second);
        assert_eq!(contacts[0].alias, None);
        assert!(!contacts[0].verified);
        let _ = b.seed;
    });
}

#[test]
fn the_service_state_answers_as_the_last_event_said() {
    run_paused(async {
        let network = MockNetwork::new();
        let b = Peer::start(&network, 2).await;
        until(|| b.runtime.service_state(&b.id()) == Ok(ServiceState::Available)).await;
        let mut events = b.runtime.subscribe();
        network.lose_service(&fixed_endpoint(b.seed));
        event(&mut events, |event| {
            matches!(
                event,
                Event::Service {
                    state: ServiceState::Unavailable,
                    ..
                }
            )
        })
        .await;
        // The state was recorded before the event was raised.
        assert_eq!(
            b.runtime.service_state(&b.id()),
            Ok(ServiceState::Unavailable)
        );
        let other = IdentityId::from_bytes([9; 16]).unwrap();
        assert_eq!(
            b.runtime.service_state(&other),
            Err(CommandError::UnknownIdentity)
        );
    });
}

#[test]
fn a_tor_check_says_what_the_endpoints_answer() {
    run_paused(async {
        let network = MockNetwork::new();
        let b = Peer::start(&network, 2).await;
        assert!(matches!(
            b.runtime.check_tor().await,
            TorCheck::Ready {
                outdated: false,
                ..
            }
        ));
        network.set_readiness(false, Bootstrap::InProgress(40));
        assert_eq!(
            b.runtime.check_tor().await,
            TorCheck::Connecting { progress: Some(40) }
        );
        network.set_socks_available(false);
        assert_eq!(b.runtime.check_tor().await, TorCheck::SocksUnavailable);
        network.set_control_available(false);
        assert_eq!(b.runtime.check_tor().await, TorCheck::NotRunning);
        network.set_socks_available(true);
        assert_eq!(b.runtime.check_tor().await, TorCheck::ControlUnavailable);
    });
}

#[test]
fn an_added_card_says_whether_it_carried_an_invitation_also_after_a_restart() {
    run_paused(async {
        let network = MockNetwork::new();
        let b = Peer::start(&network, 2).await;
        let c = Peer::start(&network, 3).await;
        let dir = MemoryDir::new();
        let installation =
            Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR).unwrap();
        let a = Peer::start_in(&network, 1, installation).await;
        let (_, invitation) = b.runtime.create_invitation(&b.id(), None).await.unwrap();
        let invited = a.runtime.add_contact(&a.id(), &invitation).await.unwrap();
        assert!(invited.invitation);
        let carded = a.runtime.add_contact(&a.id(), &c.card()).await.unwrap();
        assert!(!carded.invitation);
        let held = |runtime: &Runtime<MockTorBackend>, id: &IdentityId| {
            let mut held: Vec<(ContactId, bool, bool)> = runtime
                .contacts(id)
                .unwrap()
                .into_iter()
                .map(|summary| (summary.contact, summary.invitation, summary.accepted))
                .collect();
            held.sort();
            held
        };
        let mut expected = vec![
            (invited.contact, true, false),
            (carded.contact, false, false),
        ];
        expected.sort();
        assert_eq!(held(&a.runtime, &a.id()), expected);

        // The plain card of B over its invitation: a request to B carries
        // none from then on; the invitation again, and it does.
        let plain = a.runtime.add_contact(&a.id(), &b.card()).await.unwrap();
        assert_eq!(plain.contact, invited.contact);
        assert!(!plain.invitation);
        let summary = |runtime: &Runtime<MockTorBackend>, id: &IdentityId| {
            held(runtime, id)
                .into_iter()
                .find(|(contact, _, _)| *contact == invited.contact)
                .unwrap()
        };
        assert_eq!(
            summary(&a.runtime, &a.id()),
            (invited.contact, false, false)
        );
        let again = a.runtime.add_contact(&a.id(), &invitation).await.unwrap();
        assert_eq!(again, invited);
        assert_eq!(summary(&a.runtime, &a.id()), (invited.contact, true, false));

        // The store keeps the card, and with it whether it has one.
        let id = a.id();
        a.runtime.shutdown(Duration::ZERO).await;
        drop(a);
        let (installation, _) =
            Installation::open(Box::new(dir.restart(CrashOutcome::ALL[0])), &passphrase()).unwrap();
        let runtime = Runtime::start(installation, network.backend(), Budgets::new(), options());
        assert_eq!(held(&runtime, &id), expected);
        runtime.shutdown(Duration::ZERO).await;
    });
}

#[test]
fn a_plain_card_reaches_an_identity_that_wants_invitations_once_it_adds_the_card_back() {
    run_paused(async {
        let network = MockNetwork::new();
        let a = Peer::start(&network, 1).await;
        let b = Peer::start(&network, 2).await;
        // B is in the default mode, invitations only, and A has its card.
        assert_eq!(
            b.runtime.request_mode(&b.id()).unwrap(),
            RequestMode::Invitation
        );
        let mut events = b.runtime.subscribe();
        let at_a = request_with_card(&a, &b.card()).await;
        event(&mut events, |event| {
            matches!(event, Event::RequestReceived { queued: false, .. })
        })
        .await;
        // Dropped: B sees no request, and A only that it waits.
        assert!(b.runtime.requests(&b.id()).unwrap().is_empty());
        let summary = a.runtime.contacts(&a.id()).unwrap().remove(0);
        assert_eq!(summary.contact, at_a);
        assert!(!summary.invitation);
        assert!(!summary.accepted);
        // B adds the card of A too: the next session makes them contacts,
        // and the message arrives.
        let added = b.runtime.add_contact(&b.id(), &a.card()).await.unwrap();
        assert!(!added.invitation);
        until(|| {
            a.runtime.messages(&a.id(), &at_a, None, 10).unwrap()[0].status
                == MessageStatus::Delivered
        })
        .await;
        assert!(a.runtime.contacts(&a.id()).unwrap()[0].accepted);
    });
}

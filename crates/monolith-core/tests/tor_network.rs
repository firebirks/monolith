//! Against a real Tor: the private network of `tests/tor-network`, whose
//! script sets the endpoints of its two client Tors in the environment.
//! Ignored otherwise.
//!
//! A service whose control connection is gone is published again from
//! the key held in memory, under the same name, and is reached again; a
//! dial without its SOCKS endpoint fails and tries nothing else. A file
//! goes on across a rotation of the receiver's transport key, over a
//! session of the new key, to the same contact record, and arrives whole;
//! a contact deleted and made again inherits none of its transfers.

// Test code builds its own inputs, and the script reads what it reports.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::print_stdout
)]

use core::future::{Future, poll_fn};
use core::pin::pin;
use core::task::Poll;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::time::Duration;

use monolith_core::api::{ContactId, IdentityId, MessageStatus, TransferStatus};
use monolith_core::budget::Budgets;
use monolith_core::identity::{IdentityKeys, Installation, LocalIdentity};
use monolith_core::link::{Established, Link, LinkError, answer, dial};
use monolith_core::runtime::{Runtime, RuntimeOptions};
use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::body::{Message, MessageId};
use monolith_protocol::session::Action;
use monolith_protocol::text::ChatText;
use monolith_tor::{
    ControlAuth, Endpoint, KeySource, OnionService, PublishedOnionService, SocksProof,
    SystemTorBackend, SystemTorConfig, TorBackend,
};
use tokio::io::{AsyncRead, AsyncWrite};

/// How often, and how far apart, a dial is tried while a descriptor is
/// on its way to the directories of the test network.
const ATTEMPTS: u32 = 30;
const PAUSE: Duration = Duration::from_secs(10);

fn run<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// The configuration of client Tor `side`, from the environment.
fn config(side: &str) -> SystemTorConfig {
    let var = |name: &str| {
        let key = format!("MONOLITH_TOR_{side}_{name}");
        std::env::var(&key).unwrap_or_else(|_| panic!("{key} is not set"))
    };
    SystemTorConfig {
        socks: Endpoint::parse(&var("SOCKS")).unwrap(),
        control: Endpoint::parse(&var("CONTROL")).unwrap(),
        auth: ControlAuth::SafeCookie {
            cookie_file: PathBuf::from(var("COOKIE")),
        },
        socks_proof: SocksProof::Off,
    }
}

/// The identity of `seed`, reachable at `endpoint`, in an ephemeral
/// installation of its own, which is kept for the rest of the test: an
/// identity whose installation is gone refuses every change.
async fn identity(seed: u8, endpoint: OnionServiceKey) -> std::sync::Arc<LocalIdentity> {
    let installation = Box::leak(Box::new(Installation::ephemeral()));
    installation
        .restore_identity(
            IdentityKeys {
                seed: zeroize::Zeroizing::new([seed; 32]),
                transport: zeroize::Zeroizing::new([seed.wrapping_add(0x40); 32]),
                onion: None,
                epoch: EndpointEpoch::FIRST,
                endpoint,
            },
            None,
        )
        .await
        .unwrap()
}

fn chat(text: &str) -> Message {
    Message::ChatMessage {
        id: MessageId::from_bytes([7; 16]),
        text: ChatText::new(text).unwrap(),
    }
}

/// Sends what the session logic asks for first, applies what arrives,
/// and waits for the peer's ContactAccept, which confirms the session.
async fn confirm<S: AsyncRead + AsyncWrite + Unpin>(
    established: &mut Established<S>,
    identity: &LocalIdentity,
) {
    if established.first.contains(&Action::SendContactAccept) {
        established
            .link
            .send(&Message::ContactAccept)
            .await
            .unwrap();
    }
    if established.first.contains(&Action::SendContactRequest) {
        let request = Message::ContactRequest(Box::new(monolith_protocol::body::ContactRequest {
            card: identity.card(),
            invitation: None,
            display_name: monolith_protocol::text::DisplayName::new("").unwrap(),
            introduction: monolith_protocol::text::IntroductionText::new("").unwrap(),
        }));
        established.link.send(&request).await.unwrap();
    }
    loop {
        let received = established.link.receive().await.unwrap();
        identity.apply(&received).await.unwrap();
        if received.actions.contains(&Action::SendContactAccept) {
            established
                .link
                .send(&Message::ContactAccept)
                .await
                .unwrap();
        }
        if received.actions.contains(&Action::Confirmed) {
            return;
        }
    }
}

async fn next_chat<S: AsyncRead + AsyncWrite + Unpin>(link: &mut Link<S>) -> String {
    loop {
        let received = link.receive().await.unwrap();
        if let Message::ChatMessage { text, .. } = &received.message {
            return text.as_str().to_owned();
        }
    }
}

/// Alice dials Bob, who answers on `service`: "hello" goes to Bob,
/// "hello back" to Alice. The dial is tried again while it fails, which
/// it does until the descriptor of the service is in the directories.
async fn exchange(
    alice_tor: &SystemTorBackend,
    alice: &LocalIdentity,
    service: &mut PublishedOnionService,
    bob: &LocalIdentity,
) {
    let budgets = Budgets::new();
    for attempt in 1..=ATTEMPTS {
        let alice_side = async {
            let mut established = dial(alice_tor, &budgets, alice, &bob.card()).await?;
            confirm(&mut established, alice).await;
            established.link.send(&chat("hello")).await.unwrap();
            let reply = next_chat(&mut established.link).await;
            established.link.close().await.unwrap();
            Ok::<String, LinkError>(reply)
        };
        let bob_side = async {
            let stream = service.accept().await.unwrap();
            let mut established = answer(stream, &budgets, bob).await.unwrap();
            confirm(&mut established, bob).await;
            let hello = next_chat(&mut established.link).await;
            established.link.send(&chat("hello back")).await.unwrap();
            hello
        };
        // A failed dial ends the attempt; Bob's wait for a stream is
        // dropped with it and begins again.
        let mut alice_side = pin!(alice_side);
        let mut bob_side = pin!(bob_side);
        let mut heard = None;
        let outcome = poll_fn(|cx| {
            if heard.is_none() {
                if let Poll::Ready(hello) = bob_side.as_mut().poll(cx) {
                    heard = Some(hello);
                }
            }
            match alice_side.as_mut().poll(cx) {
                Poll::Ready(Ok(reply)) => Poll::Ready(Ok(reply)),
                Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                Poll::Pending => Poll::Pending,
            }
        })
        .await;
        match outcome {
            Ok(reply) => {
                let hello = heard.take().unwrap();
                assert_eq!(hello, "hello");
                assert_eq!(reply, "hello back");
                println!("Bob received {hello:?}; Alice received {reply:?}");
                return;
            }
            Err(error) => {
                println!("dial attempt {attempt}: {error}");
                tokio::time::sleep(PAUSE).await;
            }
        }
    }
    panic!("Bob was not reached");
}

#[test]
#[ignore = "needs the private Tor network of tests/tor-network"]
fn a_service_published_again_from_its_key_is_reached_again() {
    run(async {
        let alice_tor = SystemTorBackend::new(config("A"));
        let bob_tor = SystemTorBackend::new(config("B"));

        let mut service = bob_tor.publish_onion(KeySource::Generate).await.unwrap();
        assert!(service.is_published());
        let name = *service.service_key();
        let secret = service.take_generated_secret().unwrap();
        let bob = identity(2, name).await;
        // Alice only dials; her card names an endpoint nobody publishes.
        let elsewhere = IdentitySecretKey::from_seed(&[0x66; 32]).public_key();
        let alice = identity(
            1,
            OnionServiceKey::from_bytes(elsewhere.as_bytes()).unwrap(),
        )
        .await;
        // Each imported the other's card; the first session makes them
        // accepted contacts.
        alice.import(&bob.card()).await.unwrap();
        bob.import(&alice.card()).await.unwrap();
        exchange(&alice_tor, &alice, &mut service, &bob).await;

        // The control connection that published the service goes away,
        // and Tor removes the service with it. Published again from the
        // key held in memory, it has the same name and is reached again.
        drop(service);
        let mut service = bob_tor
            .publish_onion(KeySource::Existing {
                secret,
                expected: name,
            })
            .await
            .unwrap();
        assert_eq!(service.service_key(), &name);
        assert!(service.is_published());
        println!("Published again under the same name.");
        exchange(&alice_tor, &alice, &mut service, &bob).await;
        service.close().await.unwrap();

        // Without its SOCKS endpoint a dial fails; the trace the script
        // takes shows that nothing else is tried.
        let mut gone = config("A");
        gone.socks = Endpoint::parse(&std::env::var("MONOLITH_TOR_CLOSED_SOCKS").unwrap()).unwrap();
        let without_socks = SystemTorBackend::new(gone);
        let failed = dial(&without_socks, &Budgets::new(), &alice, &bob.card()).await;
        assert!(matches!(failed.err(), Some(LinkError::Tor(_))));
        println!("Without SOCKS the dial failed.");
    });
}

/// Waits, looking every second, for up to `seconds`, until `condition`
/// holds. Returns whether it did.
async fn eventually(seconds: u64, condition: impl Fn() -> bool) -> bool {
    for _ in 0..seconds {
        if condition() {
            return true;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    condition()
}

/// A runtime of an ephemeral installation on client Tor `side`.
fn runtime(side: &str) -> Runtime<SystemTorBackend> {
    Runtime::start(
        Installation::ephemeral(),
        SystemTorBackend::new(config(side)),
        Budgets::new(),
        RuntimeOptions {
            startup_spread: Duration::ZERO,
            ..RuntimeOptions::default()
        },
    )
}

/// Whether every message of the conversation is delivered.
fn delivered(runtime: &Runtime<SystemTorBackend>, id: &IdentityId, at: &ContactId) -> bool {
    runtime
        .messages(id, at, None, 100)
        .unwrap()
        .iter()
        .all(|message| message.status == MessageStatus::Delivered)
}

#[test]
#[ignore = "needs the private Tor network of tests/tor-network"]
fn a_transfer_goes_on_across_a_key_rotation_and_a_contact_made_again_inherits_nothing() {
    run(async {
        let alice = runtime("A");
        let bob = runtime("B");
        let a = alice.create_identity(None).await.unwrap();
        let b = bob.create_identity(None).await.unwrap();
        let (la, lb) = (
            alice.local_identity(&a).unwrap(),
            bob.local_identity(&b).unwrap(),
        );
        la.import(&lb.card()).await.unwrap();
        lb.import(&la.card()).await.unwrap();
        let at_a = la.contact_id(lb.identity()).unwrap();
        let at_b = lb.contact_id(la.identity()).unwrap();
        alice.send_message(&a, &at_a, "hello").await.unwrap();
        assert!(
            eventually(900, || delivered(&alice, &a, &at_a)).await,
            "no first session"
        );
        // Bob begins a rotation of his transport key; Alice learns the
        // successor on the next session.
        lb.begin_rotation().await.unwrap();
        alice
            .send_message(&a, &at_a, "the successor, please")
            .await
            .unwrap();
        let cards = || {
            la.dial_plan(lb.identity())
                .map_or(0, |plan| plan.cards.len())
        };
        assert!(
            eventually(900, || cards() == 2).await,
            "the successor was not learned"
        );
        println!("Alice holds the successor of Bob's key.");
        // A file goes; Bob switches to the new key while it is on its way.
        let data: Vec<u8> = (0..3_000_000_u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect();
        let sent = alice
            .offer_file(
                &a,
                &at_a,
                "rotation.bin",
                data.len() as u64,
                Box::new(Cursor::new(data.clone())),
            )
            .await
            .unwrap();
        let offered = || {
            bob.transfers(&b)
                .unwrap()
                .first()
                .map(|view| view.reference)
        };
        assert!(eventually(900, || offered().is_some()).await, "no offer");
        let received = offered().unwrap();
        bob.accept_transfer(&received).await.unwrap();
        let held = || {
            bob.transfer(&received)
                .unwrap()
                .and_then(|view| view.bytes)
                .unwrap_or(0)
        };
        assert!(eventually(1800, || held() > 0).await, "no progress");
        let rotation = lb.rotation_id().unwrap();
        assert!(lb.switch_rotation(rotation, true).await.unwrap());
        println!("Bob switched to his new key with {} bytes staged.", held());
        let completed = || {
            alice.transfer(&sent).unwrap().map(|view| view.status)
                == Some(TransferStatus::Completed)
        };
        assert!(eventually(3600, completed).await, "not completed");
        assert_eq!(
            bob.transfer(&received).unwrap().map(|view| view.status),
            Some(TransferStatus::Received)
        );
        assert_eq!(received.contact, at_b);
        assert_eq!(cards(), 1, "the new key is not the only one");
        let mut reader = bob.begin_save(&received).await.unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        drop(reader);
        bob.end_save(&received, true).await.unwrap();
        assert!(bytes == data, "other bytes");
        println!("The file went on over the new key, to the same contact, whole.");
        // Bob deletes Alice and adds her again: the new record holds none
        // of the old one's transfers, also after a session.
        bob.delete_contact(&b, &at_b).await.unwrap();
        lb.import(&la.card()).await.unwrap();
        let again = lb.contact_id(la.identity()).unwrap();
        assert_ne!(again, at_b);
        bob.send_message(&b, &again, "hello again").await.unwrap();
        assert!(
            eventually(900, || delivered(&bob, &b, &again)).await,
            "no session"
        );
        tokio::time::sleep(Duration::from_secs(30)).await;
        assert!(
            bob.transfers(&b)
                .unwrap()
                .iter()
                .all(|view| view.reference.contact != again),
            "the new record inherited a transfer"
        );
        println!("The contact made again inherited no transfer.");
        alice.shutdown(Duration::from_secs(5)).await;
        bob.shutdown(Duration::from_secs(5)).await;
    });
}

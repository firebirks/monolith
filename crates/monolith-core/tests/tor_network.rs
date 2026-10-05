//! Against a real Tor: the private network of `tests/tor-network`, whose
//! script sets the endpoints of its two client Tors in the environment.
//! Ignored otherwise.
//!
//! A service whose control connection is gone is published again from
//! the key held in memory, under the same name, and is reached again; a
//! dial without its SOCKS endpoint fails and tries nothing else.

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
use std::path::PathBuf;
use std::time::Duration;

use monolith_core::budget::Budgets;
use monolith_core::identity::{IdentityKeys, Installation, LocalIdentity};
use monolith_core::link::{Established, Link, LinkError, answer, dial};
use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::body::{Message, MessageId};
use monolith_protocol::session::Action;
use monolith_protocol::text::ChatText;
use monolith_tor::{
    ControlAuth, Endpoint, KeySource, OnionService, PublishedOnionService, SystemTorBackend,
    SystemTorConfig, TorBackend,
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
        identity
            .apply(established.link.session_ref(), &received)
            .await
            .unwrap();
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
        if let Message::ChatMessage { text, .. } = received.message {
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

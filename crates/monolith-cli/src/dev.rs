//! `dev-chat`: the two-node test of Phase 3, for a private Tor network.
//!
//! Each side makes an identity, a transport key and an ephemeral Onion
//! Service, all in memory, writes its public contact card to a file, and
//! waits for the other side's card in another file. The dialing side then
//! connects through Tor, both confirm each other as contacts, one chat
//! message goes each way, and both close the session and remove their
//! service. Nothing secret is written or printed. This is a development
//! aid, not the product's contact handling, which comes with Phase 4.

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use monolith_core::budget::Budgets;
use monolith_core::link::{Established, Link, LinkError, answer, dial as dial_link};
use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::body::{Message, MessageId};
use monolith_protocol::card::{ContactCard, EndpointSet};
use monolith_protocol::credential::Credentials;
use monolith_protocol::session::{Action, PeerRecord};
use monolith_protocol::text::ChatText;
use monolith_session::{LocalParty, TransportSecretKey};
use monolith_tor::{
    KeySource, OnionService, PublishedOnionService, SystemTorBackend, SystemTorConfig, TorBackend,
};
use tokio::io::{AsyncRead, AsyncWrite};

/// How long to wait for the other side's card.
const CARD_WAIT: Duration = Duration::from_secs(600);

/// How long to wait for the peer to connect.
const ACCEPT_WAIT: Duration = Duration::from_secs(600);

fn fail(what: &str) -> ExitCode {
    eprintln!("monolith dev-chat: {what}");
    ExitCode::FAILURE
}

/// A fresh identity whose card names `endpoint`. The identity seed comes
/// from the operating system's random source and is kept in memory only.
fn party(endpoint: OnionServiceKey) -> Option<LocalParty> {
    let mut seed = zeroize::Zeroizing::new([0_u8; 32]);
    getrandom::fill(seed.as_mut_slice()).ok()?;
    let identity = IdentitySecretKey::from_seed(&seed);
    drop(seed);
    let transport = TransportSecretKey::generate().ok()?;
    LocalParty::issue(
        &identity,
        transport,
        EndpointEpoch::FIRST,
        EndpointSet::single(endpoint),
    )
    .ok()
}

/// Publishes an ephemeral service and makes the party it belongs to.
async fn setup(
    backend: &SystemTorBackend,
    own_card: &str,
) -> Result<(PublishedOnionService, LocalParty), ExitCode> {
    let mut service = backend
        .publish_onion(KeySource::Generate)
        .await
        .map_err(|error| fail(&format!("cannot publish: {error}")))?;
    // The key is ephemeral: it is dropped, and erased, right away.
    drop(service.take_generated_secret());
    let local = party(*service.service_key()).ok_or_else(|| fail("cannot make an identity"))?;
    std::fs::write(own_card, local.card().to_text())
        .map_err(|_| fail("cannot write the own card file"))?;
    println!("Published; own card written.");
    Ok((service, local))
}

async fn wait_for_card(path: &str) -> Result<ContactCard, ExitCode> {
    let deadline = tokio::time::Instant::now()
        .checked_add(CARD_WAIT)
        .ok_or_else(|| fail("clock"))?;
    loop {
        if Path::new(path).exists() {
            let text =
                std::fs::read_to_string(path).map_err(|_| fail("cannot read the peer card"))?;
            return ContactCard::from_text(text.trim()).map_err(|_| fail("invalid peer card"));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(fail("no peer card"));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

fn chat(text: &str) -> Option<Message> {
    Some(Message::ChatMessage {
        id: MessageId::from_bytes([7; 16]),
        text: ChatText::new(text).ok()?,
    })
}

/// Sends ContactAccept if the session logic asks for it, and waits for
/// the peer's, which confirms the session.
async fn confirm<S: AsyncRead + AsyncWrite + Unpin>(
    established: &mut Established<S>,
) -> Result<(), LinkError> {
    if established.first.contains(&Action::SendContactAccept) {
        established.link.send(&Message::ContactAccept).await?;
    }
    loop {
        let received = established.link.receive().await?;
        if received.actions.contains(&Action::Confirmed) {
            return Ok(());
        }
    }
}

async fn next_chat<S: AsyncRead + AsyncWrite + Unpin>(
    link: &mut Link<S>,
) -> Result<String, LinkError> {
    loop {
        let received = link.receive().await?;
        if let Message::ChatMessage { text, .. } = received.message {
            return Ok(text.as_str().to_owned());
        }
    }
}

/// The side that publishes and waits for the other to dial.
pub(crate) async fn serve(config: SystemTorConfig, own_card: &str, peer_card: &str) -> ExitCode {
    let backend = SystemTorBackend::new(config);
    let (mut service, local) = match setup(&backend, own_card).await {
        Ok(setup) => setup,
        Err(code) => return code,
    };
    let peer = match wait_for_card(peer_card).await {
        Ok(card) => card,
        Err(code) => return code,
    };
    // The only contact of this run. Its key never changes here, so the
    // withdrawal of the session is not used.
    let mut held = Credentials::new(peer.clone());
    let result = async {
        let stream = tokio::time::timeout(ACCEPT_WAIT, service.accept())
            .await
            .map_err(|_| LinkError::TimedOut)?
            .map_err(LinkError::Tor)?;
        let mut established = answer(stream, &Budgets::new(), &local, |inbound, _| {
            if inbound.card().identity() == peer.identity() {
                inbound.admit(PeerRecord::Accepted(&mut held))
            } else {
                inbound.admit(PeerRecord::None)
            }
        })
        .await?;
        confirm(&mut established).await?;
        let text = next_chat(&mut established.link).await?;
        println!("Received: {text:?}");
        let reply = chat("pong").ok_or(LinkError::Stream)?;
        established.link.send(&reply).await?;
        // Wait for the peer's Close.
        let _ = established.link.receive().await;
        Ok::<(), LinkError>(())
    }
    .await;
    let removed = service.close().await;
    match (result, removed) {
        (Ok(()), Ok(())) => {
            println!("Done; service removed.");
            ExitCode::SUCCESS
        }
        (Err(error), _) => fail(&format!("{error}")),
        (Ok(()), Err(error)) => fail(&format!("service removal: {error}")),
    }
}

/// The side that dials.
pub(crate) async fn dial(
    config: SystemTorConfig,
    own_card: &str,
    peer_card: &str,
    message: &str,
) -> ExitCode {
    let backend = SystemTorBackend::new(config);
    let (service, local) = match setup(&backend, own_card).await {
        Ok(setup) => setup,
        Err(code) => return code,
    };
    let peer = match wait_for_card(peer_card).await {
        Ok(card) => card,
        Err(code) => return code,
    };
    let mut held = Credentials::new(peer.clone());
    let result = async {
        let isolation = backend.isolation_group().map_err(LinkError::Tor)?;
        let mut established = dial_link(
            &backend,
            &Budgets::new(),
            &local,
            &peer,
            &isolation,
            |outbound, _| outbound.admit(PeerRecord::Accepted(&mut held)),
        )
        .await?;
        confirm(&mut established).await?;
        let text = chat(message).ok_or(LinkError::Stream)?;
        established.link.send(&text).await?;
        let reply = next_chat(&mut established.link).await?;
        println!("Reply: {reply:?}");
        established.link.close().await
    }
    .await;
    let removed = service.close().await;
    match (result, removed) {
        (Ok(()), Ok(())) => {
            println!("Done; service removed.");
            ExitCode::SUCCESS
        }
        (Err(error), _) => fail(&format!("{error}")),
        (Ok(()), Err(error)) => fail(&format!("service removal: {error}")),
    }
}

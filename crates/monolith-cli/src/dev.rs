//! `dev-chat`: the two-node test of Phase 3, for a private Tor network.
//!
//! Each side makes an ephemeral installation with one identity and an
//! ephemeral Onion Service, all in memory, writes its public contact card
//! to a file, and waits for the other side's card in another file, which
//! it imports. The dialing side then connects through Tor; both sent a
//! request, so the session makes them accepted contacts and confirms; one
//! chat message goes each way, and both close the session and remove their
//! service. Nothing secret is written or printed. This is a development
//! aid for the private network test.

use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use std::sync::Arc;

use monolith_core::budget::Budgets;
use monolith_core::identity::{Installation, LocalIdentity};
use monolith_core::link::{Established, Link, LinkError, answer, dial as dial_link};
use monolith_protocol::body::{ContactRequest, Message, MessageId};
use monolith_protocol::card::ContactCard;
use monolith_protocol::session::Action;
use monolith_protocol::text::{ChatText, DisplayName, IntroductionText};
use monolith_tor::{
    KeySource, OnionService, PublishedOnionService, SystemTorBackend, SystemTorConfig, TorBackend,
    TorError,
};
use tokio::io::{AsyncRead, AsyncWrite};

/// How long to wait for the other side's card.
const CARD_WAIT: Duration = Duration::from_secs(600);

/// How long to wait for the peer to connect.
const ACCEPT_WAIT: Duration = Duration::from_secs(600);

/// How often, and how far apart, a dial is tried while Tor cannot reach
/// the peer's service yet: its descriptor may still be on its way.
const DIAL_ATTEMPTS: u32 = 30;
const DIAL_PAUSE: Duration = Duration::from_secs(10);

fn fail(what: &str) -> ExitCode {
    eprintln!("monolith dev-chat: {what}");
    ExitCode::FAILURE
}

/// Publishes an ephemeral service and makes the identity it belongs to,
/// in an ephemeral installation, which is returned with it: the
/// installation owns the identity, and the identity refuses every change
/// once it is gone. The keys are generated from the operating system's
/// random source and kept in memory only.
async fn setup(
    backend: &SystemTorBackend,
    own_card: &str,
) -> Result<(PublishedOnionService, Installation, Arc<LocalIdentity>), ExitCode> {
    let mut service = backend
        .publish_onion(KeySource::Generate)
        .await
        .map_err(|error| fail(&format!("cannot publish: {error}")))?;
    // The key is ephemeral: it is dropped, and erased, right away.
    drop(service.take_generated_secret());
    let installation = Installation::ephemeral();
    let local = installation
        .create_identity(*service.service_key(), None, None)
        .await
        .map_err(|_| fail("cannot make an identity"))?;
    std::fs::write(own_card, local.card().to_text())
        .map_err(|_| fail("cannot write the own card file"))?;
    println!("Published; own card written.");
    Ok((service, installation, local))
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

/// Sends what the session logic asks for first, applies to the contact
/// store what arrives, answers a request with ContactAccept, and waits for
/// the peer's ContactAccept, which confirms the session.
async fn confirm<S: AsyncRead + AsyncWrite + Unpin>(
    established: &mut Established<S>,
    local: &LocalIdentity,
) -> Result<(), LinkError> {
    if established.first.contains(&Action::SendContactAccept) {
        established.link.send(&Message::ContactAccept).await?;
    }
    if established.first.contains(&Action::SendContactRequest) {
        let request = Message::ContactRequest(Box::new(ContactRequest {
            card: local.card(),
            invitation: None,
            display_name: DisplayName::new("").map_err(|_| LinkError::Stream)?,
            introduction: IntroductionText::new("").map_err(|_| LinkError::Stream)?,
        }));
        established.link.send(&request).await?;
    }
    loop {
        let received = established.link.receive().await?;
        local
            .apply(established.link.session_ref(), &received)
            .await
            .map_err(|_| LinkError::Withdrawn)?;
        if received.actions.contains(&Action::SendContactAccept) {
            established.link.send(&Message::ContactAccept).await?;
        }
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
    let (mut service, _installation, local) = match setup(&backend, own_card).await {
        Ok(setup) => setup,
        Err(code) => return code,
    };
    let peer = match wait_for_card(peer_card).await {
        Ok(card) => card,
        Err(code) => return code,
    };
    if local.import(&peer).await.is_err() {
        return fail("cannot import the peer card");
    }
    let result = async {
        let stream = tokio::time::timeout(ACCEPT_WAIT, service.accept())
            .await
            .map_err(|_| LinkError::TimedOut)?
            .map_err(LinkError::Tor)?;
        let mut established = answer(stream, &Budgets::new(), &local).await?;
        confirm(&mut established, &local).await?;
        let text = next_chat(&mut established.link).await?;
        println!("Received: {text:?}");
        let reply = chat("hello back").ok_or(LinkError::Stream)?;
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
    let (service, _installation, local) = match setup(&backend, own_card).await {
        Ok(setup) => setup,
        Err(code) => return code,
    };
    let peer = match wait_for_card(peer_card).await {
        Ok(card) => card,
        Err(code) => return code,
    };
    if local.import(&peer).await.is_err() {
        return fail("cannot import the peer card");
    }
    let result = async {
        let mut attempt = 1;
        let mut established = loop {
            match dial_link(&backend, &Budgets::new(), &local, &peer).await {
                Ok(established) => break established,
                // Only a service Tor cannot reach yet is tried again.
                Err(LinkError::Tor(error @ TorError::OnionUnreachable))
                    if attempt < DIAL_ATTEMPTS =>
                {
                    println!("Dial attempt {attempt}: {error}; trying again.");
                    attempt = attempt.saturating_add(1);
                    tokio::time::sleep(DIAL_PAUSE).await;
                }
                Err(error) => return Err(error),
            }
        };
        confirm(&mut established, &local).await?;
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

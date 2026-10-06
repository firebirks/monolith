//! What the tests of the core share: nodes on the in-memory Tor, each an
//! installation with one local identity of fixed keys and a published
//! service, and the steps of a session between two of them.

// Each test file uses a part of this.
#![allow(dead_code)]

use core::future::Future;
use std::sync::Arc;

use monolith_core::budget::Budgets;
use monolith_core::contacts::ContactView;
use monolith_core::identity::{IdentityKeys, Installation, LocalIdentity, RotationState};
use monolith_core::link::{Established, Link, LinkError, answer, dial};
use monolith_identity::IdentityPublicKey;
use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::body::{ContactRequest, Message, MessageId};
use monolith_protocol::card::{ContactCard, EndpointSet, InvitationCapability};
use monolith_protocol::contact::RequestMode;
use monolith_protocol::session::Action;
use monolith_protocol::text::{ChatText, DisplayName, IntroductionText};
use monolith_session::{LocalParty, TransportSecretKey};
use monolith_storage::dir::MemoryDir;
use monolith_tor::{
    KeySource, MockNetwork, MockOnionService, MockTorBackend, OnionService, TorBackend,
};
use tokio::io::DuplexStream;
use zeroize::Zeroizing;

pub fn run<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// As `run`, with time that advances only when every task waits.
pub fn run_paused<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap()
        .block_on(future)
}

/// Runs two futures to completion on the current task.
pub async fn both<A: Future, B: Future>(a: A, b: B) -> (A::Output, B::Output) {
    let mut a = core::pin::pin!(a);
    let mut b = core::pin::pin!(b);
    let (mut out_a, mut out_b) = (None, None);
    core::future::poll_fn(|cx| {
        if out_a.is_none() {
            if let core::task::Poll::Ready(value) = a.as_mut().poll(cx) {
                out_a = Some(value);
            }
        }
        if out_b.is_none() {
            if let core::task::Poll::Ready(value) = b.as_mut().poll(cx) {
                out_b = Some(value);
            }
        }
        if out_a.is_some() && out_b.is_some() {
            core::task::Poll::Ready(())
        } else {
            core::task::Poll::Pending
        }
    })
    .await;
    (out_a.unwrap(), out_b.unwrap())
}

/// The key material of the identity of `seed` with the transport key of
/// `transport`, at `epoch`, reachable at `endpoint`.
pub fn keys(seed: u8, transport: u8, epoch: u64, endpoint: OnionServiceKey) -> IdentityKeys {
    IdentityKeys {
        seed: Zeroizing::new([seed; 32]),
        transport: Zeroizing::new([transport ^ 0xA5; 32]),
        onion: Some(Zeroizing::new([seed; 64])),
        epoch: EndpointEpoch::new(epoch).unwrap(),
        endpoint,
    }
}

/// The local party the identity of `seed` would have with the transport
/// key of `transport`, at `epoch`, reachable at `endpoint`. For a peer
/// that is not run through an installation.
pub fn party(seed: u8, transport: u8, epoch: u64, endpoint: OnionServiceKey) -> LocalParty {
    LocalParty::issue(
        &IdentitySecretKey::from_seed(&[seed; 32]),
        TransportSecretKey::from_bytes(&[transport ^ 0xA5; 32]).unwrap(),
        EndpointEpoch::new(epoch).unwrap(),
        EndpointSet::single(endpoint),
    )
    .unwrap()
}

/// The fixed endpoint number `seed`, for nodes whose cards have to be the
/// same in two runs.
pub fn fixed_endpoint(seed: u8) -> OnionServiceKey {
    OnionServiceKey::from_bytes(
        IdentitySecretKey::from_seed(&[seed.wrapping_add(150); 32])
            .public_key()
            .as_bytes(),
    )
    .unwrap()
}

/// A node as [`node_in`], published at the fixed endpoint `endpoint`, so
/// that its cards are the same in every run on a fresh network.
pub async fn node_fixed(
    installation: Installation,
    network: &MockNetwork,
    seed: u8,
    transport: u8,
    epoch: u64,
    endpoint: OnionServiceKey,
) -> Node {
    let tor = network.backend();
    let service = tor
        .publish_onion(KeySource::Existing {
            secret: monolith_tor::OnionServiceSecret::from_bytes(&[seed; 64]),
            expected: endpoint,
        })
        .await
        .unwrap();
    let identity = installation
        .restore_identity(keys(seed, transport, epoch, endpoint), None)
        .await
        .unwrap();
    Node {
        installation,
        identity,
        tor,
        service,
        budgets: Budgets::new(),
    }
}

/// An endpoint that is not the service of any node.
pub fn elsewhere() -> OnionServiceKey {
    OnionServiceKey::from_bytes(
        IdentitySecretKey::from_seed(&[0xEE; 32])
            .public_key()
            .as_bytes(),
    )
    .unwrap()
}

/// One node: an installation, one identity in it, and its service on the
/// in-memory Tor.
pub struct Node {
    pub installation: Installation,
    pub identity: Arc<LocalIdentity>,
    pub tor: MockTorBackend,
    pub service: MockOnionService,
    pub budgets: Budgets,
}

impl Node {
    /// The card the node answers with.
    pub fn card(&self) -> ContactCard {
        self.identity.card()
    }
}

/// A node of the identity of `seed`, in an ephemeral installation of its
/// own, with the transport key of `seed` at epoch 1.
pub async fn node(network: &MockNetwork, seed: u8) -> Node {
    node_with(network, seed, seed, 1).await
}

/// A node of the identity of `seed` with the transport key of `transport`
/// at `epoch`: the same identity as another node of `seed`, with another
/// key, for a peer that changed its key.
pub async fn node_with(network: &MockNetwork, seed: u8, transport: u8, epoch: u64) -> Node {
    node_in(Installation::ephemeral(), network, seed, transport, epoch).await
}

/// The same, in `installation`.
pub async fn node_in(
    installation: Installation,
    network: &MockNetwork,
    seed: u8,
    transport: u8,
    epoch: u64,
) -> Node {
    let tor = network.backend();
    let service = tor.publish_onion(KeySource::Generate).await.unwrap();
    let identity = installation
        .restore_identity(keys(seed, transport, epoch, *service.service_key()), None)
        .await
        .unwrap();
    Node {
        installation,
        identity,
        tor,
        service,
        budgets: Budgets::new(),
    }
}

pub type End = Result<Established<DuplexStream>, LinkError>;

/// `from` dials `card`; `to` answers the stream on its service.
pub async fn dial_and_answer(from: &Node, card: &ContactCard, to: &mut Node) -> (End, End) {
    let identity = to.identity.clone();
    let budgets = to.budgets.clone();
    let service = &mut to.service;
    let answering = async move {
        let stream = service.accept().await.unwrap();
        answer(stream, &budgets, &identity).await
    };
    both(
        dial(&from.tor, &from.budgets, &from.identity, card),
        answering,
    )
    .await
}

/// `from` dials `to` at its current card.
pub async fn connect(from: &Node, to: &mut Node) -> (End, End) {
    let card = to.card();
    dial_and_answer(from, &card, to).await
}

pub fn chat(text: &str) -> Message {
    Message::ChatMessage {
        id: MessageId::from_bytes([1; 16]),
        text: ChatText::new(text).unwrap(),
    }
}

/// A contact request with `card`, carrying `invitation`.
pub fn request(card: &ContactCard, invitation: Option<InvitationCapability>) -> Message {
    Message::ContactRequest(Box::new(ContactRequest {
        card: card.clone(),
        invitation,
        display_name: DisplayName::new("Peer").unwrap(),
        introduction: IntroductionText::new("hello").unwrap(),
    }))
}

/// Sends what the session logic asked for when the session authenticated.
pub async fn send_first(end: &mut Established<DuplexStream>, identity: &LocalIdentity) {
    for action in end.first.clone() {
        match action {
            Action::SendContactAccept => end.link.send(&Message::ContactAccept).await.unwrap(),
            Action::SendContactRequest => {
                // A request carries the capability of the card held of
                // the peer, and no other.
                let card = identity.card();
                let invitation = identity
                    .contact(end.link.session().peer())
                    .and_then(|view| view.credentials)
                    .and_then(|held| held.invitation().cloned());
                end.link.send(&request(&card, invitation)).await.unwrap();
            }
            _ => {}
        }
    }
}

/// Receives one message on `link`, applies it to the store of `identity`
/// and carries out its visible actions. Returns what was received.
pub async fn step(
    link: &mut Link<DuplexStream>,
    identity: &LocalIdentity,
) -> Result<monolith_session::Received, LinkError> {
    let received = link.receive().await?;
    let _ = identity.apply(link.session_ref(), &received).await;
    if received.actions.contains(&Action::SendContactAccept) {
        let _ = link.send(&Message::ContactAccept).await;
    }
    Ok(received)
}

/// Runs a session from `a` to `b` until both are confirmed: what makes a
/// requested contact accepted on both sides. Returns the two links.
pub async fn confirm_both(a: &Node, b: &mut Node) -> (Link<DuplexStream>, Link<DuplexStream>) {
    let (a_end, b_end) = connect(a, b).await;
    let (mut a_end, mut b_end) = (a_end.unwrap(), b_end.unwrap());
    send_first(&mut a_end, &a.identity).await;
    send_first(&mut b_end, &b.identity).await;
    let (mut a_link, mut b_link) = (a_end.link, b_end.link);
    let a_identity = a.identity.clone();
    let b_identity = b.identity.clone();
    let a_side = async {
        loop {
            let received = step(&mut a_link, &a_identity).await.unwrap();
            if received.actions.contains(&Action::Confirmed) {
                break;
            }
        }
    };
    let b_side = async {
        loop {
            let received = step(&mut b_link, &b_identity).await.unwrap();
            if received.actions.contains(&Action::Confirmed) {
                break;
            }
        }
    };
    both(a_side, b_side).await;
    (a_link, b_link)
}

/// Makes `a` and `b` accepted contacts of each other through the protocol:
/// each imports the other's card, one session confirms both, and it is
/// closed.
pub async fn befriend(a: &Node, b: &mut Node) {
    a.identity.import(&b.card()).await.unwrap();
    b.identity.import(&a.card()).await.unwrap();
    let (a_link, mut b_link) = confirm_both(a, b).await;
    a_link.close().await.unwrap();
    let _ = b_link.receive().await;
}

/// Everything durable an installation holds, as its public interface
/// shows it. Session counts are left out: sessions do not survive a
/// restart.
#[derive(Debug, PartialEq, Eq)]
pub struct State {
    pub identities: Vec<IdentityState>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityState {
    pub identity: IdentityPublicKey,
    pub card: ContactCard,
    pub endpoint: OnionServiceKey,
    pub rotation: RotationState,
    pub successor: Option<ContactCard>,
    pub request_mode: RequestMode,
    pub invitations: Vec<(Option<DisplayName>, Vec<u8>)>,
    pub known: Vec<(IdentityPublicKey, ContactView)>,
    pub has_onion_secret: bool,
}

pub fn state(installation: &Installation) -> State {
    let mut identities: Vec<IdentityState> = installation
        .identities()
        .iter()
        .map(|identity| {
            let mut invitations: Vec<(Option<DisplayName>, Vec<u8>)> = identity
                .invitations()
                .into_iter()
                .map(|(invitation, label)| {
                    (
                        label,
                        identity.invitation_card(invitation).unwrap().encode(),
                    )
                })
                .collect();
            invitations.sort_by(|a, b| a.1.cmp(&b.1));
            IdentityState {
                identity: *identity.identity(),
                card: identity.card(),
                endpoint: identity.endpoint(),
                rotation: identity.rotation(),
                successor: identity.successor_card(),
                request_mode: identity.request_mode(),
                invitations,
                known: identity
                    .known()
                    .into_iter()
                    .map(|remote| {
                        let mut view = identity.contact(&remote).unwrap();
                        view.sessions = 0;
                        (remote, view)
                    })
                    .collect(),
                has_onion_secret: identity.onion_secret().is_some(),
            }
        })
        .collect();
    identities.sort_by(|a, b| a.identity.as_bytes().cmp(b.identity.as_bytes()));
    State { identities }
}

/// Lets held writes go on when dropped, also when an assertion fails, so
/// that a failing test ends instead of waiting for a write it held.
pub struct Released<'a>(pub &'a MemoryDir);

impl Drop for Released<'_> {
    fn drop(&mut self) {
        self.0.release_writes();
    }
}

/// Waits until `dir` holds a write.
pub async fn held(dir: &MemoryDir) {
    while !dir.write_held() {
        tokio::time::sleep(core::time::Duration::from_millis(1)).await;
    }
}

/// `a`, in a rotation, announces its successor to `b` on a session of
/// its old key and records that, as a node does. Returns whether the
/// record was made.
pub async fn announce(a: &Node, b: &mut Node) -> bool {
    let (a_link, _b_link) = confirm_both(a, b).await;
    let announcement = a
        .identity
        .announcement_for(a_link.session_ref())
        .expect("an announcement is due");
    a.identity.mark_announced(&announcement).await.unwrap()
}

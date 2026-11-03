//! File transfers across crashes and restarts of a persistent installation
//! (`docs/adr/0013-file-transfer.md`, "Completion";
//! `docs/adr/0014-transfer-staging.md`): the receiver resumes from what it
//! holds durably and acknowledges a completion again without a second
//! file; the sender offers again what it recorded, and is completed again
//! when it lost the acknowledgement; a staging cut short leaves no offer
//! and no blob; a save cut short is reported and leaves the file.
//!
//! The persistent side runs on an in-memory directory that models what a
//! crash leaves (`MemoryDir::restart`); the other side is driven by hand.
//! Time is paused and advances only when every task waits.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation
)]

mod common;

use core::time::Duration;
use std::io::{Cursor, Read};
use std::sync::Arc;

use common::{Released, fixed_endpoint, keys, run_paused};
use monolith_core::api::{
    ContactId, Event, IdentityId, MessageStatus, TransferError, TransferReason, TransferRef,
    TransferStatus, TransferView,
};
use monolith_core::budget::Budgets;
use monolith_core::identity::{Installation, LocalIdentity};
use monolith_core::link::{Link, dial};
use monolith_core::runtime::{Runtime, RuntimeOptions};
use monolith_protocol::body::{FileChunk, Message, TransferId};
use monolith_protocol::limits::{MAX_FILE_CHUNK_LEN, TRANSFER_HOLD};
use monolith_protocol::session::Action;
use monolith_protocol::text::Filename;
use monolith_storage::StorageError;
use monolith_storage::dir::{CrashOutcome, MemoryDir, VaultDir};
use monolith_storage::vault::{KdfParams, Passphrase};
use monolith_tor::{MockNetwork, MockTorBackend};
use sha2::{Digest, Sha256};
use tokio::io::DuplexStream;
use tokio::sync::broadcast;

const L: u64 = MAX_FILE_CHUNK_LEN as u64;
const QUIET: Duration = Duration::from_secs(5);

fn passphrase() -> Passphrase {
    Passphrase::new("correct horse battery staple").unwrap()
}

fn options() -> RuntimeOptions {
    RuntimeOptions {
        startup_spread: Duration::ZERO,
        ..RuntimeOptions::default()
    }
}

/// An installation with one identity of fixed keys, and its runtime once
/// it is started: persistent on `dir` if it has one.
struct Node {
    seed: u8,
    dir: Option<MemoryDir>,
    installation: Option<Installation>,
    identity: Option<Arc<LocalIdentity>>,
    runtime: Option<Runtime<MockTorBackend>>,
}

impl Node {
    async fn new(seed: u8, persistent: bool) -> Self {
        let dir = persistent.then(MemoryDir::new);
        let installation = match &dir {
            Some(dir) => {
                Installation::create(Box::new(dir.clone()), &passphrase(), KdfParams::FLOOR)
                    .unwrap()
            }
            None => Installation::ephemeral(),
        };
        let identity = installation
            .restore_identity(keys(seed, seed, 1, fixed_endpoint(seed)), None)
            .await
            .unwrap();
        Self {
            seed,
            dir,
            installation: Some(installation),
            identity: Some(identity),
            runtime: None,
        }
    }

    fn identity(&self) -> &Arc<LocalIdentity> {
        self.identity.as_ref().expect("open")
    }

    fn id(&self) -> IdentityId {
        self.identity().id()
    }

    fn dir(&self) -> &MemoryDir {
        self.dir.as_ref().expect("persistent")
    }

    fn runtime(&self) -> &Runtime<MockTorBackend> {
        self.runtime.as_ref().expect("started")
    }

    async fn start(&mut self, network: &MockNetwork) {
        self.runtime = Some(Runtime::start(
            self.installation.clone().expect("open"),
            network.backend(),
            Budgets::new(),
            options(),
        ));
        let endpoint = fixed_endpoint(self.seed);
        until(|| network.is_published(&endpoint)).await;
    }

    async fn stop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown(Duration::ZERO).await;
        }
    }

    /// The process ends where it is; its directory keeps what `outcome`
    /// says a crash leaves.
    async fn crash(&mut self, outcome: CrashOutcome) {
        self.stop().await;
        self.identity = None;
        self.installation = None;
        self.dir = Some(self.dir().restart(outcome));
    }

    /// The process starts again and opens its vault.
    fn open(&mut self) {
        let (installation, _) =
            Installation::open(Box::new(self.dir().clone()), &passphrase()).unwrap();
        self.identity = installation.identities().into_iter().next();
        self.installation = Some(installation);
    }

    async fn add(&self, other: &Node) -> ContactId {
        self.identity()
            .import(&other.identity().card())
            .await
            .unwrap();
        self.identity()
            .contact_id(other.identity().identity())
            .expect("a record")
    }

    fn view(&self, reference: &TransferRef) -> Option<TransferView> {
        self.runtime().transfer(reference).unwrap()
    }

    fn status(&self, reference: &TransferRef) -> Option<TransferStatus> {
        self.view(reference).map(|view| view.status)
    }

    /// The blobs in the directory.
    fn blobs(&self) -> Vec<String> {
        self.dir()
            .names()
            .into_iter()
            .filter(|name| name.starts_with("transfer-"))
            .collect()
    }
}

async fn until(condition: impl Fn() -> bool) {
    for _ in 0..(24 * 60 * 60 * 10) {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the condition did not come about");
}

async fn event(events: &mut broadcast::Receiver<Event>, matches: impl Fn(&Event) -> bool) -> Event {
    let wait = async {
        loop {
            match events.recv().await {
                Ok(event) if matches(&event) => return event,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => panic!("no more events"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(24 * 60 * 60), wait)
        .await
        .expect("the event did not come")
}

/// `persistent` and `by_hand`, accepted contacts of each other, with no
/// session left between them; `by_hand` stopped, to be driven by hand.
async fn contacts(network: &MockNetwork) -> (Node, Node, ContactId, ContactId) {
    let mut persistent = Node::new(1, true).await;
    let mut by_hand = Node::new(2, false).await;
    let at_persistent = persistent.add(&by_hand).await;
    let at_hand = by_hand.add(&persistent).await;
    persistent.start(network).await;
    by_hand.start(network).await;
    by_hand
        .runtime()
        .send_message(&by_hand.id(), &at_hand, "hello")
        .await
        .unwrap();
    until(|| {
        by_hand
            .runtime()
            .messages(&by_hand.id(), &at_hand, None, 10)
            .unwrap()
            .iter()
            .all(|message| message.status == MessageStatus::Delivered)
    })
    .await;
    until(|| {
        persistent
            .runtime()
            .sessions(&persistent.id(), &at_persistent)
            .unwrap()
            == 0
    })
    .await;
    by_hand.stop().await;
    (persistent, by_hand, at_persistent, at_hand)
}

/// A session from `by` to `to`, driven by hand at `by`.
async fn hand_link(network: &MockNetwork, to: &Node, by: &Node) -> Link<DuplexStream> {
    let established = dial(
        &network.backend(),
        &Budgets::new(),
        by.identity(),
        &to.identity().card(),
    )
    .await
    .unwrap();
    let mut link = established.link;
    for action in established.first {
        assert_eq!(action, Action::SendContactAccept);
        link.send(&Message::ContactAccept).await.unwrap();
    }
    loop {
        let arrived = link.receive().await.unwrap();
        if arrived.received().actions.contains(&Action::Confirmed) {
            return link;
        }
    }
}

/// What arrives on `link` until nothing more comes for `quiet`, Pings
/// answered and left out; and whether the session ended.
async fn drain(link: &mut Link<DuplexStream>, quiet: Duration) -> (Vec<Message>, bool) {
    let mut messages = Vec::new();
    loop {
        match tokio::time::timeout(quiet, link.receive()).await {
            Err(_) => return (messages, false),
            Ok(Err(_)) => return (messages, true),
            Ok(Ok(arrived)) => match &arrived.received().message {
                Message::Ping(nonce) => {
                    if link.send(&Message::Pong(*nonce)).await.is_err() {
                        return (messages, true);
                    }
                }
                other => messages.push(other.clone()),
            },
        }
    }
}

fn file(len: u64) -> Vec<u8> {
    (0..len).map(|i| (i * 37 % 253) as u8).collect()
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn chunk(transfer: TransferId, data: &[u8], offset: u64) -> Message {
    let start = offset as usize;
    let end = (start + L as usize).min(data.len());
    Message::FileChunk(FileChunk::new(transfer, data[start..end].to_vec()).unwrap())
}

fn acceptances(messages: &[Message], transfer: TransferId) -> Vec<u64> {
    messages
        .iter()
        .filter_map(|message| match message {
            Message::FileAccept {
                transfer: named,
                offset,
            } if *named == transfer => Some(*offset),
            _ => None,
        })
        .collect()
}

const TRANSFER: TransferId = TransferId::from_bytes([0x5A; 16]);

fn offer_of(data: &[u8]) -> Message {
    Message::FileOffer {
        transfer: TRANSFER,
        size: data.len() as u64,
        filename: Filename::new("notes.txt").unwrap(),
    }
}

/// The hand side offers `data` on `link` and the persistent receiver
/// accepts: the transfer as the receiver names it.
async fn offered_and_accepted(
    receiver: &Node,
    link: &mut Link<DuplexStream>,
    data: &[u8],
) -> TransferRef {
    let mut events = receiver.runtime().subscribe();
    link.send(&offer_of(data)).await.unwrap();
    let Event::TransferOffered(received) = event(&mut events, |event| {
        matches!(event, Event::TransferOffered(_))
    })
    .await
    else {
        panic!("not an offer");
    };
    receiver.runtime().accept_transfer(&received).await.unwrap();
    let (messages, _) = drain(link, QUIET).await;
    assert_eq!(acceptances(&messages, TRANSFER), vec![0]);
    received
}

/// Offers `data` again on a new session: the offset the receiver answers.
async fn offer_again(link: &mut Link<DuplexStream>, data: &[u8]) -> Vec<Message> {
    link.send(&offer_of(data)).await.unwrap();
    let (messages, ended) = drain(link, QUIET).await;
    assert!(!ended);
    messages
}

/// Sends `data` from `offset` to the end, then its completion; returns
/// what the receiver answered.
async fn send_rest(link: &mut Link<DuplexStream>, data: &[u8], offset: u64) -> Vec<Message> {
    let mut offset = offset;
    while offset < data.len() as u64 {
        link.send(&chunk(TRANSFER, data, offset)).await.unwrap();
        offset = (offset + L).min(data.len() as u64);
    }
    link.send(&Message::FileComplete {
        transfer: TRANSFER,
        digest: sha(data),
    })
    .await
    .unwrap();
    drain(link, QUIET).await.0
}

async fn saved(node: &Node, reference: &TransferRef) -> Vec<u8> {
    let mut reader = node.runtime().begin_save(reference).await.unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    drop(reader);
    node.runtime().end_save(reference, true).await.unwrap();
    bytes
}

#[test]
fn a_receiver_that_crashes_resumes_from_what_it_holds_durably() {
    for outcome in CrashOutcome::ALL {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut b, a, _, _) = contacts(&network).await;
            let data = file(6 * L + 3);
            let mut link = hand_link(&network, &b, &a).await;
            let received = offered_and_accepted(&b, &mut link, &data).await;
            // Three chunks: two acknowledged once synced, the third only
            // written.
            for n in 0..3 {
                link.send(&chunk(TRANSFER, &data, n * L)).await.unwrap();
            }
            assert_eq!(
                acceptances(&drain(&mut link, QUIET).await.0, TRANSFER),
                vec![2 * L]
            );
            b.crash(outcome).await;
            b.open();
            b.start(&network).await;
            assert_eq!(b.status(&received), Some(TransferStatus::Receiving));
            // The offset it gives is what it holds durably: never less
            // than it acknowledged, never what a crash may have torn.
            let mut link = hand_link(&network, &b, &a).await;
            let offsets = acceptances(&offer_again(&mut link, &data).await, TRANSFER);
            assert_eq!(offsets, vec![2 * L], "{outcome:?}");
            let answer = send_rest(&mut link, &data, 2 * L).await;
            assert!(
                answer.contains(&Message::FileComplete {
                    transfer: TRANSFER,
                    digest: sha(&data),
                }),
                "{outcome:?}"
            );
            assert_eq!(saved(&b, &received).await, data, "{outcome:?}");
            b.stop().await;
        });
    }
}

/// The window binds the sender only (`docs/adr/0013-file-transfer.md`,
/// "Data, window and progress"): a receiver cannot know which of its
/// acknowledgements the sender read. A sender that ignores it gets no more
/// staged than the size it offered, holds up no chat of the receiver, and
/// a chunk beyond the size ends its session.
#[test]
fn a_sender_that_ignores_the_window_stages_no_more_than_the_file_and_holds_up_no_chat() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut b, a, at_b, _) = contacts(&network).await;
        let data = file(24 * L + 9);
        let mut link = hand_link(&network, &b, &a).await;
        let received = offered_and_accepted(&b, &mut link, &data).await;
        let blobs = b.blobs();
        assert_eq!(blobs.len(), 1);
        let reserved = b.dir().content(&blobs[0]).unwrap().len();
        let mut events = b.runtime().subscribe();
        // The whole file at once, never waiting for an acknowledgement, and
        // then one chunk more than the file.
        let sent = data.clone();
        let flood = tokio::spawn(async move {
            let mut offset = 0;
            while offset < sent.len() as u64 {
                link.send(&chunk(TRANSFER, &sent, offset)).await.unwrap();
                offset = (offset + L).min(sent.len() as u64);
            }
            let more = Message::FileChunk(FileChunk::new(TRANSFER, vec![7; L as usize]).unwrap());
            let refused = link.send(&more).await.is_err();
            let (messages, ended) = drain(&mut link, QUIET).await;
            (messages, ended || refused)
        });
        // Chat queued while the chunks pour in goes out between them.
        event(&mut events, |event| {
            matches!(event, Event::TransferProgressed { reference, .. } if *reference == received)
        })
        .await;
        b.runtime()
            .send_message(&b.id(), &at_b, "while the file pours in")
            .await
            .unwrap();
        let (messages, ended) = flood.await.unwrap();
        assert!(ended, "a chunk beyond the size ends the session");
        let chat = messages
            .iter()
            .position(|message| matches!(message, Message::ChatMessage { .. }))
            .expect("the chat message");
        let offsets = acceptances(&messages, TRANSFER);
        assert!(
            offsets.windows(2).all(|pair| pair[0] < pair[1]),
            "{offsets:?}"
        );
        assert!(offsets.iter().all(|offset| *offset <= data.len() as u64));
        let late = messages
            .iter()
            .position(|message| {
                matches!(message, Message::FileAccept { offset, .. } if *offset >= 10 * L)
            })
            .expect("acknowledgements up to the end");
        assert!(chat < late, "the chat waited behind the file: {messages:?}");
        // Nothing beyond the reservation, and nothing staged beyond the size.
        assert_eq!(b.blobs(), blobs);
        assert_eq!(b.dir().content(&blobs[0]).unwrap().len(), reserved);
        let view = b.view(&received).unwrap();
        assert_eq!(view.status, TransferStatus::Receiving);
        assert!(view.bytes.is_some_and(|bytes| bytes <= data.len() as u64));
        // A sender that keeps to the rules then completes it.
        let mut link = hand_link(&network, &b, &a).await;
        let from = acceptances(&offer_again(&mut link, &data).await, TRANSFER);
        assert_eq!(from.len(), 1);
        let answer = send_rest(&mut link, &data, from[0]).await;
        assert!(answer.contains(&Message::FileComplete {
            transfer: TRANSFER,
            digest: sha(&data),
        }));
        assert_eq!(saved(&b, &received).await, data);
        b.stop().await;
    });
}

#[test]
fn a_receiver_that_crashes_after_its_completion_acknowledges_it_again_with_one_file() {
    for outcome in CrashOutcome::ALL {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut b, a, _, _) = contacts(&network).await;
            let data = file(2 * L + 5);
            let mut link = hand_link(&network, &b, &a).await;
            let received = offered_and_accepted(&b, &mut link, &data).await;
            let answer = send_rest(&mut link, &data, 0).await;
            assert!(answer.contains(&Message::FileComplete {
                transfer: TRANSFER,
                digest: sha(&data),
            }));
            // The completion was durable before it was answered: whatever
            // the crash leaves, it is answered again, and the file is one.
            b.crash(outcome).await;
            b.open();
            b.start(&network).await;
            let mut link = hand_link(&network, &b, &a).await;
            assert_eq!(
                offer_again(&mut link, &data).await,
                vec![Message::FileComplete {
                    transfer: TRANSFER,
                    digest: sha(&data),
                }],
                "{outcome:?}"
            );
            assert_eq!(b.runtime().transfers(&b.id()).unwrap().len(), 1);
            assert_eq!(b.status(&received), Some(TransferStatus::Received));
            assert_eq!(saved(&b, &received).await, data, "{outcome:?}");
            b.stop().await;
        });
    }
}

#[test]
fn a_receiver_that_crashes_before_verifying_verifies_again() {
    for outcome in CrashOutcome::ALL {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut b, a, _, _) = contacts(&network).await;
            let data = file(4 * L);
            let mut link = hand_link(&network, &b, &a).await;
            let received = offered_and_accepted(&b, &mut link, &data).await;
            // Every byte, but no completion yet.
            for n in 0..4 {
                link.send(&chunk(TRANSFER, &data, n * L)).await.unwrap();
            }
            assert_eq!(
                acceptances(&drain(&mut link, QUIET).await.0, TRANSFER),
                vec![2 * L, 4 * L]
            );
            b.crash(outcome).await;
            b.open();
            b.start(&network).await;
            let mut link = hand_link(&network, &b, &a).await;
            // Every byte is held; the completion follows at once.
            let offsets = acceptances(&offer_again(&mut link, &data).await, TRANSFER);
            assert_eq!(offsets, vec![4 * L], "{outcome:?}");
            let answer = send_rest(&mut link, &data, 4 * L).await;
            assert!(answer.contains(&Message::FileComplete {
                transfer: TRANSFER,
                digest: sha(&data),
            }));
            assert_eq!(saved(&b, &received).await, data, "{outcome:?}");
            b.stop().await;
        });
    }
}

#[test]
fn what_is_shown_of_a_transfer_is_durable_and_no_crash_takes_it_back() {
    for outcome in CrashOutcome::ALL {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut b, a, _, _) = contacts(&network).await;
            let first = b.dir().clone();
            let _released = Released(&first);
            let data = file(2 * L + 3);
            let mut link = hand_link(&network, &b, &a).await;
            drain(&mut link, QUIET).await;
            // An offer whose record is not written yet is not listed.
            b.dir().hold_writes();
            link.send(&offer_of(&data)).await.unwrap();
            spin_until(|| b.dir().write_held()).await;
            assert!(b.runtime().transfers(&b.id()).unwrap().is_empty());
            b.dir().release_writes();
            until(|| !b.runtime().transfers(&b.id()).unwrap().is_empty()).await;
            let received = b.runtime().transfers(&b.id()).unwrap()[0].reference;
            b.runtime().accept_transfer(&received).await.unwrap();
            for n in 0..3 {
                link.send(&chunk(TRANSFER, &data, n * L)).await.unwrap();
            }
            drain(&mut link, QUIET).await;
            // Every byte is staged and verified as the completion arrives,
            // and its write is held: the file is not shown received, nor
            // after a crash at that point.
            b.dir().hold_writes();
            link.send(&Message::FileComplete {
                transfer: TRANSFER,
                digest: sha(&data),
            })
            .await
            .unwrap();
            spin_until(|| b.dir().write_held()).await;
            assert_eq!(b.status(&received), Some(TransferStatus::Receiving));
            let crashed = b.dir().restart(outcome);
            b.dir().release_writes();
            b.stop().await;
            b.identity = None;
            b.installation = None;
            b.dir = Some(crashed);
            b.open();
            b.start(&network).await;
            assert_eq!(
                b.status(&received),
                Some(TransferStatus::Receiving),
                "{outcome:?}"
            );
            // Offered again, it completes from what it holds.
            let mut link = hand_link(&network, &b, &a).await;
            let offsets = acceptances(&offer_again(&mut link, &data).await, TRANSFER);
            assert_eq!(offsets, vec![2 * L + 3], "{outcome:?}");
            let answer = send_rest(&mut link, &data, 2 * L + 3).await;
            assert!(answer.contains(&Message::FileComplete {
                transfer: TRANSFER,
                digest: sha(&data),
            }));
            assert_eq!(b.status(&received), Some(TransferStatus::Received));
            assert_eq!(saved(&b, &received).await, data, "{outcome:?}");
            b.stop().await;
        });
    }
}

/// Waits, with paused time, until `condition` holds, for up to a minute
/// of test time. Returns whether it did.
async fn within_a_minute(condition: impl Fn() -> bool) -> bool {
    for _ in 0..600 {
        if condition() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    condition()
}

/// Offers `data` on `link` to `b`, which accepts it, and sends it whole
/// with its completion, as far as `b` lets it: its storage may fail on the
/// way. Returns the transfer as `b` lists it, if it does.
async fn receive_through(
    b: &Node,
    link: &mut Link<DuplexStream>,
    data: &[u8],
) -> Option<TransferRef> {
    let listed = || {
        b.runtime()
            .transfers(&b.id())
            .ok()
            .and_then(|views| views.first().map(|view| view.reference))
    };
    if link.send(&offer_of(data)).await.is_err() {
        return None;
    }
    within_a_minute(|| b.dir().crashed() || listed().is_some()).await;
    let received = listed()?;
    if b.runtime().accept_transfer(&received).await.is_err() {
        return Some(received);
    }
    // Until the session ends: a receiver whose storage failed withdraws it.
    let mut offset = 0;
    while offset < data.len() as u64 {
        if link.send(&chunk(TRANSFER, data, offset)).await.is_err() {
            return Some(received);
        }
        offset = (offset + L).min(data.len() as u64);
    }
    let completion = Message::FileComplete {
        transfer: TRANSFER,
        digest: sha(data),
    };
    if link.send(&completion).await.is_ok() {
        drain(link, QUIET).await;
    }
    Some(received)
}

#[test]
fn a_crash_at_any_step_of_receiving_shows_nothing_it_takes_back_and_the_file_completes_whole() {
    // One thread for each crash outcome: the sweep is long.
    std::thread::scope(|scope| {
        for outcome in CrashOutcome::ALL {
            scope.spawn(move || run_paused(sweep_receiving(outcome)));
        }
    });
}

/// Crashes the receiver at every storage step of a transfer, from the
/// offer to the completion, leaving what `outcome` says.
async fn sweep_receiving(outcome: CrashOutcome) {
    let network = MockNetwork::new();
    let (mut b, a, _, _) = contacts(&network).await;
    b.stop().await;
    let base = b.dir().restart(CrashOutcome::ALL[0]);
    let data = file(3 * L + 5);
    // The storage steps of the whole transfer, with no crash.
    b.identity = None;
    b.installation = None;
    b.dir = Some(base.restart(CrashOutcome::ALL[0]));
    b.open();
    b.start(&network).await;
    let start = b.dir().steps();
    let mut link = hand_link(&network, &b, &a).await;
    let received = receive_through(&b, &mut link, &data).await.unwrap();
    assert_eq!(b.status(&received), Some(TransferStatus::Received));
    let steps = b.dir().steps() - start;
    b.stop().await;
    for step in 1..=steps {
        b.identity = None;
        b.installation = None;
        b.dir = Some(base.restart(CrashOutcome::ALL[0]));
        b.open();
        b.start(&network).await;
        let mut link = hand_link(&network, &b, &a).await;
        b.dir().crash_at(b.dir().steps() + step);
        let shown = receive_through(&b, &mut link, &data).await;
        let before = shown.and_then(|received| b.status(&received));
        b.crash(outcome).await;
        b.open();
        b.start(&network).await;
        let at = format!("step {step} of {steps}, {outcome:?}");
        // What was shown stays shown: nothing goes back, and a file
        // shown received is held whole.
        let after = shown.and_then(|received| b.status(&received));
        match before {
            None => {}
            Some(TransferStatus::Received) => {
                assert_eq!(after, Some(TransferStatus::Received), "{at}");
            }
            Some(TransferStatus::Receiving) => assert!(
                matches!(
                    after,
                    Some(TransferStatus::Receiving | TransferStatus::Received)
                ),
                "{at}: {after:?}"
            ),
            Some(TransferStatus::Offered) => assert!(
                matches!(
                    after,
                    Some(TransferStatus::Offered | TransferStatus::Receiving)
                ),
                "{at}: {after:?}"
            ),
            Some(other) => panic!("{at}: shown {other:?}"),
        }
        // Offered again, it goes on from what is held, and is
        // received whole.
        let mut link = hand_link(&network, &b, &a).await;
        let received = match after {
            Some(TransferStatus::Received) => shown.unwrap(),
            Some(TransferStatus::Receiving) => {
                let offsets = acceptances(&offer_again(&mut link, &data).await, TRANSFER);
                assert_eq!(offsets.len(), 1, "{at}: {offsets:?}");
                assert!(offsets[0] <= data.len() as u64, "{at}");
                let answer = send_rest(&mut link, &data, offsets[0]).await;
                assert!(
                    answer.contains(&Message::FileComplete {
                        transfer: TRANSFER,
                        digest: sha(&data),
                    }),
                    "{at}"
                );
                shown.unwrap()
            }
            _ => {
                let received = receive_through(&b, &mut link, &data).await.unwrap();
                assert!(!b.dir().crashed(), "{at}");
                received
            }
        };
        assert_eq!(b.status(&received), Some(TransferStatus::Received), "{at}");
        assert_eq!(saved(&b, &received).await, data, "{at}");
        b.stop().await;
    }
}

#[test]
fn a_cancellation_of_the_sender_that_meets_the_completion_ends_completed_in_every_order() {
    // The user cancels before the receiver's completion arrives; the
    // completion arrives while the write of the cancellation is held; the
    // cancellation comes while the write of the completion is held; or the
    // user cancels after the completion. Each ends as one of the two serial
    // runs does, and both end completed: a cancellation never takes back a
    // file the receiver holds.
    for order in [
        "cancel first",
        "cancel held",
        "completion held",
        "completion first",
    ] {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut a, b, at_a, _) = contacts(&network).await;
            let first = a.dir().clone();
            let _released = Released(&first);
            let data = file(2 * L + 9);
            let sent = a
                .runtime()
                .offer_file(
                    &a.id(),
                    &at_a,
                    "report.pdf",
                    data.len() as u64,
                    Box::new(Cursor::new(data.clone())),
                )
                .await
                .unwrap();
            let mut link = hand_link(&network, &a, &b).await;
            let (transfer, size, _) = offer_in(&drain(&mut link, QUIET).await.0);
            assert_eq!(receive_rest(&mut link, transfer, size, 0).await, data);
            let completion = Message::FileComplete {
                transfer,
                digest: sha(&data),
            };
            match order {
                "cancel first" => {
                    a.runtime().cancel_transfer(&sent).await.unwrap();
                    assert_eq!(a.status(&sent), Some(TransferStatus::Cancelled));
                    link.send(&completion).await.unwrap();
                    drain(&mut link, QUIET).await;
                }
                "cancel held" => {
                    a.dir().hold_writes();
                    let runtime = a.runtime().clone();
                    let cancelling =
                        tokio::spawn(async move { runtime.cancel_transfer(&sent).await });
                    spin_until(|| a.dir().write_held()).await;
                    // The session reads the completion and waits behind the
                    // held write. No timer runs while it is held: the test
                    // gives the tasks turns, not time.
                    link.send(&completion).await.unwrap();
                    for _ in 0..1000 {
                        tokio::task::yield_now().await;
                    }
                    // Neither is shown while neither is durable.
                    assert_eq!(a.status(&sent), Some(TransferStatus::Sending));
                    a.dir().release_writes();
                    cancelling.await.unwrap().unwrap();
                }
                "completion held" => {
                    a.dir().hold_writes();
                    link.send(&completion).await.unwrap();
                    spin_until(|| a.dir().write_held()).await;
                    assert_eq!(a.status(&sent), Some(TransferStatus::Sending));
                    let runtime = a.runtime().clone();
                    let cancelling =
                        tokio::spawn(async move { runtime.cancel_transfer(&sent).await });
                    for _ in 0..1000 {
                        tokio::task::yield_now().await;
                    }
                    a.dir().release_writes();
                    cancelling.await.unwrap().unwrap();
                }
                _ => {
                    link.send(&completion).await.unwrap();
                    drain(&mut link, QUIET).await;
                    assert_eq!(a.status(&sent), Some(TransferStatus::Completed));
                    a.runtime().cancel_transfer(&sent).await.unwrap();
                }
            }
            within_a_minute(|| a.status(&sent) == Some(TransferStatus::Completed)).await;
            assert_eq!(a.status(&sent), Some(TransferStatus::Completed), "{order}");
            // And so it stays, the installation opened again.
            a.crash(CrashOutcome::ALL[0]).await;
            a.open();
            a.start(&network).await;
            assert_eq!(a.status(&sent), Some(TransferStatus::Completed), "{order}");
            a.stop().await;
        });
    }
}

/// The offer the persistent sender makes on `link`.
fn offer_in(messages: &[Message]) -> (TransferId, u64, Filename) {
    messages
        .iter()
        .find_map(|message| match message {
            Message::FileOffer {
                transfer,
                size,
                filename,
            } => Some((*transfer, *size, filename.clone())),
            _ => None,
        })
        .expect("an offer")
}

/// Receives by hand, on `link`, the transfer `transfer` from `offset` to
/// the end: every chunk acknowledged, then the completion of the sender.
async fn receive_rest(
    link: &mut Link<DuplexStream>,
    transfer: TransferId,
    size: u64,
    offset: u64,
) -> Vec<u8> {
    link.send(&Message::FileAccept { transfer, offset })
        .await
        .unwrap();
    let mut data = Vec::new();
    let mut held = offset;
    loop {
        let (messages, ended) = drain(link, QUIET).await;
        assert!(!ended);
        for message in &messages {
            if let Message::FileChunk(chunk) = message {
                data.extend_from_slice(chunk.data());
                held += chunk.data().len() as u64;
            }
            if let Message::FileComplete { .. } = message {
                assert_eq!(held, size);
                return data;
            }
        }
        let acknowledged = if held == size { size } else { held / L * L };
        link.send(&Message::FileAccept {
            transfer,
            offset: acknowledged,
        })
        .await
        .unwrap();
    }
}

#[test]
fn a_sender_that_crashes_after_its_offer_offers_it_again_and_resumes() {
    for outcome in CrashOutcome::ALL {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut a, b, at_a, _) = contacts(&network).await;
            let data = file(5 * L + 1);
            let sent = a
                .runtime()
                .offer_file(
                    &a.id(),
                    &at_a,
                    "report.pdf",
                    data.len() as u64,
                    Box::new(Cursor::new(data.clone())),
                )
                .await
                .unwrap();
            let mut link = hand_link(&network, &a, &b).await;
            let (transfer, size, name) = offer_in(&drain(&mut link, QUIET).await.0);
            link.send(&Message::FileAccept {
                transfer,
                offset: 0,
            })
            .await
            .unwrap();
            let (messages, _) = drain(&mut link, QUIET).await;
            let first: Vec<u8> = messages
                .iter()
                .filter_map(|message| match message {
                    Message::FileChunk(chunk) => Some(chunk.data().to_vec()),
                    _ => None,
                })
                .take(2)
                .flatten()
                .collect();
            assert_eq!(first, data[..2 * L as usize]);
            // The offer was durable when it returned: it is offered again,
            // the same, and sent from what the receiver holds.
            a.crash(outcome).await;
            a.open();
            a.start(&network).await;
            assert_eq!(a.status(&sent), Some(TransferStatus::Sending));
            let mut link = hand_link(&network, &a, &b).await;
            let again = offer_in(&drain(&mut link, QUIET).await.0);
            assert_eq!(again, (transfer, size, name), "{outcome:?}");
            let rest = receive_rest(&mut link, transfer, size, 2 * L).await;
            assert_eq!(rest, data[2 * L as usize..], "{outcome:?}");
            link.send(&Message::FileComplete {
                transfer,
                digest: sha(&data),
            })
            .await
            .unwrap();
            drain(&mut link, QUIET).await;
            assert_eq!(a.status(&sent), Some(TransferStatus::Completed));
            // Delivered: the staged copy goes.
            until(|| a.blobs().is_empty()).await;
            a.stop().await;
        });
    }
}

#[test]
fn a_sender_that_crashes_before_recording_the_completion_is_completed_again() {
    for outcome in CrashOutcome::ALL {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut a, b, at_a, _) = contacts(&network).await;
            let data = file(L + 7);
            let sent = a
                .runtime()
                .offer_file(
                    &a.id(),
                    &at_a,
                    "report.pdf",
                    data.len() as u64,
                    Box::new(Cursor::new(data.clone())),
                )
                .await
                .unwrap();
            let mut link = hand_link(&network, &a, &b).await;
            let (transfer, size, _) = offer_in(&drain(&mut link, QUIET).await.0);
            let received = receive_rest(&mut link, transfer, size, 0).await;
            assert_eq!(received, data);
            // The process stops as the acknowledgement arrives: nothing it
            // would write is written.
            let steps = a.dir().steps();
            a.dir().crash_at(steps + 1);
            link.send(&Message::FileComplete {
                transfer,
                digest: sha(&data),
            })
            .await
            .unwrap();
            drain(&mut link, QUIET).await;
            a.crash(outcome).await;
            a.open();
            a.start(&network).await;
            assert_eq!(a.status(&sent), Some(TransferStatus::Sending));
            // Offered again, it is acknowledged again, and delivered once.
            let mut link = hand_link(&network, &a, &b).await;
            let (again, _, _) = offer_in(&drain(&mut link, QUIET).await.0);
            assert_eq!(again, transfer);
            link.send(&Message::FileComplete {
                transfer,
                digest: sha(&data),
            })
            .await
            .unwrap();
            drain(&mut link, QUIET).await;
            assert_eq!(
                a.status(&sent),
                Some(TransferStatus::Completed),
                "{outcome:?}"
            );
            assert_eq!(a.runtime().transfers(&a.id()).unwrap().len(), 1);
            a.stop().await;
        });
    }
}

#[test]
fn a_staging_cut_short_by_a_crash_leaves_no_offer_and_no_blob() {
    for outcome in CrashOutcome::ALL {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut a, _b, at_a, _) = contacts(&network).await;
            let data = file(8 * L);
            // The crash comes in the middle of the writes of the blob.
            let steps = a.dir().steps();
            a.dir().crash_at(steps + 6);
            let offered = a
                .runtime()
                .offer_file(
                    &a.id(),
                    &at_a,
                    "report.pdf",
                    data.len() as u64,
                    Box::new(Cursor::new(data.clone())),
                )
                .await;
            assert_eq!(offered, Err(TransferError::Storage), "{outcome:?}");
            a.crash(outcome).await;
            a.open();
            a.start(&network).await;
            assert!(a.runtime().transfers(&a.id()).unwrap().is_empty());
            assert!(a.blobs().is_empty(), "{outcome:?}: {:?}", a.blobs());
            a.stop().await;
        });
    }
}

#[test]
fn a_source_that_gives_fewer_or_more_bytes_than_its_size_offers_nothing() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, _) = contacts(&network).await;
        for given in [9_usize, 11] {
            let offered = a
                .runtime()
                .offer_file(
                    &a.id(),
                    &at_a,
                    "report.pdf",
                    10,
                    Box::new(Cursor::new(vec![1_u8; given])),
                )
                .await;
            assert_eq!(offered, Err(TransferError::Source), "{given}");
        }
        assert!(a.runtime().transfers(&a.id()).unwrap().is_empty());
        until(|| a.blobs().is_empty()).await;
        a.stop().await;
    });
}

#[test]
fn a_save_cut_short_by_a_crash_is_reported_and_the_file_kept() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut b, a, _, _) = contacts(&network).await;
        let data = file(3 * L);
        let mut link = hand_link(&network, &b, &a).await;
        let received = offered_and_accepted(&b, &mut link, &data).await;
        send_rest(&mut link, &data, 0).await;
        let mut reader = b.runtime().begin_save(&received).await.unwrap();
        let mut part = [0_u8; 100];
        reader.read_exact(&mut part).unwrap();
        drop(reader);
        // The process ends while the save is under way.
        b.crash(CrashOutcome::ALL[0]).await;
        b.open();
        b.start(&network).await;
        assert_eq!(
            b.runtime().interrupted_saves(&b.id()).unwrap(),
            vec![received]
        );
        // The file is still there, and is saved whole.
        assert_eq!(b.status(&received), Some(TransferStatus::Received));
        assert_eq!(saved(&b, &received).await, data);
        // Once.
        b.crash(CrashOutcome::ALL[0]).await;
        b.open();
        b.start(&network).await;
        assert!(b.runtime().interrupted_saves(&b.id()).unwrap().is_empty());
        assert!(b.view(&received).unwrap().saved);
        assert!(b.blobs().is_empty());
        b.stop().await;
    });
}

#[test]
fn a_deleted_identity_leaves_no_transfer_and_no_blob() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, _) = contacts(&network).await;
        a.runtime()
            .offer_file(
                &a.id(),
                &at_a,
                "report.pdf",
                3,
                Box::new(Cursor::new(vec![1_u8, 2, 3])),
            )
            .await
            .unwrap();
        assert_eq!(a.blobs().len(), 1);
        let id = a.id();
        a.runtime().delete_identity(&id).await.unwrap();
        until(|| a.blobs().is_empty()).await;
        a.crash(CrashOutcome::ALL[0]).await;
        a.open();
        assert!(a.identity.is_none());
        assert!(a.blobs().is_empty());
    });
}

#[test]
fn a_full_disk_holds_the_transfer_until_it_is_resumed_or_the_hold_ends() {
    for resumed in [true, false] {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut b, a, _, _) = contacts(&network).await;
            let data = file(4 * L + 1);
            let mut link = hand_link(&network, &b, &a).await;
            let received = offered_and_accepted(&b, &mut link, &data).await;
            for n in 0..2 {
                link.send(&chunk(TRANSFER, &data, n * L)).await.unwrap();
            }
            assert_eq!(
                acceptances(&drain(&mut link, QUIET).await.0, TRANSFER),
                vec![2 * L]
            );
            // The disk is full for the next chunk: nothing more is
            // acknowledged, and the session ends.
            b.dir().fail_next_write(StorageError::Full);
            link.send(&chunk(TRANSFER, &data, 2 * L)).await.unwrap();
            let (messages, ended) = drain(&mut link, QUIET).await;
            assert!(ended);
            assert!(acceptances(&messages, TRANSFER).is_empty());
            let view = b.view(&received).unwrap();
            assert_eq!(view.status, TransferStatus::Receiving);
            assert!(view.held);
            // Held: offers are answered with nothing.
            let mut link = hand_link(&network, &b, &a).await;
            assert!(offer_again(&mut link, &data).await.is_empty());
            if resumed {
                b.runtime().resume_transfer(&received).await.unwrap();
            } else {
                link.close().await.unwrap();
                tokio::time::sleep(TRANSFER_HOLD + Duration::from_secs(60)).await;
                link = hand_link(&network, &b, &a).await;
                link.send(&offer_of(&data)).await.unwrap();
            }
            // It goes on from what it holds.
            let (messages, _) = drain(&mut link, QUIET).await;
            assert_eq!(acceptances(&messages, TRANSFER), vec![2 * L], "{resumed}");
            assert!(!b.view(&received).unwrap().held);
            let answer = send_rest(&mut link, &data, 2 * L).await;
            assert!(answer.contains(&Message::FileComplete {
                transfer: TRANSFER,
                digest: sha(&data),
            }));
            assert_eq!(saved(&b, &received).await, data);
            b.stop().await;
        });
    }
}

#[test]
fn a_transfer_store_that_cannot_be_written_stops_everything_until_a_restart() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut b, a, at_b, _) = contacts(&network).await;
        let mut events = b.runtime().subscribe();
        let data = file(10);
        let mut link = hand_link(&network, &b, &a).await;
        // The next write of the directory fails: the record of the offer.
        let steps = b.dir().steps();
        b.dir().crash_at(steps + 1);
        link.send(&offer_of(&data)).await.unwrap();
        event(&mut events, |event| matches!(event, Event::StorageFailed)).await;
        let (_, ended) = drain(&mut link, QUIET).await;
        assert!(ended);
        // A session made afterwards ends at once, and none stays.
        if let Ok(established) = dial(
            &network.backend(),
            &Budgets::new(),
            a.identity(),
            &b.identity().card(),
        )
        .await
        {
            let mut link = established.link;
            let ended = loop {
                match tokio::time::timeout(QUIET, link.receive()).await {
                    Ok(Ok(_)) => {}
                    Ok(Err(_)) => break true,
                    Err(_) => break false,
                }
            };
            assert!(ended);
        }
        assert_eq!(b.runtime().sessions(&b.id(), &at_b).unwrap(), 0);
        // After a restart the offer was never recorded; made again, it is.
        b.crash(CrashOutcome::ALL[0]).await;
        b.open();
        b.start(&network).await;
        assert!(b.runtime().transfers(&b.id()).unwrap().is_empty());
        let mut events = b.runtime().subscribe();
        let mut link = hand_link(&network, &b, &a).await;
        link.send(&offer_of(&data)).await.unwrap();
        event(&mut events, |event| {
            matches!(event, Event::TransferOffered(_))
        })
        .await;
        b.stop().await;
    });
}

/// Waits until `condition` holds while a task of the blocking pool is held
/// on purpose: paused time does not advance meanwhile, so this yields
/// instead of sleeping.
async fn spin_until(condition: impl Fn() -> bool) {
    for _ in 0..100_000 {
        if condition() {
            return;
        }
        tokio::task::yield_now().await;
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("the condition did not come about");
}

/// A source that gives its first bytes, then waits until it is let go.
struct Gated {
    data: Cursor<Vec<u8>>,
    started: Option<std::sync::mpsc::Sender<()>>,
    go: std::sync::mpsc::Receiver<()>,
}

impl Read for Gated {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if let Some(started) = self.started.take() {
            let _ = started.send(());
            let _ = self.go.recv();
        }
        self.data.read(buf)
    }
}

#[test]
fn an_identity_deleted_while_its_file_is_staged_gets_no_offer_and_keeps_no_blob() {
    run_paused(async {
        let network = MockNetwork::new();
        let (a, _b, at_a, _) = contacts(&network).await;
        let (started, staging) = std::sync::mpsc::channel();
        let (go, gate) = std::sync::mpsc::channel();
        let runtime = a.runtime().clone();
        let id = a.id();
        let offering = tokio::spawn(async move {
            runtime
                .offer_file(
                    &id,
                    &at_a,
                    "report.pdf",
                    4 * L,
                    Box::new(Gated {
                        data: Cursor::new(file(4 * L)),
                        started: Some(started),
                        go: gate,
                    }),
                )
                .await
        });
        spin_until(|| staging.try_recv().is_ok()).await;
        a.runtime().delete_identity(&a.id()).await.unwrap();
        go.send(()).unwrap();
        assert!(offering.await.unwrap().is_err());
        until(|| a.blobs().is_empty()).await;
    });
}

#[test]
fn a_save_under_way_as_its_identity_is_deleted_is_cancelled_and_the_blob_then_goes() {
    run_paused(async {
        let network = MockNetwork::new();
        let (b, a, _, _) = contacts(&network).await;
        let data = file(3 * L);
        let mut link = hand_link(&network, &b, &a).await;
        let received = offered_and_accepted(&b, &mut link, &data).await;
        send_rest(&mut link, &data, 0).await;
        let mut reader = b.runtime().begin_save(&received).await.unwrap();
        let mut start = vec![0_u8; 1000];
        reader.read_exact(&mut start).unwrap();
        // The deletion does not wait for the save; the save reads nothing
        // more, and the blob, which it holds, stays until it lets go.
        b.runtime().delete_identity(&b.id()).await.unwrap();
        assert!(reader.read(&mut start).is_err());
        assert_eq!(b.blobs().len(), 1);
        drop(reader);
        until(|| b.blobs().is_empty()).await;
        assert_eq!(
            b.runtime().end_save(&received, true).await,
            Err(TransferError::UnknownIdentity)
        );
    });
}

#[test]
fn an_acceptance_as_the_contact_is_blocked_ends_in_one_order_or_the_other() {
    run_paused(async {
        let network = MockNetwork::new();
        let (b, a, at_b, _) = contacts(&network).await;
        let mut events = b.runtime().subscribe();
        let mut link = hand_link(&network, &b, &a).await;
        link.send(&offer_of(&file(3 * L))).await.unwrap();
        let Event::TransferOffered(received) = event(&mut events, |event| {
            matches!(event, Event::TransferOffered(_))
        })
        .await
        else {
            panic!("not an offer");
        };
        // The acceptance waits in the reservation of its blob while the
        // contact is blocked.
        b.dir().hold_writes();
        let runtime = b.runtime().clone();
        let accepting = tokio::spawn(async move { runtime.accept_transfer(&received).await });
        spin_until(|| b.dir().write_held()).await;
        let runtime = b.runtime().clone();
        let id = b.id();
        let blocking = tokio::spawn(async move { runtime.block_contact(&id, &at_b).await });
        spin_until(|| b.identity().current_record(&at_b).is_none()).await;
        b.dir().release_writes();
        // Refused, whichever came first: the acceptance found the contact
        // ended, or the block cancelled the offer before it.
        assert!(matches!(
            accepting.await.unwrap(),
            Err(TransferError::NotAContact | TransferError::UnknownTransfer)
        ));
        blocking.await.unwrap().unwrap();
        let view = b.view(&received).unwrap();
        assert_eq!(view.status, TransferStatus::Cancelled);
        // The blob it reserved went with it.
        until(|| b.blobs().is_empty()).await;
    });
}

#[test]
fn a_transfer_goes_on_over_the_session_that_replaces_its_own() {
    run_paused(async {
        let network = MockNetwork::new();
        let (b, a, at_b, _) = contacts(&network).await;
        let data = file(6 * L);
        let mut first = hand_link(&network, &b, &a).await;
        let received = offered_and_accepted(&b, &mut first, &data).await;
        for n in 0..2 {
            first.send(&chunk(TRANSFER, &data, n * L)).await.unwrap();
        }
        assert_eq!(
            acceptances(&drain(&mut first, QUIET).await.0, TRANSFER),
            vec![2 * L]
        );
        // A newer session of the same side replaces the first, which B
        // closes; the transfer is offered there and goes on from what B
        // holds.
        let mut second = hand_link(&network, &b, &a).await;
        assert!(drain(&mut first, QUIET).await.1);
        assert_eq!(b.runtime().sessions(&b.id(), &at_b).unwrap(), 1);
        assert_eq!(
            acceptances(&offer_again(&mut second, &data).await, TRANSFER),
            vec![2 * L]
        );
        let answer = send_rest(&mut second, &data, 2 * L).await;
        assert!(answer.contains(&Message::FileComplete {
            transfer: TRANSFER,
            digest: sha(&data),
        }));
        assert_eq!(saved(&b, &received).await, data);
    });
}

#[test]
fn a_received_file_whose_staged_copy_is_damaged_or_missing_fails_when_it_is_saved() {
    for missing in [false, true] {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut b, a, _, _) = contacts(&network).await;
            let data = file(3 * L);
            let mut link = hand_link(&network, &b, &a).await;
            let received = offered_and_accepted(&b, &mut link, &data).await;
            send_rest(&mut link, &data, 0).await;
            assert_eq!(b.status(&received), Some(TransferStatus::Received));
            // A restart, so that nothing holds the blob open; then someone
            // with access to the folder changes or removes it.
            b.crash(CrashOutcome::ALL[0]).await;
            let name = b.blobs().pop().unwrap();
            if missing {
                b.dir().remove(&name).unwrap();
            } else {
                let mut bytes = b.dir().content(&name).unwrap();
                let at = bytes.len() - 100;
                bytes[at] ^= 1;
                b.dir().overwrite(&name, &bytes).unwrap();
            }
            b.open();
            b.start(&network).await;
            let saved = async {
                let mut reader = b.runtime().begin_save(&received).await?;
                let mut bytes = Vec::new();
                let read = reader.read_to_end(&mut bytes);
                drop(reader);
                b.runtime().end_save(&received, read.is_ok()).await?;
                Ok::<_, TransferError>(read.is_ok())
            };
            match saved.await {
                Ok(read) => assert!(!read, "{missing}"),
                Err(error) => assert_eq!(error, TransferError::Storage, "{missing}"),
            }
            let view = b.view(&received).unwrap();
            assert_eq!(view.status, TransferStatus::Failed, "{missing}");
            assert_eq!(view.reason, Some(TransferReason::Storage), "{missing}");
            until(|| b.blobs().is_empty()).await;
            b.stop().await;
        });
    }
}

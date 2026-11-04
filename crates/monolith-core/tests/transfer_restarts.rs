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
use monolith_protocol::body::{FileChunk, Message, MessageId, TransferId};
use monolith_protocol::limits::{
    MAX_FILE_CHUNK_LEN, RECONNECT_DELAY_INITIAL, RECONNECT_DELAY_MAX, RECONNECT_JITTER_PERCENT,
    TRANSFER_HOLD,
};
use monolith_protocol::session::Action;
use monolith_protocol::text::{ChatText, Filename};
use monolith_storage::StorageError;
use monolith_storage::dir::{CrashOutcome, MemoryDir, TRANSFERS, VaultDir};
use monolith_storage::vault::{KdfParams, Passphrase};
use monolith_tor::{MockNetwork, MockTorBackend};
use proptest::prelude::*;
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

/// What a hostile sender does to a persistent receiver that accepted its
/// offer.
#[derive(Clone, Copy, Debug)]
enum Hostile {
    /// Offers the file again, on a new session if none is left, with its
    /// size or one more; goes on from the offset answered, or where it was.
    Offer { size_ok: bool, resume: bool },
    /// The receiver's user accepts it.
    Accept,
    /// A chunk from where the sender is, of the length a chunk there has,
    /// or `off` bytes shorter or longer, one byte changed if `flip`; on a
    /// new session, offered again first, if none is left.
    Chunk { off: i16, flip: bool },
    /// The sender moves on, or back, by `by` bytes.
    Skip { by: u16, back: bool },
    /// A completion with the digest of what the sender sends, or of the
    /// other file.
    Complete { right: bool },
    /// The sender cancels.
    Abort,
    /// The session ends.
    Close,
    /// The receiver crashes and starts again.
    Crash(u8),
}

fn hostile() -> impl Strategy<Value = Hostile> {
    prop_oneof![
        2 => (any::<bool>(), any::<bool>())
            .prop_map(|(size_ok, resume)| Hostile::Offer { size_ok, resume }),
        1 => Just(Hostile::Accept),
        8 => (
            prop_oneof![8 => Just(0_i16), 1 => -64_i16..64],
            prop::bool::weighted(0.05)
        )
            .prop_map(|(off, flip)| Hostile::Chunk { off, flip }),
        1 => (1..=MAX_FILE_CHUNK_LEN as u16, any::<bool>())
            .prop_map(|(by, back)| Hostile::Skip { by, back }),
        2 => prop::bool::weighted(0.8).prop_map(|right| Hostile::Complete { right }),
        1 => Just(Hostile::Abort),
        1 => Just(Hostile::Close),
        1 => any::<u8>().prop_map(Hostile::Crash),
    ]
}

/// Whether an incoming transfer may go from `from` to `to`, in any number
/// of steps: the receiver's table of `docs/adr/0013-file-transfer.md`.
fn reaches_incoming(from: TransferStatus, to: TransferStatus) -> bool {
    from == to
        || match from {
            TransferStatus::Offered => matches!(
                to,
                TransferStatus::Receiving
                    | TransferStatus::Received
                    | TransferStatus::Rejected
                    | TransferStatus::Expired
                    | TransferStatus::Cancelled
                    | TransferStatus::Failed
            ),
            TransferStatus::Receiving => matches!(
                to,
                TransferStatus::Received | TransferStatus::Cancelled | TransferStatus::Failed
            ),
            _ => false,
        }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 48,
        max_shrink_iters: 128,
        ..ProptestConfig::default()
    })]

    /// Whatever a sender sends, out of order, beyond the size, with any
    /// digest, across sessions and crashes of the receiver: a file is shown
    /// received only with exactly the size offered and a digest the sender
    /// declared, from bytes staged durably, and so it stays; the bytes shown
    /// never pass the size, and the status moves only along the table.
    /// That is the integrity of the transfer: the file is the one its sender
    /// declared, whole. Nothing here says that what it holds is safe.
    #[test]
    fn whatever_a_sender_sends_a_file_is_shown_received_only_whole_and_verified(
        steps in prop::collection::vec(hostile(), 1..32),
        size in prop::sample::select(vec![1_u64, L - 1, L + 3, 3 * L + 5]),
        other in prop::bool::weighted(0.2),
    ) {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut b, a, _, _) = contacts(&network).await;
            // What the sender sends, and another file of the same size: it
            // may send the other one whole, with its digest.
            let mut data = file(size);
            let mut garbage: Vec<u8> = data.iter().map(|byte| byte ^ 0x5A).collect();
            if other {
                core::mem::swap(&mut data, &mut garbage);
            }
            let mut link: Option<Link<DuplexStream>> = None;
            let mut position = 0_u64;
            let mut declared: Vec<[u8; 32]> = Vec::new();
            let mut received: Option<TransferRef> = None;
            let mut seen: Option<TransferStatus> = None;
            let mut held: Option<Vec<u8>> = None;
            let start = [
                Hostile::Offer {
                    size_ok: true,
                    resume: true,
                },
                Hostile::Accept,
            ];
            for step in start.into_iter().chain(steps) {
                match step {
                    Hostile::Offer { size_ok, resume } => {
                        if link.is_none() {
                            link = Some(hand_link(&network, &b, &a).await);
                        }
                        let offer = Message::FileOffer {
                            transfer: TRANSFER,
                            size: if size_ok { size } else { size + 1 },
                            filename: Filename::new("notes.txt").unwrap(),
                        };
                        let open = link.as_mut().unwrap();
                        if open.send(&offer).await.is_ok() {
                            let (messages, ended) = drain(open, QUIET).await;
                            if resume && let Some(offset) = acceptances(&messages, TRANSFER).last() {
                                position = *offset;
                            }
                            if ended {
                                link = None;
                            }
                        } else {
                            link = None;
                        }
                    }
                    Hostile::Accept => {
                        if let Some(transfer) = received {
                            let _ = b.runtime().accept_transfer(&transfer).await;
                        }
                    }
                    Hostile::Chunk { off, flip } => {
                        if link.is_none() {
                            let mut open = hand_link(&network, &b, &a).await;
                            open.send(&offer_of(&data)).await.unwrap();
                            let (messages, ended) = drain(&mut open, QUIET).await;
                            if let Some(offset) = acceptances(&messages, TRANSFER).last() {
                                position = *offset;
                            }
                            if !ended {
                                link = Some(open);
                            }
                        }
                        if let Some(open) = link.as_mut() {
                            let source = &data;
                            let fits = if position < size {
                                (size - position).min(L)
                            } else {
                                L
                            };
                            let by = u64::from(off.unsigned_abs());
                            let len = if off < 0 {
                                fits.saturating_sub(by)
                            } else {
                                fits + by
                            }
                            .clamp(1, L) as usize;
                            // Past the end: bytes the file does not have.
                            let bytes: Vec<u8> = (0..len)
                                .map(|at| {
                                    source
                                        .get(position as usize + at)
                                        .copied()
                                        .unwrap_or(0xEE)
                                })
                                .collect();
                            let mut bytes = bytes;
                            if flip {
                                bytes[len / 2] ^= 0x01;
                            }
                            position += bytes.len() as u64;
                            let chunk = FileChunk::new(TRANSFER, bytes).unwrap();
                            let sent = open.send(&Message::FileChunk(chunk)).await.is_ok();
                            let ended = !sent || drain(open, QUIET).await.1;
                            if ended {
                                link = None;
                            }
                        }
                    }
                    Hostile::Skip { by, back } => {
                        position = if back {
                            position.saturating_sub(u64::from(by))
                        } else {
                            position + u64::from(by)
                        };
                    }
                    Hostile::Complete { right } => {
                        if let Some(open) = link.as_mut() {
                            let digest = sha(if right { &data } else { &garbage });
                            declared.push(digest);
                            let completion = Message::FileComplete {
                                transfer: TRANSFER,
                                digest,
                            };
                            let sent = open.send(&completion).await.is_ok();
                            if !sent || drain(open, QUIET).await.1 {
                                link = None;
                            }
                        }
                    }
                    Hostile::Abort => {
                        if let Some(open) = link.as_mut() {
                            let sent = open.send(&Message::FileAbort(TRANSFER)).await.is_ok();
                            if !sent || drain(open, QUIET).await.1 {
                                link = None;
                            }
                        }
                    }
                    Hostile::Close => {
                        if let Some(open) = link.take() {
                            let _ = open.close().await;
                        }
                    }
                    Hostile::Crash(outcome) => {
                        link = None;
                        b.crash(CrashOutcome::ALL[usize::from(outcome) % CrashOutcome::ALL.len()])
                            .await;
                        b.open();
                        b.start(&network).await;
                    }
                }
                // What the receiver shows.
                if received.is_none() {
                    received = b
                        .runtime()
                        .transfers(&b.id())
                        .unwrap()
                        .first()
                        .map(|view| view.reference);
                }
                let Some(transfer) = received else { continue };
                let view = b.view(&transfer).expect("listed until removed");
                assert!(view.bytes.is_none_or(|bytes| bytes <= view.size), "{view:?}");
                if let Some(before) = seen {
                    assert!(
                        reaches_incoming(before, view.status),
                        "{before:?} to {:?}",
                        view.status
                    );
                }
                seen = Some(view.status);
                if view.status == TransferStatus::Received {
                    assert_eq!(view.bytes, Some(view.size));
                    // Read once, and the same after every later step.
                    let mut reader = b.runtime().begin_save(&transfer).await.unwrap();
                    let mut bytes = Vec::new();
                    reader.read_to_end(&mut bytes).unwrap();
                    drop(reader);
                    b.runtime().end_save(&transfer, false).await.unwrap();
                    assert_eq!(bytes.len() as u64, view.size);
                    assert!(declared.contains(&sha(&bytes)), "a digest nobody declared");
                    if let Some(earlier) = &held {
                        assert!(*earlier == bytes, "the received file changed");
                    }
                    held = Some(bytes);
                }
            }
            b.stop().await;
        });
    }
}

#[test]
fn a_contact_flooding_its_chunks_holds_up_neither_the_chat_nor_the_file_of_another() {
    run_paused(async {
        let network = MockNetwork::new();
        let mut b = Node::new(1, true).await;
        let mut x = Node::new(2, false).await;
        let mut y = Node::new(3, false).await;
        let at_b = [b.add(&x).await, b.add(&y).await];
        let at_x = x.add(&b).await;
        let at_y = y.add(&b).await;
        for node in [&mut b, &mut x, &mut y] {
            node.start(&network).await;
        }
        for (node, at) in [(&x, at_x), (&y, at_y)] {
            node.runtime()
                .send_message(&node.id(), &at, "hello")
                .await
                .unwrap();
            until(|| {
                node.runtime()
                    .messages(&node.id(), &at, None, 10)
                    .unwrap()
                    .iter()
                    .all(|message| message.status == MessageStatus::Delivered)
            })
            .await;
        }
        x.stop().await;
        y.stop().await;
        until(|| {
            at_b.iter()
                .all(|at| b.runtime().sessions(&b.id(), at).unwrap() == 0)
        })
        .await;
        // Y's session first; then X offers a large file, which B accepts.
        let mut chat = hand_link(&network, &b, &y).await;
        let mut flood = hand_link(&network, &b, &x).await;
        let size = 128 * L;
        let data = file(size);
        let mut events = b.runtime().subscribe();
        flood.send(&offer_of(&data)).await.unwrap();
        let Event::TransferOffered(from_x) = event(&mut events, |event| {
            matches!(event, Event::TransferOffered(_))
        })
        .await
        else {
            panic!("not an offer");
        };
        b.runtime().accept_transfer(&from_x).await.unwrap();
        // X sends every chunk at once, ignoring the window, and reads only
        // what has come.
        let flooding = tokio::spawn(async move {
            for n in 0..128 {
                if flood.send(&chunk(TRANSFER, &data, n * L)).await.is_err() {
                    break;
                }
                let _ = tokio::time::timeout(Duration::ZERO, flood.receive()).await;
            }
            flood
        });
        // No timer while it floods: writes on the blocking pool hold the
        // paused clock still. The tasks get turns until B staged some of it.
        while b.view(&from_x).unwrap().bytes.unwrap_or(0) == 0 {
            tokio::task::yield_now().await;
        }
        // Y's chat message is acknowledged while X's file is far from
        // staged, and Y's own file goes through meanwhile.
        let id = MessageId::from_bytes([7; 16]);
        chat.send(&Message::ChatMessage {
            id,
            text: ChatText::new("while X floods").unwrap(),
        })
        .await
        .unwrap();
        loop {
            let arrived = chat.receive().await.unwrap();
            match &arrived.received().message {
                Message::MessageAck(acked) if *acked == id => break,
                Message::Ping(nonce) => chat.send(&Message::Pong(*nonce)).await.unwrap(),
                _ => {}
            }
        }
        let staged = b.view(&from_x).unwrap().bytes.unwrap_or(0);
        assert!(
            staged < size / 4,
            "Y's message waited for {staged} of {size} bytes of X"
        );
        // Y's file, under an identifier of its own, driven with no timer
        // either.
        let theirs = TransferId::from_bytes([0x5B; 16]);
        let small = file(L + 1);
        chat.send(&Message::FileOffer {
            transfer: theirs,
            size: small.len() as u64,
            filename: Filename::new("notes.txt").unwrap(),
        })
        .await
        .unwrap();
        let from_y = loop {
            if let Ok(Event::TransferOffered(offered)) = events.recv().await
                && offered.contact == at_b[1]
            {
                break offered;
            }
        };
        b.runtime().accept_transfer(&from_y).await.unwrap();
        let mut answered = false;
        while !answered {
            let arrived = chat.receive().await.unwrap();
            match &arrived.received().message {
                Message::FileAccept { offset: 0, .. } => {
                    for n in 0..2 {
                        chat.send(&chunk(theirs, &small, n * L)).await.unwrap();
                    }
                    chat.send(&Message::FileComplete {
                        transfer: theirs,
                        digest: sha(&small),
                    })
                    .await
                    .unwrap();
                }
                Message::FileComplete { digest, .. } => {
                    assert_eq!(*digest, sha(&small));
                    answered = true;
                }
                Message::Ping(nonce) => chat.send(&Message::Pong(*nonce)).await.unwrap(),
                _ => {}
            }
        }
        let staged = b.view(&from_x).unwrap().bytes.unwrap_or(0);
        assert!(
            staged < size / 2,
            "Y's file waited for {staged} of {size} bytes of X"
        );
        assert_eq!(saved(&b, &from_y).await, small);
        drop(flooding.await.unwrap());
        b.stop().await;
    });
}

/// The offers among `messages`.
fn offers_among(messages: &[Message]) -> usize {
    messages
        .iter()
        .filter(|message| matches!(message, Message::FileOffer { .. }))
        .count()
}

#[test]
fn an_offer_the_receiver_read_is_held_by_the_sender_after_a_crash_at_any_step() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, b, at_a, _) = contacts(&network).await;
        a.stop().await;
        let base = a.dir().restart(CrashOutcome::ALL[0]);
        let data = file(2 * L + 1);
        let fresh = |a: &mut Node| {
            a.identity = None;
            a.installation = None;
            a.dir = Some(base.restart(CrashOutcome::ALL[0]));
            a.open();
        };
        // The storage steps of the offer, with no crash.
        fresh(&mut a);
        a.start(&network).await;
        let mut link = hand_link(&network, &a, &b).await;
        drain(&mut link, QUIET).await;
        let start = a.dir().steps();
        a.runtime()
            .offer_file(
                &a.id(),
                &at_a,
                "report.pdf",
                data.len() as u64,
                Box::new(Cursor::new(data.clone())),
            )
            .await
            .unwrap();
        let steps = a.dir().steps() - start;
        assert_eq!(offers_among(&drain(&mut link, QUIET).await.0), 1);
        a.stop().await;
        for step in 1..=steps {
            fresh(&mut a);
            a.start(&network).await;
            let mut link = hand_link(&network, &a, &b).await;
            drain(&mut link, QUIET).await;
            a.dir().crash_at(a.dir().steps() + step);
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
            let read = offers_among(&drain(&mut link, QUIET).await.0) > 0;
            a.crash(CrashOutcome::ALL[0]).await;
            a.open();
            a.start(&network).await;
            let held = !a.runtime().transfers(&a.id()).unwrap().is_empty();
            // What the receiver read, and what the user was told was
            // offered, the sender holds.
            assert!(
                !read || held,
                "step {step} of {steps}: an offer read, not held"
            );
            assert!(
                offered.is_err() || held,
                "step {step} of {steps}: reported, not held"
            );
            a.stop().await;
        }
    });
}

#[test]
fn a_decision_the_sender_read_is_held_by_the_receiver_after_a_crash_at_any_step() {
    for decision in ["accept", "reject", "cancel"] {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut b, a, _, _) = contacts(&network).await;
            let data = file(2 * L + 1);
            // The offer recorded, and accepted already for a cancellation.
            let mut link = hand_link(&network, &b, &a).await;
            let received = if decision == "cancel" {
                offered_and_accepted(&b, &mut link, &data).await
            } else {
                let mut events = b.runtime().subscribe();
                link.send(&offer_of(&data)).await.unwrap();
                let Event::TransferOffered(received) = event(&mut events, |event| {
                    matches!(event, Event::TransferOffered(_))
                })
                .await
                else {
                    panic!("not an offer");
                };
                received
            };
            b.stop().await;
            let base = b.dir().restart(CrashOutcome::ALL[0]);
            let fresh = |b: &mut Node| {
                b.identity = None;
                b.installation = None;
                b.dir = Some(base.restart(CrashOutcome::ALL[0]));
                b.open();
            };
            let decide = |b: &Node| {
                let runtime = b.runtime().clone();
                async move {
                    match decision {
                        "accept" => runtime.accept_transfer(&received).await,
                        "reject" => runtime.reject_transfer(&received).await,
                        _ => runtime.cancel_transfer(&received).await,
                    }
                }
            };
            let told = |messages: &[Message]| {
                messages.iter().any(|message| match decision {
                    "accept" => matches!(message, Message::FileAccept { .. }),
                    "reject" => matches!(message, Message::FileReject(_)),
                    _ => matches!(message, Message::FileAbort(_)),
                })
            };
            let decided = match decision {
                "accept" => TransferStatus::Receiving,
                "reject" => TransferStatus::Rejected,
                _ => TransferStatus::Cancelled,
            };
            // The storage steps of the decision, with no crash.
            fresh(&mut b);
            b.start(&network).await;
            let mut link = hand_link(&network, &b, &a).await;
            offer_again(&mut link, &data).await;
            let start = b.dir().steps();
            decide(&b).await.unwrap();
            let steps = b.dir().steps() - start;
            assert!(told(&drain(&mut link, QUIET).await.0), "{decision}");
            b.stop().await;
            for step in 1..=steps {
                fresh(&mut b);
                b.start(&network).await;
                let mut link = hand_link(&network, &b, &a).await;
                offer_again(&mut link, &data).await;
                b.dir().crash_at(b.dir().steps() + step);
                let result = decide(&b).await;
                let read = told(&drain(&mut link, QUIET).await.0);
                b.crash(CrashOutcome::ALL[0]).await;
                b.open();
                b.start(&network).await;
                let held = b.status(&received) == Some(decided);
                // What the sender read, and what the user was told was
                // done, the receiver holds.
                assert!(
                    !read || held,
                    "{decision}, step {step} of {steps}: told, not held"
                );
                assert!(
                    result.is_err() || held,
                    "{decision}, step {step} of {steps}"
                );
                b.stop().await;
            }
        });
    }
}

#[test]
fn a_cancellation_the_receiver_read_is_held_by_the_sender_after_a_crash_at_any_step() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, b, at_a, _) = contacts(&network).await;
        let data = file(2 * L + 1);
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
        a.stop().await;
        let base = a.dir().restart(CrashOutcome::ALL[0]);
        let fresh = |a: &mut Node| {
            a.identity = None;
            a.installation = None;
            a.dir = Some(base.restart(CrashOutcome::ALL[0]));
            a.open();
        };
        let aborted = |messages: &[Message]| {
            messages
                .iter()
                .any(|message| matches!(message, Message::FileAbort(_)))
        };
        // The storage steps of the cancellation, with no crash: the
        // receiver read the offer first.
        fresh(&mut a);
        a.start(&network).await;
        let mut link = hand_link(&network, &a, &b).await;
        assert_eq!(offers_among(&drain(&mut link, QUIET).await.0), 1);
        let start = a.dir().steps();
        a.runtime().cancel_transfer(&sent).await.unwrap();
        let steps = a.dir().steps() - start;
        assert!(aborted(&drain(&mut link, QUIET).await.0));
        a.stop().await;
        for step in 1..=steps {
            fresh(&mut a);
            a.start(&network).await;
            let mut link = hand_link(&network, &a, &b).await;
            drain(&mut link, QUIET).await;
            a.dir().crash_at(a.dir().steps() + step);
            let result = a.runtime().cancel_transfer(&sent).await;
            let read = aborted(&drain(&mut link, QUIET).await.0);
            a.crash(CrashOutcome::ALL[0]).await;
            a.open();
            a.start(&network).await;
            let held = a.status(&sent) == Some(TransferStatus::Cancelled);
            assert!(!read || held, "step {step} of {steps}: told, not held");
            assert!(result.is_err() || held, "step {step} of {steps}");
            a.stop().await;
        }
    });
}

/// What happens to a receiver that accepted a file, from a sender that
/// comes and goes.
#[derive(Clone, Copy, Debug)]
enum Weather {
    /// The sender offers the file again, on a session of its own if none
    /// is left, and sends `chunks` of it from the offset answered.
    Offer { chunks: u8 },
    /// The session ends.
    Drop,
    /// The next write of a staged file fails: the disk is full.
    Full,
    /// Time passes, in minutes.
    Wait(u16),
    /// The receiver crashes and starts again.
    Crash(u8),
}

fn weather() -> impl Strategy<Value = Weather> {
    prop_oneof![
        4 => (1..4_u8).prop_map(|chunks| Weather::Offer { chunks }),
        2 => Just(Weather::Drop),
        1 => Just(Weather::Full),
        2 => (1..600_u16).prop_map(Weather::Wait),
        1 => any::<u8>().prop_map(Weather::Crash),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 16,
        max_shrink_iters: 64,
        ..ProptestConfig::default()
    })]

    /// Through sessions that end, offers repeated, a full disk and crashes,
    /// what a receiver keeps stays bounded by what it accepted: one record,
    /// at most one staged file, never more bytes than the size; while
    /// nothing can be written or no sender can be reached it neither writes
    /// nor dials more than a few times an hour; and at the end the file
    /// arrives whole, and once removed leaves nothing behind.
    #[test]
    fn a_receiver_stays_bounded_through_lost_sessions_repeated_offers_and_a_full_disk(
        steps in prop::collection::vec(weather(), 1..24),
    ) {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut b, a, _, _) = contacts(&network).await;
            let data = file(6 * L + 7);
            let mut link = Some(hand_link(&network, &b, &a).await);
            let received = offered_and_accepted(&b, link.as_mut().unwrap(), &data).await;
            let mut position = 0_u64;
            // The sender keeps to the protocol: chunks only on a session
            // where the receiver answered with the offset to go on from.
            let mut answered = true;
            for step in steps {
                match step {
                    Weather::Offer { chunks } => {
                        if link.is_none() {
                            link = Some(hand_link(&network, &b, &a).await);
                            answered = false;
                        }
                        let open = link.as_mut().unwrap();
                        let (messages, ended) = if open.send(&offer_of(&data)).await.is_ok() {
                            drain(open, QUIET).await
                        } else {
                            (Vec::new(), true)
                        };
                        if let Some(offset) = acceptances(&messages, TRANSFER).last() {
                            position = *offset;
                            answered = true;
                        }
                        let mut ended = ended;
                        for _ in 0..chunks {
                            if ended || !answered || position >= data.len() as u64 {
                                break;
                            }
                            ended = open.send(&chunk(TRANSFER, &data, position)).await.is_err();
                            position = (position + L).min(data.len() as u64);
                        }
                        if !ended {
                            let (messages, closed) = drain(open, QUIET).await;
                            if let Some(offset) = acceptances(&messages, TRANSFER).last() {
                                position = *offset;
                            }
                            ended = closed;
                        }
                        if ended {
                            link = None;
                        }
                    }
                    Weather::Drop => {
                        if let Some(open) = link.take() {
                            let _ = open.close().await;
                        }
                    }
                    Weather::Full => b.dir().fail_next_write(StorageError::Full),
                    Weather::Wait(minutes) => {
                        // Nothing sent meanwhile: the receiver writes only a
                        // few times an hour, and dials as the reconnect
                        // delays allow: they double from the first to the
                        // longest, each as short as the jitter makes it, so
                        // a burst at first and then a few an hour.
                        let steps = b.dir().steps();
                        let dials = network.dials();
                        let seconds = u64::from(minutes) * 60;
                        tokio::time::sleep(Duration::from_secs(seconds)).await;
                        let hours = seconds.div_ceil(3600);
                        assert!(
                            b.dir().steps() - steps <= 8 * hours,
                            "{} storage steps in {minutes} minutes",
                            b.dir().steps() - steps
                        );
                        let burst = u64::from(
                            (RECONNECT_DELAY_MAX.as_secs() / RECONNECT_DELAY_INITIAL.as_secs())
                                .ilog2(),
                        ) + 3;
                        let apart = RECONNECT_DELAY_MAX.as_secs()
                            * u64::from(100 - RECONNECT_JITTER_PERCENT)
                            / 100;
                        assert!(
                            (network.dials() - dials) as u64 <= burst + seconds.div_ceil(apart),
                            "{} dials in {minutes} minutes",
                            network.dials() - dials
                        );
                    }
                    Weather::Crash(outcome) => {
                        link = None;
                        b.crash(CrashOutcome::ALL[usize::from(outcome) % CrashOutcome::ALL.len()])
                            .await;
                        b.open();
                        b.start(&network).await;
                    }
                }
                let views = b.runtime().transfers(&b.id()).unwrap();
                assert_eq!(views.len(), 1, "{views:?}");
                assert!(views[0].bytes.is_none_or(|bytes| bytes <= views[0].size));
                assert!(b.blobs().len() <= 1, "{:?}", b.blobs());
            }
            // The end: the sender offers it until it is whole.
            for _ in 0..8 {
                if b.status(&received) == Some(TransferStatus::Received) {
                    break;
                }
                tokio::time::sleep(TRANSFER_HOLD + Duration::from_secs(60)).await;
                if let Some(open) = link.take() {
                    let _ = open.close().await;
                }
                // A full disk may hold it and end the session on the way.
                let mut open = hand_link(&network, &b, &a).await;
                if open.send(&offer_of(&data)).await.is_err() {
                    continue;
                }
                let (messages, ended) = drain(&mut open, QUIET).await;
                let Some(mut offset) = acceptances(&messages, TRANSFER).last().copied() else {
                    continue;
                };
                let mut sent = !ended;
                while sent && offset < data.len() as u64 {
                    sent = open.send(&chunk(TRANSFER, &data, offset)).await.is_ok();
                    offset = (offset + L).min(data.len() as u64);
                }
                let completion = Message::FileComplete {
                    transfer: TRANSFER,
                    digest: sha(&data),
                };
                if sent && open.send(&completion).await.is_ok() {
                    drain(&mut open, QUIET).await;
                }
                link = Some(open);
            }
            let view = b.view(&received).unwrap();
            assert_eq!(view.status, TransferStatus::Received, "{view:?}");
            assert_eq!(saved(&b, &received).await, data);
            b.runtime().remove_transfer(&received).await.unwrap();
            assert!(b.blobs().is_empty(), "{:?}", b.blobs());
            assert!(b.runtime().transfers(&b.id()).unwrap().is_empty());
            b.stop().await;
        });
    }
}

/// Lets the syncs of blob files of `0` go on when dropped, also on a panic.
struct SyncsReleased<'a>(&'a MemoryDir);

impl Drop for SyncsReleased<'_> {
    fn drop(&mut self) {
        self.0.release_blob_syncs();
    }
}

#[test]
fn a_cancellation_while_the_file_is_verified_is_never_answered_with_a_completion() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut b, a, _, _) = contacts(&network).await;
        let dir = b.dir().clone();
        let _released = SyncsReleased(&dir);
        let data = file(3 * L + 5);
        let mut link = hand_link(&network, &b, &a).await;
        let received = offered_and_accepted(&b, &mut link, &data).await;
        for n in 0..4 {
            link.send(&chunk(TRANSFER, &data, n * L)).await.unwrap();
        }
        drain(&mut link, QUIET).await;
        // The verification of the whole file waits at its sync of the
        // staged file; the user cancels meanwhile.
        dir.hold_blob_syncs();
        link.send(&Message::FileComplete {
            transfer: TRANSFER,
            digest: sha(&data),
        })
        .await
        .unwrap();
        spin_until(|| dir.blob_sync_held()).await;
        b.runtime().cancel_transfer(&received).await.unwrap();
        dir.release_blob_syncs();
        // The verification ends with the right digest, but the transfer
        // was cancelled: the sender is told so, never that the file is
        // complete, and nothing here holds it.
        let (messages, _) = drain(&mut link, QUIET).await;
        assert!(
            !messages
                .iter()
                .any(|message| matches!(message, Message::FileComplete { .. })),
            "{messages:?}"
        );
        assert!(
            messages.contains(&Message::FileAbort(TRANSFER)),
            "{messages:?}"
        );
        assert_eq!(b.status(&received), Some(TransferStatus::Cancelled));
        until(|| b.blobs().is_empty()).await;
        b.stop().await;
    });
}

#[test]
fn a_transfer_cancelled_while_the_offers_wait_to_be_durable_is_not_offered() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, b, at_a, _) = contacts(&network).await;
        let first = a.dir().clone();
        let _released = Released(&first);
        let mut sent = Vec::new();
        for name in ["one.pdf", "two.pdf", "three.pdf"] {
            let data = file(2 * L);
            sent.push(
                a.runtime()
                    .offer_file(
                        &a.id(),
                        &at_a,
                        name,
                        data.len() as u64,
                        Box::new(Cursor::new(data)),
                    )
                    .await
                    .unwrap(),
            );
        }
        // A change of the store waits to be durable as the session starts:
        // its offers wait for it, listed.
        a.dir().hold_writes();
        let runtime = a.runtime().clone();
        let third = sent[2];
        let cancelling = tokio::spawn(async move { runtime.cancel_transfer(&third).await });
        spin_until(|| a.dir().write_held()).await;
        let link = {
            let network = network.clone();
            let (a_card, b_identity) = (a.identity().card(), b.identity().clone());
            tokio::spawn(async move {
                let established = dial(&network.backend(), &Budgets::new(), &b_identity, &a_card)
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
            })
        };
        for _ in 0..1000 {
            tokio::task::yield_now().await;
        }
        // The second is cancelled before it was ever offered.
        let runtime = a.runtime().clone();
        let second = sent[1];
        let cancelled = tokio::spawn(async move { runtime.cancel_transfer(&second).await });
        for _ in 0..1000 {
            tokio::task::yield_now().await;
        }
        a.dir().release_writes();
        cancelling.await.unwrap().unwrap();
        cancelled.await.unwrap().unwrap();
        let mut link = link.await.unwrap();
        let (messages, _) = drain(&mut link, QUIET).await;
        let offers: Vec<Filename> = messages
            .iter()
            .filter_map(|message| match message {
                Message::FileOffer { filename, .. } => Some(filename.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            offers,
            vec![Filename::new("one.pdf").unwrap()],
            "{messages:?}"
        );
        a.stop().await;
    });
}

#[test]
fn a_transfer_cancelled_and_removed_at_once_while_it_goes_is_ended_on_its_session() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, b, at_a, _) = contacts(&network).await;
        let first = a.dir().clone();
        let _released = Released(&first);
        let data = file(40 * L);
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
        let (transfer, _, _) = offer_in(&drain(&mut link, QUIET).await.0);
        link.send(&Message::FileAccept {
            transfer,
            offset: 0,
        })
        .await
        .unwrap();
        drain(&mut link, QUIET).await;
        // The user stops it and takes it off the list at once: both are
        // applied before the session looks at it again.
        a.dir().hold_writes();
        let runtime = a.runtime().clone();
        let cancelling = tokio::spawn(async move { runtime.cancel_transfer(&sent).await });
        let runtime = a.runtime().clone();
        let removing = tokio::spawn(async move { runtime.remove_transfer(&sent).await });
        spin_until(|| a.dir().write_held()).await;
        for _ in 0..1000 {
            tokio::task::yield_now().await;
        }
        a.dir().release_writes();
        cancelling.await.unwrap().unwrap();
        removing.await.unwrap().unwrap();
        // The receiver is told it ends, on the session it goes on.
        let (messages, _) = drain(&mut link, QUIET).await;
        assert!(
            messages.contains(&Message::FileAbort(transfer)),
            "{messages:?}"
        );
        assert_eq!(a.status(&sent), None);
        a.stop().await;
    });
}

#[test]
fn a_chunk_that_could_not_be_written_is_never_acknowledged_while_this_side_also_sends() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut b, a, at_b, _) = contacts(&network).await;
        let data = file(6 * L);
        let mut link = hand_link(&network, &b, &a).await;
        let received = offered_and_accepted(&b, &mut link, &data).await;
        // B sends a file of its own on the same session.
        b.runtime()
            .offer_file(
                &b.id(),
                &at_b,
                "theirs.bin",
                8 * L,
                Box::new(Cursor::new(file(8 * L))),
            )
            .await
            .unwrap();
        let (theirs, _, _) = offer_in(&drain(&mut link, QUIET).await.0);
        // The acceptance of B's file and two chunks of A's arrive together;
        // the first chunk cannot be written, the disk is full.
        b.dir().fail_next_write(StorageError::Full);
        link.send(&Message::FileAccept {
            transfer: theirs,
            offset: 0,
        })
        .await
        .unwrap();
        for n in 0..2 {
            link.send(&chunk(TRANSFER, &data, n * L)).await.unwrap();
        }
        // B acknowledges nothing it did not stage, and holds the transfer.
        let (messages, _) = drain(&mut link, QUIET).await;
        assert!(
            acceptances(&messages, TRANSFER)
                .iter()
                .all(|offset| *offset == 0),
            "{:?}",
            acceptances(&messages, TRANSFER)
        );
        assert!(b.view(&received).unwrap().held);
        b.stop().await;
    });
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
fn an_offer_of_the_peer_under_the_identifier_of_a_transfer_of_this_side_ends_the_session() {
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
        let (transfer, size, name) = offer_in(&drain(&mut link, QUIET).await.0);
        receive_rest(&mut link, transfer, size, 0).await;
        link.send(&Message::FileComplete {
            transfer,
            digest: sha(&data),
        })
        .await
        .unwrap();
        drain(&mut link, QUIET).await;
        assert_eq!(a.status(&sent), Some(TransferStatus::Completed));
        // On a later session the peer offers a file of its own under that
        // identifier, before anything else: a violation, recorded nowhere.
        let mut link = hand_link(&network, &a, &b).await;
        link.send(&Message::FileOffer {
            transfer,
            size,
            filename: name,
        })
        .await
        .unwrap();
        let (messages, ended) = drain(&mut link, QUIET).await;
        assert!(ended, "{messages:?}");
        assert_eq!(a.runtime().transfers(&a.id()).unwrap().len(), 1);
        assert_eq!(a.status(&sent), Some(TransferStatus::Completed));
        a.stop().await;
    });
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
fn a_staging_that_failed_and_could_not_remove_its_file_removes_it_later() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, _) = contacts(&network).await;
        // The source ends early, and the staged file cannot be removed at
        // once (another program has it open).
        a.dir().fail_next_remove(StorageError::Busy);
        let offered = a
            .runtime()
            .offer_file(
                &a.id(),
                &at_a,
                "report.pdf",
                10,
                Box::new(Cursor::new(vec![1_u8; 9])),
            )
            .await;
        assert_eq!(offered, Err(TransferError::Source));
        assert_eq!(a.blobs().len(), 1);
        // The next change of the transfers tries it again.
        let sent = a
            .runtime()
            .offer_file(
                &a.id(),
                &at_a,
                "notes.txt",
                10,
                Box::new(Cursor::new(vec![2_u8; 10])),
            )
            .await
            .unwrap();
        a.runtime().cancel_transfer(&sent).await.unwrap();
        assert!(
            within_a_minute(|| a.blobs().is_empty()).await,
            "{:?}",
            a.blobs()
        );
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

/// A block is durable in the vault before the transfers of the contact are
/// cancelled in their own store. A crash at any step between leaves
/// nothing of the blocked contact live or saveable: the vault no longer
/// holds its record, and the transfer store, matched against it when it
/// opens, drops what it held of it.
#[test]
fn a_crash_at_any_step_of_a_block_leaves_nothing_of_the_contact_live_or_saveable() {
    for outcome in CrashOutcome::ALL {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut b, a, at_b, _) = contacts(&network).await;
            let data = file(2 * L);
            let mut link = hand_link(&network, &b, &a).await;
            let received = offered_and_accepted(&b, &mut link, &data).await;
            send_rest(&mut link, &data, 0).await;
            assert_eq!(b.status(&received), Some(TransferStatus::Received));
            b.stop().await;
            let base = b.dir().restart(CrashOutcome::ALL[0]);
            let again = |b: &mut Node| {
                b.identity = None;
                b.installation = None;
                b.dir = Some(base.restart(CrashOutcome::ALL[0]));
                b.open();
            };
            // The storage steps of the block, with no crash.
            again(&mut b);
            b.start(&network).await;
            let start = b.dir().steps();
            b.runtime().block_contact(&b.id(), &at_b).await.unwrap();
            let steps = b.dir().steps() - start;
            b.stop().await;
            // Crashes with the block durable and the transfers not yet
            // cancelled in their store: the sweep reaches that window.
            let mut between = 0;
            for step in 1..=steps {
                again(&mut b);
                b.start(&network).await;
                b.dir().crash_at(b.dir().steps() + step);
                let _ = b.runtime().block_contact(&b.id(), &at_b).await;
                b.crash(outcome).await;
                // What the transfer store holds as the process starts again.
                let store_kept = b.dir().content(TRANSFERS).ok() == base.content(TRANSFERS).ok();
                b.open();
                b.start(&network).await;
                let at = format!("step {step} of {steps}, {outcome:?}");
                if b.identity().blocked().is_empty() {
                    // The block did not hold: the file is there, whole.
                    assert_eq!(saved(&b, &received).await, data, "{at}");
                } else {
                    let views = b.runtime().transfers(&b.id()).unwrap();
                    if store_kept {
                        between += 1;
                    }
                    assert!(
                        views
                            .iter()
                            .all(|view| matches!(view.status, TransferStatus::Cancelled)),
                        "{at}"
                    );
                    assert!(b.runtime().begin_save(&received).await.is_err(), "{at}");
                    assert!(within_a_minute(|| b.blobs().is_empty()).await, "{at}");
                }
                b.stop().await;
            }
            assert!(between > 0, "{outcome:?}");
        });
    }
}

#[test]
fn a_staged_file_made_again_for_a_transfer_cancelled_meanwhile_is_removed() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut b, a, _, _) = contacts(&network).await;
        let data = file(6 * L);
        let mut link = hand_link(&network, &b, &a).await;
        let received = offered_and_accepted(&b, &mut link, &data).await;
        for n in 0..2 {
            link.send(&chunk(TRANSFER, &data, n * L)).await.unwrap();
        }
        drain(&mut link, QUIET).await;
        // The staged copy goes while nothing runs.
        b.crash(CrashOutcome::ALL[0]).await;
        let name = b.blobs().pop().unwrap();
        b.dir().remove(&name).unwrap();
        b.open();
        b.start(&network).await;
        // Offered again, the transfer gets a new staged file; the user
        // cancels it while that file is made.
        let dir = b.dir().clone();
        let _released = Released(&dir);
        let mut link = hand_link(&network, &b, &a).await;
        dir.hold_writes();
        link.send(&offer_of(&data)).await.unwrap();
        spin_until(|| dir.write_held()).await;
        let runtime = b.runtime().clone();
        let cancelling = tokio::spawn(async move { runtime.cancel_transfer(&received).await });
        for _ in 0..1000 {
            tokio::task::yield_now().await;
        }
        dir.release_writes();
        cancelling.await.unwrap().unwrap();
        let (messages, _) = drain(&mut link, QUIET).await;
        assert!(
            messages.contains(&Message::FileAbort(TRANSFER)),
            "{messages:?}"
        );
        // Nothing holds the new file: it goes.
        assert!(
            within_a_minute(|| b.blobs().is_empty()).await,
            "{:?}",
            b.blobs()
        );
        b.stop().await;
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

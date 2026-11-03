//! File transfers over the in-memory Tor (`docs/adr/0013-file-transfer.md`,
//! `docs/adr/0014-transfer-staging.md`): a file offered, accepted, sent,
//! verified and saved; the window of the sender; resumption from what the
//! receiver holds durably; a peer of Phase 7; cancellations and the end of
//! a contact. Where one side is driven by hand, the test writes what that
//! side sends and reads what the other answers.
//!
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
use std::path::{Path, PathBuf};
use std::sync::Arc;

use common::{fixed_endpoint, keys, run_paused};
use monolith_core::api::{
    ContactId, Event, IdentityId, MessageStatus, SaveError, SaveOutcome, TransferError,
    TransferReason, TransferRef, TransferStatus, TransferView,
};
use monolith_core::budget::Budgets;
use monolith_core::identity::{Installation, LocalIdentity};
use monolith_core::link::{Link, answer, dial};
use monolith_core::runtime::{Runtime, RuntimeOptions};
use monolith_protocol::body::{FileChunk, Message, MessageId, TransferId};
use monolith_protocol::limits::{
    FILE_OFFER_TIMEOUT, FILE_WINDOW_CHUNKS, MAX_FILE_CHUNK_LEN,
    MAX_PENDING_FILE_OFFERS_PER_CONTACT, SESSION_LINGER, TRANSFER_GRACE,
};
use monolith_protocol::session::Action;
use monolith_protocol::text::{ChatText, Filename};
use monolith_tor::{
    KeySource, MockNetwork, MockTorBackend, OnionService, OnionServiceSecret, TorBackend,
};
use sha2::{Digest, Sha256};
use tokio::io::DuplexStream;
use tokio::sync::broadcast;

const L: u64 = MAX_FILE_CHUNK_LEN as u64;

/// One installation with one identity of fixed keys, and its runtime once
/// it is started.
struct Peer {
    seed: u8,
    installation: Installation,
    identity: Arc<LocalIdentity>,
    runtime: Option<Runtime<MockTorBackend>>,
}

impl Peer {
    async fn new(seed: u8) -> Self {
        let installation = Installation::ephemeral();
        let identity = installation
            .restore_identity(keys(seed, seed, 1, fixed_endpoint(seed)), None)
            .await
            .unwrap();
        Self {
            seed,
            installation,
            identity,
            runtime: None,
        }
    }

    fn id(&self) -> IdentityId {
        self.identity.id()
    }

    fn runtime(&self) -> &Runtime<MockTorBackend> {
        self.runtime.as_ref().expect("started")
    }

    async fn start(&mut self, network: &MockNetwork) {
        self.runtime = Some(Runtime::start(
            self.installation.clone(),
            network.backend(),
            Budgets::new(),
            RuntimeOptions {
                startup_spread: Duration::ZERO,
                ..RuntimeOptions::default()
            },
        ));
        let endpoint = fixed_endpoint(self.seed);
        until(|| network.is_published(&endpoint)).await;
    }

    async fn stop(&mut self) {
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown(Duration::ZERO).await;
        }
    }

    async fn add(&self, other: &Peer) -> ContactId {
        self.identity.import(&other.identity.card()).await.unwrap();
        self.identity
            .contact_id(other.identity.identity())
            .expect("a record")
    }

    fn view(&self, reference: &TransferRef) -> Option<TransferView> {
        self.runtime().transfer(reference).unwrap()
    }

    fn status(&self, reference: &TransferRef) -> Option<TransferStatus> {
        self.view(reference).map(|view| view.status)
    }

    fn statuses(&self, contact: &ContactId) -> Vec<MessageStatus> {
        self.runtime()
            .messages(&self.id(), contact, None, 1000)
            .unwrap()
            .into_iter()
            .map(|message| message.status)
            .collect()
    }
}

/// Waits, with paused time, until `condition` holds, for up to a day of
/// test time.
async fn until(condition: impl Fn() -> bool) {
    for _ in 0..(24 * 60 * 60 * 10) {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the condition did not come about");
}

/// Waits for an event that `matches`, for up to a day of test time.
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

/// A and B, accepted contacts of each other, both started, with no
/// session left between them.
async fn accepted(network: &MockNetwork) -> (Peer, Peer, ContactId, ContactId) {
    let mut a = Peer::new(1).await;
    let mut b = Peer::new(2).await;
    let at_a = a.add(&b).await;
    let at_b = b.add(&a).await;
    a.start(network).await;
    b.start(network).await;
    a.runtime()
        .send_message(&a.id(), &at_a, "hello")
        .await
        .unwrap();
    until(|| a.statuses(&at_a) == vec![MessageStatus::Delivered]).await;
    until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 0).await;
    (a, b, at_a, at_b)
}

fn file(len: u64) -> Vec<u8> {
    (0..len).map(|i| (i * 31 % 251) as u8).collect()
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// A offers `data` to B as `photo.jpg`.
async fn offer(a: &Peer, at_a: &ContactId, data: &[u8]) -> TransferRef {
    a.runtime()
        .offer_file(
            &a.id(),
            at_a,
            "C:\\Users\\me\\photo.jpg",
            data.len() as u64,
            Box::new(Cursor::new(data.to_vec())),
        )
        .await
        .unwrap()
}

/// The offer that reaches B, from its events.
async fn offered(events: &mut broadcast::Receiver<Event>) -> TransferRef {
    match event(events, |event| matches!(event, Event::TransferOffered(_))).await {
        Event::TransferOffered(reference) => reference,
        other => panic!("not an offer: {other:?}"),
    }
}

/// What B saves of the received file `reference`.
async fn saved(b: &Peer, reference: &TransferRef) -> Vec<u8> {
    let mut reader = b.runtime().begin_save(reference).await.unwrap();
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    drop(reader);
    b.runtime().end_save(reference, true).await.unwrap();
    bytes
}

#[test]
fn a_file_is_offered_accepted_sent_verified_and_saved_whole() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, _) = accepted(&network).await;
        let mut events = b.runtime().subscribe();
        let data = file(3 * L + 100);
        let sent = offer(&a, &at_a, &data).await;
        assert_eq!(a.status(&sent), Some(TransferStatus::Waiting));
        let received = offered(&mut events).await;
        let view = b.view(&received).unwrap();
        assert_eq!(view.status, TransferStatus::Offered);
        assert_eq!(view.name, "photo.jpg");
        assert_eq!(view.size, data.len() as u64);
        assert!(received.incoming);
        b.runtime().accept_transfer(&received).await.unwrap();
        until(|| b.status(&received) == Some(TransferStatus::Received)).await;
        until(|| a.status(&sent) == Some(TransferStatus::Completed)).await;
        assert_eq!(a.view(&sent).unwrap().bytes, Some(data.len() as u64));
        assert_eq!(saved(&b, &received).await, data);
        assert!(b.view(&received).unwrap().saved);
        // Saved once: a second save finds nothing to read.
        assert_eq!(
            b.runtime().begin_save(&received).await.unwrap_err(),
            TransferError::UnknownTransfer
        );
        a.stop().await;
        b.stop().await;
    });
}

/// A new folder of the test, by the path Windows resolves for it.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("monolith-core-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let resolved = std::fs::canonicalize(&dir).unwrap();
    let text = resolved.to_string_lossy().into_owned();
    text.strip_prefix(r"\\?\")
        .map_or(resolved.clone(), PathBuf::from)
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_file_offered_by_its_path_is_sent_as_staged_and_saved_where_the_user_chooses() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, _) = accepted(&network).await;
        let dir = scratch("by-path");
        let source = dir.join("holiday photo.jpg");
        let data = file(3 * L + 77);
        std::fs::write(&source, &data).unwrap();
        let mut events = b.runtime().subscribe();
        let sent = a
            .runtime()
            .offer_path(&a.id(), &at_a, &source)
            .await
            .unwrap();
        // What is offered is what was staged: the file changing or going
        // afterwards changes nothing.
        std::fs::write(&source, b"something else").unwrap();
        std::fs::remove_file(&source).unwrap();
        let received = offered(&mut events).await;
        let view = b.view(&received).unwrap();
        assert_eq!(view.name, "holiday photo.jpg");
        assert_eq!(view.size, data.len() as u64);
        b.runtime().accept_transfer(&received).await.unwrap();
        until(|| a.status(&sent) == Some(TransferStatus::Completed)).await;
        assert_eq!(b.status(&received), Some(TransferStatus::Received));
        // A name that exists, or a place that is not a folder of this
        // computer, is refused before the save is recorded: the staged
        // copy stays.
        let saves = dir.join("saved");
        std::fs::create_dir(&saves).unwrap();
        std::fs::write(saves.join("taken.jpg"), b"mine").unwrap();
        assert_eq!(
            b.runtime()
                .save_transfer(&received, &saves.join("taken.jpg"))
                .await,
            Err(SaveError::Exists)
        );
        assert_eq!(std::fs::read(saves.join("taken.jpg")).unwrap(), b"mine");
        assert_eq!(
            b.runtime()
                .save_transfer(&received, Path::new("relative.jpg"))
                .await,
            Err(SaveError::Refused)
        );
        assert_eq!(
            b.runtime()
                .save_transfer(&received, &saves.join("missing").join("x.jpg"))
                .await,
            Err(SaveError::Refused)
        );
        assert!(!b.view(&received).unwrap().saved);
        // The name the peer gave is a suggestion the user takes here.
        let name = Filename::new(&view.name).unwrap().save_name();
        let destination = saves.join(&name);
        assert_eq!(
            b.runtime().save_transfer(&received, &destination).await,
            Ok(SaveOutcome::Saved)
        );
        assert_eq!(std::fs::read(&destination).unwrap(), data);
        assert_eq!(names(&saves), vec![name, "taken.jpg".to_owned()]);
        assert!(b.view(&received).unwrap().saved);
        // Saved once: the staged copy is gone.
        assert_eq!(
            b.runtime()
                .save_transfer(&received, &saves.join("again.jpg"))
                .await,
            Err(SaveError::Transfer(TransferError::UnknownTransfer))
        );
        assert!(!saves.join("again.jpg").exists());
        a.stop().await;
        b.stop().await;
        let _ = std::fs::remove_dir_all(&dir);
    });
}

#[test]
fn a_path_that_is_not_a_regular_file_of_this_computer_is_never_offered() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, _) = accepted(&network).await;
        let dir = scratch("not-offered");
        std::fs::write(dir.join("real.txt"), b"real").unwrap();
        let mut refused = vec![dir.clone(), dir.join("missing.txt")];
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("real.txt"), dir.join("link.txt")).unwrap();
            refused.push(dir.join("link.txt"));
            refused.push(PathBuf::from("/dev/zero"));
        }
        #[cfg(windows)]
        {
            refused.push(dir.join("NUL.txt"));
            refused.push(PathBuf::from(r"\\server\share\file.txt"));
            refused.push(dir.join("real.txt:stream"));
        }
        for path in &refused {
            assert_eq!(
                a.runtime().offer_path(&a.id(), &at_a, path).await,
                Err(TransferError::Source),
                "{path:?}"
            );
        }
        // Nothing was staged or offered.
        assert!(a.runtime().transfers(&a.id()).unwrap().is_empty());
        a.stop().await;
        let _ = std::fs::remove_dir_all(&dir);
    });
}

#[test]
fn an_empty_file_is_sent_and_completed() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, _) = accepted(&network).await;
        let mut events = b.runtime().subscribe();
        let sent = offer(&a, &at_a, &[]).await;
        let received = offered(&mut events).await;
        b.runtime().accept_transfer(&received).await.unwrap();
        until(|| a.status(&sent) == Some(TransferStatus::Completed)).await;
        assert_eq!(b.status(&received), Some(TransferStatus::Received));
        assert_eq!(saved(&b, &received).await, Vec::<u8>::new());
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_rejected_offer_ends_on_both_sides_and_is_never_offered_again() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, _) = accepted(&network).await;
        let mut events = b.runtime().subscribe();
        let sent = offer(&a, &at_a, &file(10)).await;
        let received = offered(&mut events).await;
        b.runtime().reject_transfer(&received).await.unwrap();
        // Rejecting twice is rejecting once; accepting it now fails.
        b.runtime().reject_transfer(&received).await.unwrap();
        assert_eq!(
            b.runtime().accept_transfer(&received).await,
            Err(TransferError::UnknownTransfer)
        );
        until(|| a.status(&sent) == Some(TransferStatus::Rejected)).await;
        assert_eq!(b.status(&received), Some(TransferStatus::Rejected));
        // The rejection took nothing on B's disk, and A has nothing left
        // to offer: another session carries no offer.
        a.runtime().connect(&a.id(), &at_a).unwrap();
        until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 1).await;
        until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 0).await;
        assert!(
            b.runtime()
                .transfers(&b.id())
                .unwrap()
                .iter()
                .all(|view| view.reference == received)
        );
        a.stop().await;
        b.stop().await;
    });
}

/// A and B accepted contacts; A runs, B's runtime is stopped and B dials A
/// by hand: the link of B.
async fn b_by_hand(network: &MockNetwork) -> (Peer, Peer, ContactId, Link<DuplexStream>) {
    let (a, mut b, at_a, _) = accepted(network).await;
    b.stop().await;
    let link = hand_link(network, &a, &b).await;
    (a, b, at_a, link)
}

/// A and B accepted contacts; B runs, A's runtime is stopped and A dials B
/// by hand: the link of A.
async fn a_by_hand(network: &MockNetwork) -> (Peer, Peer, ContactId, Link<DuplexStream>) {
    let (mut a, b, _, at_b) = accepted(network).await;
    a.stop().await;
    let link = hand_link(network, &b, &a).await;
    (a, b, at_b, link)
}

/// A session from `by` to `to`, driven by hand at `by`.
async fn hand_link(network: &MockNetwork, to: &Peer, by: &Peer) -> Link<DuplexStream> {
    let established = dial(
        &network.backend(),
        &Budgets::new(),
        &by.identity,
        &to.identity.card(),
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

/// What arrives on `link` until nothing more comes for `quiet` of test
/// time, Pings answered and left out. The second value says whether the
/// session ended.
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

const QUIET: Duration = Duration::from_secs(5);

/// What arrives on `link` until nothing more comes for `quiet` of test
/// time, Pings included and left unanswered. The second value says whether
/// the session ended.
async fn take(link: &mut Link<DuplexStream>, quiet: Duration) -> (Vec<Message>, bool) {
    let mut messages = Vec::new();
    loop {
        match tokio::time::timeout(quiet, link.receive()).await {
            Err(_) => return (messages, false),
            Ok(Err(_)) => return (messages, true),
            Ok(Ok(arrived)) => messages.push(arrived.received().message.clone()),
        }
    }
}

/// The identifiers and sizes of the offers among `messages`, in order.
fn offers_in(messages: &[Message]) -> Vec<(TransferId, u64)> {
    messages
        .iter()
        .filter_map(|message| match message {
            Message::FileOffer { transfer, size, .. } => Some((*transfer, *size)),
            _ => None,
        })
        .collect()
}

fn chunks_of(messages: &[Message], transfer: TransferId) -> Vec<(usize, usize)> {
    let mut offsets = Vec::new();
    for message in messages {
        if let Message::FileChunk(chunk) = message {
            assert_eq!(*chunk.transfer(), transfer);
            offsets.push((chunk.data().len(), 0));
        }
    }
    offsets
}

/// The offer on `link`, from what arrived.
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

#[test]
fn the_sender_never_has_more_than_the_window_beyond_what_the_receiver_acknowledged() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, mut link) = b_by_hand(&network).await;
        let data = file(10 * L + 1);
        let sent = offer(&a, &at_a, &data).await;
        let (messages, ended) = drain(&mut link, QUIET).await;
        assert!(!ended);
        let (transfer, size, name) = offer_in(&messages);
        assert_eq!(size, data.len() as u64);
        assert_eq!(name.as_str(), "photo.jpg");
        // Nothing of the file before an acceptance.
        assert!(chunks_of(&messages, transfer).is_empty());
        link.send(&Message::FileAccept {
            transfer,
            offset: 0,
        })
        .await
        .unwrap();
        let (messages, _) = drain(&mut link, QUIET).await;
        let chunks: Vec<&FileChunk> = messages
            .iter()
            .filter_map(|message| match message {
                Message::FileChunk(chunk) => Some(chunk),
                _ => None,
            })
            .collect();
        // The window, and nothing more, however long the receiver waits.
        assert_eq!(chunks.len() as u64, FILE_WINDOW_CHUNKS);
        for (n, chunk) in chunks.iter().enumerate() {
            let start = n * L as usize;
            assert_eq!(chunk.data(), &data[start..start + L as usize]);
        }
        assert_eq!(a.status(&sent), Some(TransferStatus::Sending));
        let (more, _) = drain(&mut link, Duration::from_secs(50)).await;
        assert!(chunks_of(&more, transfer).is_empty());
        // Two chunks acknowledged: two more.
        link.send(&Message::FileAccept {
            transfer,
            offset: 2 * L,
        })
        .await
        .unwrap();
        let (messages, _) = drain(&mut link, QUIET).await;
        let chunks: Vec<&FileChunk> = messages
            .iter()
            .filter_map(|message| match message {
                Message::FileChunk(chunk) => Some(chunk),
                _ => None,
            })
            .collect();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].data(), &data[4 * L as usize..5 * L as usize]);
        assert_eq!(a.view(&sent).unwrap().bytes, Some(2 * L));
        // An acknowledgement that goes down ends the session.
        link.send(&Message::FileAccept {
            transfer,
            offset: L,
        })
        .await
        .unwrap();
        let (_, ended) = drain(&mut link, QUIET).await;
        assert!(ended);
        // The transfer goes on over a later session.
        assert_eq!(a.status(&sent), Some(TransferStatus::Sending));
        a.stop().await;
    });
}

#[test]
fn an_acknowledgement_that_is_misaligned_beyond_what_was_sent_or_of_another_transfer_ends_the_session()
 {
    for bad in 0..3 {
        run_paused(async {
            let network = MockNetwork::new();
            let (mut a, _b, at_a, mut link) = b_by_hand(&network).await;
            let sent = offer(&a, &at_a, &file(10 * L)).await;
            let (messages, _) = drain(&mut link, QUIET).await;
            let (transfer, _, _) = offer_in(&messages);
            link.send(&Message::FileAccept {
                transfer,
                offset: 0,
            })
            .await
            .unwrap();
            drain(&mut link, QUIET).await;
            let message = match bad {
                // Not a whole number of chunks.
                0 => Message::FileAccept {
                    transfer,
                    offset: L + 1,
                },
                // More than was sent on the session.
                1 => Message::FileAccept {
                    transfer,
                    offset: 5 * L,
                },
                // A transfer never offered on the session.
                _ => Message::FileAccept {
                    transfer: TransferId::from_bytes([0xEE; 16]),
                    offset: 0,
                },
            };
            link.send(&message).await.unwrap();
            let (_, ended) = drain(&mut link, QUIET).await;
            assert!(ended, "{bad}");
            // Nothing else changed.
            assert_eq!(a.status(&sent), Some(TransferStatus::Sending), "{bad}");
            a.stop().await;
        });
    }
}

/// B, driven by hand as A, offers `data` on `link` and is accepted: the
/// identifier, and the FileAccept that answered.
async fn offer_by_hand(
    b: &Peer,
    link: &mut Link<DuplexStream>,
    data: &[u8],
) -> (TransferId, TransferRef, u64) {
    let mut events = b.runtime().subscribe();
    let transfer = TransferId::from_bytes([0x42; 16]);
    link.send(&Message::FileOffer {
        transfer,
        size: data.len() as u64,
        filename: Filename::new("notes.txt").unwrap(),
    })
    .await
    .unwrap();
    let received = offered(&mut events).await;
    b.runtime().accept_transfer(&received).await.unwrap();
    let (messages, ended) = drain(link, QUIET).await;
    assert!(!ended);
    let offset = messages
        .iter()
        .find_map(|message| match message {
            Message::FileAccept {
                transfer: named,
                offset,
            } if *named == transfer => Some(*offset),
            _ => None,
        })
        .expect("an acceptance");
    (transfer, received, offset)
}

fn chunk(transfer: TransferId, data: &[u8], offset: u64) -> Message {
    let start = offset as usize;
    let end = (start + L as usize).min(data.len());
    Message::FileChunk(FileChunk::new(transfer, data[start..end].to_vec()).unwrap())
}

/// The acceptances on `link` in what arrived, for `transfer`.
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

/// Offers again, on `link`, the transfer `transfer` of `data`: the offset
/// the receiver answers with.
async fn offer_again(link: &mut Link<DuplexStream>, transfer: TransferId, data: &[u8]) -> u64 {
    link.send(&Message::FileOffer {
        transfer,
        size: data.len() as u64,
        filename: Filename::new("notes.txt").unwrap(),
    })
    .await
    .unwrap();
    let (messages, ended) = drain(link, QUIET).await;
    assert!(!ended);
    *acceptances(&messages, transfer)
        .last()
        .expect("an acceptance")
}

#[test]
fn a_transfer_resumes_from_what_the_receiver_holds_and_nothing_else() {
    run_paused(async {
        let network = MockNetwork::new();
        let (a, mut b, _, mut link) = a_by_hand(&network).await;
        let data = file(5 * L + 7);
        let (transfer, received, first) = offer_by_hand(&b, &mut link, &data).await;
        assert_eq!(first, 0);
        // Three chunks; the receiver acknowledges after two, once they are
        // synced.
        for n in 0..3 {
            link.send(&chunk(transfer, &data, n * L)).await.unwrap();
        }
        let (messages, _) = drain(&mut link, QUIET).await;
        assert_eq!(acceptances(&messages, transfer), vec![2 * L]);
        link.close().await.unwrap();
        // A later session: the offer again is answered with what the
        // receiver holds durably, not with what reached it. A chunk
        // carries no offset: the first of the session is taken at that
        // one.
        let mut link = hand_link(&network, &b, &a).await;
        assert_eq!(offer_again(&mut link, transfer, &data).await, 2 * L);
        // A chunk of another length than the bytes left allow ends the
        // session.
        link.send(&Message::FileChunk(
            FileChunk::new(transfer, data[2 * L as usize..3 * L as usize - 1].to_vec()).unwrap(),
        ))
        .await
        .unwrap();
        assert!(drain(&mut link, QUIET).await.1);
        // So does a completion with bytes missing.
        let mut link = hand_link(&network, &b, &a).await;
        assert_eq!(offer_again(&mut link, transfer, &data).await, 2 * L);
        link.send(&Message::FileComplete {
            transfer,
            digest: sha(&data),
        })
        .await
        .unwrap();
        assert!(drain(&mut link, QUIET).await.1);
        // From the offset the receiver gave, to the end, then the
        // completion: the whole file, staged once.
        let mut link = hand_link(&network, &b, &a).await;
        let start = offer_again(&mut link, transfer, &data).await;
        assert_eq!(start, 2 * L);
        let mut offset = start;
        while offset < data.len() as u64 {
            link.send(&chunk(transfer, &data, offset)).await.unwrap();
            offset = (offset + L).min(data.len() as u64);
        }
        link.send(&Message::FileComplete {
            transfer,
            digest: sha(&data),
        })
        .await
        .unwrap();
        let (messages, ended) = drain(&mut link, QUIET).await;
        assert!(!ended);
        assert!(messages.contains(&Message::FileComplete {
            transfer,
            digest: sha(&data),
        }));
        assert_eq!(b.status(&received), Some(TransferStatus::Received));
        assert_eq!(saved(&b, &received).await, data);
        // An abort for a complete transfer is discarded; offered again, it
        // is answered as complete.
        link.send(&Message::FileAbort(transfer)).await.unwrap();
        link.close().await.unwrap();
        let mut link = hand_link(&network, &b, &a).await;
        link.send(&Message::FileOffer {
            transfer,
            size: data.len() as u64,
            filename: Filename::new("notes.txt").unwrap(),
        })
        .await
        .unwrap();
        let (messages, _) = drain(&mut link, QUIET).await;
        assert_eq!(
            messages,
            vec![Message::FileComplete {
                transfer,
                digest: sha(&data)
            }]
        );
        b.stop().await;
    });
}

#[test]
fn a_sender_that_resumes_from_another_offset_than_the_receiver_never_completes() {
    run_paused(async {
        let network = MockNetwork::new();
        let (a, mut b, _, mut link) = a_by_hand(&network).await;
        let data = file(6 * L);
        let (transfer, received, _) = offer_by_hand(&b, &mut link, &data).await;
        for n in 0..2 {
            link.send(&chunk(transfer, &data, n * L)).await.unwrap();
        }
        drain(&mut link, QUIET).await;
        link.close().await.unwrap();
        let mut link = hand_link(&network, &b, &a).await;
        assert_eq!(offer_again(&mut link, transfer, &data).await, 2 * L);
        // The sender goes on from a later offset: the bytes are taken at
        // the offset of the receiver, and the digest does not match.
        for n in 3..6 {
            link.send(&chunk(transfer, &data, n * L)).await.unwrap();
        }
        link.send(&chunk(transfer, &data, 5 * L)).await.unwrap();
        link.send(&Message::FileComplete {
            transfer,
            digest: sha(&data),
        })
        .await
        .unwrap();
        let (messages, _) = drain(&mut link, QUIET).await;
        assert!(messages.contains(&Message::FileAbort(transfer)));
        let view = b.view(&received).unwrap();
        assert_eq!(view.status, TransferStatus::Failed);
        assert_eq!(view.reason, Some(TransferReason::Digest));
        b.stop().await;
    });
}

#[test]
fn a_file_that_does_not_match_its_digest_fails_and_is_never_offered_as_received() {
    run_paused(async {
        let network = MockNetwork::new();
        let (_a, mut b, _, mut link) = a_by_hand(&network).await;
        let data = file(L + 9);
        let (transfer, received, _) = offer_by_hand(&b, &mut link, &data).await;
        link.send(&chunk(transfer, &data, 0)).await.unwrap();
        link.send(&chunk(transfer, &data, L)).await.unwrap();
        link.send(&Message::FileComplete {
            transfer,
            digest: [9; 32],
        })
        .await
        .unwrap();
        let (messages, ended) = drain(&mut link, QUIET).await;
        assert!(!ended);
        assert!(messages.contains(&Message::FileAbort(transfer)));
        let view = b.view(&received).unwrap();
        assert_eq!(view.status, TransferStatus::Failed);
        assert_eq!(view.reason, Some(TransferReason::Digest));
        assert!(b.runtime().begin_save(&received).await.is_err());
        // Offered again: aborted, never asked for again.
        link.send(&Message::FileOffer {
            transfer: TransferId::from_bytes([0x43; 16]),
            size: 1,
            filename: Filename::new("x").unwrap(),
        })
        .await
        .unwrap();
        b.stop().await;
    });
}

#[test]
fn a_completion_before_every_byte_arrived_ends_the_session() {
    run_paused(async {
        let network = MockNetwork::new();
        let (_a, mut b, _, mut link) = a_by_hand(&network).await;
        let data = file(2 * L);
        let (transfer, received, _) = offer_by_hand(&b, &mut link, &data).await;
        link.send(&chunk(transfer, &data, 0)).await.unwrap();
        link.send(&Message::FileComplete {
            transfer,
            digest: sha(&data),
        })
        .await
        .unwrap();
        assert!(drain(&mut link, QUIET).await.1);
        assert_eq!(b.status(&received), Some(TransferStatus::Receiving));
        b.stop().await;
    });
}

#[test]
fn a_peer_of_phase_7_rejects_the_offer_and_chat_goes_on() {
    // What a Phase 7 peer does with file messages (`DESIGN_QUESTIONS.md`
    // P7-4): it answers every offer with FileReject at once, discards a
    // FileAbort that names it, and answers Pings. It never accepts, so it
    // never receives a FileAccept, a FileChunk or a FileComplete.
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, mut link) = b_by_hand(&network).await;
        let sent = offer(&a, &at_a, &file(100)).await;
        let (messages, _) = drain(&mut link, QUIET).await;
        let (transfer, _, _) = offer_in(&messages);
        link.send(&Message::FileReject(transfer)).await.unwrap();
        until(|| a.status(&sent) == Some(TransferStatus::Rejected)).await;
        // Chat on the same session, both ways.
        a.runtime()
            .send_message(&a.id(), &at_a, "still here")
            .await
            .unwrap();
        let (messages, ended) = drain(&mut link, QUIET).await;
        assert!(!ended);
        let id = messages
            .iter()
            .find_map(|message| match message {
                Message::ChatMessage { id, .. } => Some(*id),
                _ => None,
            })
            .expect("the chat message");
        // Nothing of the file but the offer reached it.
        assert!(messages.iter().all(|message| !matches!(
            message,
            Message::FileAccept { .. } | Message::FileChunk(_) | Message::FileComplete { .. }
        )));
        link.send(&Message::MessageAck(id)).await.unwrap();
        link.send(&Message::ChatMessage {
            id: MessageId::from_bytes([3; 16]),
            text: ChatText::new("hi").unwrap(),
        })
        .await
        .unwrap();
        let (messages, ended) = drain(&mut link, QUIET).await;
        assert!(!ended);
        assert!(messages.contains(&Message::MessageAck(MessageId::from_bytes([3; 16]))));
        until(|| {
            a.statuses(&at_a)
                == vec![
                    MessageStatus::Delivered,
                    MessageStatus::Delivered,
                    MessageStatus::Received,
                ]
        })
        .await;
        // A later session offers nothing more.
        link.close().await.unwrap();
        a.stop().await;
    });
}

#[test]
fn a_cancellation_of_the_receiver_is_told_and_what_crossed_it_is_discarded_within_the_window() {
    run_paused(async {
        let network = MockNetwork::new();
        let (a, mut b, _, mut link) = a_by_hand(&network).await;
        let data = file(20 * L);
        let (transfer, received, _) = offer_by_hand(&b, &mut link, &data).await;
        for n in 0..3 {
            link.send(&chunk(transfer, &data, n * L)).await.unwrap();
        }
        let (messages, _) = drain(&mut link, QUIET).await;
        assert_eq!(acceptances(&messages, transfer), vec![2 * L]);
        b.runtime().cancel_transfer(&received).await.unwrap();
        // Cancelling twice is cancelling once.
        b.runtime().cancel_transfer(&received).await.unwrap();
        let (messages, ended) = drain(&mut link, QUIET).await;
        assert!(!ended);
        assert_eq!(messages, vec![Message::FileAbort(transfer)]);
        let view = b.view(&received).unwrap();
        assert_eq!(view.status, TransferStatus::Cancelled);
        assert_eq!(view.reason, Some(TransferReason::ByUs));
        assert!(b.runtime().begin_save(&received).await.is_err());
        // The window of the sender beyond the last acknowledgement may
        // still be on the way: three chunks are discarded, a fourth ends
        // the session.
        for n in 3..6 {
            link.send(&chunk(transfer, &data, n * L)).await.unwrap();
        }
        assert!(!drain(&mut link, QUIET).await.1);
        link.send(&chunk(transfer, &data, 6 * L)).await.unwrap();
        assert!(drain(&mut link, QUIET).await.1);
        // A stale chunk recreated nothing; offered again, it is aborted.
        assert_eq!(b.status(&received), Some(TransferStatus::Cancelled));
        let mut link = hand_link(&network, &b, &a).await;
        link.send(&Message::FileOffer {
            transfer,
            size: data.len() as u64,
            filename: Filename::new("notes.txt").unwrap(),
        })
        .await
        .unwrap();
        let (messages, _) = drain(&mut link, QUIET).await;
        assert_eq!(messages, vec![Message::FileAbort(transfer)]);
        b.stop().await;
    });
}

#[test]
fn a_cancellation_of_the_sender_is_told_once_and_stops_its_chunks() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, b, at_a, mut link) = b_by_hand(&network).await;
        let sent = offer(&a, &at_a, &file(20 * L)).await;
        let (messages, _) = drain(&mut link, QUIET).await;
        let (transfer, _, _) = offer_in(&messages);
        link.send(&Message::FileAccept {
            transfer,
            offset: 0,
        })
        .await
        .unwrap();
        drain(&mut link, QUIET).await;
        a.runtime().cancel_transfer(&sent).await.unwrap();
        a.runtime().cancel_transfer(&sent).await.unwrap();
        // The abort, and nothing of the file after it; the Ping after it
        // is answered, so the sender knows the receiver read it.
        let (messages, ended) = drain(&mut link, QUIET).await;
        assert!(!ended);
        assert_eq!(messages, vec![Message::FileAbort(transfer)]);
        let view = a.view(&sent).unwrap();
        assert_eq!(view.status, TransferStatus::Cancelled);
        assert_eq!(view.reason, Some(TransferReason::ByUs));
        link.send(&Message::FileAccept {
            transfer,
            offset: 2 * L,
        })
        .await
        .unwrap();
        assert!(!drain(&mut link, QUIET).await.1);
        // Told: a later session offers it no more.
        link.close().await.unwrap();
        let mut link = hand_link(&network, &a, &b).await;
        let (messages, _) = drain(&mut link, QUIET).await;
        assert!(messages.is_empty(), "{messages:?}");
        a.stop().await;
    });
}

#[test]
fn a_cancellation_the_receiver_could_not_be_told_is_told_with_the_next_offer() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, b, at_a, mut link) = b_by_hand(&network).await;
        let sent = offer(&a, &at_a, &file(20 * L)).await;
        let (messages, _) = drain(&mut link, QUIET).await;
        let (transfer, _, _) = offer_in(&messages);
        link.close().await.unwrap();
        a.runtime().cancel_transfer(&sent).await.unwrap();
        let mut link = hand_link(&network, &a, &b).await;
        let (messages, _) = drain(&mut link, QUIET).await;
        assert_eq!(messages.len(), 2, "{messages:?}");
        assert!(
            matches!(messages[0], Message::FileOffer { transfer: named, .. } if named == transfer)
        );
        assert_eq!(messages[1], Message::FileAbort(transfer));
        // Read: never again.
        link.close().await.unwrap();
        let mut link = hand_link(&network, &a, &b).await;
        assert!(drain(&mut link, QUIET).await.0.is_empty());
        a.stop().await;
    });
}

#[test]
fn a_cancellation_the_receiver_could_not_be_told_is_removed_when_the_user_asks() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, b, at_a, mut link) = b_by_hand(&network).await;
        let sent = offer(&a, &at_a, &file(20 * L)).await;
        let (messages, _) = drain(&mut link, QUIET).await;
        offer_in(&messages);
        link.close().await.unwrap();
        a.runtime().cancel_transfer(&sent).await.unwrap();
        // Ended, its receiver not told yet: the user takes it off the
        // list, as the card of a stopped transfer offers.
        a.runtime().remove_transfer(&sent).await.unwrap();
        assert_eq!(a.status(&sent), None);
        // The receiver is offered it no more; it learns that the file is
        // no longer offered.
        let mut link = hand_link(&network, &a, &b).await;
        assert!(drain(&mut link, QUIET).await.0.is_empty());
        a.stop().await;
    });
}

/// A source that gives `after` bytes, then waits until it is let go.
struct Paused<R> {
    inner: R,
    after: usize,
    given: usize,
    started: Option<std::sync::mpsc::Sender<()>>,
    go: std::sync::mpsc::Receiver<()>,
}

impl<R: Read> Read for Paused<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.given >= self.after
            && let Some(started) = self.started.take()
        {
            let _ = started.send(());
            let _ = self.go.recv();
        }
        let room = buf.len().min(self.after.saturating_sub(self.given).max(1));
        let read = self.inner.read(&mut buf[..room])?;
        self.given += read;
        Ok(read)
    }
}

#[test]
fn a_contact_deleted_while_its_file_is_saved_cancels_the_save_and_leaves_no_file_or_part_of_one() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = accepted(&network).await;
        let mut events = b.runtime().subscribe();
        let data = file(6 * L);
        offer(&a, &at_a, &data).await;
        let received = offered(&mut events).await;
        b.runtime().accept_transfer(&received).await.unwrap();
        until(|| b.status(&received) == Some(TransferStatus::Received)).await;
        // The save of the runtime, with its source paused halfway: the
        // contact is deleted there.
        let folder = scratch("deleted-while-saved");
        let destination = folder.join("report.pdf");
        let reader = b.runtime().begin_save(&received).await.unwrap();
        let (started, reading) = std::sync::mpsc::channel();
        let (go, gate) = std::sync::mpsc::channel();
        let mut source = Paused {
            inner: reader,
            after: 3 * L as usize,
            given: 0,
            started: Some(started),
            go: gate,
        };
        let target = destination.clone();
        let saving = tokio::task::spawn_blocking(move || {
            monolith_storage::export::save_new(&target, &mut source)
        });
        while reading.try_recv().is_err() {
            tokio::task::yield_now().await;
            std::thread::sleep(Duration::from_millis(1));
        }
        b.runtime().delete_contact(&b.id(), &at_b).await.unwrap();
        go.send(()).unwrap();
        // The serial order where the deletion came first: the blob went,
        // so the save's next read failed and the save was cancelled. No file
        // is under the name, not a part of one, and no temporary file is
        // left. Ending the save of a record that went changes nothing.
        assert!(saving.await.unwrap().is_err());
        assert!(names(&folder).is_empty(), "{:?}", names(&folder));
        assert!(!destination.exists());
        assert_eq!(b.runtime().end_save(&received, false).await, Ok(()));
        assert!(b.runtime().transfers(&b.id()).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&folder);
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn two_files_to_one_contact_go_one_at_a_time_in_the_order_accepted_and_never_mixed() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, mut link) = b_by_hand(&network).await;
        let first = file(5 * L + 3);
        let second: Vec<u8> = file(4 * L + 11).iter().map(|byte| !byte).collect();
        let sent_first = offer(&a, &at_a, &first).await;
        let sent_second = offer(&a, &at_a, &second).await;
        let offers = offers_in(&drain(&mut link, QUIET).await.0);
        assert_eq!(offers.len(), 2, "{offers:?}");
        let id_of = |size: usize| {
            offers
                .iter()
                .find(|(_, offered)| *offered == size as u64)
                .unwrap()
                .0
        };
        let (one, two) = (id_of(first.len()), id_of(second.len()));
        // The later offer is accepted first.
        for transfer in [two, one] {
            link.send(&Message::FileAccept {
                transfer,
                offset: 0,
            })
            .await
            .unwrap();
        }
        let mut order = Vec::new();
        let mut held: Vec<(TransferId, Vec<u8>)> = vec![(one, Vec::new()), (two, Vec::new())];
        let mut sent = Vec::new();
        while sent.len() < 2 {
            let (messages, ended) = drain(&mut link, QUIET).await;
            assert!(!ended && !messages.is_empty(), "stalled after {sent:?}");
            for message in &messages {
                match message {
                    Message::FileChunk(chunk) => {
                        order.push(*chunk.transfer());
                        let at = held
                            .iter()
                            .position(|(id, _)| id == chunk.transfer())
                            .unwrap();
                        held[at].1.extend_from_slice(chunk.data());
                    }
                    Message::FileComplete { transfer, .. } => sent.push(*transfer),
                    _ => {}
                }
            }
            // What each holds, in whole chunks, or all of it.
            for (transfer, data) in &held {
                let size = if *transfer == one {
                    first.len()
                } else {
                    second.len()
                } as u64;
                let have = data.len() as u64;
                let offset = if have == size { size } else { have / L * L };
                if offset > 0 {
                    link.send(&Message::FileAccept {
                        transfer: *transfer,
                        offset,
                    })
                    .await
                    .unwrap();
                }
            }
        }
        // One at a time, the one accepted first first, each file whole.
        let switch = order.iter().position(|transfer| *transfer == one).unwrap();
        assert!(
            order[..switch].iter().all(|transfer| *transfer == two),
            "{order:?}"
        );
        assert!(
            order[switch..].iter().all(|transfer| *transfer == one),
            "{order:?}"
        );
        assert!(held[0].1 == first && held[1].1 == second);
        for (transfer, data) in [(one, &first), (two, &second)] {
            link.send(&Message::FileComplete {
                transfer,
                digest: sha(data),
            })
            .await
            .unwrap();
        }
        drain(&mut link, QUIET).await;
        assert_eq!(a.status(&sent_first), Some(TransferStatus::Completed));
        assert_eq!(a.status(&sent_second), Some(TransferStatus::Completed));
        a.stop().await;
    });
}

#[test]
fn a_pong_confirms_what_its_ping_followed_and_nothing_written_after_it() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, b, at_a, mut link) = b_by_hand(&network).await;
        let first = offer(&a, &at_a, &file(20 * L)).await;
        let second = offer(&a, &at_a, &file(30 * L)).await;
        // Both offers read: the Pings that follow them answered.
        let (messages, ended) = drain(&mut link, QUIET).await;
        assert!(!ended);
        let offers = offers_in(&messages);
        assert_eq!(offers.len(), 2, "{messages:?}");
        let id = |size: u64| {
            offers
                .iter()
                .find(|(_, offered)| *offered == size)
                .map(|(transfer, _)| *transfer)
                .unwrap()
        };
        let (first_id, second_id) = (id(20 * L), id(30 * L));
        a.runtime().cancel_transfer(&first).await.unwrap();
        let (messages, ended) = take(&mut link, QUIET).await;
        assert!(!ended);
        let [Message::FileAbort(aborted), Message::Ping(older)] = messages.as_slice() else {
            panic!("not a cancellation and its Ping: {messages:?}");
        };
        assert_eq!(*aborted, first_id);
        let older = *older;
        // A second cancellation while that Ping waits gets no Ping of its
        // own yet.
        a.runtime().cancel_transfer(&second).await.unwrap();
        let (messages, _) = take(&mut link, QUIET).await;
        assert_eq!(messages, vec![Message::FileAbort(second_id)]);
        // The answer to the older Ping covers the first cancellation alone:
        // the second is covered by a new Ping, which goes unanswered.
        link.send(&Message::Pong(older)).await.unwrap();
        let (messages, ended) = take(&mut link, QUIET).await;
        assert!(!ended);
        assert!(
            matches!(messages.as_slice(), [Message::Ping(newer)] if *newer != older),
            "{messages:?}"
        );
        link.close().await.unwrap();
        // The next session tells the second cancellation again, and never
        // the first.
        let mut link = hand_link(&network, &a, &b).await;
        let (messages, _) = drain(&mut link, QUIET).await;
        assert_eq!(
            offers_in(&messages),
            vec![(second_id, 30 * L)],
            "{messages:?}"
        );
        assert_eq!(
            messages.last(),
            Some(&Message::FileAbort(second_id)),
            "{messages:?}"
        );
        assert!(!messages.contains(&Message::FileAbort(first_id)));
        // Read now: neither comes again.
        link.close().await.unwrap();
        let mut link = hand_link(&network, &a, &b).await;
        assert!(drain(&mut link, QUIET).await.0.is_empty());
        a.stop().await;
    });
}

#[test]
fn a_pong_that_answers_no_ping_of_its_session_ends_it_and_confirms_nothing() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, b, at_a, mut link) = b_by_hand(&network).await;
        let sent = offer(&a, &at_a, &file(20 * L)).await;
        let (messages, _) = drain(&mut link, QUIET).await;
        let (transfer, _, _) = offer_in(&messages);
        a.runtime().cancel_transfer(&sent).await.unwrap();
        let (messages, _) = take(&mut link, QUIET).await;
        let [Message::FileAbort(_), Message::Ping(nonce)] = messages.as_slice() else {
            panic!("not a cancellation and its Ping: {messages:?}");
        };
        // A Pong with another nonce: one of an earlier Ping, or of another
        // session.
        let mut other = *nonce;
        other[0] ^= 1;
        link.send(&Message::Pong(other)).await.unwrap();
        let (_, ended) = take(&mut link, QUIET).await;
        assert!(ended);
        // Nothing was confirmed: the cancellation is told again.
        let mut link = hand_link(&network, &a, &b).await;
        let (messages, ended) = take(&mut link, QUIET).await;
        assert!(!ended);
        assert_eq!(offers_in(&messages), vec![(transfer, 20 * L)]);
        assert!(messages.contains(&Message::FileAbort(transfer)));
        // A Pong where no Ping waits ends the session too, before anything
        // is confirmed.
        let nonce = messages
            .iter()
            .find_map(|message| match message {
                Message::Ping(nonce) => Some(*nonce),
                _ => None,
            })
            .expect("a Ping");
        link.send(&Message::Pong(nonce)).await.unwrap();
        link.send(&Message::Pong(nonce)).await.unwrap();
        let (_, ended) = take(&mut link, QUIET).await;
        assert!(ended);
        // The first Pong answered the Ping: the cancellation was read, and
        // is not told again.
        let mut link = hand_link(&network, &a, &b).await;
        assert!(drain(&mut link, QUIET).await.0.is_empty());
        a.stop().await;
    });
}

#[test]
fn a_receiver_that_completed_before_it_read_a_cancellation_holds_the_file() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, mut link) = b_by_hand(&network).await;
        let data = file(2 * L);
        let sent = offer(&a, &at_a, &data).await;
        let (messages, _) = drain(&mut link, QUIET).await;
        let (transfer, _, _) = offer_in(&messages);
        link.send(&Message::FileAccept {
            transfer,
            offset: 0,
        })
        .await
        .unwrap();
        let (messages, _) = drain(&mut link, QUIET).await;
        // Every chunk, then the completion of the sender.
        assert!(messages.contains(&Message::FileComplete {
            transfer,
            digest: sha(&data),
        }));
        a.runtime().cancel_transfer(&sent).await.unwrap();
        // The completion of the receiver crossed the cancellation.
        link.send(&Message::FileComplete {
            transfer,
            digest: sha(&data),
        })
        .await
        .unwrap();
        drain(&mut link, QUIET).await;
        assert_eq!(a.status(&sent), Some(TransferStatus::Completed));
        a.stop().await;
    });
}

#[test]
fn blocking_the_sender_stops_the_transfer_and_nothing_revives_it() {
    run_paused(async {
        let network = MockNetwork::new();
        let (a, mut b, at_b, mut link) = a_by_hand(&network).await;
        let data = file(20 * L);
        let (transfer, received, _) = offer_by_hand(&b, &mut link, &data).await;
        for n in 0..3 {
            link.send(&chunk(transfer, &data, n * L)).await.unwrap();
        }
        drain(&mut link, QUIET).await;
        b.runtime().block_contact(&b.id(), &at_b).await.unwrap();
        // The session ends at once, with nothing told of the transfer.
        let (messages, ended) = drain(&mut link, QUIET).await;
        assert!(ended);
        assert!(
            messages.iter().all(|message| !matches!(
                message,
                Message::FileAbort(_) | Message::FileAccept { .. }
            ))
        );
        let view = b.view(&received).unwrap();
        assert_eq!(view.status, TransferStatus::Cancelled);
        assert_eq!(view.reason, Some(TransferReason::ContactEnded));
        assert!(b.runtime().begin_save(&received).await.is_err());
        // A cannot reach B again: no session of it is confirmed.
        if let Ok(established) = dial(
            &network.backend(),
            &Budgets::new(),
            &a.identity,
            &b.identity.card(),
        )
        .await
        {
            let mut link = established.link;
            while let Ok(Ok(arrived)) = tokio::time::timeout(QUIET, link.receive()).await {
                assert!(!arrived.received().actions.contains(&Action::Confirmed));
            }
        }
        b.stop().await;
    });
}

#[test]
fn blocking_the_receiver_stops_the_chunks() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, mut link) = b_by_hand(&network).await;
        let sent = offer(&a, &at_a, &file(20 * L)).await;
        let (messages, _) = drain(&mut link, QUIET).await;
        let (transfer, _, _) = offer_in(&messages);
        link.send(&Message::FileAccept {
            transfer,
            offset: 0,
        })
        .await
        .unwrap();
        drain(&mut link, QUIET).await;
        a.runtime().block_contact(&a.id(), &at_a).await.unwrap();
        link.send(&Message::FileAccept {
            transfer,
            offset: 4 * L,
        })
        .await
        .ok();
        let (messages, ended) = drain(&mut link, QUIET).await;
        assert!(ended);
        assert!(
            messages
                .iter()
                .all(|message| !matches!(message, Message::FileChunk(_)))
        );
        let view = a.view(&sent).unwrap();
        assert_eq!(view.status, TransferStatus::Cancelled);
        assert_eq!(view.reason, Some(TransferReason::ContactEnded));
        a.stop().await;
    });
}

#[test]
fn a_deleted_contact_loses_its_transfers_and_a_contact_made_again_inherits_none() {
    run_paused(async {
        let network = MockNetwork::new();
        let (a, mut b, at_b, mut link) = a_by_hand(&network).await;
        let data = file(4 * L);
        let (transfer, received, _) = offer_by_hand(&b, &mut link, &data).await;
        link.send(&chunk(transfer, &data, 0)).await.unwrap();
        drain(&mut link, QUIET).await;
        b.runtime().delete_contact(&b.id(), &at_b).await.unwrap();
        assert!(b.view(&received).is_none());
        assert!(b.runtime().transfers(&b.id()).unwrap().is_empty());
        // Made again: a new record, and the same offer is a new one.
        let again = b.add(&a).await;
        assert_ne!(again, at_b);
        let mut events = b.runtime().subscribe();
        let mut link = hand_link(&network, &b, &a).await;
        link.send(&Message::FileOffer {
            transfer,
            size: data.len() as u64,
            filename: Filename::new("notes.txt").unwrap(),
        })
        .await
        .unwrap();
        let fresh = offered(&mut events).await;
        assert_eq!(fresh.contact, again);
        assert_eq!(b.status(&fresh), Some(TransferStatus::Offered));
        b.stop().await;
    });
}

#[test]
fn a_chat_message_of_the_sender_is_not_held_behind_its_chunks() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, mut link) = b_by_hand(&network).await;
        offer(&a, &at_a, &file(40 * L)).await;
        let (messages, _) = drain(&mut link, QUIET).await;
        let (transfer, _, _) = offer_in(&messages);
        link.send(&Message::FileAccept {
            transfer,
            offset: 0,
        })
        .await
        .unwrap();
        drain(&mut link, QUIET).await;
        // The window is full and the receiver acknowledges nothing: a chat
        // message goes out all the same, at once.
        a.runtime()
            .send_message(&a.id(), &at_a, "while sending")
            .await
            .unwrap();
        let (messages, _) = drain(&mut link, QUIET).await;
        assert_eq!(messages.len(), 1, "{messages:?}");
        let Message::ChatMessage { id, .. } = messages[0] else {
            panic!("not a chat message: {messages:?}");
        };
        link.send(&Message::MessageAck(id)).await.unwrap();
        until(|| a.statuses(&at_a) == vec![MessageStatus::Delivered; 2]).await;
        // With the window open again, a chat message queued meanwhile waits
        // behind at most the chunks the window allows.
        link.send(&Message::FileAccept {
            transfer,
            offset: 4 * L,
        })
        .await
        .unwrap();
        a.runtime()
            .send_message(&a.id(), &at_a, "between chunks")
            .await
            .unwrap();
        let (messages, _) = drain(&mut link, QUIET).await;
        let chat = messages
            .iter()
            .position(|message| matches!(message, Message::ChatMessage { .. }))
            .expect("the chat message");
        assert!(chat as u64 <= FILE_WINDOW_CHUNKS, "{chat}");
        assert_eq!(
            chunks_of(&messages, transfer).len() as u64,
            FILE_WINDOW_CHUNKS
        );
        a.stop().await;
    });
}

#[test]
fn a_chat_message_waiting_while_chunks_go_out_is_written_after_at_most_one_chunk() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, mut link) = b_by_hand(&network).await;
        offer(&a, &at_a, &file(40 * L)).await;
        let (transfer, _, _) = offer_in(&drain(&mut link, QUIET).await.0);
        link.send(&Message::FileAccept {
            transfer,
            offset: 0,
        })
        .await
        .unwrap();
        drain(&mut link, QUIET).await;
        // The window is full: a first message goes out and is not
        // acknowledged yet, a second waits behind it.
        for text in ["first", "second"] {
            a.runtime()
                .send_message(&a.id(), &at_a, text)
                .await
                .unwrap();
        }
        let (messages, _) = drain(&mut link, QUIET).await;
        let [Message::ChatMessage { id: first, .. }] = messages.as_slice() else {
            panic!("not one chat message: {messages:?}");
        };
        // The window opens and the first message is acknowledged at once:
        // chunks may go again, and the second message goes out after at
        // most one of them (ADR 0013, "Scheduling").
        link.send(&Message::FileAccept {
            transfer,
            offset: 4 * L,
        })
        .await
        .unwrap();
        link.send(&Message::MessageAck(*first)).await.unwrap();
        let (messages, _) = drain(&mut link, QUIET).await;
        let chat = messages
            .iter()
            .position(|message| matches!(message, Message::ChatMessage { .. }))
            .expect("the second message");
        assert!(chat <= 1, "{chat} chunks before it: {messages:?}");
        a.stop().await;
    });
}

#[test]
fn a_chat_message_between_chunks_is_acknowledged_before_the_next_chunk_is_read() {
    run_paused(async {
        let network = MockNetwork::new();
        let (_a, mut b, _, mut link) = a_by_hand(&network).await;
        let data = file(20 * L);
        let (transfer, received, _) = offer_by_hand(&b, &mut link, &data).await;
        let chat = MessageId::from_bytes([9; 16]);
        link.send(&chunk(transfer, &data, 0)).await.unwrap();
        link.send(&chunk(transfer, &data, L)).await.unwrap();
        link.send(&Message::ChatMessage {
            id: chat,
            text: ChatText::new("in the middle").unwrap(),
        })
        .await
        .unwrap();
        link.send(&chunk(transfer, &data, 2 * L)).await.unwrap();
        link.send(&chunk(transfer, &data, 3 * L)).await.unwrap();
        let (messages, _) = drain(&mut link, QUIET).await;
        assert_eq!(
            messages,
            vec![
                Message::FileAccept {
                    transfer,
                    offset: 2 * L
                },
                Message::MessageAck(chat),
                Message::FileAccept {
                    transfer,
                    offset: 4 * L
                },
            ]
        );
        assert_eq!(b.view(&received).unwrap().bytes, Some(4 * L));
        b.stop().await;
    });
}

#[test]
fn chat_goes_both_ways_while_a_large_file_is_sent() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = accepted(&network).await;
        let mut events = b.runtime().subscribe();
        let data = file(120 * L);
        let sent = offer(&a, &at_a, &data).await;
        let received = offered(&mut events).await;
        b.runtime().accept_transfer(&received).await.unwrap();
        a.runtime()
            .send_message(&a.id(), &at_a, "from the sender")
            .await
            .unwrap();
        b.runtime()
            .send_message(&b.id(), &at_b, "from the receiver")
            .await
            .unwrap();
        let delivered = |statuses: Vec<MessageStatus>| {
            statuses
                .iter()
                .filter(|status| **status == MessageStatus::Delivered)
                .count()
        };
        until(|| delivered(a.statuses(&at_a)) == 2 && delivered(b.statuses(&at_b)) == 1).await;
        until(|| a.status(&sent) == Some(TransferStatus::Completed)).await;
        assert_eq!(saved(&b, &received).await, data);
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn a_session_that_carries_a_transfer_lingers_from_its_last_progress() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, _b, at_a, mut link) = b_by_hand(&network).await;
        offer(&a, &at_a, &file(40 * L)).await;
        let (messages, _) = drain(&mut link, QUIET).await;
        let (transfer, _, _) = offer_in(&messages);
        link.send(&Message::FileAccept {
            transfer,
            offset: 0,
        })
        .await
        .unwrap();
        let mut held = chunks_of(&drain(&mut link, QUIET).await.0, transfer).len() as u64 * L;
        // An acknowledgement every hundred seconds: longer, in all, than a
        // session without activity lingers.
        let start = tokio::time::Instant::now();
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_secs(95)).await;
            link.send(&Message::FileAccept {
                transfer,
                offset: held,
            })
            .await
            .unwrap();
            let (messages, ended) = drain(&mut link, QUIET).await;
            assert!(!ended);
            held += chunks_of(&messages, transfer).len() as u64 * L;
        }
        assert!(start.elapsed() > SESSION_LINGER * 3);
        // No more: the session closes when it has lingered since the last
        // progress.
        let last = tokio::time::Instant::now();
        let (_, ended) = drain(&mut link, SESSION_LINGER * 2).await;
        assert!(ended);
        let lingered = last.elapsed();
        assert!(lingered >= SESSION_LINGER - QUIET, "{lingered:?}");
        assert!(lingered <= SESSION_LINGER + QUIET, "{lingered:?}");
        a.stop().await;
    });
}

#[test]
fn an_offer_accepted_when_no_session_is_left_makes_the_receiver_dial() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, at_b) = accepted(&network).await;
        let mut events = b.runtime().subscribe();
        let data = file(3 * L);
        let sent = offer(&a, &at_a, &data).await;
        let received = offered(&mut events).await;
        // The offer was read: nothing keeps a session, or makes a dial.
        until(|| b.runtime().sessions(&b.id(), &at_b).unwrap() == 0).await;
        tokio::time::sleep(Duration::from_secs(3600)).await;
        assert_eq!(b.runtime().sessions(&b.id(), &at_b).unwrap(), 0);
        b.runtime().accept_transfer(&received).await.unwrap();
        until(|| a.status(&sent) == Some(TransferStatus::Completed)).await;
        assert_eq!(saved(&b, &received).await, data);
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn an_offer_not_decided_in_time_expires_and_the_sender_learns_it() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, _) = accepted(&network).await;
        let mut events = b.runtime().subscribe();
        let sent = offer(&a, &at_a, &file(10)).await;
        let received = offered(&mut events).await;
        tokio::time::sleep(FILE_OFFER_TIMEOUT + Duration::from_secs(60)).await;
        assert_eq!(b.status(&received), Some(TransferStatus::Expired));
        assert_eq!(
            b.runtime().accept_transfer(&received).await,
            Err(TransferError::UnknownTransfer)
        );
        // The sender learns it at its next offer.
        a.runtime().connect(&a.id(), &at_a).unwrap();
        until(|| a.status(&sent) == Some(TransferStatus::Rejected)).await;
        a.stop().await;
        b.stop().await;
    });
}

#[test]
fn offers_above_the_pending_limit_of_a_contact_are_rejected_without_a_record() {
    run_paused(async {
        let network = MockNetwork::new();
        let (_a, mut b, _, mut link) = a_by_hand(&network).await;
        let mut events = b.runtime().subscribe();
        for byte in 1..=u8::try_from(MAX_PENDING_FILE_OFFERS_PER_CONTACT).unwrap() + 1 {
            link.send(&Message::FileOffer {
                transfer: TransferId::from_bytes([byte; 16]),
                size: 10,
                filename: Filename::new("x.txt").unwrap(),
            })
            .await
            .unwrap();
        }
        let (messages, ended) = drain(&mut link, Duration::from_secs(60)).await;
        assert!(!ended);
        assert_eq!(
            messages,
            vec![Message::FileReject(TransferId::from_bytes(
                [u8::try_from(MAX_PENDING_FILE_OFFERS_PER_CONTACT).unwrap() + 1; 16]
            ))]
        );
        let mut offers = 0;
        while let Ok(event) = events.try_recv() {
            if matches!(event, Event::TransferOffered(_)) {
                offers += 1;
            }
        }
        assert_eq!(offers, MAX_PENDING_FILE_OFFERS_PER_CONTACT);
        assert_eq!(
            b.runtime().transfers(&b.id()).unwrap().len(),
            MAX_PENDING_FILE_OFFERS_PER_CONTACT
        );
        b.stop().await;
    });
}

#[test]
fn an_accepted_transfer_the_sender_no_longer_offers_fails() {
    run_paused(async {
        let network = MockNetwork::new();
        let (a, mut b, _, mut link) = a_by_hand(&network).await;
        let data = file(4 * L);
        let (_, received, _) = offer_by_hand(&b, &mut link, &data).await;
        link.close().await.unwrap();
        // Sessions of the sender that last and offer nothing.
        for round in 0..3 {
            assert_eq!(
                b.status(&received),
                Some(TransferStatus::Receiving),
                "{round}"
            );
            let mut link = hand_link(&network, &b, &a).await;
            drain(&mut link, TRANSFER_GRACE + QUIET).await;
            link.close().await.unwrap();
        }
        until(|| b.status(&received) == Some(TransferStatus::Failed)).await;
        assert_eq!(
            b.view(&received).unwrap().reason,
            Some(TransferReason::OffererGone)
        );
        b.stop().await;
    });
}

#[test]
fn a_dial_for_a_transfer_alone_that_brings_no_progress_is_repeated_with_growing_delays() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, mut b, at_a, _) = accepted(&network).await;
        b.stop().await;
        // B is driven by hand behind its own service: it takes every
        // session, reads the offer, and drops the session without an
        // answer. Nothing makes progress, and the offer is never known to
        // be read, so A keeps wanting a session.
        let mut service = network
            .backend()
            .publish_onion(KeySource::Existing {
                secret: OnionServiceSecret::from_bytes(&[2; 64]),
                expected: fixed_endpoint(2),
            })
            .await
            .unwrap();
        offer(&a, &at_a, &file(10)).await;
        let start = tokio::time::Instant::now();
        let window = Duration::from_secs(300);
        let mut sessions = Vec::new();
        while let Some(left) = window.checked_sub(start.elapsed()) {
            let Ok(stream) = tokio::time::timeout(left, service.accept()).await else {
                break;
            };
            let Ok(established) = answer(stream.unwrap(), &Budgets::new(), &b.identity).await
            else {
                continue;
            };
            sessions.push(start.elapsed());
            let mut link = established.link;
            for action in established.first {
                if action == Action::SendContactAccept {
                    link.send(&Message::ContactAccept).await.unwrap();
                }
            }
            while let Ok(Ok(arrived)) = tokio::time::timeout(QUIET, link.receive()).await {
                if matches!(arrived.received().message, Message::FileOffer { .. }) {
                    break;
                }
            }
            drop(link);
        }
        // The delays of the scheduler grow from ten seconds, with their
        // jitter, and none is skipped: a few sessions in five minutes, and
        // never one right after another.
        assert!(sessions.len() >= 2, "{sessions:?}");
        assert!(sessions.len() <= 7, "{sessions:?}");
        for pair in sessions.windows(2) {
            assert!(pair[1] - pair[0] >= Duration::from_secs(4), "{sessions:?}");
        }
        a.stop().await;
    });
}

/// What a peer of Phase 7 does with a file message: the logic of
/// `crates/monolith-core/src/transfers.rs` at the tag `phase-7-complete`,
/// as it was there. Its FileAccept had a body of 16 bytes, so the
/// FileAccept of Phase 8 would not even decode there: it is a violation
/// here as well.
mod phase_7 {
    use std::collections::VecDeque;

    use monolith_protocol::body::{Message, TransferId};
    use monolith_protocol::limits::MAX_ENDED_TRANSFER_IDS;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Answer {
        Reject(TransferId),
        Discard,
        Violation,
    }

    #[derive(Debug, Default)]
    pub struct Transfers {
        ended: VecDeque<TransferId>,
    }

    impl Transfers {
        pub fn receive(&mut self, message: &Message) -> Option<Answer> {
            let (transfer, offer, abort) = match message {
                Message::FileOffer { transfer, .. } => (*transfer, true, false),
                Message::FileAbort(transfer) => (*transfer, false, true),
                Message::FileAccept { .. } => return Some(Answer::Violation),
                Message::FileReject(transfer) => (*transfer, false, false),
                Message::FileChunk(chunk) => (*chunk.transfer(), false, false),
                Message::FileComplete { transfer, .. } => (*transfer, false, false),
                _ => return None,
            };
            if self.ended.contains(&transfer) {
                return Some(if offer || abort {
                    Answer::Discard
                } else {
                    Answer::Violation
                });
            }
            if !offer {
                return Some(Answer::Violation);
            }
            if self.ended.len() >= MAX_ENDED_TRANSFER_IDS {
                self.ended.pop_front();
            }
            self.ended.push_back(transfer);
            Some(Answer::Reject(transfer))
        }
    }
}

/// Plays a peer of Phase 7 on `link` for `quiet` of silence: every file
/// message goes through its logic, an offer is rejected at once, a chat
/// message is acknowledged. Returns the chat messages it took. A file
/// message that would have ended its session fails the test.
async fn as_phase_7(
    link: &mut Link<DuplexStream>,
    peer: &mut phase_7::Transfers,
    quiet: Duration,
) -> Vec<MessageId> {
    let mut chat = Vec::new();
    loop {
        let (messages, ended) = drain(link, quiet).await;
        assert!(!ended, "the session ended");
        if messages.is_empty() {
            return chat;
        }
        for message in messages {
            match peer.receive(&message) {
                Some(phase_7::Answer::Violation) => {
                    panic!("a peer of Phase 7 ends the session for {message:?}")
                }
                Some(phase_7::Answer::Reject(transfer)) => {
                    link.send(&Message::FileReject(transfer)).await.unwrap();
                }
                Some(phase_7::Answer::Discard) | None => {}
            }
            if let Message::ChatMessage { id, .. } = message {
                link.send(&Message::MessageAck(id)).await.unwrap();
                chat.push(id);
            }
        }
    }
}

#[test]
fn a_sender_of_phase_8_sends_a_peer_of_phase_7_nothing_that_ends_its_session() {
    run_paused(async {
        let network = MockNetwork::new();
        let (mut a, b, at_a, mut link) = b_by_hand(&network).await;
        let mut peer = phase_7::Transfers::default();
        // Three files, the empty one among them, and a chat message.
        let mut sent = Vec::new();
        for data in [file(10), file(0), file(3 * L)] {
            sent.push(offer(&a, &at_a, &data).await);
        }
        a.runtime()
            .send_message(&a.id(), &at_a, "files and words")
            .await
            .unwrap();
        let chat = as_phase_7(&mut link, &mut peer, QUIET).await;
        assert_eq!(chat.len(), 1);
        for reference in &sent {
            until(|| a.status(reference) == Some(TransferStatus::Rejected)).await;
        }
        // Chat from the peer goes on, and a later session carries no
        // offer: the rejections are final.
        link.send(&Message::ChatMessage {
            id: MessageId::from_bytes([4; 16]),
            text: ChatText::new("old peer").unwrap(),
        })
        .await
        .unwrap();
        let (messages, _) = drain(&mut link, QUIET).await;
        assert_eq!(
            messages,
            vec![Message::MessageAck(MessageId::from_bytes([4; 16]))]
        );
        link.close().await.unwrap();
        let mut link = hand_link(&network, &a, &b).await;
        let mut peer = phase_7::Transfers::default();
        assert!(as_phase_7(&mut link, &mut peer, QUIET).await.is_empty());
        a.stop().await;
    });
}

/// The cards A dials for B, in order.
fn plan(a: &Peer, b: &Peer) -> Vec<monolith_protocol::card::ContactCard> {
    a.identity
        .dial_plan(b.identity.identity())
        .map(|plan| plan.cards)
        .unwrap_or_default()
}

#[test]
fn a_transfer_goes_on_over_a_session_of_the_new_key_of_its_contact() {
    run_paused(async {
        let network = MockNetwork::new();
        let mut a = Peer::new(1).await;
        let mut b = Peer::new(2).await;
        let at_a = a.add(&b).await;
        let at_b = b.add(&a).await;
        // B is changing its transport key; A learns the successor on the
        // first session.
        b.identity.begin_rotation().await.unwrap();
        a.start(&network).await;
        b.start(&network).await;
        a.runtime()
            .send_message(&a.id(), &at_a, "before")
            .await
            .unwrap();
        until(|| a.statuses(&at_a) == vec![MessageStatus::Delivered]).await;
        until(|| plan(&a, &b).len() == 2).await;
        let successor = plan(&a, &b)[0].clone();
        until(|| a.runtime().sessions(&a.id(), &at_a).unwrap() == 0).await;
        // A offers a file over a session of the old key.
        let mut events = b.runtime().subscribe();
        let data = file(5 * L);
        let sent = offer(&a, &at_a, &data).await;
        let received = offered(&mut events).await;
        until(|| b.runtime().sessions(&b.id(), &at_b).unwrap() == 0).await;
        // B switches to its new key, and only then accepts: the transfer of
        // the same contact record goes on over a session of the new key.
        let rotation = b.identity.rotation_id().unwrap();
        assert!(b.identity.switch_rotation(rotation, true).await.unwrap());
        b.runtime().accept_transfer(&received).await.unwrap();
        until(|| a.status(&sent) == Some(TransferStatus::Completed)).await;
        assert_eq!(saved(&b, &received).await, data);
        assert_eq!(received.contact, at_b);
        // The successor answered and is the active key of B now.
        assert_eq!(plan(&a, &b), vec![successor]);
        a.stop().await;
        b.stop().await;
    });
}

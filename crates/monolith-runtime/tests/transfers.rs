//! A file sent from one front end to another through the boundary alone
//! (`docs/adr/0013-file-transfer.md`, `docs/adr/0014-transfer-staging.md`):
//! offered by its path, accepted, sent in chunks, the receiver stopped and
//! started again in the middle, resumed from what it held durably,
//! verified, completed on both sides and saved where the user chose, with
//! the bytes of the file and nothing else.
//!
//! Two data directories on disk, the in-memory Tor, time paused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation
)]

use core::time::Duration;
use std::future::Future;
use std::path::{Path, PathBuf};

use monolith_protocol::limits::MAX_FILE_CHUNK_LEN;
use monolith_runtime::api::{
    ContactId, Event, IdentityId, MessageStatus, SaveError, SaveOutcome, ServiceState, TransferRef,
    TransferStatus,
};
use monolith_runtime::{Events, Messenger, save_name};
use monolith_tor::{MockNetwork, MockTorBackend};

const L: u64 = MAX_FILE_CHUNK_LEN as u64;
const PASSPHRASE: &str = "correct horse battery staple";

fn run_paused(test: impl Future<Output = ()>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap()
        .block_on(test);
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("monolith-runtime-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let resolved = std::fs::canonicalize(&dir).unwrap();
    let text = resolved.to_string_lossy().into_owned();
    text.strip_prefix(r"\\?\")
        .map_or(resolved.clone(), PathBuf::from)
}

fn file(len: u64) -> Vec<u8> {
    (0..len).map(|i| (i * 7 % 241) as u8).collect()
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

/// Waits for an event that `matches`.
async fn event(events: &mut Events, matches: impl Fn(&Event) -> bool) -> Event {
    let wait = async {
        loop {
            match events.next().await {
                Ok(event) if matches(&event) => return event,
                Ok(_) | Err(monolith_runtime::Missed::Lagged(_)) => {}
                Err(monolith_runtime::Missed::Closed) => panic!("no more events"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(24 * 60 * 60), wait)
        .await
        .expect("the event did not come")
}

fn status(messenger: &Messenger<MockTorBackend>, transfer: &TransferRef) -> Option<TransferStatus> {
    messenger
        .transfer(transfer)
        .unwrap()
        .map(|view| view.status)
}

/// A messenger with one identity whose service is published.
async fn node(
    network: &MockNetwork,
    dir: &Path,
    label: &str,
) -> (Messenger<MockTorBackend>, IdentityId) {
    let messenger = Messenger::create(dir, PASSPHRASE, network.backend()).unwrap();
    let identity = messenger.create_identity(Some(label)).await.unwrap();
    until(|| messenger.service_state(&identity) == Ok(ServiceState::Available)).await;
    (messenger, identity)
}

/// Opens the data directory at `dir` again, once the tasks of the
/// messenger that had it ended and let go of its lock: as a process that
/// starts again after one that stopped.
async fn reopen(network: &MockNetwork, dir: &Path) -> Messenger<MockTorBackend> {
    for _ in 0..600 {
        match Messenger::open(dir, PASSPHRASE, network.backend()) {
            Ok(messenger) => return messenger,
            Err(monolith_runtime::OpenError::Locked) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(error) => panic!("{error}"),
        }
    }
    panic!("the data directory stayed locked");
}

/// Makes `a` and `b` accepted contacts of each other: the records of each.
async fn contacts(
    a: &Messenger<MockTorBackend>,
    alice: &IdentityId,
    b: &Messenger<MockTorBackend>,
    bob: &IdentityId,
) -> (ContactId, ContactId) {
    let at_a = a
        .add_contact(alice, &b.own_card(bob).unwrap())
        .await
        .unwrap()
        .contact;
    let at_b = b
        .add_contact(bob, &a.own_card(alice).unwrap())
        .await
        .unwrap()
        .contact;
    a.send_message(alice, &at_a, "hello").await.unwrap();
    until(|| {
        a.messages(alice, &at_a, None, 10)
            .unwrap()
            .iter()
            .all(|message| message.status == MessageStatus::Delivered)
    })
    .await;
    (at_a, at_b)
}

#[test]
fn a_file_goes_from_one_front_end_to_another_across_a_restart_and_is_saved_whole() {
    run_paused(async {
        let network = MockNetwork::new();
        let dir = scratch("chain");
        let (a, alice) = node(&network, &dir.join("a"), "Alice").await;
        let (b, bob) = node(&network, &dir.join("b"), "Bob").await;
        let (at_a, _) = contacts(&a, &alice, &b, &bob).await;

        let data = file(40 * L + 5);
        let source = dir.join("report 2026.pdf");
        std::fs::write(&source, &data).unwrap();
        let mut events = b.events();
        let sent = a.offer_file(&alice, &at_a, &source).await.unwrap();
        let Event::TransferOffered(received) = event(&mut events, |event| {
            matches!(event, Event::TransferOffered(_))
        })
        .await
        else {
            panic!("not an offer");
        };
        let view = b.transfer(&received).unwrap().unwrap();
        assert_eq!(view.status, TransferStatus::Offered);
        assert_eq!(view.name, "report 2026.pdf");
        assert_eq!(view.size, data.len() as u64);
        b.accept_transfer(&received).await.unwrap();

        // Some of the file is durable at Bob, then Bob stops.
        let Event::TransferProgressed { bytes, .. } = event(&mut events, |event| {
            matches!(event, Event::TransferProgressed { reference, bytes } if *reference == received && *bytes > 0)
        })
        .await
        else {
            panic!("not progress");
        };
        b.shutdown(Duration::ZERO).await;
        drop(events);
        drop(b);
        assert_eq!(status(&a, &sent), Some(TransferStatus::Sending));

        // Bob starts again: the transfer goes on from what he held durably
        // once Alice reaches him, never from the start. What is staged is
        // known again once the staged file is read (`TransferView::bytes`).
        let b = reopen(&network, &dir.join("b")).await;
        let mut events = b.events();
        let view = b.transfer(&received).unwrap().unwrap();
        assert_eq!(view.status, TransferStatus::Receiving);
        let Event::TransferProgressed { bytes: resumed, .. } = event(&mut events, |event| {
            matches!(event, Event::TransferProgressed { reference, .. } if *reference == received)
        })
        .await
        else {
            panic!("not progress");
        };
        assert!(resumed > bytes, "{resumed} <= {bytes}: started again");
        until(|| status(&b, &received) == Some(TransferStatus::Received)).await;
        until(|| status(&a, &sent) == Some(TransferStatus::Completed)).await;

        // Saved where Bob chooses, under the name suggested to him.
        let saves = dir.join("saved");
        std::fs::create_dir(&saves).unwrap();
        let destination = saves.join(save_name(&view.name));
        assert_eq!(
            b.save_transfer(&received, &destination).await,
            Ok(SaveOutcome::Saved)
        );
        assert_eq!(std::fs::read(&destination).unwrap(), data);
        assert_eq!(std::fs::read_dir(&saves).unwrap().count(), 1);
        assert!(b.transfer(&received).unwrap().unwrap().saved);
        assert!(matches!(
            b.save_transfer(&received, &saves.join("again.pdf")).await,
            Err(SaveError::Transfer(_))
        ));
        a.shutdown(Duration::ZERO).await;
        b.shutdown(Duration::ZERO).await;
        let _ = std::fs::remove_dir_all(&dir);
    });
}

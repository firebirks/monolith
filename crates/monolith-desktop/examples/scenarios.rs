//! The workflows of the desktop client, A to G of the Phase 6 brief, run
//! by script: two clients, each with a profile on disk, the real bridge and
//! the real runtime, on the mock Tor network, in real time. The script does
//! what the screens do when the user acts (the actions of the UI model) and
//! waits for what the runtime says; nothing is answered in its place. A
//! picture of the window is written at each step:
//! `cargo run --release -p monolith-desktop --example scenarios -- <dir>`.
//!
//! It takes a few minutes: the runtime waits between delivery attempts as
//! it always does.

#![allow(
    clippy::print_stdout,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::unreachable,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use std::path::{Path, PathBuf};
use std::time::Instant;

use core::time::Duration;

use common::Client;
use monolith_tor::MockNetwork;
use monolith_ui_model::{Dialog, Phase, RowState, Status, TorIndicator};

const PATIENCE: Duration = Duration::from_secs(240);

fn start(network: &MockNetwork, name: &'static str, data: &Path, shots: &Path) -> Client {
    let network = network.clone();
    Client::start(
        name,
        data,
        move || network.backend(),
        common::in_memory_tor(),
        PATIENCE,
    )
    .with_pictures(shots)
}

fn scratch(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let _ = std::fs::remove_dir_all(&path);
    path
}

fn main() {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "scenarios".to_owned()),
    );
    std::fs::create_dir_all(&out).expect("output folder");
    let data = std::env::temp_dir().join(format!("monolith-scenarios-{}", std::process::id()));
    let data_a = scratch(&data, "a");
    let data_b = scratch(&data, "b");
    let network = MockNetwork::new();
    let begun = Instant::now();

    println!("A. First launch, an identity, an invitation");
    let mut alice = start(&network, "alice", &data_a, &out);
    alice.until("the first start is shown", |model| {
        model.phase() == Phase::Welcome
    });
    alice.picture("first-start");
    alice.open_profile();
    alice.picture("no-identity");
    let personal = alice.create_identity("Personal");
    let invitation = alice.create_invitation("For Bob");
    alice.picture("invitation-made");
    alice.model().close();

    println!("B. Two clients become contacts and talk");
    let mut bob = start(&network, "bob", &data_b, &out);
    bob.open_profile();
    bob.create_identity("Bob");
    let at_bob = bob.add_contact(&invitation, "Alice");
    bob.picture("contact-added");
    bob.send(at_bob, "Hello Alice, it is Bob.");
    alice.until("Bob's request arrives", |model| {
        model
            .current_slice()
            .is_some_and(|slice| !slice.requests.is_empty())
    });
    alice.model().show_requests();
    alice.picture("request");
    let request = alice.app.model.current_slice().unwrap().requests[0].id;
    let at_alice = alice.accept_request(request, "Bob");
    assert_eq!(at_alice.identity(), personal);
    bob.wait_status(at_bob, "Hello Alice, it is Bob.", Status::Delivered);
    alice.wait_status(at_alice, "Hello Alice, it is Bob.", Status::Received);
    alice.send(at_alice, "Hi Bob, welcome.");
    alice.wait_status(at_alice, "Hi Bob, welcome.", Status::Delivered);
    bob.wait_status(at_bob, "Hi Bob, welcome.", Status::Received);
    alice.picture("conversation");
    bob.picture("conversation");

    println!("C. Bob is offline: messages wait, then are delivered");
    bob.close();
    alice.send(at_alice, "Are you there?");
    alice.send(at_alice, "I will wait.");
    assert_eq!(
        alice.status_of(at_alice, "Are you there?"),
        Some(Status::Queued)
    );
    alice.until("an attempt to deliver failed", |model| {
        model
            .current_slice()
            .is_some_and(|slice| slice.deferrals.contains_key(&at_alice))
    });
    alice.picture("queued-while-offline");
    let mut bob = start(&network, "bob", &data_b, &out);
    bob.open_profile();
    alice.wait_status(at_alice, "Are you there?", Status::Delivered);
    alice.wait_status(at_alice, "I will wait.", Status::Delivered);
    alice.picture("delivered-after-return");

    println!("D. Tor stops, the window stays usable, Tor comes back");
    network.set_socks_available(false);
    network.set_control_available(false);
    alice.model().check_tor();
    alice.until("Tor is shown unavailable", |model| {
        model.tor_indicator() == TorIndicator::Unavailable
    });
    alice.send(at_alice, "Sent while Tor was down.");
    assert_eq!(
        alice.status_of(at_alice, "Sent while Tor was down."),
        Some(Status::Queued)
    );
    alice.picture("tor-unavailable");
    network.set_control_available(true);
    network.set_socks_available(true);
    alice.model().check_tor();
    alice.until("Tor is shown working again", |model| {
        model.tor_indicator() != TorIndicator::Unavailable
    });
    alice.wait_status(at_alice, "Sent while Tor was down.", Status::Delivered);
    alice.picture("tor-back");

    println!("E. Two identities, switched while messages arrive");
    let work = alice.create_identity("Work");
    bob.open_conversation(at_bob);
    for round in 0..6 {
        let text = format!("Message {round} while Alice switches");
        bob.send(at_bob, &text);
        alice
            .model()
            .select_identity(if round % 2 == 0 { personal } else { work });
        alice.frame();
        if alice.app.model.current() == Some(work) {
            let slice = alice.app.model.current_slice().unwrap();
            assert!(
                slice.contacts.iter().all(|view| view.id.identity() == work),
                "Work shows a contact of Personal"
            );
            assert_eq!(alice.app.model.selection(), None);
        }
    }
    alice.model().select_identity(work);
    alice.picture("work-identity");
    alice.model().select_identity(personal);
    alice.model().select_conversation(at_alice);
    alice.wait_status(at_alice, "Message 5 while Alice switches", Status::Received);
    for round in 0..6 {
        assert_eq!(
            alice.status_of(at_alice, &format!("Message {round} while Alice switches")),
            Some(Status::Received)
        );
    }
    alice.picture("personal-after-switching");

    println!("F. Blocking and deleting while the conversation is open");
    alice.model().open(Dialog::BlockContact {
        contact: at_alice,
        busy: None,
        error: None,
    });
    alice.picture("confirm-block");
    alice.model().confirm();
    alice.until("the conversation is ended", |model| {
        model
            .sidebar()
            .iter()
            .any(|row| row.contact == at_alice && row.state == RowState::Ended)
    });
    assert!(!alice.app.model.can_send(at_alice));
    alice.picture("blocked");
    alice.model().open(Dialog::DeleteConversation {
        contact: at_alice,
        busy: None,
        error: None,
    });
    alice.model().confirm();
    alice.until("the conversation is gone", |model| {
        model.dialog().is_none() && model.sidebar().iter().all(|row| row.contact != at_alice)
    });
    alice.picture("deleted");

    println!("G. Alice's client starts again and shows what was kept");
    let identities: Vec<_> = alice
        .app
        .model
        .identities()
        .iter()
        .map(|view| (view.id, view.name.clone()))
        .collect();
    alice.close();
    let mut alice = start(&network, "alice", &data_a, &out);
    alice.open_profile();
    let again: Vec<_> = alice
        .app
        .model
        .identities()
        .iter()
        .map(|view| (view.id, view.name.clone()))
        .collect();
    assert_eq!(again, identities);
    alice.model().select_identity(personal);
    alice.until("Personal is read", |model| {
        model.current_slice().is_some_and(|slice| slice.loaded)
    });
    assert!(
        alice
            .app
            .model
            .sidebar()
            .iter()
            .all(|row| row.contact != at_alice),
        "the deleted conversation is back"
    );
    alice.picture("after-restart");

    alice.close();
    bob.close();
    let _ = std::fs::remove_dir_all(&data);
    println!(
        "all scenarios passed in {:.0} s; pictures in {}",
        begun.elapsed().as_secs_f64(),
        out.display()
    );
}

//! File transfers in the UI model (`docs/adr/0013-file-transfer.md`, the
//! Phase 8 brief, sections 32 to 36): each card says where its transfer
//! stands from the runtime's state alone; a file being prepared is never
//! shown as sent; progress not known after a restart is never shown as 0;
//! a file goes only to an accepted contact; events and answers of one
//! identity change nothing shown of another, and a command acts on the
//! transfer it named.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use monolith_ui_model::{
    Command, ContactView, Dialog, Failure, IdentitySnapshot, IdentityView, Model, NoticeKind,
    Outcome, Phase, ProfileOnDisk, RequestMode, RuntimeEvent, Selection, Service, TransferItem,
    TransferKey, TransferPhase, TransferReason, TransferStatus, TransferView, UiContactId,
    UiIdentityId, UiTransferId, Update,
};
use zeroize::Zeroizing;

fn identity(n: u8) -> UiIdentityId {
    UiIdentityId::from_bytes([n; 16])
}

fn contact(identity_n: u8, record: u8) -> UiContactId {
    UiContactId::new(identity(identity_n), [record; 16])
}

fn transfer(contact: UiContactId, handle: u64) -> UiTransferId {
    UiTransferId::new(contact, handle)
}

fn identity_view(n: u8, name: &str) -> IdentityView {
    IdentityView {
        id: identity(n),
        name: Some(name.to_owned()),
        fingerprint: format!("ID{n:02} AAAA BBBB"),
        full_fingerprint: String::new(),
    }
}

fn contact_view(id: UiContactId, name: &str, accepted: bool) -> ContactView {
    ContactView {
        id,
        name: Some(name.to_owned()),
        fingerprint: "CCCC 1111".to_owned(),
        full_fingerprint: String::new(),
        accepted,
        verified: false,
        invitation: false,
    }
}

fn view(id: UiTransferId, incoming: bool, status: TransferStatus) -> TransferView {
    TransferView {
        id,
        incoming,
        status,
        reason: None,
        size: 64_000_000,
        name: "project-backup.zip".to_owned(),
        bytes: Some(0),
        created: 1_791_981_000,
        held: false,
        saved: false,
    }
}

/// What the runtime holds, as the fake bridge answers it.
#[derive(Default)]
struct World {
    contacts: BTreeMap<UiIdentityId, Vec<ContactView>>,
    transfers: BTreeMap<UiIdentityId, Vec<TransferView>>,
    interrupted: BTreeMap<UiIdentityId, Vec<UiTransferId>>,
}

impl World {
    fn snapshot(&self, owner: UiIdentityId) -> IdentitySnapshot {
        IdentitySnapshot {
            identity: owner,
            contacts: self.contacts.get(&owner).cloned().unwrap_or_default(),
            conversations: Vec::new(),
            requests: Vec::new(),
            invitations: Vec::new(),
            blocked: Vec::new(),
            service: Service::Available,
            request_mode: RequestMode::Invitation,
            transfers: self.transfers.get(&owner).cloned().unwrap_or_default(),
            interrupted: self.interrupted.get(&owner).cloned().unwrap_or_default(),
        }
    }

    /// Answers the reads the model asks for, until it asks for none, and
    /// returns every other command it sent, unanswered.
    fn settle(&self, model: &mut Model) -> Vec<(monolith_ui_model::CommandId, Command)> {
        let mut others = Vec::new();
        for _ in 0..4 {
            for (id, command) in model.take_commands() {
                let outcome = match &command {
                    Command::LoadIdentities => Some(Outcome::Identities(vec![
                        identity_view(1, "Alice"),
                        identity_view(2, "Work"),
                    ])),
                    Command::LoadIdentity(owner) => {
                        Some(Outcome::Identity(Box::new(self.snapshot(*owner))))
                    }
                    Command::CheckTor => Some(Outcome::Tor(monolith_ui_model::TorCheck::Ready {
                        version: "0.4.9.13".to_owned(),
                        outdated: false,
                    })),
                    Command::LoadMessages { contact, .. } => Some(Outcome::Messages {
                        contact: *contact,
                        messages: Vec::new(),
                        complete: true,
                    }),
                    _ => None,
                };
                match outcome {
                    Some(outcome) => model.apply(Update::Done {
                        command: id,
                        outcome,
                    }),
                    None => others.push((id, command)),
                }
            }
        }
        others
    }
}

/// A world where A (1) has Carol (1/10, accepted) and Dan (1/11, not
/// accepted yet), and B (2) has Erin (2/20).
fn world() -> World {
    let mut world = World::default();
    world.contacts.insert(
        identity(1),
        vec![
            contact_view(contact(1, 10), "Carol", true),
            contact_view(contact(1, 11), "Dan", false),
        ],
    );
    world.contacts.insert(
        identity(2),
        vec![contact_view(contact(2, 20), "Erin", true)],
    );
    world
}

/// A model with a profile open, A shown, and the conversation with Carol.
fn ready(world: &World) -> Model {
    let mut model = Model::new();
    model.apply(Update::Started {
        profile: ProfileOnDisk::Exists,
        location: "/home/user/.local/share/monolith".to_owned(),
    });
    model.unlock(Zeroizing::new("passphrase".to_owned()));
    for (id, command) in model.take_commands() {
        if matches!(command, Command::OpenProfile { .. }) {
            model.apply(Update::Done {
                command: id,
                outcome: Outcome::ProfileOpen {
                    ephemeral: false,
                    message_store_missing: false,
                },
            });
        }
    }
    assert_eq!(model.phase(), Phase::Ready);
    world.settle(&mut model);
    model.select_identity(identity(1));
    world.settle(&mut model);
    model.select_conversation(contact(1, 10));
    world.settle(&mut model);
    model.take_notices();
    model
}

fn phases(model: &Model, contact: UiContactId) -> Vec<TransferPhase> {
    model
        .transfers_of(contact)
        .iter()
        .map(|item: &TransferItem| item.phase)
        .collect()
}

#[test]
fn each_card_says_where_its_transfer_stands_from_the_runtime_alone() {
    let mut world = world();
    let carol = contact(1, 10);
    let mut listed = Vec::new();
    let mut cases = Vec::new();
    let mut add = |status: TransferStatus,
                   incoming: bool,
                   change: &dyn Fn(&mut TransferView),
                   phase: TransferPhase| {
        let mut made = view(transfer(carol, listed.len() as u64), incoming, status);
        change(&mut made);
        listed.push(made);
        cases.push(phase);
    };
    add(
        TransferStatus::Waiting,
        false,
        &|_| {},
        TransferPhase::Waiting,
    );
    add(
        TransferStatus::Sending,
        false,
        &|view| view.bytes = Some(41_000_000),
        TransferPhase::Transferring { bytes: 41_000_000 },
    );
    // Not known since a restart: never 0, and no percentage.
    add(
        TransferStatus::Sending,
        false,
        &|view| view.bytes = None,
        TransferPhase::Restoring,
    );
    add(
        TransferStatus::Receiving,
        true,
        &|view| view.bytes = None,
        TransferPhase::Restoring,
    );
    add(
        TransferStatus::Receiving,
        true,
        &|view| view.held = true,
        TransferPhase::Paused,
    );
    add(
        TransferStatus::Completed,
        false,
        &|_| {},
        TransferPhase::Completed,
    );
    add(
        TransferStatus::Offered,
        true,
        &|_| {},
        TransferPhase::Offered,
    );
    add(
        TransferStatus::Received,
        true,
        &|_| {},
        TransferPhase::Received,
    );
    add(
        TransferStatus::Received,
        true,
        &|view| view.saved = true,
        TransferPhase::Saved,
    );
    add(
        TransferStatus::Rejected,
        false,
        &|_| {},
        TransferPhase::Rejected,
    );
    add(
        TransferStatus::Expired,
        true,
        &|_| {},
        TransferPhase::Expired,
    );
    add(
        TransferStatus::Cancelled,
        false,
        &|view| view.reason = Some(TransferReason::ByUs),
        TransferPhase::Cancelled { by_us: true },
    );
    add(
        TransferStatus::Cancelled,
        true,
        &|view| view.reason = Some(TransferReason::ByPeer),
        TransferPhase::Cancelled { by_us: false },
    );
    add(
        TransferStatus::Failed,
        true,
        &|view| view.reason = Some(TransferReason::Digest),
        TransferPhase::Failed(Some(TransferReason::Digest)),
    );
    world.transfers.insert(identity(1), listed);
    let model = ready(&world);
    assert_eq!(phases(&model, carol), cases);
    // Only waiting and moving transfers can be cancelled, only ended ones
    // taken off the list.
    assert!(TransferPhase::Restoring.cancellable());
    assert!(!TransferPhase::Received.cancellable());
    assert!(!TransferPhase::Preparing.cancellable());
    assert!(TransferPhase::Failed(None).ended());
    assert!(!TransferPhase::Received.ended());
}

#[test]
fn a_file_being_prepared_is_never_shown_as_offered_or_sent() {
    let mut world = world();
    let carol = contact(1, 10);
    let mut model = ready(&world);
    model.open_send_file(carol, Some("/home/me/project-backup.zip".to_owned()));
    assert!(matches!(
        model.dialog(),
        Some(Dialog::SendFile { error: None, .. })
    ));
    model.confirm();
    // The dialog closes; the runtime is asked with the path as typed.
    assert!(model.dialog().is_none());
    let others = world.settle(&mut model);
    let [(offer, Command::OfferFile { contact: to, path })] = others.as_slice() else {
        panic!("not one offer: {others:?}");
    };
    assert_eq!(*to, carol);
    assert_eq!(*path, PathBuf::from("/home/me/project-backup.zip"));
    let items = model.transfers_of(carol);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].phase, TransferPhase::Preparing);
    assert_eq!(items[0].name, "project-backup.zip");
    assert_eq!(items[0].size, None);
    // Staged and offered: shown as waiting once the runtime lists it.
    let made = transfer(carol, 1);
    world.transfers.insert(
        identity(1),
        vec![view(made, false, TransferStatus::Waiting)],
    );
    model.apply(Update::Done {
        command: *offer,
        outcome: Outcome::TransferOffered(made),
    });
    world.settle(&mut model);
    assert_eq!(phases(&model, carol), vec![TransferPhase::Waiting]);
}

#[test]
fn a_file_that_could_not_be_offered_says_why_until_it_is_dismissed() {
    let world = world();
    let carol = contact(1, 10);
    let mut model = ready(&world);
    model.open_send_file(carol, Some("/home/me/huge.iso".to_owned()));
    model.confirm();
    let others = world.settle(&mut model);
    let (offer, _) = others[0];
    model.apply(Update::Done {
        command: offer,
        outcome: Outcome::Failed(Failure::FileTooLargeToSend),
    });
    world.settle(&mut model);
    let items = model.transfers_of(carol);
    assert_eq!(
        items.iter().map(|item| item.phase).collect::<Vec<_>>(),
        vec![TransferPhase::Unsent(Failure::FileTooLargeToSend)]
    );
    assert_eq!(items[0].name, "huge.iso");
    let TransferKey::Unsent(number) = items[0].key else {
        panic!("not unsent");
    };
    model.dismiss_unsent(carol, number);
    assert!(model.transfers_of(carol).is_empty());
}

#[test]
fn a_contact_that_has_not_accepted_gets_no_file_and_the_dialog_says_why() {
    let world = world();
    let dan = contact(1, 11);
    let mut model = ready(&world);
    assert!(model.can_offer(contact(1, 10)));
    assert!(!model.can_offer(dan));
    model.open_send_file(dan, Some("/home/me/a.txt".to_owned()));
    assert!(matches!(
        model.dialog(),
        Some(Dialog::SendFile {
            error: Some(Failure::NotAcceptedForFiles),
            ..
        })
    ));
    model.confirm();
    assert!(
        world
            .settle(&mut model)
            .iter()
            .all(|(_, command)| !matches!(command, Command::OfferFile { .. }))
    );
}

#[test]
fn a_dropped_file_and_a_typed_path_make_the_same_command() {
    let world = world();
    let carol = contact(1, 10);
    let mut dropped = ready(&world);
    dropped.open_send_file(carol, Some("/home/me/photo.jpg".to_owned()));
    dropped.confirm();
    let mut typed = ready(&world);
    typed.open_send_file(carol, None);
    if let Some(Dialog::SendFile { path, .. }) = typed.dialog_mut() {
        path.push_str("  /home/me/photo.jpg ");
    }
    typed.confirm();
    let a = world.settle(&mut dropped);
    let b = world.settle(&mut typed);
    assert_eq!(
        a.into_iter()
            .map(|(_, command)| command)
            .collect::<Vec<_>>(),
        b.into_iter()
            .map(|(_, command)| command)
            .collect::<Vec<_>>()
    );
}

#[test]
fn progress_is_kept_from_events_until_a_later_read_and_never_shown_as_zero() {
    let mut world = world();
    let carol = contact(1, 10);
    let moving = transfer(carol, 1);
    let mut sending = view(moving, false, TransferStatus::Sending);
    sending.bytes = None;
    world.transfers.insert(identity(1), vec![sending.clone()]);
    let mut model = ready(&world);
    assert_eq!(phases(&model, carol), vec![TransferPhase::Restoring]);
    // The runtime knows again: a progress event says so at once.
    model.apply(Update::Event(RuntimeEvent::TransferProgressed {
        transfer: moving,
        bytes: 128_980,
    }));
    assert_eq!(
        phases(&model, carol),
        vec![TransferPhase::Transferring { bytes: 128_980 }]
    );
    // Progress asks for no read of its own.
    assert!(world.settle(&mut model).is_empty());
    // A read asked before a later event does not take it back.
    model.apply(Update::Event(RuntimeEvent::TransferChanged(moving)));
    let commands = model.take_commands();
    model.apply(Update::Event(RuntimeEvent::TransferProgressed {
        transfer: moving,
        bytes: 257_960,
    }));
    sending.bytes = Some(128_980);
    for (id, command) in commands {
        if let Command::LoadIdentity(owner) = command {
            world.transfers.insert(identity(1), vec![sending.clone()]);
            model.apply(Update::Done {
                command: id,
                outcome: Outcome::Identity(Box::new(world.snapshot(owner))),
            });
        }
    }
    assert_eq!(
        phases(&model, carol),
        vec![TransferPhase::Transferring { bytes: 257_960 }]
    );
    // A read asked after it answers for it.
    sending.bytes = Some(386_940);
    world.transfers.insert(identity(1), vec![sending]);
    model.apply(Update::Event(RuntimeEvent::TransferChanged(moving)));
    world.settle(&mut model);
    assert_eq!(
        phases(&model, carol),
        vec![TransferPhase::Transferring { bytes: 386_940 }]
    );
}

#[test]
fn late_events_and_answers_of_one_identity_change_nothing_shown_of_another() {
    let mut world = world();
    let carol = contact(1, 10);
    let erin = contact(2, 20);
    let mut model = ready(&world);
    // A file offered to Carol is being prepared; the user goes to B.
    model.open_send_file(carol, Some("/home/me/a.txt".to_owned()));
    model.confirm();
    let others = world.settle(&mut model);
    let (offer, _) = others[0];
    model.select_identity(identity(2));
    world.settle(&mut model);
    model.select_conversation(erin);
    world.settle(&mut model);
    model.take_notices();
    // Late: A's offer fails, A gets an offer, A's transfer moves.
    model.apply(Update::Done {
        command: offer,
        outcome: Outcome::Failed(Failure::FileNotSendable),
    });
    let from_carol = transfer(carol, 7);
    world.transfers.insert(
        identity(1),
        vec![view(from_carol, true, TransferStatus::Offered)],
    );
    model.apply(Update::Event(RuntimeEvent::TransferOffered(from_carol)));
    model.apply(Update::Event(RuntimeEvent::TransferProgressed {
        transfer: from_carol,
        bytes: 99,
    }));
    let asked = world.settle(&mut model);
    assert!(asked.is_empty(), "{asked:?}");
    // Nothing of B changed, and nothing was said while B is shown.
    assert!(model.transfers_of(erin).is_empty());
    assert!(model.take_notices().is_empty());
    assert_eq!(model.current(), Some(identity(2)));
    assert_eq!(model.selection(), Some(Selection::Conversation(erin)));
    // A holds it all, read again.
    let a = model.transfers_of(carol);
    assert_eq!(
        a.iter().map(|item| item.phase).collect::<Vec<_>>(),
        vec![
            TransferPhase::Offered,
            TransferPhase::Unsent(Failure::FileNotSendable)
        ]
    );
}

#[test]
fn an_offer_is_told_only_for_the_identity_shown_and_not_in_its_own_conversation() {
    let mut world = world();
    let carol = contact(1, 10);
    let dan = contact(1, 11);
    let mut model = ready(&world);
    let offered = transfer(carol, 3);
    world.transfers.insert(
        identity(1),
        vec![view(offered, true, TransferStatus::Offered)],
    );
    // Carol's conversation is shown: its card says it.
    model.apply(Update::Event(RuntimeEvent::TransferOffered(offered)));
    world.settle(&mut model);
    assert!(model.take_notices().is_empty());
    // Another conversation of the same identity: a notice names Carol.
    model.select_conversation(dan);
    world.settle(&mut model);
    model.apply(Update::Event(RuntimeEvent::TransferOffered(offered)));
    let notices = model.take_notices();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].kind, NoticeKind::Info);
    assert_eq!(notices[0].text.plain(), "Carol offers you a file.");
}

#[test]
fn a_decision_names_its_transfer_and_one_no_longer_listed_asks_for_nothing() {
    let mut world = world();
    let carol = contact(1, 10);
    let offered = transfer(carol, 3);
    world.transfers.insert(
        identity(1),
        vec![view(offered, true, TransferStatus::Offered)],
    );
    let mut model = ready(&world);
    model.accept_transfer(offered);
    // Twice is once while it runs.
    model.accept_transfer(offered);
    let others = world.settle(&mut model);
    assert_eq!(
        others
            .iter()
            .map(|(_, command)| command.clone())
            .collect::<Vec<_>>(),
        vec![Command::AcceptTransfer(offered)]
    );
    assert!(model.transfers_of(carol)[0].busy);
    // The contact was removed meanwhile: the runtime refuses, and says so.
    model.apply(Update::Done {
        command: others[0].0,
        outcome: Outcome::Failed(Failure::NoSuchTransfer),
    });
    let notices = model.take_notices();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].text.plain(), Failure::NoSuchTransfer.message());
    // Once a read no longer lists it, nothing is asked for it at all: a
    // contact made again never gets the old handle's decision.
    world.transfers.insert(identity(1), Vec::new());
    model.apply(Update::Event(RuntimeEvent::TransferChanged(offered)));
    world.settle(&mut model);
    model.accept_transfer(offered);
    model.reject_transfer(offered);
    model.open_save(offered);
    assert!(world.settle(&mut model).is_empty());
    assert!(model.dialog().is_none());
}

#[test]
fn a_save_starts_from_the_suggested_place_and_keeps_what_the_user_typed() {
    let mut world = world();
    let carol = contact(1, 10);
    let received = transfer(carol, 4);
    world.transfers.insert(
        identity(1),
        vec![view(received, true, TransferStatus::Received)],
    );
    let mut model = ready(&world);
    model.open_save(received);
    let others = world.settle(&mut model);
    let [(place, Command::SavePlace(asked))] = others.as_slice() else {
        panic!("not one place: {others:?}");
    };
    assert_eq!(*asked, received);
    model.apply(Update::Done {
        command: *place,
        outcome: Outcome::SavePlace {
            transfer: received,
            place: PathBuf::from("/home/me/Downloads/project-backup.zip"),
        },
    });
    let Some(Dialog::SaveFile { place, .. }) = model.dialog() else {
        panic!("no save dialog");
    };
    assert_eq!(place, "/home/me/Downloads/project-backup.zip");
    // A name that exists: the dialog says so and stays.
    model.confirm();
    let others = world.settle(&mut model);
    let [
        (
            save,
            Command::SaveTransfer {
                transfer: on,
                destination,
            },
        ),
    ] = others.as_slice()
    else {
        panic!("not one save: {others:?}");
    };
    assert_eq!(*on, received);
    assert_eq!(
        *destination,
        PathBuf::from("/home/me/Downloads/project-backup.zip")
    );
    assert!(model.transfer_busy(received));
    model.apply(Update::Done {
        command: *save,
        outcome: Outcome::Failed(Failure::FileExists),
    });
    assert!(matches!(
        model.dialog(),
        Some(Dialog::SaveFile {
            error: Some(Failure::FileExists),
            busy: None,
            ..
        })
    ));
    // Another name: saved, the dialog closes and says so.
    if let Some(Dialog::SaveFile { place, .. }) = model.dialog_mut() {
        *place = "/home/me/Downloads/backup-2.zip".to_owned();
    }
    model.confirm();
    let others = world.settle(&mut model);
    model.apply(Update::Done {
        command: others[0].0,
        outcome: Outcome::Saved {
            transfer: received,
            confirmed: true,
        },
    });
    assert!(model.dialog().is_none());
    let notices = model.take_notices();
    assert_eq!(notices[0].kind, NoticeKind::Success);
    assert_eq!(notices[0].text.plain(), "File saved.");
}

#[test]
fn a_suggested_place_that_comes_after_the_user_typed_changes_nothing() {
    let mut world = world();
    let carol = contact(1, 10);
    let received = transfer(carol, 4);
    world.transfers.insert(
        identity(1),
        vec![view(received, true, TransferStatus::Received)],
    );
    let mut model = ready(&world);
    model.open_save(received);
    let others = world.settle(&mut model);
    if let Some(Dialog::SaveFile { place, .. }) = model.dialog_mut() {
        place.push_str("/tmp/mine.zip");
    }
    model.apply(Update::Done {
        command: others[0].0,
        outcome: Outcome::SavePlace {
            transfer: received,
            place: PathBuf::from("/home/me/Downloads/project-backup.zip"),
        },
    });
    let Some(Dialog::SaveFile { place, .. }) = model.dialog() else {
        panic!("no save dialog");
    };
    assert_eq!(place, "/tmp/mine.zip");
}

#[test]
fn an_interrupted_save_is_told_once_for_the_identity_shown() {
    let mut world = world();
    let carol = contact(1, 10);
    let erin = contact(2, 20);
    let cut = transfer(erin, 9);
    world
        .transfers
        .insert(identity(2), vec![view(cut, true, TransferStatus::Received)]);
    world.interrupted.insert(identity(2), vec![cut]);
    let mut model = ready(&world);
    // Read while A is shown: nothing said yet.
    model.apply(Update::Event(RuntimeEvent::SaveInterrupted(cut)));
    world.settle(&mut model);
    assert!(model.take_notices().is_empty());
    model.select_identity(identity(2));
    world.settle(&mut model);
    let notices = model.take_notices();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].text.plain().contains("monolith-partial-"));
    // Once.
    model.select_identity(identity(1));
    model.select_identity(identity(2));
    model.apply(Update::Event(RuntimeEvent::TransferChanged(cut)));
    world.settle(&mut model);
    assert!(model.take_notices().is_empty());
    assert!(model.transfers_of(carol).is_empty());
}

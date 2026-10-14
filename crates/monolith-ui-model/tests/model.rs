//! The UI model against the stale-instance and identity-mixing scenarios
//! of the Phase 6 brief (section 42), and the rules of
//! `docs/adr/0011-ui-security-boundary.md`: nothing late, stale or of
//! another identity changes what is shown of another, a dialog acts on
//! what it captured, and a draft goes only once the runtime queued it.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use monolith_ui_model::{
    Command, CommandId, ContactView, ConversationView, Dialog, Failure, IdentitySnapshot,
    IdentityView, InvitationView, MessageView, Model, Outcome, Phase, ProfileOnDisk, RequestMode,
    RequestView, RuntimeEvent, Selection, Service, Status, UiContactId, UiIdentityId,
    UiInvitationId, UiMessageId, UiRequestId, Update,
};
use zeroize::Zeroizing;

fn identity(n: u8) -> UiIdentityId {
    UiIdentityId::from_bytes([n; 16])
}

fn contact(identity_n: u8, record: u8) -> UiContactId {
    UiContactId::new(identity(identity_n), [record; 16])
}

fn identity_view(n: u8, name: &str) -> IdentityView {
    IdentityView {
        id: identity(n),
        name: Some(name.to_owned()),
        fingerprint: format!("ID{n:02} AAAA BBBB"),
        full_fingerprint: String::new(),
    }
}

fn contact_view(id: UiContactId, name: &str, fingerprint: &str) -> ContactView {
    ContactView {
        id,
        name: Some(name.to_owned()),
        fingerprint: fingerprint.to_owned(),
        full_fingerprint: String::new(),
        accepted: true,
        verified: false,
    }
}

fn message(contact: UiContactId, sequence: u64, status: Status, text: &str) -> MessageView {
    MessageView {
        id: UiMessageId::new(contact, sequence),
        status,
        time: 1_791_981_000 + sequence,
        text: text.to_owned(),
    }
}

fn snapshot(identity: UiIdentityId, contacts: Vec<ContactView>) -> IdentitySnapshot {
    IdentitySnapshot {
        identity,
        contacts,
        conversations: Vec::new(),
        requests: Vec::new(),
        invitations: Vec::new(),
        service: Service::Available,
        request_mode: RequestMode::Invitation,
    }
}

/// Answers every pending command with what `answer` gives, and returns
/// the commands that were answered.
fn answer_all(
    model: &mut Model,
    mut answer: impl FnMut(&Command) -> Option<Outcome>,
) -> Vec<Command> {
    let commands = model.take_commands();
    let mut seen = Vec::new();
    for (id, command) in commands {
        if let Some(outcome) = answer(&command) {
            model.apply(Update::Done {
                command: id,
                outcome,
            });
        }
        seen.push(command);
    }
    seen
}

/// A model with a profile open and two identities, A (1) and B (2): A has
/// the contacts C (1/10) and D (1/11), B has E (2/20).
fn ready() -> Model {
    let mut model = Model::new();
    model.apply(Update::Started {
        profile: ProfileOnDisk::Exists,
        location: "/home/user/.local/share/monolith".to_owned(),
    });
    assert_eq!(model.phase(), Phase::Locked);
    model.unlock(Zeroizing::new("passphrase".to_owned()));
    answer_all(&mut model, |command| match command {
        Command::OpenProfile { .. } => Some(Outcome::ProfileOpen {
            ephemeral: false,
            message_store_missing: false,
        }),
        _ => None,
    });
    assert_eq!(model.phase(), Phase::Ready);
    load(&mut model);
    model
}

fn load(model: &mut Model) {
    for _ in 0..3 {
        answer_all(model, |command| match command {
            Command::LoadIdentities => Some(Outcome::Identities(vec![
                identity_view(1, "Alice"),
                identity_view(2, "Work"),
            ])),
            Command::LoadIdentity(id) if *id == identity(1) => {
                Some(Outcome::Identity(Box::new(snapshot(
                    identity(1),
                    vec![
                        contact_view(contact(1, 10), "Carol", "CCCC 1111"),
                        contact_view(contact(1, 11), "Dan", "DDDD 2222"),
                    ],
                ))))
            }
            Command::LoadIdentity(id) if *id == identity(2) => {
                Some(Outcome::Identity(Box::new(snapshot(
                    identity(2),
                    vec![contact_view(contact(2, 20), "Erin", "EEEE 3333")],
                ))))
            }
            Command::CheckTor => Some(Outcome::Tor(monolith_ui_model::TorCheck::Ready {
                version: "0.4.9.13".to_owned(),
                outdated: false,
            })),
            _ => None,
        });
    }
}

fn messages_of(model: &mut Model, contact: UiContactId, list: Vec<MessageView>) {
    answer_all(model, |command| match command {
        Command::LoadMessages { contact: asked, .. } if *asked == contact => {
            Some(Outcome::Messages {
                contact,
                messages: list.clone(),
                complete: true,
            })
        }
        _ => None,
    });
}

#[test]
fn a_late_event_of_one_identity_changes_nothing_shown_of_another() {
    let mut model = ready();
    model.select_identity(identity(1));
    model.select_conversation(contact(1, 10));
    messages_of(
        &mut model,
        contact(1, 10),
        vec![message(contact(1, 10), 1, Status::Received, "hi")],
    );
    // B is shown; then an event of A arrives.
    model.select_identity(identity(2));
    model.select_conversation(contact(2, 20));
    messages_of(&mut model, contact(2, 20), Vec::new());
    assert!(model.take_commands().is_empty());
    let before: Vec<_> = model.sidebar();
    model.apply(Update::Event(RuntimeEvent::MessageReceived(
        UiMessageId::new(contact(1, 10), 2),
    )));
    assert_eq!(model.sidebar(), before);
    assert_eq!(
        model.selection(),
        Some(Selection::Conversation(contact(2, 20)))
    );
    // A's unread count is A's own; what is asked again is A's only.
    assert_eq!(model.slice(identity(1)).unwrap().unread[&contact(1, 10)], 1);
    let commands = model.take_commands();
    assert!(!commands.is_empty());
    for (_, command) in &commands {
        match command {
            Command::LoadIdentity(id) => assert_eq!(*id, identity(1)),
            Command::LoadMessages { contact: asked, .. } => {
                assert_eq!(asked.identity(), identity(1));
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[test]
fn a_contact_made_again_inherits_nothing_of_the_old_record() {
    let mut model = ready();
    model.select_identity(identity(1));
    let old = contact(1, 10);
    model.select_conversation(old);
    messages_of(
        &mut model,
        old,
        vec![message(old, 5, Status::Queued, "hello")],
    );
    model.open(Dialog::RemoveContact {
        contact: old,
        busy: None,
        error: None,
    });
    model.confirm();
    let commands = answer_all(&mut model, |command| match command {
        Command::RemoveContact(asked) => {
            assert_eq!(*asked, old);
            Some(Outcome::Done)
        }
        _ => None,
    });
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::RemoveContact(_)))
    );
    // The identity is read again: the same person has a new record now.
    let new = contact(1, 12);
    answer_all(&mut model, |command| match command {
        Command::LoadIdentity(id) if *id == identity(1) => Some(Outcome::Identity(Box::new(
            snapshot(identity(1), vec![contact_view(new, "Carol", "CCCC 1111")]),
        ))),
        _ => None,
    });
    assert_eq!(model.selection(), None);
    assert!(model.thread(old).is_none());
    // A late event of the old record changes nothing of the new one.
    model.select_conversation(new);
    messages_of(
        &mut model,
        new,
        vec![message(new, 5, Status::Queued, "again")],
    );
    model.apply(Update::Event(RuntimeEvent::MessageDelivered(
        UiMessageId::new(old, 5),
    )));
    assert_eq!(
        model.thread(new).unwrap().messages[0].status,
        Status::Queued
    );
    for (_, command) in model.take_commands() {
        if let Command::LoadMessages { contact: asked, .. } = command {
            assert_ne!(asked, new, "the new record is not read for the old one");
        }
    }
}

#[test]
fn a_delivered_status_comes_only_from_the_runtime_and_never_from_a_position() {
    let mut model = ready();
    model.select_identity(identity(1));
    let c = contact(1, 10);
    model.select_conversation(c);
    messages_of(&mut model, c, vec![message(c, 1, Status::Queued, "one")]);
    // The event says delivered: the message is read again, not marked.
    model.apply(Update::Event(RuntimeEvent::MessageDelivered(
        UiMessageId::new(c, 1),
    )));
    assert_eq!(model.thread(c).unwrap().messages[0].status, Status::Queued);
    messages_of(&mut model, c, vec![message(c, 1, Status::Delivered, "one")]);
    assert_eq!(
        model.thread(c).unwrap().messages[0].status,
        Status::Delivered
    );
    // An answer for another conversation is not taken for this one.
    let other = contact(1, 11);
    let commands = model.take_commands();
    assert!(commands.is_empty());
    model.apply(Update::Done {
        command: CommandId::new(u64::MAX),
        outcome: Outcome::Messages {
            contact: other,
            messages: vec![message(other, 1, Status::Queued, "x")],
            complete: true,
        },
    });
    assert_eq!(
        model.thread(c).unwrap().messages[0].status,
        Status::Delivered
    );
}

#[test]
fn a_draft_goes_only_once_the_runtime_queued_it() {
    let mut model = ready();
    model.select_identity(identity(1));
    let c = contact(1, 10);
    model.select_conversation(c);
    messages_of(&mut model, c, Vec::new());
    model.draft_mut(c).unwrap().push_str("hello");
    model.send(c);
    // While it is queued the draft stays and cannot be edited.
    assert!(model.draft_mut(c).is_none());
    assert_eq!(model.thread(c).unwrap().draft, "hello");
    answer_all(&mut model, |command| match command {
        Command::Send { contact, text } => {
            assert_eq!((*contact, text.as_str()), (c, "hello"));
            Some(Outcome::Failed(Failure::QueueFull))
        }
        _ => None,
    });
    let thread = model.thread(c).unwrap();
    assert_eq!(thread.draft, "hello");
    assert_eq!(thread.send_error, Some(Failure::QueueFull));
    model.send(c);
    answer_all(&mut model, |command| match command {
        Command::Send { .. } => Some(Outcome::MessageQueued(UiMessageId::new(c, 1))),
        _ => None,
    });
    assert_eq!(model.thread(c).unwrap().draft, "");
}

#[test]
fn a_queued_answer_for_another_conversation_clears_nothing() {
    let mut model = ready();
    model.select_identity(identity(1));
    let c = contact(1, 10);
    model.select_conversation(c);
    messages_of(&mut model, c, Vec::new());
    model.draft_mut(c).unwrap().push_str("keep me");
    model.send(c);
    answer_all(&mut model, |command| match command {
        Command::Send { .. } => Some(Outcome::MessageQueued(UiMessageId::new(contact(1, 11), 1))),
        _ => None,
    });
    assert_eq!(model.thread(c).unwrap().draft, "keep me");
}

#[test]
fn a_confirmation_acts_on_what_it_captured_not_on_the_selection() {
    let mut model = ready();
    model.select_identity(identity(1));
    let c = contact(1, 10);
    let d = contact(1, 11);
    model.select_conversation(c);
    model.open(Dialog::BlockContact {
        contact: c,
        busy: None,
        error: None,
    });
    // The selection moves while the dialog is open.
    model.select_conversation(d);
    model.confirm();
    let commands = model.take_commands();
    let blocks: Vec<_> = commands
        .iter()
        .filter_map(|(_, command)| match command {
            Command::BlockContact(asked) => Some(*asked),
            _ => None,
        })
        .collect();
    assert_eq!(blocks, vec![c]);
}

#[test]
fn a_dialog_of_one_identity_closes_when_another_is_shown() {
    let mut model = ready();
    model.select_identity(identity(1));
    model.open(Dialog::NewInvitation {
        identity: identity(1),
        label: "Friends".to_owned(),
        created: None,
        busy: None,
        error: None,
    });
    model.select_identity(identity(2));
    assert!(model.dialog().is_none());
    // A new invitation dialog of B makes B's invitation, never A's.
    model.open(Dialog::NewInvitation {
        identity: identity(2),
        label: String::new(),
        created: None,
        busy: None,
        error: None,
    });
    model.confirm();
    let commands = model.take_commands();
    let made: Vec<_> = commands
        .iter()
        .filter_map(|(_, command)| match command {
            Command::CreateInvitation { identity, .. } => Some(*identity),
            _ => None,
        })
        .collect();
    assert_eq!(made, vec![identity(2)]);
}

#[test]
fn an_answer_to_a_replaced_request_never_reaches_the_new_one() {
    let mut model = ready();
    model.select_identity(identity(1));
    let first = UiRequestId::new(identity(1), 1);
    let second = UiRequestId::new(identity(1), 2);
    let request = |id: UiRequestId| RequestView {
        id,
        fingerprint: "FFFF 0000".to_owned(),
        full_fingerprint: String::new(),
        display_name: "Frank".to_owned(),
        introduction: String::new(),
        invitation: None,
    };
    let with_requests = |list: Vec<RequestView>| {
        let mut snap = snapshot(identity(1), Vec::new());
        snap.requests = list;
        Outcome::Identity(Box::new(snap))
    };
    model.apply(Update::Event(RuntimeEvent::RequestReceived {
        identity: identity(1),
        queued: true,
    }));
    answer_all(&mut model, |command| match command {
        Command::LoadIdentity(_) => Some(with_requests(vec![request(first)])),
        _ => None,
    });
    model.open(Dialog::AcceptRequest {
        request: first,
        name: "Frank".to_owned(),
        busy: None,
        error: None,
    });
    // The request is replaced by another of the same sender meanwhile.
    model.apply(Update::Event(RuntimeEvent::RequestReceived {
        identity: identity(1),
        queued: true,
    }));
    answer_all(&mut model, |command| match command {
        Command::LoadIdentity(_) => Some(with_requests(vec![request(second)])),
        _ => None,
    });
    model.confirm();
    let accepted: Vec<_> = model
        .take_commands()
        .into_iter()
        .filter_map(|(_, command)| match command {
            Command::AcceptRequest { request, .. } => Some(request),
            _ => None,
        })
        .collect();
    // The command names the request the dialog captured; the runtime
    // refuses it, and nothing turns it into the new one.
    assert_eq!(accepted, vec![first]);
    // Declining a request no longer listed asks nothing.
    model.decline(first);
    assert!(
        model
            .take_commands()
            .iter()
            .all(|(_, command)| !matches!(command, Command::DeclineRequest(_)))
    );
}

#[test]
fn an_invitation_command_carries_the_identity_of_its_invitation() {
    let mut model = ready();
    model.select_identity(identity(2));
    let invitation = UiInvitationId::new(identity(1), 7);
    model.open(Dialog::RevokeInvitation {
        invitation,
        discard_requests: false,
        busy: None,
        error: None,
    });
    model.confirm();
    let revoked: Vec<_> = model
        .take_commands()
        .into_iter()
        .filter_map(|(_, command)| match command {
            Command::RevokeInvitation { invitation, .. } => Some(invitation.identity()),
            _ => None,
        })
        .collect();
    assert_eq!(revoked, vec![identity(1)]);
    let _ = InvitationView {
        id: invitation,
        label: None,
    };
}

#[test]
fn missed_events_make_the_model_read_everything_again() {
    let mut model = ready();
    model.select_identity(identity(1));
    model.select_conversation(contact(1, 10));
    messages_of(&mut model, contact(1, 10), Vec::new());
    assert!(model.take_commands().is_empty());
    model.apply(Update::Event(RuntimeEvent::Missed));
    let commands = model.take_commands();
    let mut identities = 0;
    let mut slices = Vec::new();
    let mut threads = Vec::new();
    for (_, command) in commands {
        match command {
            Command::LoadIdentities => identities += 1,
            Command::LoadIdentity(id) => slices.push(id),
            Command::LoadMessages { contact, .. } => threads.push(contact),
            _ => {}
        }
    }
    assert_eq!(identities, 1);
    slices.sort();
    assert_eq!(slices, vec![identity(1), identity(2)]);
    assert_eq!(threads, vec![contact(1, 10)]);
}

#[test]
fn a_burst_of_events_makes_one_read() {
    let mut model = ready();
    model.select_identity(identity(1));
    model.select_conversation(contact(1, 10));
    messages_of(&mut model, contact(1, 10), Vec::new());
    for sequence in 0..50 {
        model.apply(Update::Event(RuntimeEvent::MessageReceived(
            UiMessageId::new(contact(1, 10), sequence),
        )));
    }
    let commands = model.take_commands();
    assert_eq!(commands.len(), 2, "{commands:?}");
    // While a read runs, more events wait for it instead of adding reads.
    model.apply(Update::Event(RuntimeEvent::MessageReceived(
        UiMessageId::new(contact(1, 10), 99),
    )));
    assert!(model.take_commands().is_empty());
}

#[test]
fn a_deleted_identity_takes_its_state_and_its_selection_with_it() {
    let mut model = ready();
    model.select_identity(identity(2));
    model.select_conversation(contact(2, 20));
    model.apply(Update::Event(RuntimeEvent::IdentityClosed(identity(2))));
    answer_all(&mut model, |command| match command {
        Command::LoadIdentities => Some(Outcome::Identities(vec![identity_view(1, "Alice")])),
        _ => None,
    });
    assert_eq!(model.current(), Some(identity(1)));
    assert!(model.slice(identity(2)).is_none());
    // A late event of the deleted identity changes nothing and reads
    // nothing of it.
    model.apply(Update::Event(RuntimeEvent::MessageReceived(
        UiMessageId::new(contact(2, 20), 1),
    )));
    for (_, command) in model.take_commands() {
        if let Command::LoadIdentity(id) = command {
            assert_ne!(id, identity(2));
        }
    }
}

#[test]
fn a_conversation_of_another_identity_cannot_be_selected() {
    let mut model = ready();
    model.select_identity(identity(1));
    model.select_conversation(contact(2, 20));
    assert_eq!(model.selection(), None);
    // Nor an unknown record of the identity shown.
    model.select_conversation(contact(1, 99));
    assert_eq!(model.selection(), None);
}

#[test]
fn an_ended_conversation_shows_but_takes_no_message() {
    let mut model = ready();
    model.select_identity(identity(1));
    let ended = contact(1, 13);
    answer_all(&mut model, |_| None);
    model.apply(Update::Event(RuntimeEvent::MessageReceived(
        UiMessageId::new(ended, 1),
    )));
    answer_all(&mut model, |command| match command {
        Command::LoadIdentity(id) if *id == identity(1) => {
            let mut snap = snapshot(identity(1), Vec::new());
            snap.conversations = vec![ConversationView {
                contact: ended,
                fingerprint: "GGGG 4444".to_owned(),
                active: false,
                messages: 1,
                queued: 0,
                latest: Some(message(ended, 1, Status::Received, "bye")),
            }];
            Some(Outcome::Identity(Box::new(snap)))
        }
        _ => None,
    });
    model.select_conversation(ended);
    assert_eq!(model.selection(), Some(Selection::Conversation(ended)));
    assert!(!model.can_send(ended));
    assert_eq!(model.contact_name(ended), "Former contact GGGG");
}

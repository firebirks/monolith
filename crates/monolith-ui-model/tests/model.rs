//! The UI model against the stale-instance and identity-mixing scenarios
//! of the Phase 6 brief (section 42), and the rules of
//! `docs/adr/0011-ui-security-boundary.md`: nothing late, stale or of
//! another identity changes what is shown of another, a dialog acts on
//! what it captured, a draft goes only once the runtime queued it, and an
//! unblock names someone listed.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use monolith_ui_model::model::UNBLOCKED;
use monolith_ui_model::{
    BlockedView, Command, CommandId, ContactView, ConversationView, Deferral, Dialog, Failure,
    IdentitySnapshot, IdentityView, InvitationView, MessageView, Model, NoticeKind, Outcome, Phase,
    ProfileOnDisk, RequestMode, RequestView, RuntimeEvent, Selection, Service, Status, UiBlockedId,
    UiContactId, UiIdentityId, UiInvitationId, UiMessageId, UiRequestId, Update, present,
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
        invitation: false,
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
        blocked: Vec::new(),
        service: Service::Available,
        request_mode: RequestMode::Invitation,
    }
}

fn blocked_view(id: UiBlockedId, fingerprint: &str) -> BlockedView {
    BlockedView {
        id,
        fingerprint: fingerprint.to_owned(),
        full_fingerprint: String::new(),
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
    // While it is queued the draft stays, and what is typed meanwhile is
    // kept; the text sent is the one taken when Send was pressed.
    assert_eq!(model.thread(c).unwrap().draft, "hello");
    model.draft_mut(c).unwrap().push_str(" again");
    answer_all(&mut model, |command| match command {
        Command::Send { contact, text } => {
            assert_eq!((*contact, text.as_str()), (c, "hello"));
            Some(Outcome::Failed(Failure::QueueFull))
        }
        _ => None,
    });
    let thread = model.thread(c).unwrap();
    assert_eq!(thread.draft, "hello again");
    assert_eq!(thread.send_error, Some(Failure::QueueFull));
    model.send(c);
    // Typed while it is queued: kept; only the text sent goes.
    model.draft_mut(c).unwrap().push('!');
    answer_all(&mut model, |command| match command {
        Command::Send { text, .. } => {
            assert_eq!(text, "hello again");
            Some(Outcome::MessageQueued(UiMessageId::new(c, 1)))
        }
        _ => None,
    });
    assert_eq!(model.thread(c).unwrap().draft, "!");
    model.draft_mut(c).unwrap().clear();
    model.draft_mut(c).unwrap().push_str("next");
    model.send(c);
    answer_all(&mut model, |command| match command {
        Command::Send { .. } => Some(Outcome::MessageQueued(UiMessageId::new(c, 2))),
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
fn typing_while_a_message_is_queued_stays_once_it_is_queued() {
    let mut model = ready();
    model.select_identity(identity(1));
    let c = contact(1, 10);
    model.select_conversation(c);
    messages_of(&mut model, c, Vec::new());
    model.draft_mut(c).unwrap().push_str("See you at eight.");
    model.send(c);
    // A new line typed while the runtime queues it.
    model.draft_mut(c).unwrap().push_str("\nBring the map");
    answer_all(&mut model, |command| match command {
        Command::Send { text, .. } => {
            assert_eq!(text, "See you at eight.");
            Some(Outcome::MessageQueued(UiMessageId::new(c, 1)))
        }
        _ => None,
    });
    let thread = model.thread(c).unwrap();
    assert_eq!(thread.draft, "Bring the map");
    assert_eq!(thread.sending, None);
    // A draft changed within the text sent stays whole: nothing that may
    // not have been sent is taken from it.
    model.send(c);
    model.draft_mut(c).unwrap().insert_str(10, "big ");
    answer_all(&mut model, |command| match command {
        Command::Send { text, .. } => {
            assert_eq!(text, "Bring the map");
            Some(Outcome::MessageQueued(UiMessageId::new(c, 2)))
        }
        _ => None,
    });
    assert_eq!(model.thread(c).unwrap().draft, "Bring the big map");
}

#[test]
fn a_refused_send_keeps_the_text_sent_and_what_was_typed_after_it() {
    let mut model = ready();
    model.select_identity(identity(1));
    let c = contact(1, 10);
    model.select_conversation(c);
    messages_of(&mut model, c, Vec::new());
    model.draft_mut(c).unwrap().push_str("See you at eight.");
    model.send(c);
    model.draft_mut(c).unwrap().push_str(" Bring the map.");
    answer_all(&mut model, |command| match command {
        Command::Send { .. } => Some(Outcome::Failed(Failure::StoreFull)),
        _ => None,
    });
    // The draft is the text sent with the typing after it, the reason is
    // shown, and all of it can be sent again.
    let thread = model.thread(c).unwrap();
    assert_eq!(thread.draft, "See you at eight. Bring the map.");
    assert_eq!(thread.send_error, Some(Failure::StoreFull));
    assert_eq!(thread.sending, None);
    assert!(model.can_send(c));
    model.send(c);
    let sent: Vec<String> = model
        .take_commands()
        .into_iter()
        .filter_map(|(_, command)| match command {
            Command::Send { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(sent, ["See you at eight. Bring the map."]);
}

#[test]
fn the_next_try_counts_from_when_the_attempt_failed_and_goes_with_the_queue() {
    let mut model = ready();
    let c = contact(1, 10);
    let e = contact(2, 20);
    let queued = |identity: UiIdentityId, contacts: Vec<ContactView>, waiting: usize| {
        let mut snap = snapshot(identity, contacts);
        snap.conversations = snap
            .contacts
            .iter()
            .map(|view| ConversationView {
                contact: view.id,
                fingerprint: view.fingerprint.clone(),
                active: true,
                messages: waiting,
                queued: waiting,
                latest: None,
            })
            .collect();
        Some(Outcome::Identity(Box::new(snap)))
    };
    model.set_time(1_000);
    for (contact, retry_in) in [(c, 300), (e, 20)] {
        model.apply(Update::Event(RuntimeEvent::DeliveryDeferred {
            contact,
            reason: Deferral::Unreachable,
            retry_in,
        }));
    }
    assert_eq!(model.slice(identity(1)).unwrap().retries[&c], 1_300);
    assert_eq!(model.slice(identity(2)).unwrap().retries[&e], 1_020);
    assert!(!model.slice(identity(1)).unwrap().retries.contains_key(&e));
    // A read that still finds messages queued keeps it.
    model.apply(Update::Event(RuntimeEvent::Missed));
    answer_all(&mut model, |command| match command {
        Command::LoadIdentity(id) if *id == identity(1) => {
            queued(identity(1), vec![contact_view(c, "Carol", "CCCC 1111")], 2)
        }
        Command::LoadIdentity(id) if *id == identity(2) => {
            queued(identity(2), vec![contact_view(e, "Erin", "EEEE 3333")], 0)
        }
        _ => None,
    });
    assert_eq!(model.slice(identity(1)).unwrap().retries[&c], 1_300);
    // Nothing queued is tried no more.
    assert!(model.slice(identity(2)).unwrap().retries.is_empty());
    assert_eq!(model.undelivered(), 2);
}

#[test]
fn undelivered_messages_are_counted_over_every_identity_but_not_ended_ones() {
    let mut model = ready();
    assert_eq!(model.undelivered(), 0);
    let conversation = |contact: UiContactId, active: bool, queued: usize| ConversationView {
        contact,
        fingerprint: String::new(),
        active,
        messages: queued,
        queued,
        latest: None,
    };
    model.apply(Update::Event(RuntimeEvent::Missed));
    answer_all(&mut model, |command| match command {
        Command::LoadIdentity(id) if *id == identity(1) => {
            let mut snap = snapshot(identity(1), Vec::new());
            snap.conversations = vec![
                conversation(contact(1, 10), true, 2),
                // Blocked or removed: never sent.
                conversation(contact(1, 13), false, 5),
            ];
            Some(Outcome::Identity(Box::new(snap)))
        }
        Command::LoadIdentity(id) if *id == identity(2) => {
            let mut snap = snapshot(identity(2), Vec::new());
            snap.conversations = vec![conversation(contact(2, 20), true, 1)];
            Some(Outcome::Identity(Box::new(snap)))
        }
        _ => None,
    });
    assert_eq!(model.undelivered(), 3);
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
    let mut tor = 0;
    for (_, command) in commands {
        match command {
            Command::LoadIdentities => identities += 1,
            Command::LoadIdentity(id) => slices.push(id),
            Command::LoadMessages { contact, .. } => threads.push(contact),
            // The state of Tor may have changed in what was missed too.
            Command::CheckTor => tor += 1,
            _ => {}
        }
    }
    assert_eq!(identities, 1);
    slices.sort();
    assert_eq!(slices, vec![identity(1), identity(2)]);
    assert_eq!(threads, vec![contact(1, 10)]);
    assert_eq!(tor, 1);
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

#[test]
fn a_failure_is_told_in_its_sentence_never_in_its_debug_form() {
    let mut model = ready();
    model.select_identity(identity(1));
    let request = UiRequestId::new(identity(1), 1);
    let mut snap = snapshot(identity(1), Vec::new());
    snap.requests = vec![RequestView {
        id: request,
        fingerprint: "FFFF 0000".to_owned(),
        full_fingerprint: String::new(),
        display_name: String::new(),
        introduction: String::new(),
        invitation: None,
    }];
    model.apply(Update::Event(RuntimeEvent::RequestReceived {
        identity: identity(1),
        queued: true,
    }));
    answer_all(&mut model, |command| match command {
        Command::LoadIdentity(_) => Some(Outcome::Identity(Box::new(snap.clone()))),
        _ => None,
    });
    model.take_notices();
    model.decline(request);
    answer_all(&mut model, |command| match command {
        Command::DeclineRequest(_) => Some(Outcome::Failed(Failure::NoSuchRequest)),
        _ => None,
    });
    let texts: Vec<String> = model
        .take_notices()
        .into_iter()
        .map(|notice| notice.text)
        .collect();
    assert_eq!(texts, vec![Failure::NoSuchRequest.message().to_owned()]);
}

#[test]
fn a_snapshot_shows_only_what_belongs_to_its_identity() {
    let mut model = ready();
    model.select_identity(identity(1));
    model.apply(Update::Event(RuntimeEvent::Missed));
    let foreign = contact(2, 20);
    answer_all(&mut model, |command| match command {
        Command::LoadIdentity(id) if *id == identity(1) => {
            let mut snap = snapshot(
                identity(1),
                vec![
                    contact_view(contact(1, 10), "Carol", "CCCC 1111"),
                    contact_view(foreign, "Erin", "EEEE 3333"),
                ],
            );
            snap.conversations = vec![ConversationView {
                contact: foreign,
                fingerprint: "EEEE 3333".to_owned(),
                active: true,
                messages: 0,
                queued: 0,
                latest: None,
            }];
            snap.requests = vec![RequestView {
                id: UiRequestId::new(identity(2), 4),
                fingerprint: "FFFF 0000".to_owned(),
                full_fingerprint: String::new(),
                display_name: String::new(),
                introduction: String::new(),
                invitation: None,
            }];
            snap.invitations = vec![InvitationView {
                id: UiInvitationId::new(identity(2), 5),
                label: None,
            }];
            snap.blocked = vec![blocked_view(UiBlockedId::new(identity(2), 6), "GGGG 1111")];
            Some(Outcome::Identity(Box::new(snap)))
        }
        _ => None,
    });
    let slice = model.slice(identity(1)).unwrap();
    assert!(
        slice
            .contacts
            .iter()
            .all(|view| view.id.identity() == identity(1))
    );
    assert_eq!(slice.contacts.len(), 1);
    assert!(slice.conversations.is_empty());
    assert!(slice.requests.is_empty());
    assert!(slice.invitations.is_empty());
    assert!(slice.blocked.is_empty());
}

#[test]
fn no_secret_or_text_is_printed_by_debug() {
    let commands = [
        Command::OpenProfile {
            passphrase: Zeroizing::new("hunter2 secret".to_owned()),
        },
        Command::CreateProfile {
            passphrase: Zeroizing::new("hunter2 secret".to_owned()),
        },
        Command::Send {
            contact: contact(1, 10),
            text: "secret words".to_owned(),
        },
        Command::AddContact {
            identity: identity(1),
            card: Zeroizing::new("MONOLITH secret card".to_owned()),
            name: Some("secret name".to_owned()),
        },
        Command::SetContactName {
            contact: contact(1, 10),
            name: Some("secret name".to_owned()),
        },
        Command::CreateIdentity {
            name: Some("secret name".to_owned()),
        },
        Command::CreateInvitation {
            identity: identity(1),
            label: Some("secret label".to_owned()),
        },
    ];
    for command in &commands {
        let text = format!("{command:?} {command:#?}");
        assert!(!text.contains("secret"), "{text}");
    }
    let outcomes = [
        Outcome::Card(Zeroizing::new("MONOLITH secret card".to_owned())),
        Outcome::InvitationCreated {
            invitation: UiInvitationId::new(identity(1), 1),
            card: Zeroizing::new("MONOLITH secret card".to_owned()),
        },
        Outcome::Messages {
            contact: contact(1, 10),
            messages: vec![message(contact(1, 10), 1, Status::Received, "secret")],
            complete: true,
        },
    ];
    for outcome in &outcomes {
        let text = format!("{outcome:?} {outcome:#?}");
        assert!(!text.contains("secret"), "{text}");
    }

    // The state of the model holds names, cards, drafts and texts in
    // flight; none of them is printed either.
    let mut model = ready();
    model.select_identity(identity(1));
    let c = contact(1, 10);
    model.apply(Update::Event(RuntimeEvent::Missed));
    answer_all(&mut model, |command| match command {
        Command::LoadIdentity(id) if *id == identity(1) => {
            let mut snap = snapshot(
                identity(1),
                vec![contact_view(c, "secret contact", "CCCC 1111")],
            );
            snap.requests = vec![RequestView {
                id: UiRequestId::new(identity(1), 1),
                fingerprint: "FFFF 0000".to_owned(),
                full_fingerprint: String::new(),
                display_name: "secret sender".to_owned(),
                introduction: "secret introduction".to_owned(),
                invitation: None,
            }];
            snap.invitations = vec![InvitationView {
                id: UiInvitationId::new(identity(1), 2),
                label: Some("secret label".to_owned()),
            }];
            Some(Outcome::Identity(Box::new(snap)))
        }
        _ => None,
    });
    model.select_conversation(c);
    messages_of(
        &mut model,
        c,
        vec![message(c, 1, Status::Received, "secret message")],
    );
    model.draft_mut(c).unwrap().push_str("secret draft");
    model.send(c);
    model.draft_mut(c).unwrap().push_str(" and more secret");
    model.apply(Update::Event(RuntimeEvent::ContactAccepted(c)));
    model.open(Dialog::AddContact {
        identity: identity(1),
        card: "MONOLITH secret card".to_owned(),
        name: "secret name".to_owned(),
        busy: None,
        error: None,
        added: None,
    });
    let text = format!(
        "{model:?} {model:#?} {:?} {:?} {:?} {:?}",
        model.dialog(),
        model.slice(identity(1)),
        model.thread(c),
        model.sidebar(),
    );
    assert!(!text.contains("secret"), "{text}");
    let notices = model.take_notices();
    let text = format!("{notices:?}");
    assert!(!text.contains("secret"), "{text}");
    model.close();
    let mut model = ready();
    model.open(Dialog::NewInvitation {
        identity: identity(1),
        label: "secret label".to_owned(),
        created: Some((
            UiInvitationId::new(identity(1), 3),
            Zeroizing::new("MONOLITH secret card".to_owned()),
        )),
        busy: None,
        error: None,
    });
    let text = format!("{model:?} {:?}", model.dialog());
    assert!(!text.contains("secret"), "{text}");
}

#[test]
fn a_dialog_opened_again_is_another_instance() {
    let mut model = ready();
    model.select_identity(identity(1));
    let add = || Dialog::AddContact {
        identity: identity(1),
        card: String::new(),
        name: String::new(),
        busy: None,
        error: None,
        added: None,
    };
    assert_eq!(model.dialog_instance(), None);
    model.open(add());
    let first = model.dialog_instance().unwrap();
    model.close();
    assert_eq!(model.dialog_instance(), None);
    // The same dialog of the same identity, opened again, is not the one
    // that was closed.
    model.open(add());
    let second = model.dialog_instance().unwrap();
    assert_ne!(first, second);
    // Confirming it keeps it the same instance until it closes.
    if let Some(Dialog::AddContact { card, .. }) = model.dialog_mut() {
        card.push_str("MONOLITH card");
    }
    model.confirm();
    assert_eq!(model.dialog_instance(), Some(second));
    let commands = answer_all(&mut model, |command| match command {
        Command::AddContact { card, .. } => {
            assert_eq!(card.as_str(), "MONOLITH card");
            Some(Outcome::ContactAdded {
                contact: contact(1, 12),
                invitation: true,
            })
        }
        _ => None,
    });
    assert!(
        commands
            .iter()
            .any(|command| matches!(command, Command::AddContact { .. }))
    );
    assert_eq!(model.dialog_instance(), None);
}

#[test]
fn an_unlock_that_fails_stays_on_the_unlock_screen() {
    let mut model = Model::new();
    model.apply(Update::Started {
        profile: ProfileOnDisk::Exists,
        location: "/home/user/.local/share/monolith".to_owned(),
    });
    // A passphrase too long for the vault cannot open it; the profile
    // exists, so the screen to create one is never shown.
    for failure in [
        Failure::UnusablePassphrase,
        Failure::WrongPassphrase,
        Failure::Disk,
    ] {
        model.unlock(Zeroizing::new("a".repeat(2_000)));
        assert_eq!(model.phase(), Phase::Opening);
        answer_all(&mut model, |command| match command {
            Command::OpenProfile { .. } => Some(Outcome::Failed(failure)),
            _ => None,
        });
        assert_eq!(model.phase(), Phase::Locked);
        assert_eq!(model.profile_error(), Some(failure));
    }
    // A session that saves nothing, started from there and refused, goes
    // back there too.
    model.start_without_saving();
    answer_all(&mut model, |_| Some(Outcome::Failed(Failure::Unexpected)));
    assert_eq!(model.phase(), Phase::Locked);

    // A profile that cannot be created leaves the first start shown.
    let mut model = Model::new();
    model.apply(Update::Started {
        profile: ProfileOnDisk::Absent,
        location: "/home/user/.local/share/monolith".to_owned(),
    });
    model.create_profile(Zeroizing::new("a".repeat(2_000)));
    answer_all(&mut model, |command| match command {
        Command::CreateProfile { .. } => Some(Outcome::Failed(Failure::UnusablePassphrase)),
        _ => None,
    });
    assert_eq!(model.phase(), Phase::Welcome);
    assert_eq!(model.profile_error(), Some(Failure::UnusablePassphrase));
}

#[test]
fn a_failed_store_raises_no_notice_of_its_own() {
    let mut model = ready();
    model.take_notices();
    model.apply(Update::Event(RuntimeEvent::StorageFailed));
    model.apply(Update::Event(RuntimeEvent::StorageFailed));
    // The screens say it once, for as long as it lasts.
    assert!(model.storage_failed());
    assert!(model.take_notices().is_empty());
}

#[test]
fn a_declined_request_and_a_blocked_sender_are_told_apart() {
    let mut model = ready();
    model.select_identity(identity(1));
    let first = UiRequestId::new(identity(1), 1);
    let second = UiRequestId::new(identity(1), 2);
    let request = |id: UiRequestId| RequestView {
        id,
        fingerprint: "FFFF 0000".to_owned(),
        full_fingerprint: String::new(),
        display_name: String::new(),
        introduction: String::new(),
        invitation: None,
    };
    model.apply(Update::Event(RuntimeEvent::RequestReceived {
        identity: identity(1),
        queued: true,
    }));
    answer_all(&mut model, |command| match command {
        Command::LoadIdentity(_) => {
            let mut snap = snapshot(identity(1), Vec::new());
            snap.requests = vec![request(first), request(second)];
            Some(Outcome::Identity(Box::new(snap)))
        }
        _ => None,
    });
    model.take_notices();
    model.decline(first);
    model.block_sender(second);
    assert!(model.answering(first));
    assert!(model.answering(second));
    // Answered in the other order: each answer says what it did.
    let commands = model.take_commands();
    for (id, _) in commands.into_iter().rev() {
        model.apply(Update::Done {
            command: id,
            outcome: Outcome::Done,
        });
    }
    let texts: Vec<String> = model
        .take_notices()
        .into_iter()
        .map(|notice| notice.text)
        .collect();
    assert_eq!(texts, ["Sender blocked.", "Request declined."]);
}

#[test]
fn an_answer_to_no_command_in_flight_changes_nothing() {
    let mut model = ready();
    model.select_identity(identity(1));
    let c = contact(1, 10);
    model.select_conversation(c);
    // The read of the conversation is in flight.
    let read = model
        .take_commands()
        .into_iter()
        .find(|(_, command)| matches!(command, Command::LoadMessages { .. }))
        .map(|(id, _)| id)
        .unwrap();
    // An answer that names no command in flight is not taken for it.
    model.apply(Update::Done {
        command: CommandId::new(u64::MAX),
        outcome: Outcome::Messages {
            contact: c,
            messages: vec![message(c, 1, Status::Delivered, "forged")],
            complete: true,
        },
    });
    assert!(model.thread(c).unwrap().messages.is_empty());
    // The read is still in flight, and its own answer is taken.
    model.apply(Update::Done {
        command: read,
        outcome: Outcome::Messages {
            contact: c,
            messages: vec![message(c, 1, Status::Queued, "real")],
            complete: true,
        },
    });
    let messages = &model.thread(c).unwrap().messages;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text, "real");
    assert_eq!(messages[0].status, Status::Queued);
}

#[test]
fn a_dialog_takes_no_answer_that_names_another_identity() {
    let mut model = ready();
    model.select_identity(identity(1));
    let shown = contact(1, 10);
    model.select_conversation(shown);
    messages_of(&mut model, shown, Vec::new());
    model.take_notices();
    // An invitation made for A is answered with one of B: it is not shown
    // as A's.
    model.open(Dialog::NewInvitation {
        identity: identity(1),
        label: String::new(),
        created: None,
        busy: None,
        error: None,
    });
    model.confirm();
    answer_all(&mut model, |command| match command {
        Command::CreateInvitation { .. } => Some(Outcome::InvitationCreated {
            invitation: UiInvitationId::new(identity(2), 7),
            card: Zeroizing::new("MONOLITH card".to_owned()),
        }),
        _ => None,
    });
    assert!(model.dialog().is_none());
    // A contact added to B, or a request of B accepted, is answered with a
    // contact of A, the identity shown: once A is read again, it does not
    // open that contact.
    let dialogs = [
        Dialog::AddContact {
            identity: identity(2),
            card: "card".to_owned(),
            name: String::new(),
            busy: None,
            error: None,
            added: None,
        },
        Dialog::AcceptRequest {
            request: UiRequestId::new(identity(2), 3),
            name: String::new(),
            busy: None,
            error: None,
        },
    ];
    for dialog in dialogs {
        load(&mut model);
        model.open(dialog);
        model.confirm();
        answer_all(&mut model, |command| match command {
            Command::AddContact { .. } => Some(Outcome::ContactAdded {
                contact: contact(1, 11),
                invitation: false,
            }),
            Command::AcceptRequest { .. } => Some(Outcome::RequestAccepted(contact(1, 11))),
            _ => None,
        });
        assert!(model.dialog().is_none());
        model.apply(Update::Event(RuntimeEvent::Missed));
        load(&mut model);
        assert_eq!(model.selection(), Some(Selection::Conversation(shown)));
    }
    // None of these answers was told as a success.
    assert!(
        model
            .take_notices()
            .iter()
            .all(|notice| notice.kind != monolith_ui_model::NoticeKind::Success)
    );
}

#[test]
fn deleting_the_messages_of_the_conversation_shown_keeps_it_shown_and_read() {
    let mut model = ready();
    model.select_identity(identity(1));
    let c = contact(1, 10);
    model.select_conversation(c);
    messages_of(&mut model, c, vec![message(c, 1, Status::Received, "hi")]);
    model.draft_mut(c).unwrap().push_str("unsent");
    model.open(Dialog::DeleteConversation {
        contact: c,
        busy: None,
        error: None,
    });
    model.confirm();
    answer_all(&mut model, |command| match command {
        Command::DeleteConversation(asked) => {
            assert_eq!(*asked, c);
            Some(Outcome::Done)
        }
        _ => None,
    });
    // The conversation stays shown, without its messages and with its
    // draft, and can take a message.
    assert_eq!(model.selection(), Some(Selection::Conversation(c)));
    let thread = model.thread(c).unwrap();
    assert!(thread.messages.is_empty());
    assert_eq!(thread.draft, "unsent");
    assert!(model.can_send(c));
    assert!(model.draft_mut(c).is_some());
    // It is read again, and a message that arrives afterwards is shown.
    model.apply(Update::Event(RuntimeEvent::MessageReceived(
        UiMessageId::new(c, 2),
    )));
    messages_of(
        &mut model,
        c,
        vec![message(c, 2, Status::Received, "again")],
    );
    let messages = &model.thread(c).unwrap().messages;
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].text, "again");
}

/// Adds a card to identity A, answered as the runtime does with `contact`
/// and whether the card was an invitation.
fn add_card(model: &mut Model, contact: UiContactId, invitation: bool) {
    model.open(Dialog::AddContact {
        identity: identity(1),
        card: "MONOLITH card".to_owned(),
        name: String::new(),
        busy: None,
        error: None,
        added: None,
    });
    model.confirm();
    answer_all(model, |command| match command {
        Command::AddContact { .. } => Some(Outcome::ContactAdded {
            contact,
            invitation,
        }),
        _ => None,
    });
}

#[test]
fn a_card_that_is_not_an_invitation_keeps_the_dialog_to_say_so() {
    let mut model = ready();
    model.select_identity(identity(1));
    model.take_notices();
    let added = contact(1, 12);
    add_card(&mut model, added, false);
    // The dialog stays, names the contact, waits for nothing, and holds
    // the card no longer; it is not told as done in a notice.
    match model.dialog() {
        Some(Dialog::AddContact {
            identity: owner,
            card,
            busy: None,
            added: Some(shown),
            ..
        }) => {
            assert_eq!(*owner, identity(1));
            assert_eq!(*shown, added);
            assert!(card.is_empty());
        }
        other => panic!("{other:?}"),
    }
    assert!(model.take_notices().is_empty());
    // The conversation opens behind it once A is read again.
    answer_all(&mut model, |command| match command {
        Command::LoadIdentity(id) if *id == identity(1) => {
            let mut view = contact_view(added, "Gina", "GGGG 4444");
            view.accepted = false;
            Some(Outcome::Identity(Box::new(snapshot(
                identity(1),
                vec![view],
            ))))
        }
        _ => None,
    });
    assert_eq!(model.selection(), Some(Selection::Conversation(added)));
    // Confirming it again adds nothing: it closes.
    model.confirm();
    assert!(model.dialog().is_none());
    assert!(
        !model
            .take_commands()
            .iter()
            .any(|(_, command)| matches!(command, Command::AddContact { .. }))
    );
}

#[test]
fn an_invitation_or_the_card_of_an_accepted_contact_closes_the_dialog() {
    let mut model = ready();
    model.select_identity(identity(1));
    model.take_notices();
    // An invitation: done.
    add_card(&mut model, contact(1, 12), true);
    assert!(model.dialog().is_none());
    // The plain card of Carol, who accepted already: nothing to wait for.
    load(&mut model);
    add_card(&mut model, contact(1, 10), false);
    assert!(model.dialog().is_none());
    let texts: Vec<String> = model
        .take_notices()
        .into_iter()
        .map(|notice| notice.text)
        .collect();
    assert_eq!(texts, ["Contact added.", "Contact added."]);
}

#[test]
fn a_contact_says_whether_its_card_was_an_invitation() {
    let mut model = ready();
    model.select_identity(identity(1));
    let invited = contact(1, 12);
    let carded = contact(1, 13);
    model.apply(Update::Event(RuntimeEvent::Missed));
    answer_all(&mut model, |command| match command {
        Command::LoadIdentity(id) if *id == identity(1) => {
            let mut first = contact_view(invited, "Gina", "GGGG 4444");
            first.accepted = false;
            first.invitation = true;
            let mut second = contact_view(carded, "Hal", "HHHH 5555");
            second.accepted = false;
            Some(Outcome::Identity(Box::new(snapshot(
                identity(1),
                vec![first, second],
            ))))
        }
        _ => None,
    });
    assert!(model.contact(invited).unwrap().invitation);
    assert!(!model.contact(carded).unwrap().invitation);
}

/// The model with identity 1 shown and, blocked by it, the people `list`.
fn with_blocked(list: Vec<BlockedView>) -> Model {
    let mut model = ready();
    model.select_identity(identity(1));
    model.apply(Update::Event(RuntimeEvent::Missed));
    answer_all(&mut model, |command| match command {
        Command::LoadIdentity(id) if *id == identity(1) => {
            let mut snap = snapshot(identity(1), Vec::new());
            snap.blocked = list.clone();
            Some(Outcome::Identity(Box::new(snap)))
        }
        Command::LoadIdentity(id) => Some(Outcome::Identity(Box::new(snapshot(*id, Vec::new())))),
        Command::LoadIdentities => Some(Outcome::Identities(vec![
            identity_view(1, "Alice"),
            identity_view(2, "Work"),
        ])),
        _ => None,
    });
    model.take_notices();
    model
}

#[test]
fn an_unblock_names_someone_listed_once_and_says_what_it_did() {
    let first = UiBlockedId::new(identity(1), 1);
    let second = UiBlockedId::new(identity(1), 2);
    let mut model = with_blocked(vec![
        blocked_view(first, "GGGG 1111"),
        blocked_view(second, "HHHH 2222"),
    ]);
    assert_eq!(model.slice(identity(1)).unwrap().blocked.len(), 2);
    // Someone not listed, of this identity or of another, is not asked
    // for.
    model.unblock(UiBlockedId::new(identity(1), 9));
    model.unblock(UiBlockedId::new(identity(2), 1));
    assert!(model.take_commands().is_empty());

    model.unblock(first);
    assert!(model.unblocking(first));
    assert!(!model.unblocking(second));
    // Asked once while it is under way.
    model.unblock(first);
    let commands = model.take_commands();
    let asked: Vec<&Command> = commands.iter().map(|(_, command)| command).collect();
    assert_eq!(asked, [&Command::Unblock(first)]);
    model.apply(Update::Done {
        command: commands[0].0,
        outcome: Outcome::Done,
    });
    assert!(!model.unblocking(first));
    let notices = model.take_notices();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].kind, NoticeKind::Success);
    assert_eq!(notices[0].text, UNBLOCKED);
    // Who is blocked is read again; until then the list stays as it was
    // said.
    assert_eq!(model.slice(identity(1)).unwrap().blocked.len(), 2);
    let reads = model.take_commands();
    assert_eq!(reads.len(), 1);
    assert_eq!(reads[0].1, Command::LoadIdentity(identity(1)));
    let mut snap = snapshot(identity(1), Vec::new());
    snap.blocked = vec![blocked_view(second, "HHHH 2222")];
    model.apply(Update::Done {
        command: reads[0].0,
        outcome: Outcome::Identity(Box::new(snap)),
    });
    let listed: Vec<UiBlockedId> = model
        .slice(identity(1))
        .unwrap()
        .blocked
        .iter()
        .map(|view| view.id)
        .collect();
    assert_eq!(listed, [second]);
    // No longer listed: not asked for again.
    model.unblock(first);
    assert!(model.take_commands().is_empty());

    // Refused: the reason is told, and the list is read again.
    model.unblock(second);
    let (id, _) = model.take_commands().remove(0);
    model.apply(Update::Done {
        command: id,
        outcome: Outcome::Failed(Failure::NotBlocked),
    });
    let notices = model.take_notices();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].kind, NoticeKind::Error);
    assert_eq!(notices[0].text, Failure::NotBlocked.message());
    assert!(
        model
            .take_commands()
            .iter()
            .any(|(_, command)| *command == Command::LoadIdentity(identity(1)))
    );
}

#[test]
fn an_unblock_answered_while_another_identity_is_shown_changes_only_its_own() {
    let blocked = UiBlockedId::new(identity(1), 1);
    let mut model = with_blocked(vec![blocked_view(blocked, "GGGG 1111")]);
    model.unblock(blocked);
    let (id, _) = model.take_commands().remove(0);
    // Work is shown when the answer comes.
    model.select_identity(identity(2));
    let before = model.sidebar();
    model.apply(Update::Done {
        command: id,
        outcome: Outcome::Done,
    });
    assert_eq!(model.current(), Some(identity(2)));
    assert_eq!(model.sidebar(), before);
    let reads: Vec<Command> = model
        .take_commands()
        .into_iter()
        .map(|(_, command)| command)
        .collect();
    assert_eq!(reads, [Command::LoadIdentity(identity(1))]);
    // An answer to the unblock, late again, changes nothing.
    let before = format!("{model:?}");
    model.apply(Update::Done {
        command: id,
        outcome: Outcome::Done,
    });
    assert_eq!(format!("{model:?}"), before);
    assert_eq!(model.take_notices().len(), 1);
}

/// Answers the read of identity 1 with Carol (1/10), dan (1/11) and a
/// contact without a name (1/12), and a conversation with each of the
/// first two whose newest message is `carol` and `dan`.
fn conversations_of(model: &mut Model, carol: &str, dan: &str) {
    answer_all(model, |command| match command {
        Command::LoadIdentity(id) if *id == identity(1) => {
            let c = contact(1, 10);
            let d = contact(1, 11);
            let mut unnamed = contact_view(contact(1, 12), "", "FFFF 3333");
            unnamed.name = None;
            let mut snap = snapshot(
                identity(1),
                vec![
                    contact_view(c, "Carol", "CCCC 1111"),
                    contact_view(d, "dan", "DDDD 2222"),
                    unnamed,
                ],
            );
            snap.conversations = vec![
                ConversationView {
                    contact: c,
                    fingerprint: "CCCC 1111".to_owned(),
                    active: true,
                    messages: 1,
                    queued: 0,
                    latest: Some(message(c, 1, Status::Received, carol)),
                },
                ConversationView {
                    contact: d,
                    fingerprint: "DDDD 2222".to_owned(),
                    active: true,
                    messages: 1,
                    queued: 0,
                    latest: Some(message(d, 1, Status::Received, dan)),
                },
            ];
            Some(Outcome::Identity(Box::new(snap)))
        }
        _ => None,
    });
}

#[test]
fn a_message_is_prepared_once_and_kept_while_it_reads_the_same() {
    let mut model = ready();
    let c = contact(1, 10);
    model.select_conversation(c);
    let one = UiMessageId::new(c, 1);
    let two = UiMessageId::new(c, 2);
    let first = vec![
        message(c, 1, Status::Received, "hello\u{202E}"),
        message(c, 2, Status::Queued, "again"),
    ];
    messages_of(&mut model, c, first.clone());
    // Nothing is prepared before it is drawn, and then as `prepare` does.
    assert!(model.thread(c).unwrap().prepared(one).is_none());
    model.prepare_messages(c, 0..1);
    let thread = model.thread(c).unwrap();
    assert_eq!(
        thread.prepared(one),
        Some(&present::prepare("hello\u{202E}", false))
    );
    assert!(thread.prepared(two).is_none());
    // A range past the end prepares what there is.
    model.prepare_messages(c, 1..10);
    assert!(model.thread(c).unwrap().prepared(two).is_some());
    // Read again, with a status changed and a message more: kept.
    let revision = model.thread(c).unwrap().revision();
    model.apply(Update::Event(RuntimeEvent::MessageDelivered(two)));
    let mut again = first;
    again[1].status = Status::Delivered;
    again.push(message(c, 3, Status::Received, "new"));
    messages_of(&mut model, c, again);
    let thread = model.thread(c).unwrap();
    assert_ne!(thread.revision(), revision);
    assert!(thread.prepared(one).is_some());
    assert!(thread.prepared(two).is_some());
    assert!(thread.prepared(UiMessageId::new(c, 3)).is_none());
    // A message that reads otherwise, or left the window, is not kept.
    model.apply(Update::Event(RuntimeEvent::MessageReceived(
        UiMessageId::new(c, 4),
    )));
    messages_of(
        &mut model,
        c,
        vec![
            message(c, 1, Status::Received, "otherwise"),
            message(c, 4, Status::Received, "later"),
        ],
    );
    let thread = model.thread(c).unwrap();
    assert!(thread.prepared(one).is_none());
    assert!(thread.prepared(two).is_none());
    model.prepare_messages(c, 0..2);
    assert_eq!(
        model.thread(c).unwrap().prepared(one).unwrap().text,
        "otherwise"
    );
}

#[test]
fn an_expanded_message_is_prepared_again_whole() {
    let mut model = ready();
    let c = contact(1, 10);
    model.select_conversation(c);
    let long = vec!["line"; present::MAX_RENDERED_MESSAGE_LINES + 5].join("\n");
    messages_of(&mut model, c, vec![message(c, 1, Status::Received, &long)]);
    let one = UiMessageId::new(c, 1);
    model.prepare_messages(c, 0..1);
    assert!(model.thread(c).unwrap().prepared(one).unwrap().collapsed);
    let revision = model.thread(c).unwrap().revision();
    model.expand(one);
    let thread = model.thread(c).unwrap();
    assert!(thread.prepared(one).is_none());
    assert_ne!(thread.revision(), revision);
    model.prepare_messages(c, 0..1);
    let whole = model.thread(c).unwrap().prepared(one).unwrap();
    assert!(!whole.collapsed);
    assert_eq!(whole.text, long);
    // Expanding it again changes nothing.
    let revision = model.thread(c).unwrap().revision();
    model.expand(one);
    assert_eq!(model.thread(c).unwrap().revision(), revision);
    assert!(model.thread(c).unwrap().prepared(one).is_some());
}

#[test]
fn a_revision_is_never_given_twice() {
    let mut model = ready();
    let mut seen = std::collections::BTreeSet::new();
    for record in [10, 11] {
        let c = contact(1, record);
        model.select_conversation(c);
        answer_all(&mut model, |command| match command {
            Command::LoadMessages { .. } => Some(Outcome::Messages {
                contact: c,
                messages: vec![message(c, 5, Status::Received, "hi")],
                complete: false,
            }),
            _ => None,
        });
        assert!(seen.insert(model.thread(c).unwrap().revision()));
        model.expand(UiMessageId::new(c, 5));
        assert!(seen.insert(model.thread(c).unwrap().revision()));
    }
    // Older messages put above.
    let c = contact(1, 10);
    model.load_earlier(c);
    answer_all(&mut model, |command| match command {
        Command::LoadEarlier { .. } => Some(Outcome::Messages {
            contact: c,
            messages: vec![message(c, 4, Status::Received, "before")],
            complete: true,
        }),
        _ => None,
    });
    assert_eq!(model.thread(c).unwrap().messages.len(), 2);
    assert!(seen.insert(model.thread(c).unwrap().revision()));
    assert!(!seen.contains(&0));
}

#[test]
fn deleted_messages_take_their_prepared_texts_and_the_window_forgets_them() {
    let mut model = ready();
    let c = contact(1, 10);
    model.select_conversation(c);
    messages_of(&mut model, c, vec![message(c, 1, Status::Received, "hi")]);
    model.prepare_messages(c, 0..1);
    assert!(model.take_forgotten().is_empty());
    model.open(Dialog::DeleteConversation {
        contact: c,
        busy: None,
        error: None,
    });
    model.confirm();
    answer_all(&mut model, |command| match command {
        Command::DeleteConversation(_) => Some(Outcome::Done),
        _ => None,
    });
    assert!(
        model
            .thread(c)
            .unwrap()
            .prepared(UiMessageId::new(c, 1))
            .is_none()
    );
    assert_eq!(model.take_forgotten(), [c]);
    assert!(model.take_forgotten().is_empty());
}

#[test]
fn a_contact_or_an_identity_that_goes_is_forgotten_by_the_window() {
    let mut model = ready();
    let d = contact(1, 11);
    model.select_conversation(d);
    model.select_identity(identity(2));
    model.select_conversation(contact(2, 20));
    assert!(model.take_forgotten().is_empty());
    // Dan is removed: identity 1 is read without him.
    model.apply(Update::Event(RuntimeEvent::Missed));
    answer_all(&mut model, |command| match command {
        Command::LoadIdentities => Some(Outcome::Identities(vec![
            identity_view(1, "Alice"),
            identity_view(2, "Work"),
        ])),
        Command::LoadIdentity(id) if *id == identity(1) => {
            Some(Outcome::Identity(Box::new(snapshot(
                identity(1),
                vec![contact_view(contact(1, 10), "Carol", "CCCC 1111")],
            ))))
        }
        _ => None,
    });
    assert_eq!(model.take_forgotten(), [d]);
    // Identity 2 is deleted, with its conversation.
    model.apply(Update::Event(RuntimeEvent::IdentityClosed(identity(2))));
    answer_all(&mut model, |command| match command {
        Command::LoadIdentities => Some(Outcome::Identities(vec![identity_view(1, "Alice")])),
        _ => None,
    });
    assert_eq!(model.take_forgotten(), [contact(2, 20)]);
}

#[test]
fn the_rows_of_the_sidebar_are_made_again_only_when_what_they_show_changes() {
    let mut model = ready();
    let c = contact(1, 10);
    let d = contact(1, 11);
    let rows = model.sidebar_revision();
    assert_ne!(rows, 0);
    // Nothing the rows show changes.
    model.select_conversation(c);
    messages_of(&mut model, c, vec![message(c, 1, Status::Queued, "hi")]);
    model.draft_mut(c).unwrap().push_str("draft");
    model.check_tor();
    answer_all(&mut model, |command| match command {
        Command::CheckTor => Some(Outcome::Tor(monolith_ui_model::TorCheck::NotRunning)),
        _ => None,
    });
    model.open(Dialog::TorDetails);
    model.close();
    model.expand(UiMessageId::new(c, 1));
    model.apply(Update::Event(RuntimeEvent::MessageDelivered(
        UiMessageId::new(c, 1),
    )));
    // An answer to no command in flight, and an event of another identity.
    model.apply(Update::Done {
        command: CommandId::new(u64::MAX),
        outcome: Outcome::Identity(Box::new(snapshot(identity(1), Vec::new()))),
    });
    model.apply(Update::Event(RuntimeEvent::MessageReceived(
        UiMessageId::new(contact(2, 20), 1),
    )));
    assert_eq!(model.sidebar_revision(), rows);
    // The identity read again: new rows, with their previews and names.
    let long = format!("Hello{}\u{202E}", " ".repeat(65_536));
    conversations_of(&mut model, "See you\nat nine", &long);
    let rows = model.sidebar_revision();
    let shown = model.sidebar_rows();
    assert_eq!(shown.len(), 3);
    let carol = shown.iter().find(|row| row.contact == c).unwrap();
    assert_eq!(carol.preview, "See you at nine");
    assert!(carol.named);
    let dan = shown.iter().find(|row| row.contact == d).unwrap();
    assert_eq!(dan.preview, "Hello\u{2026}");
    let unnamed = shown.iter().find(|row| row.name == "Contact FFFF").unwrap();
    assert!(!unnamed.named);
    // By time, then by state, then by name in any case.
    let names: Vec<&str> = shown.iter().map(|row| row.name.as_str()).collect();
    assert_eq!(names, ["Carol", "dan", "Contact FFFF"]);
    // The filter is matched in lower case.
    assert!(dan.matches("da") && carol.matches("car") && carol.matches(""));
    assert!(!carol.matches("dan"));
    // Read again with Carol's newest message changed: her preview follows.
    model.apply(Update::Event(RuntimeEvent::MessageDelivered(
        UiMessageId::new(c, 1),
    )));
    conversations_of(&mut model, "Changed", &long);
    assert_ne!(model.sidebar_revision(), rows);
    let rows = model.sidebar_revision();
    let preview = |model: &Model, contact: UiContactId| {
        model
            .sidebar_rows()
            .iter()
            .find(|row| row.contact == contact)
            .unwrap()
            .preview
            .clone()
    };
    assert_eq!(preview(&model, c), "Changed");
    assert_eq!(preview(&model, d), "Hello\u{2026}");
    // An unread message changes the rows, and reading it too.
    model.apply(Update::Event(RuntimeEvent::MessageReceived(
        UiMessageId::new(d, 2),
    )));
    assert_ne!(model.sidebar_revision(), rows);
    let rows = model.sidebar_revision();
    let unread = |model: &Model| {
        model
            .sidebar_rows()
            .iter()
            .find(|row| row.contact == d)
            .unwrap()
            .unread
    };
    assert_eq!(unread(&model), 1);
    model.select_conversation(d);
    assert_ne!(model.sidebar_revision(), rows);
    assert_eq!(unread(&model), 0);
    // Shown again without unread messages: the same rows.
    let rows = model.sidebar_revision();
    model.select_conversation(c);
    model.select_conversation(d);
    assert_eq!(model.sidebar_revision(), rows);
    // A copy is what the rows are.
    assert_eq!(model.sidebar(), model.sidebar_rows());
}

//! The state of the desktop client under an arbitrary sequence of what the
//! user does and what the runtime says: selecting an identity or a
//! conversation, opening, confirming and closing dialogs, drafts and
//! sends, and every kind of update the bridge makes (`Started`, `Done`
//! with any command number and many outcomes, every `RuntimeEvent`,
//! `Stopped`), with `take_commands` and `take_notices` between them.
//!
//! After every step: the identity shown is one of the identities listed;
//! the conversation selected for an identity belongs to that identity;
//! every command taken carries handles of one identity, is numbered above
//! the commands before it, and uses only handles of the pools below; and
//! the commands and notices of the whole sequence stay within a constant
//! times the number of steps. An answer to a number that was never issued,
//! or already answered, changes nothing: everything the public accessors
//! of the model show (phase, identities, and per identity its contacts,
//! conversations, requests, invitations, threads with their messages and
//! drafts, selection, the open dialog) is the same before and after, and
//! no command or notice follows.
//!
//! Every step is five bytes: the action (modulo 24), then four bytes of
//! arguments. Handles come from small pools: two identities, three
//! contact records each, four messages per contact, three requests and
//! three invitations per identity. Arguments pick from the pools without
//! regard to which identity is shown, so handles of the other identity
//! occur everywhere. An answer to a command picks one of the commands
//! taken so far; its outcome is one of sixteen kinds built from the pools,
//! of the identity or contact the arguments say, which is often not the
//! one the command named.

#![no_main]

use std::collections::BTreeSet;

use libfuzzer_sys::fuzz_target;
use monolith_ui_model::data::{Deferral, Service, Status, TorCheck};
use monolith_ui_model::{
    Command, CommandId, ContactView, ConversationView, Dialog, Failure, IdentitySnapshot,
    IdentityView, InvitationView, MessageView, Model, Outcome, Phase, ProfileOnDisk, RequestMode,
    RequestView, RuntimeEvent, Selection, SidebarRow, TorIndicator, UiContactId, UiIdentityId,
    UiInvitationId, UiMessageId, UiRequestId, Update,
};
use zeroize::Zeroizing;

const STEP: usize = 5;
const MAX_STEPS: usize = 2_000;

fn identity(n: u8) -> UiIdentityId {
    UiIdentityId::from_bytes([1 + n % 2; 16])
}

fn contact(i: u8, k: u8) -> UiContactId {
    UiContactId::new(identity(i), [0x10 + k % 3; 16])
}

fn message(i: u8, k: u8, s: u8) -> UiMessageId {
    UiMessageId::new(contact(i, k), u64::from(s % 4))
}

fn request(i: u8, h: u8) -> UiRequestId {
    UiRequestId::new(identity(i), u64::from(h % 3))
}

fn invitation(i: u8, h: u8) -> UiInvitationId {
    UiInvitationId::new(identity(i), u64::from(h % 3))
}

const NAMES: [Option<&str>; 4] = [None, Some(""), Some("Bob"), Some("\u{202E}Eve")];

const DRAFTS: [&str; 5] = ["", "hello", " \n\t ", "line\nline", "\u{200B}"];

fn identity_view(i: u8, flags: u8) -> IdentityView {
    IdentityView {
        id: identity(i),
        name: NAMES[usize::from(flags & 3)].map(str::to_owned),
        fingerprint: format!("ID{} AAAA BBBB", i % 2),
        full_fingerprint: String::new(),
    }
}

fn contact_view(i: u8, k: u8, flags: u8) -> ContactView {
    ContactView {
        id: contact(i, k),
        name: NAMES[usize::from(flags & 3)].map(str::to_owned),
        fingerprint: format!("C{}{} CCCC", i % 2, k % 3),
        full_fingerprint: String::new(),
        accepted: flags & 4 != 0,
        verified: flags & 8 != 0,
    }
}

fn message_view(i: u8, k: u8, s: u8, flags: u8) -> MessageView {
    let status = match flags % 3 {
        0 => Status::Queued,
        1 => Status::Delivered,
        _ => Status::Received,
    };
    MessageView {
        id: message(i, k, s),
        status,
        time: 1_791_981_000 + u64::from(s),
        text: DRAFTS[usize::from(flags >> 2) % DRAFTS.len()].to_owned(),
    }
}

fn conversation_view(i: u8, k: u8, flags: u8) -> ConversationView {
    ConversationView {
        contact: contact(i, k),
        fingerprint: format!("C{}{} CCCC", i % 2, k % 3),
        active: flags & 1 != 0,
        messages: usize::from(flags >> 1) % 5,
        queued: usize::from(flags >> 4) % 3,
        latest: (flags & 0x80 != 0).then(|| message_view(i, k, flags, flags)),
    }
}

/// A snapshot of identity `i`; each bit of `mask` adds a record, a
/// conversation, a request or an invitation, of the identity read or, for
/// the high bits, of the other one.
fn snapshot(i: u8, mask: u8, flags: u8) -> IdentitySnapshot {
    let owner = |bit: u8| if mask & bit != 0 { i } else { i.wrapping_add(1) };
    let mut contacts = Vec::new();
    let mut conversations = Vec::new();
    let mut requests = Vec::new();
    let mut invitations = Vec::new();
    for k in 0..3_u8 {
        if mask & (1 << k) != 0 {
            contacts.push(contact_view(owner(0x40), k, flags));
        }
        if mask & (1 << (k + 2)) != 0 {
            conversations.push(conversation_view(owner(0x80), k, flags));
        }
    }
    if mask & 0x01 != 0 {
        requests.push(RequestView {
            id: request(owner(0x40), flags),
            fingerprint: "RRRR".to_owned(),
            full_fingerprint: String::new(),
            display_name: "Req".to_owned(),
            introduction: String::new(),
            invitation: (flags & 1 != 0).then(|| invitation(owner(0x40), flags)),
        });
    }
    if mask & 0x02 != 0 {
        invitations.push(InvitationView {
            id: invitation(owner(0x80), flags),
            label: None,
        });
    }
    IdentitySnapshot {
        identity: identity(i),
        contacts,
        conversations,
        requests,
        invitations,
        service: [
            Service::Publishing,
            Service::Available,
            Service::Unavailable,
            Service::NoKey,
            Service::Stopped,
        ][usize::from(flags) % 5],
        request_mode: [
            RequestMode::Invitation,
            RequestMode::Open,
            RequestMode::Closed,
        ][usize::from(flags >> 3) % 3],
    }
}

fn failure(n: u8) -> Failure {
    [
        Failure::WrongPassphrase,
        Failure::UnusablePassphrase,
        Failure::NoSuchIdentity,
        Failure::NoSuchContact,
        Failure::NoSuchRequest,
        Failure::InvalidCard,
        Failure::TooLong,
        Failure::Storage,
    ][usize::from(n) % 8]
}

fn card() -> Zeroizing<String> {
    Zeroizing::new("card".to_owned())
}

fn outcome(kind: u8, a: u8, b: u8) -> Outcome {
    match kind % 16 {
        0 => Outcome::Failed(failure(a)),
        1 => Outcome::Done,
        2 => Outcome::ProfileOpen {
            ephemeral: a & 1 != 0,
            message_store_missing: a & 2 != 0,
        },
        3 => Outcome::Identities(
            (0..2_u8)
                .filter(|n| a & (1 << n) != 0)
                .map(|n| identity_view(n, b >> n))
                .collect(),
        ),
        4 => Outcome::Identity(Box::new(snapshot(a, b, a >> 1))),
        5 => Outcome::Messages {
            contact: contact(a, a >> 1),
            messages: (0..4_u8)
                .filter(|s| b & (1 << s) != 0)
                .map(|s| {
                    let owner = if b & 0x10 != 0 { a } else { a.wrapping_add(b >> 7) };
                    message_view(owner, a >> 1, s, b >> 3)
                })
                .collect(),
            complete: a & 0x80 != 0,
        },
        6 => Outcome::Tor(match a % 4 {
            0 => TorCheck::Ready {
                version: "0.4.8".to_owned(),
                outdated: b & 1 != 0,
            },
            1 => TorCheck::Connecting {
                progress: (b & 1 != 0).then_some(b),
            },
            2 => TorCheck::NotRunning,
            _ => TorCheck::TooOld {
                version: "0.2".to_owned(),
            },
        }),
        7 => Outcome::Card(card()),
        8 => Outcome::IdentityCreated(identity(a)),
        9 => Outcome::ContactAdded(contact(a, b)),
        10 => Outcome::MessageQueued(message(a, b, a >> 4)),
        11 => Outcome::RequestAccepted(contact(a, b)),
        12 => Outcome::InvitationCreated {
            invitation: invitation(a, b),
            card: card(),
        },
        13 => Outcome::Revoked {
            discarded: usize::from(a),
        },
        14 => Outcome::Failed(Failure::NoSuchIdentity),
        _ => Outcome::Failed(failure(b)),
    }
}

fn event(kind: u8, a: u8, b: u8, c: u8) -> RuntimeEvent {
    match kind % 11 {
        0 => RuntimeEvent::MessageQueued(message(a, b, c)),
        1 => RuntimeEvent::MessageReceived(message(a, b, c)),
        2 => RuntimeEvent::MessageDelivered(message(a, b, c)),
        3 => RuntimeEvent::MessageRefused(contact(a, b)),
        4 => RuntimeEvent::DeliveryDeferred {
            contact: contact(a, b),
            reason: [
                Deferral::Tor,
                Deferral::Unreachable,
                Deferral::NoAddress,
                Deferral::NoAcknowledgement,
                Deferral::Busy,
            ][usize::from(c) % 5],
            retry_in: u64::from(c),
        },
        5 => RuntimeEvent::Service {
            identity: identity(a),
            state: [
                Service::Publishing,
                Service::Available,
                Service::Unavailable,
                Service::NoKey,
                Service::Stopped,
            ][usize::from(c) % 5],
        },
        6 => RuntimeEvent::ContactAccepted(contact(a, b)),
        7 => RuntimeEvent::RequestReceived {
            identity: identity(a),
            queued: c & 1 != 0,
        },
        8 => RuntimeEvent::IdentityClosed(identity(a)),
        9 => RuntimeEvent::StorageFailed,
        _ => RuntimeEvent::Missed,
    }
}

fn dialog(kind: u8, a: u8, b: u8) -> Dialog {
    let none = (None, None);
    match kind % 13 {
        0 => Dialog::NewIdentity {
            name: "Name".to_owned(),
            busy: none.0,
            error: none.1,
        },
        1 => Dialog::AddContact {
            identity: identity(a),
            card: " card ".to_owned(),
            name: "Bob".to_owned(),
            busy: none.0,
            error: none.1,
        },
        2 => Dialog::OwnCard {
            identity: identity(a),
            card: None,
            busy: none.0,
            error: none.1,
        },
        3 => Dialog::NewInvitation {
            identity: identity(a),
            label: String::new(),
            created: None,
            busy: none.0,
            error: none.1,
        },
        4 => Dialog::ShowInvitation {
            invitation: invitation(a, b),
            card: None,
            busy: none.0,
            error: none.1,
        },
        5 => Dialog::RevokeInvitation {
            invitation: invitation(a, b),
            discard_requests: b & 0x80 != 0,
            busy: none.0,
            error: none.1,
        },
        6 => Dialog::ContactDetails {
            contact: contact(a, b),
            name: "New name".to_owned(),
            busy: none.0,
            error: none.1,
        },
        7 => Dialog::BlockContact {
            contact: contact(a, b),
            busy: none.0,
            error: none.1,
        },
        8 => Dialog::RemoveContact {
            contact: contact(a, b),
            busy: none.0,
            error: none.1,
        },
        9 => Dialog::DeleteConversation {
            contact: contact(a, b),
            busy: none.0,
            error: none.1,
        },
        10 => Dialog::AcceptRequest {
            request: request(a, b),
            name: String::new(),
            busy: none.0,
            error: none.1,
        },
        11 => Dialog::DeleteIdentity {
            identity: identity(a),
            busy: none.0,
            error: none.1,
        },
        _ => Dialog::TorDetails,
    }
}

/// The identities a command names, from the handles it holds.
fn identities_of(command: &Command) -> Vec<UiIdentityId> {
    match command {
        Command::OpenProfile { .. }
        | Command::CreateProfile { .. }
        | Command::StartWithoutSaving
        | Command::LoadIdentities
        | Command::CheckTor
        | Command::CreateIdentity { .. }
        | Command::Shutdown => Vec::new(),
        Command::LoadIdentity(id)
        | Command::OwnCard(id)
        | Command::DeleteIdentity(id)
        | Command::AddContact { identity: id, .. }
        | Command::CreateInvitation { identity: id, .. }
        | Command::SetRequestMode { identity: id, .. } => vec![*id],
        Command::LoadMessages { contact, .. }
        | Command::LoadEarlier { contact, .. }
        | Command::SetContactName { contact, .. }
        | Command::SetVerified { contact, .. }
        | Command::Send { contact, .. }
        | Command::BlockContact(contact)
        | Command::RemoveContact(contact)
        | Command::DeleteConversation(contact) => vec![contact.identity()],
        Command::InvitationCard(invitation)
        | Command::RevokeInvitation { invitation, .. } => vec![invitation.identity()],
        Command::AcceptRequest { request, .. }
        | Command::DeclineRequest(request)
        | Command::BlockRequest(request) => vec![request.identity()],
    }
}

/// What a thread shows: its messages with their ids, statuses and texts,
/// its draft and the state of its reads and sends.
type ObservedThread = (
    UiContactId,
    Vec<MessageView>,
    bool,
    bool,
    String,
    Option<CommandId>,
    Option<Failure>,
    Vec<UiMessageId>,
    bool,
);

/// What is shown of one identity.
#[derive(Debug, PartialEq)]
struct ObservedSlice {
    identity: UiIdentityId,
    loaded: bool,
    contacts: Vec<ContactView>,
    conversations: Vec<ConversationView>,
    requests: Vec<RequestView>,
    invitations: Vec<InvitationView>,
    service: Option<Service>,
    request_mode: Option<RequestMode>,
    selection: Option<Selection>,
    threads: Vec<ObservedThread>,
    unread: Vec<(UiContactId, u32)>,
    deferrals: Vec<(UiContactId, Deferral)>,
}

/// What is shown of the open dialog: its kind, the identity it belongs
/// to, the command it waits for, its error, and whether it holds a card.
type ObservedDialog = (
    core::mem::Discriminant<Dialog>,
    Option<UiIdentityId>,
    Option<CommandId>,
    Option<Failure>,
    bool,
    Option<u64>,
);

/// Everything the public accessors of the model show, for the identities
/// and requests of the pools.
#[derive(Debug, PartialEq)]
struct Observed {
    phase: Phase,
    location: String,
    ephemeral: bool,
    message_store_missing: bool,
    profile_error: Option<Failure>,
    storage_failed: bool,
    identities: Vec<IdentityView>,
    identities_loaded: bool,
    current: Option<UiIdentityId>,
    selection: Option<Selection>,
    slices: Vec<ObservedSlice>,
    sidebar: Vec<SidebarRow>,
    tor: Option<TorCheck>,
    tor_checking: bool,
    tor_indicator: TorIndicator,
    dialog: Option<ObservedDialog>,
    answering: Vec<bool>,
}

fn observe_dialog(model: &Model) -> Option<ObservedDialog> {
    let dialog = model.dialog()?;
    let (error, card) = match dialog {
        Dialog::OwnCard { card, error, .. } | Dialog::ShowInvitation { card, error, .. } => {
            (*error, card.is_some())
        }
        Dialog::NewInvitation { created, error, .. } => (*error, created.is_some()),
        Dialog::AddContact { card, error, .. } => (*error, !card.is_empty()),
        Dialog::NewIdentity { error, .. }
        | Dialog::RevokeInvitation { error, .. }
        | Dialog::ContactDetails { error, .. }
        | Dialog::BlockContact { error, .. }
        | Dialog::RemoveContact { error, .. }
        | Dialog::DeleteConversation { error, .. }
        | Dialog::AcceptRequest { error, .. }
        | Dialog::DeleteIdentity { error, .. } => (*error, false),
        Dialog::TorDetails => (None, false),
    };
    Some((
        core::mem::discriminant(dialog),
        dialog.identity(),
        dialog.busy(),
        error,
        card,
        model.dialog_instance(),
    ))
}

fn observe(model: &Model) -> Observed {
    let slices = (0..2_u8)
        .filter_map(|n| {
            let slice = model.slice(identity(n))?;
            Some(ObservedSlice {
                identity: identity(n),
                loaded: slice.loaded,
                contacts: slice.contacts.clone(),
                conversations: slice.conversations.clone(),
                requests: slice.requests.clone(),
                invitations: slice.invitations.clone(),
                service: slice.service,
                request_mode: slice.request_mode,
                selection: slice.selection,
                threads: slice
                    .threads
                    .iter()
                    .map(|(contact, thread)| {
                        (
                            *contact,
                            thread.messages.clone(),
                            thread.complete,
                            thread.loading,
                            thread.draft.clone(),
                            thread.sending,
                            thread.send_error,
                            thread.expanded.iter().copied().collect(),
                            thread.refused,
                        )
                    })
                    .collect(),
                unread: slice.unread.iter().map(|(k, v)| (*k, *v)).collect(),
                deferrals: slice.deferrals.iter().map(|(k, v)| (*k, *v)).collect(),
            })
        })
        .collect();
    Observed {
        phase: model.phase(),
        location: model.location().to_owned(),
        ephemeral: model.ephemeral(),
        message_store_missing: model.message_store_missing(),
        profile_error: model.profile_error(),
        storage_failed: model.storage_failed(),
        identities: model.identities().to_vec(),
        identities_loaded: model.identities_loaded(),
        current: model.current(),
        selection: model.selection(),
        slices,
        sidebar: model.sidebar(),
        tor: model.tor_check().cloned(),
        tor_checking: model.tor_checking(),
        tor_indicator: model.tor_indicator(),
        dialog: observe_dialog(model),
        answering: (0..2_u8)
            .flat_map(|i| (0..3_u8).map(move |h| request(i, h)))
            .map(|id| model.answering(id))
            .collect(),
    }
}

/// What must hold after any step.
fn check(model: &Model) {
    if let Some(current) = model.current() {
        assert!(
            model.identities().iter().any(|view| view.id == current),
            "the identity shown is not listed"
        );
    }
    for n in 0..2_u8 {
        let owner = identity(n);
        if let Some(slice) = model.slice(owner) {
            if let Some(Selection::Conversation(selected)) = slice.selection {
                assert_eq!(selected.identity(), owner, "a conversation of another identity");
            }
            for (held, thread) in &slice.threads {
                assert_eq!(held.identity(), owner);
                assert!(
                    thread
                        .messages
                        .iter()
                        .all(|message| message.id.contact() == *held)
                );
            }
        }
    }
    if let Some(Selection::Conversation(selected)) = model.selection() {
        assert_eq!(Some(selected.identity()), model.current());
    }
}

fuzz_target!(|data: &[u8]| {
    let mut model = Model::new();
    // Every command taken so far, and those already answered.
    let mut taken: Vec<CommandId> = Vec::new();
    let mut answered: BTreeSet<CommandId> = BTreeSet::new();
    let mut last: Option<CommandId> = None;
    let mut commands_total = 0_usize;
    let mut notices_total = 0_usize;
    let mut steps = 0_usize;

    for step in data.chunks_exact(STEP).take(MAX_STEPS) {
        steps += 1;
        let (action, a, b, c, d) = (step[0] % 24, step[1], step[2], step[3], step[4]);
        match action {
            0 => model.select_identity(identity(a)),
            1 => model.select_conversation(contact(a, b)),
            2 => model.show_requests(),
            3 => model.open(dialog(c, a, b)),
            4 => model.confirm(),
            5 => model.close(),
            6 => {
                if let Some(draft) = model.draft_mut(contact(a, b)) {
                    *draft = DRAFTS[usize::from(c) % DRAFTS.len()].to_owned();
                    if c & 0x80 != 0 {
                        *draft = "a".repeat(16_385);
                    }
                }
            }
            7 => model.send(contact(a, b)),
            8 => {
                let issued = model.take_commands();
                commands_total += issued.len();
                for (id, command) in issued {
                    assert!(last.is_none_or(|before| id > before), "a number reused");
                    last = Some(id);
                    taken.push(id);
                    let owners = identities_of(&command);
                    assert!(
                        owners.windows(2).all(|pair| pair[0] == pair[1]),
                        "a command of two identities"
                    );
                    assert!(
                        owners.iter().all(|owner| (0..2_u8).any(|n| *owner == identity(n))),
                        "a handle outside the pools"
                    );
                }
            }
            9 => {
                // An answer to a command taken, maybe already answered.
                if !taken.is_empty() {
                    let command = taken[usize::from(a) % taken.len()];
                    answered.insert(command);
                    model.apply(Update::Done {
                        command,
                        outcome: outcome(b, c, d),
                    });
                }
            }
            10 => {
                // An answer to a number never issued, or already
                // answered, changes nothing.
                let issued = model.take_commands();
                commands_total += issued.len();
                for (id, _) in issued {
                    last = Some(id);
                    taken.push(id);
                }
                let _ = model.take_notices();
                let number = last.map_or(0, |id| id.get()).saturating_add(1 + u64::from(a));
                let fresh = CommandId::new(number);
                let command = match answered.iter().nth(usize::from(a) % answered.len().max(1)) {
                    Some(old) if c & 1 != 0 => *old,
                    _ => fresh,
                };
                let before = observe(&model);
                model.apply(Update::Done {
                    command,
                    outcome: outcome(b, c >> 1, d),
                });
                assert!(before == observe(&model), "a stale answer changed the state");
                assert!(model.take_commands().is_empty());
                assert!(model.take_notices().is_empty());
            }
            11 => model.apply(Update::Event(event(b, a, c, d))),
            12 => model.apply(Update::Stopped),
            13 => model.apply(Update::Started {
                profile: if a & 1 != 0 {
                    ProfileOnDisk::Exists
                } else {
                    ProfileOnDisk::Absent
                },
                location: "data".to_owned(),
            }),
            14 => {
                let notices = model.take_notices();
                notices_total += notices.len();
            }
            15 => model.expand(message(a, b, c)),
            16 => model.load_earlier(contact(a, b)),
            17 => model.check_tor(),
            18 => model.unlock(Zeroizing::new("pass".to_owned())),
            19 => model.create_profile(Zeroizing::new(
                ["", " ", "pass"][usize::from(a) % 3].to_owned(),
            )),
            20 => model.start_without_saving(),
            21 => {
                if b & 1 != 0 {
                    model.decline(request(a, c));
                } else {
                    model.block_sender(request(a, c));
                }
                let _ = model.answering(request(a, c));
            }
            22 => {
                if b & 1 != 0 {
                    model.set_request_mode(identity(a), RequestMode::Open);
                } else {
                    model.set_verified(contact(a, c), b & 2 != 0);
                }
            }
            _ => model.shut_down(),
        }
        check(&model);
    }

    // What the sequence made is bounded by what it did.
    let issued = model.take_commands();
    commands_total += issued.len();
    notices_total += model.take_notices().len();
    assert!(commands_total <= 8 * (steps + 1), "commands grew past the steps");
    assert!(notices_total <= 4 * (steps + 1), "notices grew past the steps");
});

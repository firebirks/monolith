//! The UI model under generated sequences of what the user does and what
//! the runtime does (`docs/adr/0011-ui-security-boundary.md`). The runtime
//! is played by an environment that keeps the truth in a model of its own,
//! written here: identities, their contact records, the conversations with
//! their messages and statuses, the file transfers of each record with
//! their states and bytes, requests, invitations and the people each
//! identity blocked. It runs the
//! commands the model issues against that truth and answers them at once,
//! late, in another order, never, or with handles of another identity or
//! record, as the sequence says. It changes the truth by itself (messages
//! arrive and are acknowledged, requests arrive, contacts accept, a record
//! is made again for the same person, an identity goes away, files are
//! offered, accepted, sent, held, ended, and the runtime starts again and
//! no longer knows how far a transfer got) and raises the events of the
//! changes, or loses them; and it raises events that name anything, deleted
//! records and other identities included, and progress said before.
//!
//! After every step:
//!
//! - commands are numbered in order; a command of the user names exactly
//!   what the action named (for a confirmation, what its dialog captured);
//!   answers and events make the model issue reads only, of identities it
//!   lists and conversations it knows;
//! - a transfer action of the user issues its command, naming that
//!   transfer, only for one the model lists, that no command of the model
//!   is busy with, in a state the action applies to; a file is offered only
//!   to a contact the model shows accepted;
//! - the identities listed, and what is shown of each identity, are exactly
//!   an answer the truth gave for that identity, never mixed with another;
//!   a message is shown with a status and a text the truth gave for that
//!   exact handle, and once shown delivered it is never shown queued again;
//! - once no read lists a transfer, it is never listed again; every
//!   transfer the identity shown lists has one card, in the conversation
//!   of its record, that says what the read listing it said, its name drawn
//!   on one line with no control character and no mark that hides text or
//!   turns its direction; bytes shown were said by the runtime for that
//!   handle, are at most its size, and were said after the runtime last
//!   lost track of them in the read shown, whenever their event came; once
//!   shown ended, a transfer is never shown under way again nor in another
//!   ended state, but for a cancelled one the peer then completed; a card
//!   waits only for a command not answered yet;
//! - a selection names a conversation its identity knows, never one whose
//!   deletion the model has read; it changes only by the user, or to a
//!   contact a command of the user made; the identity shown changes only by
//!   the user, to an identity the user made, or when it is gone;
//! - a dialog keeps the handles it was opened with and shows nothing of
//!   another identity; a draft changes only by the user, or goes when the
//!   runtime queued it, and stays while its conversation is known; a dialog
//!   or a draft waits only for a command not answered yet;
//! - an answer that names something its command did not act on is never
//!   told as a success, and an answer to a number nothing waits for
//!   changes nothing.
//!
//! At the end, and where the sequence says, the late answers arrive,
//! `Missed` is raised and every read is answered: the cards of every
//! identity hold as those of the identity shown do, the model shows
//! exactly the truth, the transfers and the bytes of each included, and
//! the conversation shown is read.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use monolith_ui_model::{
    BlockedView, Command, CommandId, ContactView, ConversationView, Deferral, Dialog, Failure,
    IdentitySnapshot, IdentityView, InvitationView, MessageView, Model, NoticeKind, Outcome, Phase,
    ProfileOnDisk, RequestMode, RequestView, RuntimeEvent, Selection, Service, Slice, Status,
    TorCheck, TransferItem, TransferKey, TransferPhase, TransferReason, TransferStatus,
    TransferView, UiBlockedId, UiContactId, UiIdentityId, UiInvitationId, UiMessageId, UiRequestId,
    UiTransferId, Update,
};
use proptest::prelude::*;
use zeroize::Zeroizing;

/// The time of the first message, in seconds since the epoch.
const EPOCH: u64 = 1_791_981_000;

/// The most identities the runtime keeps.
const MAX_IDENTITIES: usize = 4;

/// How many people contacts and requests come from.
const PEOPLE: u8 = 8;

/// Texts of messages and drafts; the last two cannot be sent.
const TEXTS: [&str; 5] = ["hello", "see you at 14:05", "line\nline", "  \n\t ", ""];

const SERVICES: [Service; 5] = [
    Service::Publishing,
    Service::Available,
    Service::Unavailable,
    Service::NoKey,
    Service::Stopped,
];

const MODES: [RequestMode; 3] = [
    RequestMode::Invitation,
    RequestMode::Open,
    RequestMode::Closed,
];

const DEFERRALS: [Deferral; 5] = [
    Deferral::Tor,
    Deferral::Unreachable,
    Deferral::NoAddress,
    Deferral::NoAcknowledgement,
    Deferral::Busy,
];

/// Sizes of files, in bytes.
const SIZES: [u64; 4] = [2, 1_000, 65_536, 64_000_000];

/// What the user drops or types as the file to send: a folder cannot be
/// sent, and blank names nothing.
const PATHS: [&str; 4] = [
    "/home/user/report.pdf",
    "C:\\Users\\user\\photo.jpg",
    "/home/user/folder/",
    "  ",
];

/// What the user types where a received file is to be saved: nothing, a
/// file that exists, a place whose save cannot be confirmed, a new file.
const PLACES: [&str; 4] = [
    "",
    "/home/user/Downloads/exists.zip",
    "/media/usb/copy.bin",
    "/home/user/Documents/copy.bin",
];

/// The name a contact gives a file it offers: its own text, hostile or not.
fn file_name(n: u8) -> String {
    match n % 7 {
        0 => "project-backup.zip".to_owned(),
        1 => "invoice\u{202e}fdp.exe".to_owned(),
        2 => "\u{2066}report\u{2069}\u{200b}.pdf".to_owned(),
        3 => "line\nbreak\tand\r\nreturn.txt".to_owned(),
        4 => "\u{1b}[31mred\u{7}\u{0}.txt".to_owned(),
        5 => "../../.ssh/authorized_keys".to_owned(),
        _ => "long".repeat(20),
    }
}

/// The place the runtime suggests for saving `transfer`: its own.
fn place_of(transfer: UiTransferId) -> String {
    format!(
        "/home/user/Downloads/{:02x}{:02x}-{}",
        transfer.identity().bytes()[0],
        transfer.contact().record()[0],
        transfer.handle()
    )
}

/// Whether `name` is drawn on one line, with no control character and no
/// character that is drawn as nothing or turns the direction of the text
/// around it.
fn drawable(name: &str) -> bool {
    name.chars().all(|character| {
        (character == ' ' || !character.is_whitespace())
            && !character.is_control()
            && !matches!(
                character,
                '\u{061c}'
                    | '\u{200b}'..='\u{200f}'
                    | '\u{2028}'..='\u{202e}'
                    | '\u{2060}'..='\u{206f}'
                    | '\u{feff}'
            )
    })
}

/// Whether a card in `phase` says its transfer ended: it can only be taken
/// off the list.
const fn ended(phase: TransferPhase) -> bool {
    matches!(
        phase,
        TransferPhase::Completed
            | TransferPhase::Saved
            | TransferPhase::Rejected
            | TransferPhase::Expired
            | TransferPhase::Cancelled { .. }
            | TransferPhase::Failed(_)
    )
}

/// Whether a card of a transfer the model read as `view` may say `phase`,
/// from what each state of the runtime means.
fn agrees(phase: TransferPhase, view: &TransferView) -> bool {
    let moving = view.status == TransferStatus::Sending
        || (view.status == TransferStatus::Receiving && !view.held);
    match phase {
        TransferPhase::Waiting => view.status == TransferStatus::Waiting,
        TransferPhase::Offered => view.status == TransferStatus::Offered,
        TransferPhase::Transferring { .. } => moving,
        TransferPhase::Restoring => moving && view.bytes.is_none(),
        TransferPhase::Paused => view.status == TransferStatus::Receiving && view.held,
        TransferPhase::Completed => view.status == TransferStatus::Completed,
        TransferPhase::Received => view.status == TransferStatus::Received && !view.saved,
        TransferPhase::Saved => view.status == TransferStatus::Received && view.saved,
        TransferPhase::Rejected => view.status == TransferStatus::Rejected,
        TransferPhase::Expired => view.status == TransferStatus::Expired,
        TransferPhase::Cancelled { by_us } => {
            view.status == TransferStatus::Cancelled
                && by_us == (view.reason == Some(TransferReason::ByUs))
        }
        TransferPhase::Failed(reason) => {
            view.status == TransferStatus::Failed && reason == view.reason
        }
        TransferPhase::Preparing | TransferPhase::Unsent(_) => false,
    }
}

/// What the user does on a transfer card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    Accept,
    Reject,
    Resume,
    Remove,
    Save,
    Cancel,
}

const ACTS: [Act; 6] = [
    Act::Accept,
    Act::Reject,
    Act::Resume,
    Act::Remove,
    Act::Save,
    Act::Cancel,
];

/// The acts a card in `phase` offers, as the window draws it
/// (`monolith-ui`, `transfers.rs`).
const fn offered(phase: TransferPhase) -> &'static [Act] {
    match phase {
        TransferPhase::Waiting | TransferPhase::Transferring { .. } | TransferPhase::Restoring => {
            &[Act::Cancel]
        }
        TransferPhase::Offered => &[Act::Accept, Act::Reject],
        TransferPhase::Paused => &[Act::Resume, Act::Cancel],
        TransferPhase::Received => &[Act::Save, Act::Remove],
        TransferPhase::Completed
        | TransferPhase::Saved
        | TransferPhase::Rejected
        | TransferPhase::Expired
        | TransferPhase::Cancelled { .. }
        | TransferPhase::Failed(_) => &[Act::Remove],
        TransferPhase::Preparing | TransferPhase::Unsent(_) => &[],
    }
}

/// Whether the act `act`, taken without a dialog, applies to a transfer the
/// model read as `view`: one that does not is refused before the runtime
/// is asked.
fn applies(act: Act, view: &TransferView) -> bool {
    match act {
        Act::Accept | Act::Reject => view.incoming && view.status == TransferStatus::Offered,
        Act::Resume => view.incoming && view.status == TransferStatus::Receiving && view.held,
        Act::Remove => {
            matches!(
                view.status,
                TransferStatus::Completed
                    | TransferStatus::Rejected
                    | TransferStatus::Expired
                    | TransferStatus::Cancelled
                    | TransferStatus::Failed
            ) || (view.status == TransferStatus::Received && view.saved)
        }
        Act::Save | Act::Cancel => false,
    }
}

fn tor_state(n: u8) -> TorCheck {
    match n % 4 {
        0 => TorCheck::Ready {
            version: "0.4.8.13".to_owned(),
            outdated: false,
        },
        1 => TorCheck::Connecting { progress: Some(40) },
        2 => TorCheck::NotRunning,
        _ => TorCheck::SocksUnavailable,
    }
}

fn identity(n: u8) -> UiIdentityId {
    UiIdentityId::from_bytes([n; 16])
}

/// An identity that never existed.
fn stranger() -> UiIdentityId {
    identity(0xEE)
}

fn record(owner: UiIdentityId, n: u8) -> UiContactId {
    UiContactId::new(owner, [n; 16])
}

fn fingerprint(person: u8) -> String {
    format!("P{person}{person}{person}{person} 0000 1111")
}

/// The card of `person`, or text that is not a card.
fn card(person: u8) -> String {
    if person < PEOPLE {
        format!(" card-P{person} ")
    } else {
        "not a card".to_owned()
    }
}

/// Whether the card of `person` is an invitation: the others are plain
/// cards, after which "Add contact" stays open to say so.
const fn invited(person: u8) -> bool {
    person % 2 == 1
}

/// The person whose card `text` is.
fn person_of(text: &str) -> Option<u8> {
    let person: u8 = text.strip_prefix("card-P")?.parse().ok()?;
    (person < PEOPLE).then_some(person)
}

/// The element `pick` of `list`, if it has any.
fn choose<T: Copy>(list: &[T], pick: u8) -> Option<T> {
    (!list.is_empty()).then(|| list[usize::from(pick) % list.len()])
}

/// The records a slice knows: its contacts and its conversations.
fn known(slice: &Slice) -> BTreeSet<UiContactId> {
    slice
        .contacts
        .iter()
        .map(|view| view.id)
        .chain(slice.conversations.iter().map(|view| view.contact))
        .collect()
}

/// A message no runtime said, shown as delivered.
fn forged_message(contact: UiContactId, sequence: u64) -> MessageView {
    MessageView {
        id: UiMessageId::new(contact, sequence),
        status: Status::Delivered,
        time: EPOCH,
        text: "forged".to_owned(),
    }
}

fn forged_contact(contact: UiContactId) -> ContactView {
    ContactView {
        id: contact,
        name: Some("Mallory".to_owned()),
        fingerprint: fingerprint(9),
        full_fingerprint: String::new(),
        accepted: true,
        verified: true,
        invitation: false,
    }
}

fn forged_conversation(contact: UiContactId) -> ConversationView {
    ConversationView {
        contact,
        fingerprint: fingerprint(9),
        active: true,
        messages: 1,
        queued: 0,
        latest: Some(forged_message(contact, 1)),
    }
}

fn forged_request(owner: UiIdentityId) -> RequestView {
    RequestView {
        id: UiRequestId::new(owner, 900),
        fingerprint: fingerprint(9),
        full_fingerprint: String::new(),
        display_name: "Mallory".to_owned(),
        introduction: String::new(),
        invitation: None,
    }
}

/// A transfer no runtime said, shown as received.
fn forged_transfer(contact: UiContactId) -> TransferView {
    TransferView {
        id: UiTransferId::new(contact, 905),
        incoming: true,
        status: TransferStatus::Received,
        reason: None,
        size: 1,
        name: "forged".to_owned(),
        bytes: Some(1),
        created: EPOCH,
        held: false,
        saved: false,
    }
}

/// A snapshot of an identity that holds nothing.
fn empty_snapshot(owner: UiIdentityId) -> IdentitySnapshot {
    IdentitySnapshot {
        identity: owner,
        contacts: Vec::new(),
        conversations: Vec::new(),
        requests: Vec::new(),
        invitations: Vec::new(),
        blocked: Vec::new(),
        service: Service::Stopped,
        request_mode: RequestMode::Closed,
        transfers: Vec::new(),
        interrupted: Vec::new(),
    }
}

// ---- The truth ----

/// A message, as the runtime holds it.
#[derive(Clone, Debug)]
struct Message {
    sequence: u64,
    status: Status,
    time: u64,
    text: String,
}

impl Message {
    fn view(&self, contact: UiContactId) -> MessageView {
        MessageView {
            id: UiMessageId::new(contact, self.sequence),
            status: self.status,
            time: self.time,
            text: self.text.clone(),
        }
    }
}

/// A file transfer, as the runtime holds it (`monolith-core`,
/// `transfers.rs`).
#[derive(Clone, Debug)]
struct Transfer {
    id: UiTransferId,
    incoming: bool,
    status: TransferStatus,
    reason: Option<TransferReason>,
    size: u64,
    name: String,
    /// The bytes acknowledged or staged: they only grow.
    staged: u64,
    /// What the runtime knows of them: nothing after it started again, or
    /// after a hold, until the transfer goes on.
    bytes: Option<u64>,
    /// How many times the runtime lost track of the bytes.
    resets: u32,
    created: u64,
    held: bool,
    saved: bool,
}

impl Transfer {
    fn view(&self) -> TransferView {
        TransferView {
            id: self.id,
            incoming: self.incoming,
            status: self.status,
            reason: self.reason,
            size: self.size,
            name: self.name.clone(),
            bytes: self.bytes,
            created: self.created,
            held: self.held,
            saved: self.saved,
        }
    }

    /// On its way: acknowledged or staged bit by bit.
    fn moving(&self) -> bool {
        matches!(
            self.status,
            TransferStatus::Sending | TransferStatus::Receiving
        )
    }

    /// Ended, as the runtime counts it: it can be removed.
    fn finished(&self) -> bool {
        matches!(
            self.status,
            TransferStatus::Completed
                | TransferStatus::Received
                | TransferStatus::Rejected
                | TransferStatus::Expired
                | TransferStatus::Cancelled
                | TransferStatus::Failed
        )
    }

    /// Ends it in `status`; a hold ends with it.
    fn end(&mut self, status: TransferStatus, reason: Option<TransferReason>) {
        self.status = status;
        self.reason = reason;
        self.held = false;
    }

    /// The runtime no longer knows how far it got.
    fn lose_track(&mut self) {
        if self.bytes.is_some() {
            self.bytes = None;
            self.resets += 1;
        }
    }
}

/// A contact record: a contact while `current`; after that an ended
/// conversation while it holds messages, and nothing once it holds neither
/// messages nor transfers.
#[derive(Clone, Debug)]
struct Record {
    id: UiContactId,
    person: u8,
    name: Option<String>,
    accepted: bool,
    verified: bool,
    current: bool,
    messages: Vec<Message>,
    /// The sequence of the next message: none is given twice.
    next: u64,
    transfers: Vec<Transfer>,
}

impl Record {
    fn exists(&self) -> bool {
        self.current || !self.messages.is_empty() || !self.transfers.is_empty()
    }

    /// The contact ended: blocked, what is not ended is cancelled and a
    /// received file not saved goes; removed, every transfer goes. Returns
    /// the transfers changed.
    fn end_transfers(&mut self, removed: bool) -> Vec<UiTransferId> {
        let mut changed = Vec::new();
        for held in &mut self.transfers {
            let ends = matches!(
                (held.incoming, held.status),
                (false, TransferStatus::Waiting | TransferStatus::Sending)
                    | (true, TransferStatus::Offered | TransferStatus::Receiving)
            );
            if removed || ends || (held.status == TransferStatus::Received && !held.saved) {
                changed.push(held.id);
            }
            if ends && !removed {
                held.end(
                    TransferStatus::Cancelled,
                    Some(TransferReason::ContactEnded),
                );
            }
        }
        self.transfers
            .retain(|held| !removed && (held.status != TransferStatus::Received || held.saved));
        changed
    }

    fn contact(&self) -> ContactView {
        ContactView {
            id: self.id,
            name: self.name.clone(),
            fingerprint: fingerprint(self.person),
            full_fingerprint: format!("{} 2222 3333", fingerprint(self.person)),
            accepted: self.accepted,
            verified: self.verified,
            invitation: invited(self.person),
        }
    }

    fn conversation(&self) -> ConversationView {
        ConversationView {
            contact: self.id,
            fingerprint: fingerprint(self.person),
            active: self.current,
            messages: self.messages.len(),
            queued: self
                .messages
                .iter()
                .filter(|message| message.status == Status::Queued)
                .count(),
            latest: self.messages.last().map(|message| message.view(self.id)),
        }
    }
}

#[derive(Clone, Debug)]
struct Request {
    id: UiRequestId,
    person: u8,
    invitation: Option<UiInvitationId>,
}

#[derive(Clone, Debug)]
struct Invitation {
    id: UiInvitationId,
    label: Option<String>,
}

/// Someone an identity blocked: the runtime keeps their key only.
#[derive(Clone, Debug)]
struct Blocked {
    id: UiBlockedId,
    person: u8,
}

#[derive(Clone, Debug)]
struct Identity {
    id: UiIdentityId,
    name: Option<String>,
    records: Vec<Record>,
    requests: Vec<Request>,
    invitations: Vec<Invitation>,
    blocked: Vec<Blocked>,
    service: Service,
    mode: RequestMode,
    /// The saves cut short when the runtime last started.
    interrupted: Vec<UiTransferId>,
}

impl Identity {
    fn view(&self) -> IdentityView {
        IdentityView {
            id: self.id,
            name: self.name.clone(),
            fingerprint: format!("ID{:02x} AAAA BBBB", self.id.bytes()[0]),
            full_fingerprint: String::new(),
        }
    }

    fn snapshot(&self) -> IdentitySnapshot {
        IdentitySnapshot {
            identity: self.id,
            contacts: self
                .records
                .iter()
                .filter(|found| found.current)
                .map(Record::contact)
                .collect(),
            conversations: self
                .records
                .iter()
                .filter(|found| !found.messages.is_empty())
                .map(Record::conversation)
                .collect(),
            requests: self
                .requests
                .iter()
                .map(|request| RequestView {
                    id: request.id,
                    fingerprint: fingerprint(request.person),
                    full_fingerprint: String::new(),
                    display_name: format!("Person {}", request.person),
                    introduction: "Hello".to_owned(),
                    invitation: request.invitation,
                })
                .collect(),
            invitations: self
                .invitations
                .iter()
                .map(|invitation| InvitationView {
                    id: invitation.id,
                    label: invitation.label.clone(),
                })
                .collect(),
            blocked: self
                .blocked
                .iter()
                .map(|blocked| BlockedView {
                    id: blocked.id,
                    fingerprint: fingerprint(blocked.person),
                    full_fingerprint: format!("{} 2222 3333", fingerprint(blocked.person)),
                })
                .collect(),
            service: self.service,
            request_mode: self.mode,
            transfers: {
                let mut views: Vec<TransferView> = self.transfers().map(Transfer::view).collect();
                views.sort_by_key(|view| (view.created, view.id));
                views
            },
            interrupted: self.interrupted.clone(),
        }
    }

    fn blocks(&self, person: u8) -> bool {
        self.blocked.iter().any(|held| held.person == person)
    }

    fn record(&self, contact: UiContactId) -> Option<&Record> {
        self.records.iter().find(|found| found.id == contact)
    }

    fn transfers(&self) -> impl Iterator<Item = &Transfer> {
        self.records.iter().flat_map(|found| &found.transfers)
    }
}

/// What a slice shows of its identity, as a snapshot gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Part {
    contacts: Vec<ContactView>,
    conversations: Vec<ConversationView>,
    requests: Vec<RequestView>,
    invitations: Vec<InvitationView>,
    blocked: Vec<BlockedView>,
    mode: RequestMode,
    transfers: Vec<TransferView>,
    interrupted: Vec<UiTransferId>,
}

impl Part {
    fn of(snapshot: &IdentitySnapshot) -> Self {
        Self {
            contacts: snapshot.contacts.clone(),
            conversations: snapshot.conversations.clone(),
            requests: snapshot.requests.clone(),
            invitations: snapshot.invitations.clone(),
            blocked: snapshot.blocked.clone(),
            mode: snapshot.request_mode,
            transfers: snapshot.transfers.clone(),
            interrupted: snapshot.interrupted.clone(),
        }
    }

    fn shown(slice: &Slice) -> Option<Self> {
        Some(Self {
            contacts: slice.contacts.clone(),
            conversations: slice.conversations.clone(),
            requests: slice.requests.clone(),
            invitations: slice.invitations.clone(),
            blocked: slice.blocked.clone(),
            mode: slice.request_mode?,
            transfers: slice.transfers.clone(),
            interrupted: slice.interrupted.clone(),
        })
    }
}

/// Everything the truth answered, or was in, for the checks.
#[derive(Default)]
struct Said {
    /// Lists of identities answered, each sorted by handle.
    lists: Vec<Vec<IdentityView>>,
    /// What the snapshots answered for each identity showed.
    parts: BTreeMap<UiIdentityId, Vec<Part>>,
    /// The statuses answered for each message.
    statuses: BTreeSet<(UiMessageId, Status)>,
    /// Every state the service of each identity was in.
    services: BTreeMap<UiIdentityId, Vec<Service>>,
    /// Every state Tor was in.
    tor: Vec<TorCheck>,
    /// The bytes said of each transfer, by a read or an event, with the
    /// times the runtime had lost track of them then (the most, if said
    /// more than once).
    bytes: BTreeMap<(UiTransferId, u64), u32>,
    /// The failures answered to offers of a file, with their contact.
    unsent: Vec<(UiContactId, Failure)>,
}

impl Said {
    fn list(&mut self, list: &[IdentityView]) {
        if !self.lists.iter().any(|said| said == list) {
            self.lists.push(list.to_vec());
        }
    }

    fn snapshot(&mut self, snapshot: &IdentitySnapshot) {
        self.messages(
            snapshot
                .conversations
                .iter()
                .filter_map(|conversation| conversation.latest.as_ref()),
        );
        let part = Part::of(snapshot);
        let parts = self.parts.entry(snapshot.identity).or_default();
        if !parts.contains(&part) {
            parts.push(part);
        }
    }

    fn messages<'a>(&mut self, messages: impl IntoIterator<Item = &'a MessageView>) {
        for message in messages {
            self.statuses.insert((message.id, message.status));
        }
    }

    fn service(&mut self, owner: UiIdentityId, state: Service) {
        let states = self.services.entry(owner).or_default();
        if !states.contains(&state) {
            states.push(state);
        }
    }

    fn tor(&mut self, check: &TorCheck) {
        if !self.tor.contains(check) {
            self.tor.push(check.clone());
        }
    }

    fn bytes(&mut self, transfer: &Transfer) {
        if let Some(bytes) = transfer.bytes {
            let resets = self.bytes.entry((transfer.id, bytes)).or_insert(0);
            *resets = (*resets).max(transfer.resets);
        }
    }
}

// ---- The environment: the runtime and the bridge ----

/// How the environment answers a command it runs.
#[derive(Clone, Copy, Debug)]
enum Fate {
    /// At once.
    Now,
    /// Later, when the sequence says, in any order.
    Hold,
    /// Never.
    Drop,
    /// At once, with handles of another identity or record in the answer.
    Foreign(u8),
    /// It fails without running.
    Fail,
}

/// What an answer, once applied, has told the model is gone.
#[derive(Clone, Debug)]
enum Tells {
    Nothing,
    Records {
        owner: UiIdentityId,
        gone: Vec<UiContactId>,
    },
    Identities(Vec<UiIdentityId>),
}

/// What running a command did.
struct Ran {
    outcome: Outcome,
    tells: Tells,
    events: Vec<RuntimeEvent>,
    /// For a read of an identity, how many times the runtime had lost
    /// track of the bytes of each of its transfers when it was made.
    resets: BTreeMap<UiTransferId, u32>,
}

impl Ran {
    fn new(outcome: Outcome) -> Self {
        Self {
            outcome,
            tells: Tells::Nothing,
            events: Vec::new(),
            resets: BTreeMap::new(),
        }
    }
}

/// An answer, with the command it answers.
#[derive(Debug)]
struct Answer {
    id: CommandId,
    command: Command,
    outcome: Outcome,
    tells: Tells,
    /// It names something its command did not act on.
    foreign: bool,
    resets: BTreeMap<UiTransferId, u32>,
}

/// A change of the truth the runtime makes by itself.
#[derive(Clone, Copy, Debug)]
enum Change {
    /// A message arrives for a contact who accepted.
    Incoming { pick: u8, text: u8 },
    /// A queued message is acknowledged.
    Acknowledged(u8),
    /// A requested contact accepts.
    Accepts(u8),
    /// A request arrives.
    Request { pick: u8, person: u8 },
    /// A contact is removed and made again, as a new record of the same
    /// person.
    MadeAgain(u8),
    /// The service of an identity changes state.
    Service { pick: u8, state: u8 },
    /// An identity is deleted.
    IdentityGone(u8),
    /// The system Tor changes state; nothing tells the model.
    Tor(u8),
    /// A contact who accepted offers a file.
    Offer { pick: u8, file: u8 },
    /// The peer accepts a file offered to it.
    PeerAccepts(u8),
    /// A transfer under way moves on by a share of its size.
    Progress { pick: u8, share: u8 },
    /// The peer confirms the whole file, or the whole file is here and
    /// verified; also for a cancellation the peer had not read.
    Completes(u8),
    /// The peer rejects or cancels a transfer.
    PeerEnds { pick: u8, reject: bool },
    /// A transfer fails.
    Fails { pick: u8, reason: u8 },
    /// An offer is not answered in time.
    Expires(u8),
    /// The writes of a file being received wait for room, or no longer do.
    Hold(u8),
    /// The runtime starts again: it no longer knows how far transfers got,
    /// and a save may have been cut short.
    Restart(u8),
}

struct Env {
    identities: Vec<Identity>,
    tor: TorCheck,
    said: Said,
    /// The time and text of every message; they never change.
    bodies: BTreeMap<UiMessageId, (u64, String)>,
    /// Commands taken from the model and not run, oldest first.
    inbox: Vec<(CommandId, Command)>,
    /// Answers held back.
    held: Vec<Answer>,
    /// Commands whose answer never comes.
    lost: Vec<Command>,
    /// Commands the model waits for: taken and not answered.
    waiting: BTreeSet<CommandId>,
    /// Commands answered.
    answered: BTreeSet<CommandId>,
    /// The runtime stopped.
    stopped: bool,
    identities_ever: Vec<UiIdentityId>,
    contacts_ever: Vec<UiContactId>,
    requests_ever: Vec<UiRequestId>,
    invitations_ever: Vec<UiInvitationId>,
    blocked_ever: Vec<UiBlockedId>,
    messages_ever: Vec<UiMessageId>,
    transfers_ever: Vec<UiTransferId>,
    /// How many times the runtime lost track of the bytes of each transfer
    /// ever made.
    resets: BTreeMap<UiTransferId, u32>,
    /// For the read of each identity the model applied last, how many
    /// times the runtime had lost track of the bytes of each transfer when
    /// it was made, and the number of the read.
    read_resets: BTreeMap<UiIdentityId, (CommandId, BTreeMap<UiTransferId, u32>)>,
    /// The commands taken that act on a transfer, and the transfer.
    on_transfer: BTreeMap<CommandId, UiTransferId>,
    /// The offers of a file taken, and their contact.
    offers: BTreeMap<CommandId, UiContactId>,
    /// Identities a command of the user made.
    made: BTreeSet<UiIdentityId>,
    /// Contacts a command of the user made, by adding or accepting.
    opened: BTreeSet<UiContactId>,
    /// Records and identities an answer applied by the model said are gone.
    heard_gone_records: BTreeSet<UiContactId>,
    heard_gone_identities: BTreeSet<UiIdentityId>,
    next_identity: u8,
    next_record: u8,
    next_handle: u64,
    clock: u64,
}

impl Env {
    /// Alice and Bob. Alice has Carol, with a conversation, a file Carol
    /// offers under a hostile name, one being sent to her and one received
    /// from her; Dan, who has not answered a file offered to him, a file
    /// from whom is being received and was when the runtime last started,
    /// and another whose writes wait for room; and Erin, who has not
    /// accepted yet. Alice has an invitation and a request through it, and
    /// has blocked someone. Bob has Frank, the same person as Carol, with a
    /// conversation, a file received whose save was cut short and one
    /// sent, an ended conversation with a cancelled file, an invitation and
    /// a request.
    fn new() -> Self {
        let mut env = Self {
            identities: Vec::new(),
            tor: tor_state(0),
            said: Said::default(),
            bodies: BTreeMap::new(),
            inbox: Vec::new(),
            held: Vec::new(),
            lost: Vec::new(),
            waiting: BTreeSet::new(),
            answered: BTreeSet::new(),
            stopped: false,
            identities_ever: Vec::new(),
            contacts_ever: Vec::new(),
            requests_ever: Vec::new(),
            invitations_ever: Vec::new(),
            blocked_ever: Vec::new(),
            messages_ever: Vec::new(),
            transfers_ever: Vec::new(),
            resets: BTreeMap::new(),
            read_resets: BTreeMap::new(),
            on_transfer: BTreeMap::new(),
            offers: BTreeMap::new(),
            made: BTreeSet::new(),
            opened: BTreeSet::new(),
            heard_gone_records: BTreeSet::new(),
            heard_gone_identities: BTreeSet::new(),
            next_identity: 1,
            next_record: 0x10,
            next_handle: 1,
            clock: 0,
        };
        let tor = env.tor.clone();
        env.said.tor(&tor);
        let alice = env.add_identity(
            Some("Alice".to_owned()),
            Service::Available,
            RequestMode::Invitation,
        );
        let carol = env.add_record(alice, 0, true, Some("Carol".to_owned()));
        env.add_message(carol, Status::Received, TEXTS[0]);
        env.add_message(carol, Status::Queued, TEXTS[1]);
        env.add_transfer(carol, true, SIZES[2], file_name(1));
        let sending = env.add_transfer(carol, false, SIZES[1], "notes.txt".to_owned());
        let held = env.transfer_mut(sending).unwrap();
        held.status = TransferStatus::Sending;
        held.staged = 200;
        held.bytes = Some(200);
        let received = env.add_transfer(carol, true, SIZES[1], file_name(2));
        let held = env.transfer_mut(received).unwrap();
        held.status = TransferStatus::Received;
        held.staged = SIZES[1];
        held.bytes = Some(SIZES[1]);
        let dan = env.add_record(alice, 1, true, Some("Dan".to_owned()));
        env.add_transfer(dan, false, SIZES[0], "slides.pdf".to_owned());
        let receiving = env.add_transfer(dan, true, SIZES[3], file_name(0));
        let held = env.transfer_mut(receiving).unwrap();
        held.status = TransferStatus::Receiving;
        held.staged = 1_000_000;
        held.resets = 1;
        held.bytes = None;
        let paused = env.add_transfer(dan, true, SIZES[2], file_name(6));
        let held = env.transfer_mut(paused).unwrap();
        held.status = TransferStatus::Receiving;
        held.staged = 4_096;
        held.resets = 1;
        held.bytes = None;
        held.held = true;
        env.add_record(alice, 2, false, Some("Erin".to_owned()));
        let invitation = env.add_invitation(alice, Some("Friends".to_owned()));
        env.add_request(alice, 3, Some(invitation));
        env.block(alice, 4);
        let bob = env.add_identity(
            Some("Bob".to_owned()),
            Service::Publishing,
            RequestMode::Open,
        );
        let frank = env.add_record(bob, 0, true, Some("Frank".to_owned()));
        env.add_message(frank, Status::Delivered, TEXTS[2]);
        let received = env.add_transfer(frank, true, SIZES[0], file_name(3));
        let held = env.transfer_mut(received).unwrap();
        held.status = TransferStatus::Received;
        held.staged = SIZES[0];
        held.bytes = Some(SIZES[0]);
        env.find_mut(bob).unwrap().interrupted.push(received);
        let sent = env.add_transfer(frank, false, SIZES[1], "minutes.odt".to_owned());
        let held = env.transfer_mut(sent).unwrap();
        held.status = TransferStatus::Completed;
        held.staged = SIZES[1];
        held.bytes = Some(SIZES[1]);
        let ended = env.add_record(bob, 3, true, None);
        env.add_message(ended, Status::Received, TEXTS[0]);
        let cancelled = env.add_transfer(ended, true, SIZES[1], file_name(5));
        env.transfer_mut(cancelled).unwrap().end(
            TransferStatus::Cancelled,
            Some(TransferReason::ContactEnded),
        );
        env.record_mut(ended).unwrap().current = false;
        env.add_invitation(bob, None);
        env.add_request(bob, 1, None);
        env
    }

    fn find(&self, owner: UiIdentityId) -> Option<&Identity> {
        self.identities.iter().find(|found| found.id == owner)
    }

    fn find_mut(&mut self, owner: UiIdentityId) -> Option<&mut Identity> {
        self.identities.iter_mut().find(|found| found.id == owner)
    }

    fn record_mut(&mut self, contact: UiContactId) -> Option<&mut Record> {
        self.find_mut(contact.identity())?
            .records
            .iter_mut()
            .find(|found| found.id == contact)
    }

    fn transfer_mut(&mut self, transfer: UiTransferId) -> Option<&mut Transfer> {
        self.record_mut(transfer.contact())?
            .transfers
            .iter_mut()
            .find(|held| held.id == transfer)
    }

    /// Records that are no contact and hold no message nor transfer are
    /// gone; a save cut short is told while its transfer is listed.
    fn tidy(&mut self, owner: UiIdentityId) {
        if let Some(found) = self.find_mut(owner) {
            found.records.retain(Record::exists);
            let listed: BTreeSet<UiTransferId> = found.transfers().map(|held| held.id).collect();
            found
                .interrupted
                .retain(|transfer| listed.contains(transfer));
        }
    }

    fn list(&self) -> Vec<IdentityView> {
        let mut list: Vec<IdentityView> = self.identities.iter().map(Identity::view).collect();
        list.sort_by_key(|view| view.id);
        list
    }

    fn records(&self) -> impl Iterator<Item = &Record> {
        self.identities.iter().flat_map(|found| &found.records)
    }

    /// A transfer of any identity that passes `test`.
    fn pick_transfer(&self, pick: u8, test: impl Fn(&Transfer) -> bool) -> Option<UiTransferId> {
        let found: Vec<UiTransferId> = self
            .records()
            .flat_map(|found| &found.transfers)
            .filter(|held| test(held))
            .map(|held| held.id)
            .collect();
        choose(&found, pick)
    }

    /// Every transfer handle there ever was, and some never made: of a
    /// record that never was, of another identity, and the handle of a
    /// transfer named with another record.
    fn transfer_pool(&self) -> Vec<UiTransferId> {
        let mut pool = self.transfers_ever.clone();
        pool.extend([
            UiTransferId::new(record(identity(1), 0xEE), 900),
            UiTransferId::new(record(stranger(), 0xEE), 901),
        ]);
        if let Some(first) = self.transfers_ever.first() {
            pool.extend([
                UiTransferId::new(record(first.identity(), 0xEE), first.handle()),
                UiTransferId::new(record(identity(2), 0x10), first.handle()),
            ]);
        }
        pool
    }

    /// A transfer handle other than `transfer`.
    fn other_transfer(&self, transfer: UiTransferId, k: u8) -> UiTransferId {
        let pool: Vec<UiTransferId> = self
            .transfer_pool()
            .into_iter()
            .filter(|other| *other != transfer)
            .collect();
        choose(&pool, k).unwrap()
    }

    fn add_identity(
        &mut self,
        name: Option<String>,
        service: Service,
        mode: RequestMode,
    ) -> UiIdentityId {
        let id = identity(self.next_identity);
        self.next_identity += 1;
        self.identities.push(Identity {
            id,
            name,
            records: Vec::new(),
            requests: Vec::new(),
            invitations: Vec::new(),
            blocked: Vec::new(),
            service,
            mode,
            interrupted: Vec::new(),
        });
        self.identities_ever.push(id);
        self.said.service(id, service);
        id
    }

    fn add_record(
        &mut self,
        owner: UiIdentityId,
        person: u8,
        accepted: bool,
        name: Option<String>,
    ) -> UiContactId {
        let id = record(owner, self.next_record);
        self.next_record += 1;
        self.find_mut(owner).unwrap().records.push(Record {
            id,
            person,
            name,
            accepted,
            verified: false,
            current: true,
            messages: Vec::new(),
            next: 1,
            transfers: Vec::new(),
        });
        self.contacts_ever.push(id);
        id
    }

    /// A file offered to `contact`, or by it if `incoming`, under a handle
    /// never given before: waiting for an answer, nothing sent yet.
    fn add_transfer(
        &mut self,
        contact: UiContactId,
        incoming: bool,
        size: u64,
        name: String,
    ) -> UiTransferId {
        let id = UiTransferId::new(contact, self.next_handle);
        self.next_handle += 1;
        self.clock += 1;
        let created = EPOCH + self.clock;
        self.record_mut(contact).unwrap().transfers.push(Transfer {
            id,
            incoming,
            status: if incoming {
                TransferStatus::Offered
            } else {
                TransferStatus::Waiting
            },
            reason: None,
            size,
            name,
            staged: 0,
            bytes: Some(0),
            resets: 0,
            created,
            held: false,
            saved: false,
        });
        self.transfers_ever.push(id);
        id
    }

    /// Keeps how many times the runtime lost track of the bytes of each
    /// transfer, also once it is gone.
    fn note_resets(&mut self) {
        let resets: Vec<(UiTransferId, u32)> = self
            .records()
            .flat_map(|found| &found.transfers)
            .map(|held| (held.id, held.resets))
            .collect();
        self.resets.extend(resets);
    }

    fn add_message(&mut self, contact: UiContactId, status: Status, text: &str) -> UiMessageId {
        self.clock += 1;
        let time = EPOCH + self.clock;
        let found = self.record_mut(contact).unwrap();
        let sequence = found.next;
        found.next += 1;
        found.messages.push(Message {
            sequence,
            status,
            time,
            text: text.to_owned(),
        });
        let id = UiMessageId::new(contact, sequence);
        self.bodies.insert(id, (time, text.to_owned()));
        self.messages_ever.push(id);
        id
    }

    fn add_request(
        &mut self,
        owner: UiIdentityId,
        person: u8,
        invitation: Option<UiInvitationId>,
    ) -> UiRequestId {
        let id = UiRequestId::new(owner, self.next_handle);
        self.next_handle += 1;
        self.find_mut(owner).unwrap().requests.push(Request {
            id,
            person,
            invitation,
        });
        self.requests_ever.push(id);
        id
    }

    fn add_invitation(&mut self, owner: UiIdentityId, label: Option<String>) -> UiInvitationId {
        let id = UiInvitationId::new(owner, self.next_handle);
        self.next_handle += 1;
        self.find_mut(owner)
            .unwrap()
            .invitations
            .push(Invitation { id, label });
        self.invitations_ever.push(id);
        id
    }

    /// `owner` blocks `person`: a request of theirs goes, and nothing of
    /// theirs is taken from then on.
    fn block(&mut self, owner: UiIdentityId, person: u8) {
        let id = UiBlockedId::new(owner, self.next_handle);
        let Some(found) = self.find_mut(owner) else {
            return;
        };
        found.requests.retain(|request| request.person != person);
        if found.blocks(person) {
            return;
        }
        found.blocked.push(Blocked { id, person });
        self.next_handle += 1;
        self.blocked_ever.push(id);
    }

    fn take_request(&mut self, request: UiRequestId) -> Option<Request> {
        let found = self.find_mut(request.identity())?;
        let index = found.requests.iter().position(|held| held.id == request)?;
        Some(found.requests.remove(index))
    }

    // ---- Running commands ----

    /// Runs `command` against the truth.
    fn execute(&mut self, command: &Command) -> Ran {
        let mut ran = Ran::new(Outcome::Done);
        ran.outcome = match command {
            Command::LoadIdentities => {
                let list = self.list();
                self.said.list(&list);
                ran.tells = Tells::Identities(
                    self.identities_ever
                        .iter()
                        .copied()
                        .filter(|id| self.find(*id).is_none())
                        .collect(),
                );
                Outcome::Identities(list)
            }
            Command::LoadIdentity(asked) => match self.find(*asked) {
                Some(found) => {
                    let snapshot = found.snapshot();
                    let gone = self
                        .contacts_ever
                        .iter()
                        .copied()
                        .filter(|contact| {
                            contact.identity() == *asked && found.record(*contact).is_none()
                        })
                        .collect();
                    ran.tells = Tells::Records {
                        owner: *asked,
                        gone,
                    };
                    ran.resets = found
                        .transfers()
                        .map(|held| (held.id, held.resets))
                        .collect();
                    let read: Vec<Transfer> = found.transfers().cloned().collect();
                    for held in &read {
                        self.said.bytes(held);
                    }
                    self.said.snapshot(&snapshot);
                    Outcome::Identity(Box::new(snapshot))
                }
                None => Outcome::Failed(Failure::NoSuchIdentity),
            },
            Command::LoadMessages { contact, limit } => self.read(*contact, None, *limit),
            Command::LoadEarlier {
                contact,
                before,
                limit,
            } => self.read(*contact, Some(*before), *limit),
            Command::CheckTor => Outcome::Tor(self.tor.clone()),
            Command::OwnCard(owner) => {
                if self.find(*owner).is_some() {
                    Outcome::Card(Zeroizing::new(format!("card of {owner:?}")))
                } else {
                    Outcome::Failed(Failure::NoSuchIdentity)
                }
            }
            Command::InvitationCard(invitation) => {
                let active = self.find(invitation.identity()).is_some_and(|found| {
                    found.invitations.iter().any(|held| held.id == *invitation)
                });
                if active {
                    Outcome::Card(Zeroizing::new("invitation card".to_owned()))
                } else {
                    Outcome::Failed(Failure::NoSuchInvitation)
                }
            }
            Command::CreateIdentity { name } => {
                if self.identities.len() >= MAX_IDENTITIES {
                    Outcome::Failed(Failure::Full)
                } else {
                    let made = self.add_identity(
                        name.clone(),
                        Service::Publishing,
                        RequestMode::Invitation,
                    );
                    self.made.insert(made);
                    Outcome::IdentityCreated(made)
                }
            }
            Command::DeleteIdentity(owner) => {
                if self.find(*owner).is_some() {
                    self.identities.retain(|found| found.id != *owner);
                    ran.events.push(RuntimeEvent::IdentityClosed(*owner));
                    Outcome::Done
                } else {
                    Outcome::Failed(Failure::NoSuchIdentity)
                }
            }
            Command::AddContact {
                identity: owner,
                card: text,
                name,
            } => self.add_contact(*owner, text, name.clone()),
            Command::SetContactName { contact, name } => {
                self.change_contact(*contact, |found| found.name = name.clone())
            }
            Command::SetVerified { contact, verified } => {
                self.change_contact(*contact, |found| found.verified = *verified)
            }
            Command::BlockContact(contact) => {
                let person = self
                    .find(contact.identity())
                    .and_then(|found| found.record(*contact))
                    .filter(|held| held.current)
                    .map(|held| held.person);
                let mut ended = Vec::new();
                let outcome = self.change_contact(*contact, |found| {
                    found.current = false;
                    ended = found.end_transfers(false);
                });
                if let (Outcome::Done, Some(person)) = (&outcome, person) {
                    self.block(contact.identity(), person);
                }
                ran.events
                    .extend(ended.into_iter().map(RuntimeEvent::TransferChanged));
                outcome
            }
            Command::RemoveContact(contact) => {
                let mut ended = Vec::new();
                let outcome = self.change_contact(*contact, |found| {
                    found.current = false;
                    ended = found.end_transfers(true);
                });
                ran.events
                    .extend(ended.into_iter().map(RuntimeEvent::TransferChanged));
                outcome
            }
            Command::DeleteConversation(contact) => match self.record_mut(*contact) {
                Some(found) => {
                    found.messages.clear();
                    self.tidy(contact.identity());
                    Outcome::Done
                }
                None => Outcome::Failed(Failure::NoSuchContact),
            },
            Command::Send { contact, text } => {
                let current = self
                    .find(contact.identity())
                    .map(|found| found.record(*contact).map(|held| held.current));
                match current {
                    None => Outcome::Failed(Failure::NoSuchIdentity),
                    Some(None) => Outcome::Failed(Failure::NoSuchContact),
                    Some(Some(false)) => Outcome::Failed(Failure::NotAContact),
                    Some(Some(true)) => {
                        let message = self.add_message(*contact, Status::Queued, text);
                        ran.events.push(RuntimeEvent::MessageQueued(message));
                        Outcome::MessageQueued(message)
                    }
                }
            }
            Command::AcceptRequest { request, name } => match self.take_request(*request) {
                Some(taken) => {
                    let contact =
                        self.add_record(request.identity(), taken.person, true, name.clone());
                    self.opened.insert(contact);
                    Outcome::RequestAccepted(contact)
                }
                None => Outcome::Failed(Failure::NoSuchRequest),
            },
            Command::DeclineRequest(request) => {
                if self.take_request(*request).is_some() {
                    Outcome::Done
                } else {
                    Outcome::Failed(Failure::NoSuchRequest)
                }
            }
            Command::BlockRequest(request) => match self.take_request(*request) {
                Some(taken) => {
                    self.block(request.identity(), taken.person);
                    Outcome::Done
                }
                None => Outcome::Failed(Failure::NoSuchRequest),
            },
            Command::Unblock(blocked) => match self.find_mut(blocked.identity()) {
                Some(found) => {
                    let before = found.blocked.len();
                    found.blocked.retain(|held| held.id != *blocked);
                    if found.blocked.len() < before {
                        Outcome::Done
                    } else {
                        Outcome::Failed(Failure::NotBlocked)
                    }
                }
                None => Outcome::Failed(Failure::NoSuchIdentity),
            },
            Command::CreateInvitation {
                identity: owner,
                label,
            } => {
                if self.find(*owner).is_some() {
                    let invitation = self.add_invitation(*owner, label.clone());
                    Outcome::InvitationCreated {
                        invitation,
                        card: Zeroizing::new("invitation card".to_owned()),
                    }
                } else {
                    Outcome::Failed(Failure::NoSuchIdentity)
                }
            }
            Command::RevokeInvitation {
                invitation,
                discard_requests,
            } => self.revoke(*invitation, *discard_requests),
            Command::SetRequestMode {
                identity: owner,
                mode,
            } => match self.find_mut(*owner) {
                Some(found) => {
                    found.mode = *mode;
                    Outcome::Done
                }
                None => Outcome::Failed(Failure::NoSuchIdentity),
            },
            Command::Shutdown => Outcome::Done,
            Command::OpenProfile { .. }
            | Command::CreateProfile { .. }
            | Command::StartWithoutSaving => Outcome::Failed(Failure::Unexpected),
            Command::OfferFile { contact, path } => self.offer(*contact, path, &mut ran.events),
            Command::AcceptTransfer(transfer) => self.decide(*transfer, &mut ran.events, |held| {
                if !held.incoming || held.status != TransferStatus::Offered {
                    return Err(Failure::NoSuchTransfer);
                }
                held.status = TransferStatus::Receiving;
                held.bytes = Some(held.staged);
                Ok(true)
            }),
            Command::RejectTransfer(transfer) => {
                self.decide(*transfer, &mut ran.events, |held| {
                    match (held.incoming, held.status) {
                        (true, TransferStatus::Offered) => {
                            held.end(TransferStatus::Rejected, None);
                            Ok(true)
                        }
                        // Rejecting twice is rejecting once.
                        (true, TransferStatus::Rejected) => Ok(false),
                        _ => Err(Failure::NoSuchTransfer),
                    }
                })
            }
            // An undecided offer is rejected; a received file can only be
            // removed; cancelling what ended changes nothing.
            Command::CancelTransfer(transfer) => self.decide(*transfer, &mut ran.events, |held| {
                match (held.incoming, held.status) {
                    (false, TransferStatus::Waiting | TransferStatus::Sending)
                    | (true, TransferStatus::Receiving) => {
                        held.end(TransferStatus::Cancelled, Some(TransferReason::ByUs));
                        Ok(true)
                    }
                    (true, TransferStatus::Offered) => {
                        held.end(TransferStatus::Rejected, None);
                        Ok(true)
                    }
                    (true, TransferStatus::Received) => Err(Failure::NoSuchTransfer),
                    _ => Ok(false),
                }
            }),
            Command::RemoveTransfer(transfer) => self.remove_transfer(*transfer, &mut ran.events),
            Command::ResumeTransfer(transfer) => self.decide(*transfer, &mut ran.events, |held| {
                if !held.incoming || held.status != TransferStatus::Receiving {
                    return Err(Failure::NoSuchTransfer);
                }
                Ok(core::mem::replace(&mut held.held, false))
            }),
            Command::SavePlace(transfer) => match self.transfer_mut(*transfer) {
                Some(_) => Outcome::SavePlace {
                    transfer: *transfer,
                    place: PathBuf::from(place_of(*transfer)),
                },
                None => Outcome::Failed(Failure::NoSuchTransfer),
            },
            Command::SaveTransfer {
                transfer,
                destination,
            } => {
                let outcome = self.decide(*transfer, &mut ran.events, |held| {
                    if !held.incoming || held.status != TransferStatus::Received || held.saved {
                        return Err(Failure::NoSuchTransfer);
                    }
                    if destination == &PathBuf::from(PLACES[1]) {
                        return Err(Failure::FileExists);
                    }
                    held.saved = true;
                    Ok(true)
                });
                match outcome {
                    Outcome::Done => Outcome::Saved {
                        transfer: *transfer,
                        confirmed: destination != &PathBuf::from(PLACES[2]),
                    },
                    failed => failed,
                }
            }
        };
        ran
    }

    /// Offers the file at `path` to `contact`: the last part of the path
    /// names it, and a folder cannot be sent.
    fn offer(
        &mut self,
        contact: UiContactId,
        path: &std::path::Path,
        events: &mut Vec<RuntimeEvent>,
    ) -> Outcome {
        let Some(found) = self.find(contact.identity()) else {
            return Outcome::Failed(Failure::NoSuchIdentity);
        };
        if !found
            .record(contact)
            .is_some_and(|held| held.current && held.accepted)
        {
            return Outcome::Failed(Failure::NotAcceptedForFiles);
        }
        let text = path.to_string_lossy();
        let name = text.rsplit(['/', '\\']).next().unwrap_or_default();
        if name.is_empty() {
            return Outcome::Failed(Failure::FileNotSendable);
        }
        let size = SIZES[name.len() % SIZES.len()];
        let made = self.add_transfer(contact, false, size, name.to_owned());
        events.push(RuntimeEvent::TransferChanged(made));
        Outcome::TransferOffered(made)
    }

    /// Runs a decision on `transfer`: `decide` changes it and says whether
    /// it did, or says why it does not apply.
    fn decide(
        &mut self,
        transfer: UiTransferId,
        events: &mut Vec<RuntimeEvent>,
        decide: impl FnOnce(&mut Transfer) -> Result<bool, Failure>,
    ) -> Outcome {
        let Some(held) = self.transfer_mut(transfer) else {
            return Outcome::Failed(Failure::NoSuchTransfer);
        };
        match decide(held) {
            Ok(changed) => {
                if changed {
                    events.push(RuntimeEvent::TransferChanged(transfer));
                }
                Outcome::Done
            }
            Err(failure) => Outcome::Failed(failure),
        }
    }

    /// Takes an ended transfer, or a received file, off the list.
    fn remove_transfer(
        &mut self,
        transfer: UiTransferId,
        events: &mut Vec<RuntimeEvent>,
    ) -> Outcome {
        let Some(found) = self.record_mut(transfer.contact()) else {
            return Outcome::Failed(Failure::NoSuchTransfer);
        };
        let Some(index) = found
            .transfers
            .iter()
            .position(|held| held.id == transfer && held.finished())
        else {
            return Outcome::Failed(Failure::NoSuchTransfer);
        };
        found.transfers.remove(index);
        self.tidy(transfer.identity());
        events.push(RuntimeEvent::TransferChanged(transfer));
        Outcome::Done
    }

    fn read(&mut self, contact: UiContactId, before: Option<u64>, limit: usize) -> Outcome {
        let Some(owner) = self.find(contact.identity()) else {
            return Outcome::Failed(Failure::NoSuchIdentity);
        };
        let Some(found) = owner.record(contact) else {
            return Outcome::Failed(Failure::NoSuchContact);
        };
        let mut older: Vec<MessageView> = found
            .messages
            .iter()
            .filter(|message| before.is_none_or(|before| message.sequence < before))
            .map(|message| message.view(contact))
            .collect();
        let skip = older.len().saturating_sub(limit);
        let messages = older.split_off(skip);
        self.said.messages(&messages);
        Outcome::Messages {
            contact,
            messages,
            complete: skip == 0,
        }
    }

    fn add_contact(&mut self, owner: UiIdentityId, text: &str, name: Option<String>) -> Outcome {
        let Some(found) = self.find(owner) else {
            return Outcome::Failed(Failure::NoSuchIdentity);
        };
        let Some(person) = person_of(text) else {
            return Outcome::Failed(Failure::InvalidCard);
        };
        if found.blocks(person) {
            return Outcome::Failed(Failure::Blocked);
        }
        if found
            .records
            .iter()
            .any(|held| held.current && held.person == person)
        {
            return Outcome::Failed(Failure::Full);
        }
        // A request of that person is answered by the contact.
        if let Some(found) = self.find_mut(owner) {
            found.requests.retain(|request| request.person != person);
        }
        let contact = self.add_record(owner, person, false, name);
        self.opened.insert(contact);
        Outcome::ContactAdded {
            contact,
            invitation: invited(person),
        }
    }

    fn change_contact(
        &mut self,
        contact: UiContactId,
        change: impl FnOnce(&mut Record),
    ) -> Outcome {
        if self.find(contact.identity()).is_none() {
            return Outcome::Failed(Failure::NoSuchIdentity);
        }
        match self.record_mut(contact) {
            Some(found) if found.current => change(found),
            _ => return Outcome::Failed(Failure::NoSuchContact),
        }
        self.tidy(contact.identity());
        Outcome::Done
    }

    fn revoke(&mut self, invitation: UiInvitationId, discard: bool) -> Outcome {
        let Some(found) = self.find_mut(invitation.identity()) else {
            return Outcome::Failed(Failure::NoSuchInvitation);
        };
        let Some(index) = found
            .invitations
            .iter()
            .position(|held| held.id == invitation)
        else {
            return Outcome::Failed(Failure::NoSuchInvitation);
        };
        found.invitations.remove(index);
        let before = found.requests.len();
        if discard {
            found
                .requests
                .retain(|request| request.invitation != Some(invitation));
        }
        Outcome::Revoked {
            discarded: before - found.requests.len(),
        }
    }

    /// A contact handle of an identity other than `owner`.
    fn foreign_contact(&self, owner: UiIdentityId, k: u8) -> UiContactId {
        let mut pool: Vec<UiContactId> = self
            .contacts_ever
            .iter()
            .copied()
            .filter(|contact| contact.identity() != owner)
            .collect();
        pool.push(record(stranger(), 0xEE));
        choose(&pool, k).unwrap()
    }

    /// A contact handle other than `contact`: another record of its
    /// identity, deleted or never made, or a record of another identity.
    fn other_contact(&self, contact: UiContactId, k: u8) -> UiContactId {
        let mut pool: Vec<UiContactId> = self
            .contacts_ever
            .iter()
            .copied()
            .filter(|other| *other != contact)
            .collect();
        pool.push(record(contact.identity(), 0xEE));
        choose(&pool, k).unwrap()
    }

    /// An identity handle other than `owner`.
    fn other_identity(&self, owner: UiIdentityId, k: u8) -> UiIdentityId {
        let mut pool: Vec<UiIdentityId> = self
            .identities_ever
            .iter()
            .copied()
            .filter(|other| *other != owner)
            .collect();
        pool.push(stranger());
        choose(&pool, k).unwrap()
    }

    /// The answer of `ran`, with handles of another identity or record
    /// put in it: what the command did in the truth stays done.
    fn garble(&self, command: &Command, ran: Ran, k: u8) -> Ran {
        let Ran {
            outcome,
            tells,
            events,
            resets,
        } = ran;
        let (outcome, tells) = match (command, outcome) {
            (Command::LoadIdentity(asked), Outcome::Identity(mut snapshot)) => {
                let foreign = self.foreign_contact(*asked, k);
                if k % 2 == 0 {
                    // The snapshot of another identity.
                    let other = foreign.identity();
                    let snapshot = self
                        .find(other)
                        .map_or_else(|| empty_snapshot(other), Identity::snapshot);
                    (Outcome::Identity(Box::new(snapshot)), Tells::Nothing)
                } else {
                    // Its own, with what belongs to another identity mixed in.
                    snapshot.contacts.insert(0, forged_contact(foreign));
                    snapshot.conversations.push(forged_conversation(foreign));
                    snapshot
                        .requests
                        .insert(0, forged_request(foreign.identity()));
                    snapshot.invitations.push(InvitationView {
                        id: UiInvitationId::new(foreign.identity(), 901),
                        label: Some("forged".to_owned()),
                    });
                    snapshot.blocked.push(BlockedView {
                        id: UiBlockedId::new(foreign.identity(), 903),
                        fingerprint: fingerprint(9),
                        full_fingerprint: String::new(),
                    });
                    snapshot.transfers.insert(0, forged_transfer(foreign));
                    snapshot.interrupted.push(UiTransferId::new(foreign, 905));
                    (Outcome::Identity(snapshot), tells)
                }
            }
            (
                Command::LoadMessages { contact: asked, .. }
                | Command::LoadEarlier { contact: asked, .. },
                Outcome::Messages {
                    contact,
                    mut messages,
                    complete,
                },
            ) => {
                let other = self.other_contact(*asked, k);
                if k % 2 == 0 {
                    // The messages of another conversation.
                    let messages = vec![forged_message(other, 1)];
                    (
                        Outcome::Messages {
                            contact: other,
                            messages,
                            complete,
                        },
                        tells,
                    )
                } else {
                    // Its own, with messages of another one mixed in.
                    messages.insert(0, forged_message(other, 1));
                    messages.push(forged_message(other, 2));
                    (
                        Outcome::Messages {
                            contact,
                            messages,
                            complete,
                        },
                        tells,
                    )
                }
            }
            (Command::Send { contact, .. }, Outcome::MessageQueued(message)) => (
                Outcome::MessageQueued(UiMessageId::new(
                    self.other_contact(*contact, k),
                    message.sequence(),
                )),
                tells,
            ),
            (
                Command::AddContact {
                    identity: owner, ..
                },
                Outcome::ContactAdded { invitation, .. },
            ) => (
                Outcome::ContactAdded {
                    contact: self.foreign_contact(*owner, k),
                    invitation,
                },
                tells,
            ),
            (Command::AcceptRequest { request, .. }, Outcome::RequestAccepted(_)) => (
                Outcome::RequestAccepted(self.foreign_contact(request.identity(), k)),
                tells,
            ),
            (
                Command::CreateInvitation {
                    identity: owner, ..
                },
                Outcome::InvitationCreated { card: text, .. },
            ) => (
                Outcome::InvitationCreated {
                    invitation: UiInvitationId::new(self.other_identity(*owner, k), 902),
                    card: text,
                },
                tells,
            ),
            (Command::OfferFile { contact, .. }, Outcome::TransferOffered(made)) => (
                Outcome::TransferOffered(UiTransferId::new(
                    self.other_contact(*contact, k),
                    made.handle(),
                )),
                tells,
            ),
            (Command::SavePlace(asked), Outcome::SavePlace { .. }) => {
                let other = self.other_transfer(*asked, k);
                (
                    Outcome::SavePlace {
                        transfer: other,
                        place: PathBuf::from(place_of(other)),
                    },
                    tells,
                )
            }
            (Command::SaveTransfer { transfer, .. }, Outcome::Saved { confirmed, .. }) => (
                Outcome::Saved {
                    transfer: self.other_transfer(*transfer, k),
                    confirmed,
                },
                tells,
            ),
            (_, Outcome::Failed(failure)) => (Outcome::Failed(failure), tells),
            // An answer of the wrong kind.
            (
                Command::LoadIdentities
                | Command::CheckTor
                | Command::OwnCard(_)
                | Command::InvitationCard(_),
                _,
            ) => (Outcome::Done, Tells::Nothing),
            (_, outcome) => (outcome, tells),
        };
        Ran {
            outcome,
            tells,
            events,
            resets,
        }
    }

    /// Takes the commands the model issued: they wait for an answer.
    fn take(&mut self, commands: Vec<(CommandId, Command)>) {
        for (id, command) in &commands {
            self.waiting.insert(*id);
            match command {
                Command::AcceptTransfer(transfer)
                | Command::RejectTransfer(transfer)
                | Command::CancelTransfer(transfer)
                | Command::RemoveTransfer(transfer)
                | Command::ResumeTransfer(transfer)
                | Command::SaveTransfer { transfer, .. } => {
                    self.on_transfer.insert(*id, *transfer);
                }
                Command::OfferFile { contact, .. } => {
                    self.offers.insert(*id, *contact);
                }
                _ => {}
            }
        }
        if self.stopped {
            self.lost
                .extend(commands.into_iter().map(|(_, command)| command));
        } else {
            self.inbox.extend(commands);
        }
    }

    /// Runs one command, the newest for `pick` 0, and answers it as `fate`
    /// says; the events it raised reach the model if `notify`.
    fn run(&mut self, model: &mut Model, round: &mut Round, pick: u8, fate: Fate, notify: bool) {
        if self.inbox.is_empty() {
            return;
        }
        let index = self.inbox.len() - 1 - usize::from(pick) % self.inbox.len();
        let (id, command) = self.inbox.remove(index);
        let mut foreign = false;
        let ran = match fate {
            Fate::Fail => Ran::new(Outcome::Failed(Failure::Disk)),
            Fate::Foreign(k) => {
                let ran = self.execute(&command);
                let truthful = ran.outcome.clone();
                let ran = self.garble(&command, ran, k);
                foreign = ran.outcome != truthful && ran.outcome != Outcome::Done;
                ran
            }
            Fate::Now | Fate::Hold | Fate::Drop => self.execute(&command),
        };
        if notify {
            for event in ran.events {
                model.apply(Update::Event(event));
            }
        }
        let answer = Answer {
            id,
            command,
            outcome: ran.outcome,
            tells: ran.tells,
            foreign,
            resets: ran.resets,
        };
        match fate {
            Fate::Hold => self.held.push(answer),
            Fate::Drop => self.lost.push(answer.command),
            Fate::Now | Fate::Foreign(_) | Fate::Fail => self.deliver(model, round, answer),
        }
    }

    /// Runs every command, oldest first, and answers each at once.
    fn run_all(&mut self, model: &mut Model, round: &mut Round) {
        for (id, command) in core::mem::take(&mut self.inbox) {
            let ran = self.execute(&command);
            for event in ran.events {
                model.apply(Update::Event(event));
            }
            let answer = Answer {
                id,
                command,
                outcome: ran.outcome,
                tells: ran.tells,
                foreign: false,
                resets: ran.resets,
            };
            self.deliver(model, round, answer);
        }
    }

    fn deliver_held(&mut self, model: &mut Model, round: &mut Round, pick: u8) {
        if self.held.is_empty() {
            return;
        }
        let answer = self.held.remove(usize::from(pick) % self.held.len());
        self.deliver(model, round, answer);
    }

    fn deliver(&mut self, model: &mut Model, round: &mut Round, answer: Answer) {
        assert!(self.waiting.remove(&answer.id), "{answer:?} answered twice");
        self.answered.insert(answer.id);
        match &answer.tells {
            Tells::Records { owner, gone } => {
                // Applied only to a slice the model has.
                if model.slice(*owner).is_some() {
                    self.heard_gone_records.extend(gone.iter().copied());
                }
            }
            Tells::Identities(gone) => self.heard_gone_identities.extend(gone.iter().copied()),
            Tells::Nothing => {}
        }
        match (&answer.command, &answer.outcome) {
            // A read of its own identity is applied to a slice the model
            // has.
            (Command::LoadIdentity(asked), Outcome::Identity(snapshot))
                if snapshot.identity == *asked && model.slice(*asked).is_some() =>
            {
                self.read_resets
                    .insert(*asked, (answer.id, answer.resets.clone()));
            }
            (Command::OfferFile { contact, .. }, Outcome::Failed(failure)) => {
                self.said.unsent.push((*contact, *failure));
            }
            _ => {}
        }
        model.take_notices();
        model.apply(Update::Done {
            command: answer.id,
            outcome: answer.outcome.clone(),
        });
        // An answer that names what its command did not act on is not a
        // success of that command.
        if answer.foreign {
            assert!(
                model
                    .take_notices()
                    .iter()
                    .all(|notice| notice.kind != NoticeKind::Success),
                "{answer:?} was told as a success of its command"
            );
        }
        round.delivered.push((answer.command, answer.outcome));
    }

    fn stop(&mut self) {
        self.stopped = true;
        let inbox = core::mem::take(&mut self.inbox);
        self.lost
            .extend(inbox.into_iter().map(|(_, command)| command));
    }

    /// Changes the truth by itself; the event of the change, if one is
    /// raised.
    fn change(&mut self, change: Change) -> Option<RuntimeEvent> {
        match change {
            Change::Incoming { pick, text } => {
                let contacts: Vec<UiContactId> = self
                    .records()
                    .filter(|found| found.current && found.accepted)
                    .map(|found| found.id)
                    .collect();
                let contact = choose(&contacts, pick)?;
                let text = TEXTS[usize::from(text) % TEXTS.len()];
                let message = self.add_message(contact, Status::Received, text);
                Some(RuntimeEvent::MessageReceived(message))
            }
            Change::Acknowledged(pick) => {
                let queued: Vec<UiMessageId> = self
                    .records()
                    .flat_map(|found| {
                        found
                            .messages
                            .iter()
                            .filter(|message| message.status == Status::Queued)
                            .map(|message| UiMessageId::new(found.id, message.sequence))
                    })
                    .collect();
                let message = choose(&queued, pick)?;
                self.record_mut(message.contact())?
                    .messages
                    .iter_mut()
                    .find(|held| held.sequence == message.sequence())?
                    .status = Status::Delivered;
                Some(RuntimeEvent::MessageDelivered(message))
            }
            Change::Accepts(pick) => {
                let requested: Vec<UiContactId> = self
                    .records()
                    .filter(|found| found.current && !found.accepted)
                    .map(|found| found.id)
                    .collect();
                let contact = choose(&requested, pick)?;
                self.record_mut(contact)?.accepted = true;
                Some(RuntimeEvent::ContactAccepted(contact))
            }
            Change::Request { pick, person } => {
                let owners: Vec<UiIdentityId> =
                    self.identities.iter().map(|found| found.id).collect();
                let owner = choose(&owners, pick)?;
                let found = self.find(owner)?;
                if found
                    .records
                    .iter()
                    .any(|held| held.current && held.person == person)
                    || found.requests.iter().any(|held| held.person == person)
                    || found.blocks(person)
                {
                    return None;
                }
                let invitation = if pick % 2 == 0 {
                    found.invitations.first().map(|held| held.id)
                } else {
                    None
                };
                self.add_request(owner, person, invitation);
                Some(RuntimeEvent::RequestReceived {
                    identity: owner,
                    queued: true,
                })
            }
            Change::MadeAgain(pick) => {
                let contacts: Vec<UiContactId> = self
                    .records()
                    .filter(|found| found.current)
                    .map(|found| found.id)
                    .collect();
                let old = choose(&contacts, pick)?;
                let found = self.record_mut(old)?;
                found.current = false;
                found.end_transfers(true);
                let person = found.person;
                self.tidy(old.identity());
                let new = self.add_record(old.identity(), person, true, None);
                Some(RuntimeEvent::ContactAccepted(new))
            }
            Change::Service { pick, state } => {
                let owners: Vec<UiIdentityId> =
                    self.identities.iter().map(|found| found.id).collect();
                let owner = choose(&owners, pick)?;
                let state = SERVICES[usize::from(state) % SERVICES.len()];
                self.find_mut(owner)?.service = state;
                self.said.service(owner, state);
                Some(RuntimeEvent::Service {
                    identity: owner,
                    state,
                })
            }
            Change::IdentityGone(pick) => {
                let owners: Vec<UiIdentityId> =
                    self.identities.iter().map(|found| found.id).collect();
                let owner = choose(&owners, pick)?;
                self.identities.retain(|found| found.id != owner);
                Some(RuntimeEvent::IdentityClosed(owner))
            }
            Change::Tor(n) => {
                self.tor = tor_state(n);
                let tor = self.tor.clone();
                self.said.tor(&tor);
                None
            }
            Change::Offer { pick, file } => {
                let contacts: Vec<UiContactId> = self
                    .records()
                    .filter(|found| found.current && found.accepted)
                    .map(|found| found.id)
                    .collect();
                let contact = choose(&contacts, pick)?;
                let size = SIZES[usize::from(file) % SIZES.len()];
                let offered = self.add_transfer(contact, true, size, file_name(file));
                Some(RuntimeEvent::TransferOffered(offered))
            }
            Change::PeerAccepts(pick) => {
                let transfer = self.pick_transfer(pick, |held| {
                    !held.incoming && held.status == TransferStatus::Waiting
                })?;
                let held = self.transfer_mut(transfer)?;
                held.status = TransferStatus::Sending;
                held.bytes = Some(held.staged);
                Some(RuntimeEvent::TransferChanged(transfer))
            }
            Change::Progress { pick, share } => {
                let transfer = self.pick_transfer(pick, |held| {
                    held.moving() && !held.held && held.staged < held.size
                })?;
                let held = self.transfer_mut(transfer)?;
                let step = 1 + u64::from(share) * held.size / 256;
                held.staged = held.size.min(held.staged + step);
                held.bytes = Some(held.staged);
                let (said, bytes) = (held.clone(), held.staged);
                self.said.bytes(&said);
                Some(RuntimeEvent::TransferProgressed { transfer, bytes })
            }
            Change::Completes(pick) => {
                let transfer = self.pick_transfer(pick, |held| {
                    matches!(
                        (held.incoming, held.status),
                        (
                            false,
                            TransferStatus::Waiting
                                | TransferStatus::Sending
                                | TransferStatus::Cancelled
                        ) | (true, TransferStatus::Receiving)
                    )
                })?;
                let held = self.transfer_mut(transfer)?;
                let done = if held.incoming {
                    TransferStatus::Received
                } else {
                    TransferStatus::Completed
                };
                held.end(done, None);
                held.staged = held.size;
                held.bytes = Some(held.size);
                Some(RuntimeEvent::TransferChanged(transfer))
            }
            Change::PeerEnds { pick, reject } => {
                let transfer = self.pick_transfer(pick, |held| {
                    matches!(
                        (held.incoming, held.status),
                        (false, TransferStatus::Waiting | TransferStatus::Sending)
                            | (true, TransferStatus::Offered | TransferStatus::Receiving)
                    )
                })?;
                let held = self.transfer_mut(transfer)?;
                if reject && !held.incoming {
                    held.end(TransferStatus::Rejected, None);
                } else {
                    held.end(TransferStatus::Cancelled, Some(TransferReason::ByPeer));
                }
                Some(RuntimeEvent::TransferChanged(transfer))
            }
            Change::Fails { pick, reason } => {
                let transfer = self.pick_transfer(pick, |held| {
                    matches!(
                        (held.incoming, held.status),
                        (false, TransferStatus::Waiting | TransferStatus::Sending)
                            | (true, TransferStatus::Receiving)
                    ) || (held.status == TransferStatus::Received && !held.saved)
                })?;
                let held = self.transfer_mut(transfer)?;
                let reasons: &[TransferReason] = match (held.incoming, held.status) {
                    // A received file that no longer reads back.
                    (true, TransferStatus::Received) => {
                        held.lose_track();
                        &[TransferReason::Storage]
                    }
                    (true, _) => &[
                        TransferReason::Storage,
                        TransferReason::Digest,
                        TransferReason::Stalled,
                        TransferReason::OffererGone,
                    ],
                    (false, _) => &[
                        TransferReason::Storage,
                        TransferReason::Digest,
                        TransferReason::Stalled,
                    ],
                };
                held.end(TransferStatus::Failed, choose(reasons, reason));
                Some(RuntimeEvent::TransferChanged(transfer))
            }
            Change::Expires(pick) => {
                let transfer = self.pick_transfer(pick, |held| {
                    held.incoming && held.status == TransferStatus::Offered
                })?;
                self.transfer_mut(transfer)?
                    .end(TransferStatus::Expired, None);
                Some(RuntimeEvent::TransferChanged(transfer))
            }
            Change::Hold(pick) => {
                let transfer = self.pick_transfer(pick, |held| {
                    held.incoming && held.status == TransferStatus::Receiving
                })?;
                let held = self.transfer_mut(transfer)?;
                if held.held {
                    held.held = false;
                } else {
                    // What was written since the last sync counts as not
                    // written: how much is staged is found again.
                    held.held = true;
                    held.lose_track();
                }
                Some(RuntimeEvent::TransferChanged(transfer))
            }
            Change::Restart(pick) => {
                for found in &mut self.identities {
                    found.interrupted.clear();
                    for held in found
                        .records
                        .iter_mut()
                        .flat_map(|found| &mut found.transfers)
                    {
                        held.held = false;
                        if !matches!(
                            held.status,
                            TransferStatus::Waiting
                                | TransferStatus::Offered
                                | TransferStatus::Completed
                                | TransferStatus::Received
                        ) {
                            held.lose_track();
                        }
                    }
                }
                // A save of a received file was under way.
                let cut = self
                    .pick_transfer(pick / 2, |held| {
                        held.status == TransferStatus::Received && !held.saved
                    })
                    .filter(|_| pick % 2 == 0);
                match cut {
                    Some(transfer) => {
                        self.find_mut(transfer.identity())?
                            .interrupted
                            .push(transfer);
                        Some(RuntimeEvent::SaveInterrupted(transfer))
                    }
                    None => Some(RuntimeEvent::Missed),
                }
            }
        }
    }

    /// An event of any kind about any handle, deleted records, other
    /// identities and handles never made included, that changes nothing in
    /// the truth. A service is reported in a state it was in; a transfer
    /// with bytes said of it since the runtime last lost track of them.
    fn spurious(&self, kind: u8, pick: u8) -> RuntimeEvent {
        let mut messages = self.messages_ever.clone();
        messages.push(UiMessageId::new(record(identity(1), 0xEE), 7));
        let mut contacts = self.contacts_ever.clone();
        contacts.extend([record(identity(1), 0xEE), record(stranger(), 0xEE)]);
        let mut identities = self.identities_ever.clone();
        identities.push(stranger());
        let message = choose(&messages, pick).unwrap();
        let contact = choose(&contacts, pick).unwrap();
        let owner = choose(&identities, pick).unwrap();
        let transfer = choose(&self.transfer_pool(), pick).unwrap();
        match kind % 13 {
            0 => RuntimeEvent::MessageQueued(message),
            1 => RuntimeEvent::MessageReceived(message),
            2 => RuntimeEvent::MessageDelivered(message),
            3 => RuntimeEvent::MessageRefused(contact),
            4 => RuntimeEvent::DeliveryDeferred {
                contact,
                reason: DEFERRALS[usize::from(pick) % DEFERRALS.len()],
                retry_in: u64::from(pick),
            },
            5 => RuntimeEvent::ContactAccepted(contact),
            6 => RuntimeEvent::RequestReceived {
                identity: owner,
                queued: pick % 2 == 0,
            },
            7 => RuntimeEvent::IdentityClosed(owner),
            8 => {
                let states = self
                    .said
                    .services
                    .get(&owner)
                    .map_or(&[][..], Vec::as_slice);
                RuntimeEvent::Service {
                    identity: owner,
                    state: choose(states, pick).unwrap_or(Service::Stopped),
                }
            }
            9 => RuntimeEvent::TransferOffered(transfer),
            10 => RuntimeEvent::TransferChanged(transfer),
            11 => {
                let said: Vec<u64> = match self.resets.get(&transfer) {
                    Some(resets) => self
                        .said
                        .bytes
                        .iter()
                        .filter(|(key, said)| key.0 == transfer && **said == *resets)
                        .map(|(key, _)| key.1)
                        .collect(),
                    // A transfer never made.
                    None => vec![u64::from(pick)],
                };
                match choose(&said, kind / 13) {
                    Some(bytes) => RuntimeEvent::TransferProgressed { transfer, bytes },
                    None => RuntimeEvent::TransferChanged(transfer),
                }
            }
            _ => RuntimeEvent::SaveInterrupted(transfer),
        }
    }

    /// An outcome of any kind, for a number nothing waits for.
    fn any_outcome(&self, kind: u8, pick: u8) -> Outcome {
        let contact = choose(&self.contacts_ever, pick).unwrap();
        let owner = choose(&self.identities_ever, pick).unwrap();
        let transfer = choose(&self.transfer_pool(), pick).unwrap();
        match kind % 15 {
            0 => Outcome::Identities(self.list()),
            1 => Outcome::Identity(Box::new(
                self.find(owner)
                    .map_or_else(|| empty_snapshot(owner), Identity::snapshot),
            )),
            2 => Outcome::Messages {
                contact,
                messages: vec![forged_message(contact, 1)],
                complete: true,
            },
            3 => Outcome::Done,
            4 => Outcome::Failed(Failure::NoSuchIdentity),
            5 => Outcome::ContactAdded {
                contact,
                invitation: pick % 2 == 0,
            },
            6 => Outcome::MessageQueued(UiMessageId::new(contact, 1)),
            7 => Outcome::RequestAccepted(contact),
            8 => Outcome::InvitationCreated {
                invitation: UiInvitationId::new(owner, 77),
                card: Zeroizing::new("invitation card".to_owned()),
            },
            9 => Outcome::Tor(TorCheck::NotRunning),
            10 => Outcome::IdentityCreated(owner),
            11 => Outcome::ProfileOpen {
                ephemeral: true,
                message_store_missing: true,
            },
            12 => Outcome::TransferOffered(transfer),
            13 => Outcome::SavePlace {
                transfer,
                place: PathBuf::from(place_of(transfer)),
            },
            _ => Outcome::Saved {
                transfer,
                confirmed: true,
            },
        }
    }
}

// ---- The sequences ----

#[derive(Clone, Copy, Debug)]
enum DialogKind {
    NewIdentity,
    /// With the card of a person; `PEOPLE` is no card.
    AddContact {
        person: u8,
    },
    OwnCard,
    NewInvitation,
    ShowInvitation,
    RevokeInvitation {
        discard: bool,
    },
    ContactDetails,
    BlockContact,
    RemoveContact,
    DeleteConversation,
    AcceptRequest,
    DeleteIdentity,
    TorDetails,
}

/// One step. Where a step of the user has `local`, it picks among what
/// the identity shown holds, as the screens offer it; otherwise among every
/// handle there ever was, of any identity, deleted ones and some never
/// made included.
#[derive(Clone, Copy, Debug)]
enum Step {
    SelectIdentity(u8),
    SelectConversation {
        local: bool,
        pick: u8,
    },
    ShowRequests,
    /// Opens a dialog on what it picks, and confirms it if `confirm`.
    Open {
        dialog: DialogKind,
        local: bool,
        pick: u8,
        confirm: bool,
    },
    Confirm,
    Close,
    /// Types a text as the draft of a conversation, and sends it.
    Send {
        local: bool,
        pick: u8,
        text: u8,
    },
    /// Declines a request, or blocks its sender.
    Answer {
        local: bool,
        pick: u8,
        block: bool,
    },
    Verify {
        local: bool,
        pick: u8,
        verified: bool,
    },
    /// Takes one of the acts the card of a transfer offers (any act for a
    /// transfer the model shows no card of), and confirms its dialog if
    /// `confirm`; in an open save dialog, types the place to save to, and
    /// saves if `confirm`.
    Card {
        local: bool,
        pick: u8,
        act: u8,
        confirm: bool,
        place: u8,
    },
    /// Drops a file on a conversation, or types its path, and sends it if
    /// `confirm`.
    SendFile {
        local: bool,
        pick: u8,
        path: u8,
        confirm: bool,
    },
    Unblock {
        local: bool,
        pick: u8,
    },
    Mode {
        local: bool,
        pick: u8,
        mode: u8,
    },
    CheckTor,
    /// Runs one command taken from the model, the newest for `pick` 0, and
    /// answers it as `fate` says; its events reach the model if `notify`.
    Run {
        pick: u8,
        fate: Fate,
        notify: bool,
    },
    /// Runs every command taken, in order, answering each at once.
    RunAll,
    /// Delivers an answer held back.
    Deliver(u8),
    /// Answers a number nothing waits for: one answered, or one not issued.
    StaleAnswer {
        pick: u8,
        kind: u8,
    },
    /// Changes the truth; the event reaches the model if `notify`.
    Change {
        change: Change,
        notify: bool,
    },
    Missed,
    /// An event of any kind, about anything, that changes nothing.
    Spurious {
        kind: u8,
        pick: u8,
    },
    StorageFails,
    Stopped,
    /// The late answers arrive, events are missed, and every read is
    /// answered.
    Settle,
}

fn dialog_kind() -> impl Strategy<Value = DialogKind> {
    prop_oneof![
        Just(DialogKind::NewIdentity),
        (0..=PEOPLE).prop_map(|person| DialogKind::AddContact { person }),
        Just(DialogKind::OwnCard),
        Just(DialogKind::NewInvitation),
        Just(DialogKind::ShowInvitation),
        any::<bool>().prop_map(|discard| DialogKind::RevokeInvitation { discard }),
        Just(DialogKind::ContactDetails),
        Just(DialogKind::BlockContact),
        Just(DialogKind::RemoveContact),
        Just(DialogKind::DeleteConversation),
        Just(DialogKind::AcceptRequest),
        Just(DialogKind::DeleteIdentity),
        Just(DialogKind::TorDetails),
    ]
}

fn fate() -> impl Strategy<Value = Fate> {
    prop_oneof![
        8 => Just(Fate::Now),
        5 => Just(Fate::Hold),
        1 => Just(Fate::Drop),
        6 => any::<u8>().prop_map(Fate::Foreign),
        2 => Just(Fate::Fail),
    ]
}

fn change() -> impl Strategy<Value = Change> {
    prop_oneof![
        4 => (any::<u8>(), any::<u8>()).prop_map(|(pick, text)| Change::Incoming { pick, text }),
        4 => any::<u8>().prop_map(Change::Acknowledged),
        1 => any::<u8>().prop_map(Change::Accepts),
        2 => (any::<u8>(), 0..PEOPLE).prop_map(|(pick, person)| Change::Request { pick, person }),
        2 => any::<u8>().prop_map(Change::MadeAgain),
        1 => (any::<u8>(), any::<u8>()).prop_map(|(pick, state)| Change::Service { pick, state }),
        1 => any::<u8>().prop_map(Change::IdentityGone),
        1 => any::<u8>().prop_map(Change::Tor),
        3 => (any::<u8>(), any::<u8>()).prop_map(|(pick, file)| Change::Offer { pick, file }),
        2 => any::<u8>().prop_map(Change::PeerAccepts),
        4 => (any::<u8>(), any::<u8>()).prop_map(|(pick, share)| Change::Progress { pick, share }),
        2 => any::<u8>().prop_map(Change::Completes),
        1 => (any::<u8>(), any::<bool>()).prop_map(|(pick, reject)| Change::PeerEnds { pick, reject }),
        1 => (any::<u8>(), any::<u8>()).prop_map(|(pick, reason)| Change::Fails { pick, reason }),
        1 => any::<u8>().prop_map(Change::Expires),
        2 => any::<u8>().prop_map(Change::Hold),
        1 => any::<u8>().prop_map(Change::Restart),
    ]
}

fn step() -> impl Strategy<Value = Step> {
    let local = || any::<bool>();
    let pick = || any::<u8>();
    // Mostly the command just made.
    let newest = || prop_oneof![2 => Just(0), 1 => any::<u8>()];
    // Events are mostly passed on.
    let notify = || prop_oneof![3 => Just(true), 1 => Just(false)];
    prop_oneof![
        12 => pick().prop_map(Step::SelectIdentity),
        24 => (local(), pick())
            .prop_map(|(local, pick)| Step::SelectConversation { local, pick }),
        6 => Just(Step::ShowRequests),
        42 => (dialog_kind(), local(), pick(), any::<bool>()).prop_map(
            |(dialog, local, pick, confirm)| Step::Open { dialog, local, pick, confirm }
        ),
        9 => Just(Step::Confirm),
        6 => Just(Step::Close),
        24 => (local(), pick(), pick())
            .prop_map(|(local, pick, text)| Step::Send { local, pick, text }),
        6 => (local(), pick(), any::<bool>())
            .prop_map(|(local, pick, block)| Step::Answer { local, pick, block }),
        3 => (local(), pick(), any::<bool>())
            .prop_map(|(local, pick, verified)| Step::Verify { local, pick, verified }),
        18 => (local(), pick(), pick(), any::<bool>(), pick()).prop_map(
            |(local, pick, act, confirm, place)| Step::Card { local, pick, act, confirm, place }
        ),
        8 => (local(), pick(), pick(), any::<bool>())
            .prop_map(|(local, pick, path, confirm)| Step::SendFile { local, pick, path, confirm }),
        3 => (local(), pick()).prop_map(|(local, pick)| Step::Unblock { local, pick }),
        3 => (local(), pick(), pick())
            .prop_map(|(local, pick, mode)| Step::Mode { local, pick, mode }),
        3 => Just(Step::CheckTor),
        48 => (newest(), fate(), notify())
            .prop_map(|(pick, fate, notify)| Step::Run { pick, fate, notify }),
        30 => Just(Step::RunAll),
        18 => pick().prop_map(Step::Deliver),
        6 => (pick(), pick()).prop_map(|(pick, kind)| Step::StaleAnswer { pick, kind }),
        60 => (change(), notify()).prop_map(|(change, notify)| Step::Change { change, notify }),
        6 => Just(Step::Missed),
        12 => (pick(), pick()).prop_map(|(kind, pick)| Step::Spurious { kind, pick }),
        1 => Just(Step::StorageFails),
        1 => Just(Step::Stopped),
        3 => Just(Step::Settle),
    ]
}

// ---- What the model is asked to do, and what it holds ----

/// What a command or a dialog acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    None,
    Identity(UiIdentityId),
    Contact(UiContactId),
    Request(UiRequestId),
    Invitation(UiInvitationId),
    Blocked(UiBlockedId),
    Transfer(UiTransferId),
}

/// A command of the user, by name, with what it acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Write {
    command: &'static str,
    target: Target,
}

/// The command of the user `command` is; `None` for a read.
fn write_of(command: &Command) -> Option<Write> {
    let (name, target) = match command {
        Command::LoadIdentities
        | Command::LoadIdentity(_)
        | Command::LoadMessages { .. }
        | Command::LoadEarlier { .. }
        | Command::CheckTor => return None,
        Command::OpenProfile { .. } => ("OpenProfile", Target::None),
        Command::CreateProfile { .. } => ("CreateProfile", Target::None),
        Command::StartWithoutSaving => ("StartWithoutSaving", Target::None),
        Command::Shutdown => ("Shutdown", Target::None),
        Command::CreateIdentity { .. } => ("CreateIdentity", Target::None),
        Command::OwnCard(owner) => ("OwnCard", Target::Identity(*owner)),
        Command::DeleteIdentity(owner) => ("DeleteIdentity", Target::Identity(*owner)),
        Command::AddContact {
            identity: owner, ..
        } => ("AddContact", Target::Identity(*owner)),
        Command::CreateInvitation {
            identity: owner, ..
        } => ("CreateInvitation", Target::Identity(*owner)),
        Command::SetRequestMode {
            identity: owner, ..
        } => ("SetRequestMode", Target::Identity(*owner)),
        Command::SetContactName { contact, .. } => ("SetContactName", Target::Contact(*contact)),
        Command::SetVerified { contact, .. } => ("SetVerified", Target::Contact(*contact)),
        Command::BlockContact(contact) => ("BlockContact", Target::Contact(*contact)),
        Command::RemoveContact(contact) => ("RemoveContact", Target::Contact(*contact)),
        Command::DeleteConversation(contact) => ("DeleteConversation", Target::Contact(*contact)),
        Command::Send { contact, .. } => ("Send", Target::Contact(*contact)),
        Command::AcceptRequest { request, .. } => ("AcceptRequest", Target::Request(*request)),
        Command::DeclineRequest(request) => ("DeclineRequest", Target::Request(*request)),
        Command::BlockRequest(request) => ("BlockRequest", Target::Request(*request)),
        Command::InvitationCard(invitation) => ("InvitationCard", Target::Invitation(*invitation)),
        Command::RevokeInvitation { invitation, .. } => {
            ("RevokeInvitation", Target::Invitation(*invitation))
        }
        Command::Unblock(blocked) => ("Unblock", Target::Blocked(*blocked)),
        Command::OfferFile { contact, .. } => ("OfferFile", Target::Contact(*contact)),
        Command::AcceptTransfer(transfer) => ("AcceptTransfer", Target::Transfer(*transfer)),
        Command::RejectTransfer(transfer) => ("RejectTransfer", Target::Transfer(*transfer)),
        Command::CancelTransfer(transfer) => ("CancelTransfer", Target::Transfer(*transfer)),
        Command::RemoveTransfer(transfer) => ("RemoveTransfer", Target::Transfer(*transfer)),
        Command::ResumeTransfer(transfer) => ("ResumeTransfer", Target::Transfer(*transfer)),
        Command::SavePlace(transfer) => ("SavePlace", Target::Transfer(*transfer)),
        Command::SaveTransfer { transfer, .. } => ("SaveTransfer", Target::Transfer(*transfer)),
    };
    Some(Write {
        command: name,
        target,
    })
}

/// A dialog, by kind, with the handle it captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Captured {
    kind: &'static str,
    target: Target,
}

impl Captured {
    fn of(dialog: &Dialog) -> Self {
        let (kind, target) = match dialog {
            Dialog::NewIdentity { .. } => ("NewIdentity", Target::None),
            Dialog::TorDetails => ("TorDetails", Target::None),
            Dialog::AddContact { identity, .. } => ("AddContact", Target::Identity(*identity)),
            Dialog::OwnCard { identity, .. } => ("OwnCard", Target::Identity(*identity)),
            Dialog::NewInvitation { identity, .. } => {
                ("NewInvitation", Target::Identity(*identity))
            }
            Dialog::DeleteIdentity { identity, .. } => {
                ("DeleteIdentity", Target::Identity(*identity))
            }
            Dialog::ShowInvitation { invitation, .. } => {
                ("ShowInvitation", Target::Invitation(*invitation))
            }
            Dialog::RevokeInvitation { invitation, .. } => {
                ("RevokeInvitation", Target::Invitation(*invitation))
            }
            Dialog::ContactDetails { contact, .. } => ("ContactDetails", Target::Contact(*contact)),
            Dialog::BlockContact { contact, .. } => ("BlockContact", Target::Contact(*contact)),
            Dialog::RemoveContact { contact, .. } => ("RemoveContact", Target::Contact(*contact)),
            Dialog::DeleteConversation { contact, .. } => {
                ("DeleteConversation", Target::Contact(*contact))
            }
            Dialog::AcceptRequest { request, .. } => ("AcceptRequest", Target::Request(*request)),
            Dialog::SendFile { contact, .. } => ("SendFile", Target::Contact(*contact)),
            Dialog::SaveFile { transfer, .. } => ("SaveFile", Target::Transfer(*transfer)),
            Dialog::CancelTransfer { transfer, .. } => {
                ("CancelTransfer", Target::Transfer(*transfer))
            }
            Dialog::RemoveFile { transfer, .. } => ("RemoveFile", Target::Transfer(*transfer)),
        };
        Self { kind, target }
    }

    /// What opening the dialog asks for.
    fn opens(self) -> Option<Write> {
        let command = match self.kind {
            "OwnCard" => "OwnCard",
            "ShowInvitation" => "InvitationCard",
            "SaveFile" => "SavePlace",
            _ => return None,
        };
        Some(Write {
            command,
            target: self.target,
        })
    }

    /// What confirming the dialog asks for; `created` if it shows an
    /// invitation already made.
    fn confirms(self, created: bool) -> Option<Write> {
        let command = match self.kind {
            "NewIdentity" => "CreateIdentity",
            "AddContact" => "AddContact",
            "NewInvitation" if !created => "CreateInvitation",
            "RevokeInvitation" => "RevokeInvitation",
            "ContactDetails" => "SetContactName",
            "BlockContact" => "BlockContact",
            "RemoveContact" => "RemoveContact",
            "DeleteConversation" => "DeleteConversation",
            "AcceptRequest" => "AcceptRequest",
            "DeleteIdentity" => "DeleteIdentity",
            "SendFile" => "OfferFile",
            "SaveFile" => "SaveTransfer",
            "CancelTransfer" => "CancelTransfer",
            "RemoveFile" => "RemoveTransfer",
            _ => return None,
        };
        Some(Write {
            command,
            target: self.target,
        })
    }
}

/// What the model showed before a step.
#[derive(Default)]
struct Seen {
    current: Option<UiIdentityId>,
    selections: BTreeMap<UiIdentityId, Option<Selection>>,
    drafts: BTreeMap<UiContactId, String>,
}

impl Seen {
    fn of(model: &Model) -> Self {
        let mut seen = Self {
            current: model.current(),
            ..Self::default()
        };
        for view in model.identities() {
            if let Some(slice) = model.slice(view.id) {
                seen.selections.insert(view.id, slice.selection);
                for (contact, thread) in &slice.threads {
                    seen.drafts.insert(*contact, thread.draft.clone());
                }
            }
        }
        seen
    }
}

/// One step: what it may do, and what it did.
#[derive(Default)]
struct Round {
    before: Seen,
    /// Answers delivered, with their commands.
    delivered: Vec<(Command, Outcome)>,
    /// The commands of the user the step may issue.
    allowed: Vec<Write>,
    /// The command of the user the step has to issue.
    must: Option<Write>,
    /// The selection the user asked for, in the identity shown.
    select: Option<(UiIdentityId, Selection)>,
    /// The identity the user asked to show.
    show: Option<UiIdentityId>,
    /// The conversation whose draft the user typed.
    draft: Option<UiContactId>,
}

/// What the checks remember from step to step.
#[derive(Default)]
struct Track {
    /// The last command taken.
    last: Option<CommandId>,
    /// What the open dialog was opened with.
    captured: Option<Captured>,
    /// Messages shown delivered in a conversation, and in the list.
    thread_delivered: BTreeSet<UiMessageId>,
    latest_delivered: BTreeSet<UiMessageId>,
    /// Transfers shown ended, as last shown.
    ended: BTreeMap<UiTransferId, TransferPhase>,
    /// Transfers listed, and those a read no longer listed.
    listed: BTreeSet<UiTransferId>,
    unlisted: BTreeSet<UiTransferId>,
}

// ---- The checks ----

/// A read names an identity the model lists, or a conversation it knows
/// and has read before.
fn check_read(model: &Model, command: &Command) {
    assert_eq!(
        model.phase(),
        Phase::Ready,
        "{command:?} asked while not ready"
    );
    match command {
        Command::LoadIdentity(asked) => assert!(
            model.identities().iter().any(|view| view.id == *asked),
            "{command:?}: an identity not listed"
        ),
        Command::LoadMessages { contact, .. } | Command::LoadEarlier { contact, .. } => {
            let read = model.slice(contact.identity()).is_some_and(|slice| {
                known(slice).contains(contact) && slice.threads.contains_key(contact)
            });
            assert!(read, "{command:?}: a conversation the model does not hold");
        }
        _ => {}
    }
}

/// A message is shown as the truth said it, for its own handle, and never
/// queued again after it was shown delivered at the same place.
fn check_message(
    env: &Env,
    delivered: &mut BTreeSet<UiMessageId>,
    contact: UiContactId,
    message: &MessageView,
) {
    assert_eq!(
        message.id.contact(),
        contact,
        "{:?} shown in the conversation of {contact:?}",
        message.id
    );
    assert!(
        env.said.statuses.contains(&(message.id, message.status)),
        "{:?} shown {:?}, which the runtime never said of it",
        message.id,
        message.status
    );
    let body = env
        .bodies
        .get(&message.id)
        .map(|(time, text)| (*time, text.as_str()));
    assert_eq!(
        body,
        Some((message.time, message.text.as_str())),
        "{:?} shown with a time or text it never had",
        message.id
    );
    match message.status {
        Status::Delivered => {
            delivered.insert(message.id);
        }
        Status::Queued => assert!(
            !delivered.contains(&message.id),
            "{:?} shown queued after it was shown delivered",
            message.id
        ),
        Status::Received => {}
    }
}

/// The card of a transfer `owner` lists as `view`, in the conversation of
/// `contact`.
fn check_card(
    env: &Env,
    track: &mut Track,
    owner: UiIdentityId,
    contact: UiContactId,
    view: &TransferView,
    item: &TransferItem,
) {
    let id = view.id;
    assert_eq!(id.contact(), contact, "{id:?} shown with {contact:?}");
    assert!(
        item.incoming == view.incoming && item.size == Some(view.size),
        "the card of {id:?} says another direction or size than its read"
    );
    assert!(
        agrees(item.phase, view),
        "{id:?} shown {:?}, read as {view:?}",
        item.phase
    );
    assert!(
        drawable(&item.name),
        "the name of {id:?} is drawn with a character that hides text or turns it: {}",
        item.name.escape_unicode()
    );
    // Bytes the runtime said of this transfer since it last lost track of
    // them in the read shown, never 0 or a number from before in place of
    // what it does not know, whenever the event that said them came.
    if let TransferPhase::Transferring { bytes } = item.phase {
        let said = env.said.bytes.get(&(id, bytes)).copied();
        let read = env
            .read_resets
            .get(&owner)
            .map(|(_, resets)| resets.get(&id).copied().unwrap_or(0))
            .unwrap_or_default();
        assert!(
            bytes <= view.size && said.is_some_and(|said| said >= read),
            "{id:?} shown with {bytes} of {} bytes, said {said:?}; the read shown \
             lost track of them {read} times",
            view.size
        );
    }
    if item.busy {
        assert!(
            env.waiting
                .iter()
                .any(|command| env.on_transfer.get(command) == Some(&id)),
            "the card of {id:?} waits, and no command on it does"
        );
    }
    // No resurrection: what ended stays ended, as it was, but for a
    // cancelled transfer the peer then completed.
    if let Some(before) = track.ended.get(&id).copied() {
        let completed = matches!(before, TransferPhase::Cancelled { .. })
            && item.phase == TransferPhase::Completed
            && !view.incoming;
        assert!(
            item.phase == before || completed,
            "{id:?} shown {:?} after it was shown {before:?}",
            item.phase
        );
    }
    if ended(item.phase) {
        track.ended.insert(id, item.phase);
    }
}

/// The transfers `owner` lists, and the cards of its conversations.
fn check_transfers(
    model: &Model,
    env: &Env,
    track: &mut Track,
    owner: UiIdentityId,
    slice: &Slice,
) {
    let contacts: BTreeSet<UiContactId> = slice
        .transfers
        .iter()
        .map(|view| view.id.contact())
        .chain(slice.unsent.iter().map(|unsent| unsent.contact))
        .chain(known(slice))
        .collect();
    let mut cards = 0_usize;
    for contact in contacts {
        for item in model.transfers_of(contact) {
            match item.key {
                TransferKey::Transfer(id) => {
                    cards += 1;
                    let view = slice
                        .transfers
                        .iter()
                        .find(|view| view.id == id)
                        .unwrap_or_else(|| {
                            panic!("a card of {id:?}, which {owner:?} does not list")
                        });
                    check_card(env, track, owner, contact, view, &item);
                }
                // A file being prepared waits for its offer.
                TransferKey::Preparing(command) => assert!(
                    env.waiting.contains(&command) && env.offers.get(&command) == Some(&contact),
                    "a file for {contact:?} shown prepared by {command:?}, which does not wait"
                ),
                TransferKey::Unsent(_) => assert!(
                    matches!(item.phase, TransferPhase::Unsent(failure)
                        if env.said.unsent.contains(&(contact, failure))),
                    "a file shown unsent to {contact:?} as {:?}, which no offer to it was answered with",
                    item.phase
                ),
            }
        }
    }
    assert_eq!(
        cards,
        slice.transfers.len(),
        "the transfers {owner:?} lists and their cards"
    );
}

fn check_slice(
    model: &Model,
    env: &Env,
    track: &mut Track,
    round: &Round,
    owner: UiIdentityId,
    slice: &Slice,
) {
    let known = known(slice);
    assert!(
        known.iter().all(|contact| contact.identity() == owner)
            && slice
                .transfers
                .iter()
                .all(|view| view.id.identity() == owner)
            && slice
                .interrupted
                .iter()
                .all(|transfer| transfer.identity() == owner)
            && slice
                .unsent
                .iter()
                .all(|unsent| unsent.contact.identity() == owner)
            && slice
                .requests
                .iter()
                .all(|view| view.id.identity() == owner)
            && slice
                .invitations
                .iter()
                .all(|view| view.id.identity() == owner)
            && slice.blocked.iter().all(|view| view.id.identity() == owner)
            && slice
                .threads
                .keys()
                .all(|contact| contact.identity() == owner)
            && slice
                .unread
                .keys()
                .all(|contact| contact.identity() == owner)
            && slice
                .deferrals
                .keys()
                .all(|contact| contact.identity() == owner)
            && slice
                .retries
                .keys()
                .all(|contact| contact.identity() == owner),
        "{owner:?} holds something of another identity"
    );
    // What is shown of the identity is an answer for it.
    if slice.loaded {
        let part = Part::shown(slice).expect("an identity read without its request mode");
        assert!(
            env.said
                .parts
                .get(&owner)
                .is_some_and(|parts| parts.contains(&part)),
            "what {owner:?} shows was never answered for it: {part:?}"
        );
    } else {
        assert!(
            known.is_empty()
                && slice.requests.is_empty()
                && slice.invitations.is_empty()
                && slice.blocked.is_empty()
                && slice.transfers.is_empty()
                && slice.interrupted.is_empty(),
            "{owner:?} shows something before it was read"
        );
    }
    // The window draws the cards of the identity shown; those of the others
    // are checked once everything is read again.
    if model.current() == Some(owner) {
        check_transfers(model, env, track, owner, slice);
    }
    if let Some(service) = slice.service {
        assert!(
            env.said
                .services
                .get(&owner)
                .is_some_and(|states| states.contains(&service)),
            "{owner:?} shows its service {service:?}, a state it was never in"
        );
    }
    for contact in &known {
        assert!(
            !env.heard_gone_records.contains(contact),
            "{contact:?} is shown after the model read that it is gone"
        );
    }
    // The selection names what the identity knows, and changes only by the
    // user, or to a contact a command of the user made.
    let before = round.before.selections.get(&owner).copied().flatten();
    if let Some(selection) = slice.selection {
        if let Selection::Conversation(contact) = selection {
            assert!(
                known.contains(&contact),
                "{owner:?} shows {contact:?}, which it does not know"
            );
        }
        if Some(selection) != before {
            let asked = round.select == Some((owner, selection));
            let opened = matches!(selection,
                Selection::Conversation(contact) if env.opened.contains(&contact));
            assert!(
                asked || opened,
                "{owner:?} shows {selection:?} instead of {before:?}, and nobody asked for it"
            );
        }
    }
    for (contact, thread) in &slice.threads {
        assert!(
            known.contains(contact),
            "a conversation of {contact:?}, which {owner:?} does not know"
        );
        assert!(
            thread
                .messages
                .windows(2)
                .all(|pair| pair[0].id.sequence() < pair[1].id.sequence()),
            "the messages of {contact:?} are out of order"
        );
        for message in &thread.messages {
            check_message(env, &mut track.thread_delivered, *contact, message);
        }
        if let Some(sending) = thread.sending {
            assert!(
                env.waiting.contains(&sending),
                "the draft of {contact:?} waits for {sending:?}, which was answered"
            );
        }
    }
    for conversation in &slice.conversations {
        if let Some(latest) = &conversation.latest {
            check_message(
                env,
                &mut track.latest_delivered,
                conversation.contact,
                latest,
            );
        }
    }
}

/// What the model holds of transfers, which its `Debug` leaves out.
fn transfers_held(model: &Model) -> String {
    let mut held = String::new();
    for view in model.identities() {
        let Some(slice) = model.slice(view.id) else {
            continue;
        };
        held.push_str(&format!(
            "{:?} {:?} {:?}",
            slice.transfers, slice.interrupted, slice.unsent
        ));
        let contacts: BTreeSet<UiContactId> = slice
            .transfers
            .iter()
            .map(|view| view.id.contact())
            .chain(known(slice))
            .collect();
        for contact in contacts {
            held.push_str(&format!("{:?}", model.transfers_of(contact)));
        }
    }
    held
}

/// A draft changes only by the user, or goes once the runtime queued it;
/// it stays while its conversation is known.
fn check_drafts(model: &Model, round: &Round) {
    for (contact, before) in &round.before.drafts {
        if round.draft == Some(*contact) {
            continue;
        }
        let after = model.thread(*contact).map(|thread| thread.draft.as_str());
        if after == Some(before.as_str()) {
            continue;
        }
        let still_known = model
            .slice(contact.identity())
            .is_some_and(|slice| known(slice).contains(contact));
        if after.is_none() && (before.is_empty() || !still_known) {
            // The conversation is gone, and its draft with it.
            continue;
        }
        let queued = after == Some("")
            && round.delivered.iter().any(|(command, outcome)| {
                matches!(command, Command::Send { contact: sent, text }
                    if sent == contact && text == before)
                    && matches!(outcome, Outcome::MessageQueued(message)
                        if message.contact() == *contact)
            });
        assert!(
            queued,
            "the draft of {contact:?} went from {before:?} to {after:?} by itself"
        );
    }
}

/// Everything that holds after every step.
fn check(model: &Model, env: &Env, track: &mut Track, round: &Round) {
    let listed: BTreeSet<UiIdentityId> = model.identities().iter().map(|view| view.id).collect();
    let mut list = model.identities().to_vec();
    list.sort_by_key(|view| view.id);
    if model.identities_loaded() {
        assert!(
            env.said.lists.contains(&list),
            "the identities {listed:?} were never answered"
        );
    } else {
        assert!(list.is_empty(), "identities listed before they were read");
    }
    for gone in &env.heard_gone_identities {
        assert!(
            !listed.contains(gone)
                && model.slice(*gone).is_none()
                && model.current() != Some(*gone),
            "{gone:?} is shown after the model read that it is gone"
        );
    }
    // The identity shown is listed, and changes only by the user, to an
    // identity the user made, or when it is gone.
    if let Some(current) = model.current() {
        assert!(
            listed.contains(&current),
            "{current:?} is shown, not listed"
        );
    }
    if model.current() != round.before.current {
        let moved = round.show == model.current()
            || model
                .current()
                .is_some_and(|current| env.made.contains(&current))
            || round
                .before
                .current
                .is_none_or(|before| !listed.contains(&before));
        assert!(
            moved,
            "the identity shown went from {:?} to {:?} by itself",
            round.before.current,
            model.current()
        );
    }
    let mut transfers = BTreeSet::new();
    for owner in env.identities_ever.iter().copied().chain([stranger()]) {
        let Some(slice) = model.slice(owner) else {
            continue;
        };
        assert!(listed.contains(&owner), "{owner:?} is held, not listed");
        check_slice(model, env, track, round, owner, slice);
        transfers.extend(slice.transfers.iter().map(|view| view.id));
    }
    // A transfer a read no longer listed is never listed again.
    for transfer in &transfers {
        assert!(
            !track.unlisted.contains(transfer),
            "{transfer:?} is listed again"
        );
    }
    let unlisted: Vec<UiTransferId> = track.listed.difference(&transfers).copied().collect();
    track.unlisted.extend(unlisted);
    track.listed = transfers;
    check_drafts(model, round);
    // A dialog keeps what it captured, and shows nothing of another
    // identity.
    match model.dialog() {
        None => track.captured = None,
        Some(dialog) => {
            assert_eq!(
                Some(Captured::of(dialog)),
                track.captured,
                "the dialog holds other handles than it was opened with"
            );
            if let Dialog::NewInvitation {
                identity: owner,
                created: Some((invitation, _)),
                ..
            } = dialog
            {
                assert_eq!(
                    invitation.identity(),
                    *owner,
                    "the invitation dialog of {owner:?} shows {invitation:?} as made"
                );
            }
            if let Some(busy) = dialog.busy() {
                assert!(
                    env.waiting.contains(&busy),
                    "the dialog waits for {busy:?}, which was answered"
                );
            }
            // The place to save to is typed, or the one suggested for the
            // transfer the dialog saves.
            if let Dialog::SaveFile {
                transfer,
                place,
                suggesting,
                ..
            } = dialog
            {
                assert!(
                    PLACES.contains(&place.as_str()) || *place == place_of(*transfer),
                    "the save dialog of {transfer:?} offers a place it was never given"
                );
                if let Some(asked) = suggesting {
                    assert!(
                        env.waiting.contains(asked),
                        "the save dialog waits for {asked:?}, which was answered"
                    );
                }
            }
        }
    }
    if model.phase() == Phase::Stopped {
        assert!(
            model.dialog().is_none(),
            "a dialog after the runtime stopped"
        );
    }
    if let Some(tor) = model.tor_check() {
        assert!(
            env.said.tor.contains(tor),
            "Tor shown as {tor:?}, never said"
        );
    }
}

// ---- The world: the model, the environment, and the checks ----

struct World {
    model: Model,
    env: Env,
    track: Track,
}

impl World {
    /// A profile open, with its identities listed and read.
    fn new() -> Self {
        let mut world = Self {
            model: Model::new(),
            env: Env::new(),
            track: Track::default(),
        };
        world.model.apply(Update::Started {
            profile: ProfileOnDisk::Exists,
            location: "/home/user/.local/share/monolith".to_owned(),
        });
        world
            .model
            .unlock(Zeroizing::new("correct horse battery staple".to_owned()));
        let opening = world.model.take_commands();
        assert_eq!(opening.len(), 1);
        let id = opening[0].0;
        world.track.last = Some(id);
        world.model.apply(Update::Done {
            command: id,
            outcome: Outcome::ProfileOpen {
                ephemeral: false,
                message_store_missing: false,
            },
        });
        assert_eq!(world.model.phase(), Phase::Ready);
        let round = world.begin();
        world.finish(round);
        world.quiesce();
        world
    }

    fn begin(&self) -> Round {
        Round {
            before: Seen::of(&self.model),
            ..Round::default()
        }
    }

    /// Takes the commands the step made, and checks them and the state.
    fn finish(&mut self, round: Round) {
        let commands = self.model.take_commands();
        let mut writes = Vec::new();
        for (id, command) in &commands {
            if let Some(last) = self.track.last {
                assert!(*id > last, "{id:?} issued after {last:?}");
            }
            self.track.last = Some(*id);
            match write_of(command) {
                Some(write) => writes.push(write),
                None => check_read(&self.model, command),
            }
            if let Command::Send { contact, text } = command {
                let draft = self
                    .model
                    .thread(*contact)
                    .map(|thread| thread.draft.as_str());
                assert_eq!(
                    draft,
                    Some(text.as_str()),
                    "a send of other text than the draft of {contact:?}"
                );
            }
            if let Command::OfferFile { contact, .. } = command {
                assert!(
                    self.model
                        .contact(*contact)
                        .is_some_and(|view| view.accepted)
                        && !self.model.storage_failed(),
                    "a file offered to {contact:?}, not shown as a contact who accepted"
                );
            }
        }
        assert!(writes.len() <= 1, "one step issued {writes:?}");
        for write in &writes {
            assert!(
                round.allowed.contains(write),
                "{write:?} issued; the step allowed {:?}",
                round.allowed
            );
        }
        if let Some(must) = round.must {
            assert_eq!(writes, [must], "the act did not issue its command");
        }
        self.env.take(commands);
        self.env.note_resets();
        self.model.take_notices();
        check(&self.model, &self.env, &mut self.track, &round);
    }

    /// Applies `event`. An event raises a notice only when it is a contact
    /// of the identity shown accepting, or a file offered to it outside the
    /// conversation shown: an event of another identity changes nothing
    /// shown (ADR 0011, P7-17).
    fn event(&mut self, event: RuntimeEvent) {
        let noticed = match event {
            RuntimeEvent::ContactAccepted(contact) => {
                self.model.current() == Some(contact.identity())
            }
            RuntimeEvent::TransferOffered(transfer) => {
                self.model.current() == Some(transfer.identity())
                    && self.model.selection() != Some(Selection::Conversation(transfer.contact()))
            }
            _ => false,
        };
        self.model.take_notices();
        self.model.apply(Update::Event(event.clone()));
        if !noticed {
            assert!(
                self.model.take_notices().is_empty(),
                "{event:?} raised a notice over {:?}",
                self.model.current()
            );
        }
    }

    fn pick_identity(&self, local: bool, pick: u8) -> UiIdentityId {
        if local {
            if let Some(current) = self.model.current() {
                return current;
            }
        }
        let mut pool = self.env.identities_ever.clone();
        pool.push(stranger());
        choose(&pool, pick).unwrap()
    }

    fn pick_contact(&self, local: bool, pick: u8) -> UiContactId {
        if local {
            if let Some(slice) = self.model.current_slice() {
                if let (Some(Selection::Conversation(shown)), 0) = (slice.selection, pick % 2) {
                    return shown;
                }
                let known: Vec<UiContactId> = known(slice).into_iter().collect();
                if let Some(contact) = choose(&known, pick / 2) {
                    return contact;
                }
            }
        }
        let mut pool = self.env.contacts_ever.clone();
        pool.extend([record(identity(1), 0xEE), record(stranger(), 0xEE)]);
        choose(&pool, pick).unwrap()
    }

    fn pick_request(&self, local: bool, pick: u8) -> UiRequestId {
        if local {
            if let Some(slice) = self.model.current_slice() {
                let ids: Vec<UiRequestId> = slice.requests.iter().map(|view| view.id).collect();
                if let Some(request) = choose(&ids, pick) {
                    return request;
                }
            }
        }
        let mut pool = self.env.requests_ever.clone();
        pool.push(UiRequestId::new(identity(1), 999));
        choose(&pool, pick).unwrap()
    }

    fn pick_invitation(&self, local: bool, pick: u8) -> UiInvitationId {
        if local {
            if let Some(slice) = self.model.current_slice() {
                let ids: Vec<UiInvitationId> =
                    slice.invitations.iter().map(|view| view.id).collect();
                if let Some(invitation) = choose(&ids, pick) {
                    return invitation;
                }
            }
        }
        let mut pool = self.env.invitations_ever.clone();
        pool.push(UiInvitationId::new(identity(2), 999));
        choose(&pool, pick).unwrap()
    }

    fn pick_blocked(&self, local: bool, pick: u8) -> UiBlockedId {
        if local {
            if let Some(slice) = self.model.current_slice() {
                let ids: Vec<UiBlockedId> = slice.blocked.iter().map(|view| view.id).collect();
                if let Some(blocked) = choose(&ids, pick) {
                    return blocked;
                }
            }
        }
        let mut pool = self.env.blocked_ever.clone();
        pool.push(UiBlockedId::new(identity(1), 999));
        choose(&pool, pick).unwrap()
    }

    fn pick_transfer(&self, local: bool, pick: u8) -> UiTransferId {
        if local {
            if let Some(slice) = self.model.current_slice() {
                let ids: Vec<UiTransferId> = slice.transfers.iter().map(|view| view.id).collect();
                if let Some(transfer) = choose(&ids, pick) {
                    return transfer;
                }
            }
        }
        choose(&self.env.transfer_pool(), pick).unwrap()
    }

    /// Opens `dialog` through `open`, as the window does; what it captured
    /// if it opened.
    fn opened(&mut self, round: &mut Round, open: impl FnOnce(&mut Model)) {
        let before = self.model.dialog_instance();
        open(&mut self.model);
        if self.model.dialog_instance() != before {
            if let Some(dialog) = self.model.dialog() {
                let captured = Captured::of(dialog);
                round.allowed.extend(captured.opens());
                self.track.captured = Some(captured);
            }
        }
    }

    /// Types the place `place` in the save dialog, if one is open and does
    /// not wait, and saves if `confirm`. Whether one was.
    fn save(&mut self, round: &mut Round, confirm: bool, place: u8) -> bool {
        let Some(Dialog::SaveFile {
            place: typed,
            busy: None,
            ..
        }) = self.model.dialog_mut()
        else {
            return false;
        };
        let place = PLACES[usize::from(place) % PLACES.len()];
        if !place.is_empty() {
            *typed = place.to_owned();
        }
        if confirm {
            self.confirm(round);
        }
        true
    }

    /// Takes one of the acts the card of `transfer` offers, as the window
    /// does; any act if the model shows no card of it. A save dialog that
    /// is open, or opens, takes the place typed, and saves if `confirm`.
    fn card(
        &mut self,
        round: &mut Round,
        transfer: UiTransferId,
        act: u8,
        confirm: bool,
        place: u8,
    ) {
        if self.save(round, confirm, place) {
            return;
        }
        let card = self
            .model
            .transfers_of(transfer.contact())
            .into_iter()
            .find(|item| item.key == TransferKey::Transfer(transfer));
        // Mostly what the window offers on the card; otherwise any act,
        // which the model refuses unless it applies.
        let acts = match card {
            Some(item) if act & 0x80 == 0 => offered(item.phase),
            _ => &ACTS[..],
        };
        let Some(act) = choose(acts, act) else {
            return;
        };
        let view = self.model.transfer(transfer).cloned();
        match act {
            // The dialog opens over none, and asks where the file would go;
            // then, as a step of its own, the user types and saves.
            Act::Save => {
                self.model.close();
                self.opened(round, |model| model.open_save(transfer));
                let opening = core::mem::replace(round, self.begin());
                self.finish(opening);
                self.save(round, confirm, place);
                return;
            }
            Act::Cancel => self.opened(round, |model| {
                model.open(Dialog::CancelTransfer {
                    transfer,
                    busy: None,
                    error: None,
                });
            }),
            // A received file not saved goes once the user confirms.
            Act::Remove
                if view
                    .as_ref()
                    .is_some_and(|view| view.status == TransferStatus::Received && !view.saved) =>
            {
                self.opened(round, |model| {
                    model.open(Dialog::RemoveFile {
                        transfer,
                        busy: None,
                        error: None,
                    });
                });
            }
            Act::Accept | Act::Reject | Act::Resume | Act::Remove => {
                let command = match act {
                    Act::Accept => "AcceptTransfer",
                    Act::Reject => "RejectTransfer",
                    Act::Resume => "ResumeTransfer",
                    _ => "RemoveTransfer",
                };
                let write = Write {
                    command,
                    target: Target::Transfer(transfer),
                };
                // Refused before the runtime is asked unless the model
                // lists the transfer, in a state the act applies to, and
                // shows no command on it under way.
                if view.as_ref().is_some_and(|view| applies(act, view))
                    && !self.model.transfer_busy(transfer)
                {
                    round.allowed.push(write);
                    round.must = Some(write);
                }
                match act {
                    Act::Accept => self.model.accept_transfer(transfer),
                    Act::Reject => self.model.reject_transfer(transfer),
                    Act::Resume => self.model.resume_transfer(transfer),
                    _ => self.model.remove_transfer(transfer),
                }
                return;
            }
        }
        if confirm {
            self.confirm(round);
        }
    }

    fn dialog(&self, kind: DialogKind, local: bool, pick: u8) -> Dialog {
        let (busy, error) = (None, None);
        match kind {
            DialogKind::NewIdentity => Dialog::NewIdentity {
                name: "Work".to_owned(),
                busy,
                error,
            },
            DialogKind::AddContact { person } => Dialog::AddContact {
                identity: self.pick_identity(local, pick),
                card: card(person),
                name: "Gina".to_owned(),
                busy,
                error,
                added: None,
            },
            DialogKind::OwnCard => Dialog::OwnCard {
                identity: self.pick_identity(local, pick),
                card: None,
                busy,
                error,
            },
            DialogKind::NewInvitation => Dialog::NewInvitation {
                identity: self.pick_identity(local, pick),
                label: "Friends".to_owned(),
                created: None,
                busy,
                error,
            },
            DialogKind::ShowInvitation => Dialog::ShowInvitation {
                invitation: self.pick_invitation(local, pick),
                card: None,
                busy,
                error,
            },
            DialogKind::RevokeInvitation { discard } => Dialog::RevokeInvitation {
                invitation: self.pick_invitation(local, pick),
                discard_requests: discard,
                busy,
                error,
            },
            DialogKind::ContactDetails => Dialog::ContactDetails {
                contact: self.pick_contact(local, pick),
                name: "Hal".to_owned(),
                busy,
                error,
            },
            DialogKind::BlockContact => Dialog::BlockContact {
                contact: self.pick_contact(local, pick),
                busy,
                error,
            },
            DialogKind::RemoveContact => Dialog::RemoveContact {
                contact: self.pick_contact(local, pick),
                busy,
                error,
            },
            DialogKind::DeleteConversation => Dialog::DeleteConversation {
                contact: self.pick_contact(local, pick),
                busy,
                error,
            },
            DialogKind::AcceptRequest => Dialog::AcceptRequest {
                request: self.pick_request(local, pick),
                name: "Ivy".to_owned(),
                busy,
                error,
            },
            DialogKind::DeleteIdentity => Dialog::DeleteIdentity {
                identity: self.pick_identity(local, pick),
                busy,
                error,
            },
            DialogKind::TorDetails => Dialog::TorDetails,
        }
    }

    fn confirm(&mut self, round: &mut Round) {
        if let Some(dialog) = self.model.dialog() {
            if dialog.busy().is_none() {
                let created = matches!(
                    dialog,
                    Dialog::NewInvitation {
                        created: Some(_),
                        ..
                    }
                );
                if let Some(captured) = self.track.captured {
                    round.allowed.extend(captured.confirms(created));
                }
            }
        }
        self.model.confirm();
    }

    /// An answer to a number nothing waits for changes nothing.
    fn stale_answer(&mut self, pick: u8, kind: u8) {
        let answered: Vec<CommandId> = self.env.answered.iter().copied().collect();
        let id = match choose(&answered, pick / 2) {
            Some(old) if pick % 2 == 0 => old,
            _ => CommandId::new(self.track.last.map_or(0, |last| last.get() + 1) + u64::from(pick)),
        };
        assert!(!self.env.waiting.contains(&id));
        let outcome = self.env.any_outcome(kind, pick);
        let before = format!("{:?} {}", self.model, transfers_held(&self.model));
        self.model.apply(Update::Done {
            command: id,
            outcome,
        });
        assert_eq!(
            before,
            format!("{:?} {}", self.model, transfers_held(&self.model)),
            "an answer to {id:?}, which nothing waits for, changed the model"
        );
    }

    fn act(&mut self, step: Step, round: &mut Round) {
        match step {
            Step::SelectIdentity(pick) => {
                let chosen = self.pick_identity(false, pick);
                round.show = Some(chosen);
                self.model.select_identity(chosen);
            }
            Step::SelectConversation { local, pick } => {
                let contact = self.pick_contact(local, pick);
                if let Some(current) = self.model.current() {
                    round.select = Some((current, Selection::Conversation(contact)));
                }
                self.model.select_conversation(contact);
            }
            Step::ShowRequests => {
                if let Some(current) = self.model.current() {
                    round.select = Some((current, Selection::Requests));
                }
                self.model.show_requests();
            }
            Step::Open {
                dialog,
                local,
                pick,
                confirm,
            } => {
                let dialog = self.dialog(dialog, local, pick);
                let captured = Captured::of(&dialog);
                // A dialog that waits for its command stays open instead.
                if self.model.dialog().and_then(Dialog::busy).is_none() {
                    round.allowed.extend(captured.opens());
                    self.track.captured = Some(captured);
                }
                self.model.open(dialog);
                if confirm {
                    self.confirm(round);
                }
            }
            Step::Confirm => self.confirm(round),
            Step::Close => self.model.close(),
            Step::Send { local, pick, text } => {
                let contact = self.pick_contact(local, pick);
                round.draft = Some(contact);
                if let Some(draft) = self.model.draft_mut(contact) {
                    *draft = TEXTS[usize::from(text) % TEXTS.len()].to_owned();
                }
                round.allowed.push(Write {
                    command: "Send",
                    target: Target::Contact(contact),
                });
                self.model.send(contact);
            }
            Step::Answer { local, pick, block } => {
                let request = self.pick_request(local, pick);
                let command = if block {
                    self.model.block_sender(request);
                    "BlockRequest"
                } else {
                    self.model.decline(request);
                    "DeclineRequest"
                };
                round.allowed.push(Write {
                    command,
                    target: Target::Request(request),
                });
            }
            Step::Verify {
                local,
                pick,
                verified,
            } => {
                let contact = self.pick_contact(local, pick);
                round.allowed.push(Write {
                    command: "SetVerified",
                    target: Target::Contact(contact),
                });
                self.model.set_verified(contact, verified);
            }
            Step::Unblock { local, pick } => {
                let blocked = self.pick_blocked(local, pick);
                round.allowed.push(Write {
                    command: "Unblock",
                    target: Target::Blocked(blocked),
                });
                self.model.unblock(blocked);
            }
            Step::Mode { local, pick, mode } => {
                let owner = self.pick_identity(local, pick);
                round.allowed.push(Write {
                    command: "SetRequestMode",
                    target: Target::Identity(owner),
                });
                self.model
                    .set_request_mode(owner, MODES[usize::from(mode) % MODES.len()]);
            }
            Step::Card {
                local,
                pick,
                act,
                confirm,
                place,
            } => {
                let transfer = self.pick_transfer(local, pick);
                self.card(round, transfer, act, confirm, place);
            }
            Step::SendFile {
                local,
                pick,
                path,
                confirm,
            } => {
                let contact = self.pick_contact(local, pick);
                let path = PATHS[usize::from(path) % PATHS.len()].to_owned();
                // The dialog opens over none.
                self.model.close();
                self.opened(round, |model| model.open_send_file(contact, Some(path)));
                if confirm {
                    self.confirm(round);
                }
            }
            Step::CheckTor => self.model.check_tor(),
            Step::Run { pick, fate, notify } => {
                self.env.run(&mut self.model, round, pick, fate, notify);
            }
            Step::RunAll => self.env.run_all(&mut self.model, round),
            Step::Deliver(pick) => self.env.deliver_held(&mut self.model, round, pick),
            Step::StaleAnswer { pick, kind } => self.stale_answer(pick, kind),
            Step::Change { change, notify } => {
                if let Some(event) = self.env.change(change) {
                    if notify {
                        self.event(event);
                    }
                }
            }
            Step::Missed => self.event(RuntimeEvent::Missed),
            Step::Spurious { kind, pick } => {
                let event = self.env.spurious(kind, pick);
                self.event(event);
            }
            Step::StorageFails => self.model.apply(Update::Event(RuntimeEvent::StorageFailed)),
            Step::Stopped => {
                self.env.stop();
                self.model.apply(Update::Stopped);
            }
            Step::Settle => {}
        }
    }

    fn play(&mut self, step: &Step) {
        if matches!(step, Step::Settle) {
            self.settle();
            return;
        }
        let mut round = self.begin();
        if self.env.stopped {
            // The window says Monolith stopped; late answers still come.
            match *step {
                Step::Deliver(pick) => self.env.deliver_held(&mut self.model, &mut round, pick),
                Step::StaleAnswer { pick, kind } => self.stale_answer(pick, kind),
                _ => {}
            }
        } else {
            self.act(*step, &mut round);
        }
        self.finish(round);
    }

    /// Runs and answers everything asked, until nothing more is.
    fn quiesce(&mut self) {
        for _ in 0..32 {
            if self.env.inbox.is_empty() {
                return;
            }
            let mut round = self.begin();
            self.env.run_all(&mut self.model, &mut round);
            self.finish(round);
        }
        panic!("the model keeps asking: {:?}", self.env.inbox);
    }

    /// The late answers arrive; then events are missed, and every read is
    /// answered. The model shows the truth.
    fn settle(&mut self) {
        let mut round = self.begin();
        for answer in core::mem::take(&mut self.env.held) {
            self.env.deliver(&mut self.model, &mut round, answer);
        }
        self.finish(round);
        self.cards();
        if self.env.stopped {
            return;
        }
        self.quiesce();
        let round = self.begin();
        self.model.apply(Update::Event(RuntimeEvent::Missed));
        self.finish(round);
        self.quiesce();
        self.cards();
        self.reconstructed();
    }

    /// Checks the cards of every identity, as after every step those of
    /// the identity shown.
    fn cards(&mut self) {
        let Self { model, env, track } = self;
        for view in model.identities() {
            if let Some(slice) = model.slice(view.id) {
                check_transfers(model, env, track, view.id, slice);
            }
        }
    }

    /// The model shows exactly the truth, except for what waits for an
    /// answer that never comes.
    fn reconstructed(&self) {
        let (model, env) = (&self.model, &self.env);
        let lost = |lost: &dyn Fn(&Command) -> bool| env.lost.iter().any(lost);
        if lost(&|command| matches!(command, Command::LoadIdentities)) {
            return;
        }
        let mut list = model.identities().to_vec();
        list.sort_by_key(|view| view.id);
        assert_eq!(list, env.list(), "the identities, once read again");
        match model.current() {
            Some(current) => assert!(env.find(current).is_some(), "{current:?} shown, gone"),
            None => assert!(env.identities.is_empty(), "no identity shown"),
        }
        for truth in &env.identities {
            if lost(
                &|command| matches!(command, Command::LoadIdentity(asked) if *asked == truth.id),
            ) {
                continue;
            }
            let slice = model.slice(truth.id).expect("an identity not held");
            let snapshot = truth.snapshot();
            assert!(slice.loaded, "{:?} not read", truth.id);
            assert_eq!(
                Part::shown(slice),
                Some(Part::of(&snapshot)),
                "{:?}, once read again",
                truth.id
            );
            assert_eq!(slice.service, Some(snapshot.service));
            // No bytes are left over from before: those of the truth, or
            // none it does not know.
            for held in truth.transfers() {
                if !held.moving() || held.held {
                    continue;
                }
                let shown = model
                    .transfers_of(held.id.contact())
                    .into_iter()
                    .find(|item| item.key == TransferKey::Transfer(held.id))
                    .map(|item| item.phase);
                let bytes = held.bytes.map_or(TransferPhase::Restoring, |bytes| {
                    TransferPhase::Transferring { bytes }
                });
                assert_eq!(shown, Some(bytes), "{:?}, once read again", held.id);
            }
            let unread = |contact: UiContactId| {
                lost(&|command| {
                    matches!(command,
                        Command::LoadMessages { contact: asked, .. }
                        | Command::LoadEarlier { contact: asked, .. } if *asked == contact)
                })
            };
            if let Some(Selection::Conversation(shown)) = slice.selection {
                assert!(
                    unread(shown) || slice.threads.contains_key(&shown),
                    "the conversation shown, {shown:?}, is not read"
                );
            }
            for (contact, thread) in &slice.threads {
                if unread(*contact) {
                    continue;
                }
                let held = truth.record(*contact).expect("a conversation that is gone");
                let messages: Vec<MessageView> = held
                    .messages
                    .iter()
                    .map(|message| message.view(*contact))
                    .collect();
                assert_eq!(
                    thread.messages, messages,
                    "the messages of {contact:?}, once read again"
                );
                assert!(thread.complete && !thread.loading);
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 1024,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    #[test]
    fn the_model_shows_what_the_runtime_said_and_acts_on_what_was_named(
        steps in prop::collection::vec(step(), 1..=60)
    ) {
        let mut world = World::new();
        for step in &steps {
            world.play(step);
        }
        world.settle();
    }
}

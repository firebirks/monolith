//! The UI model under generated sequences of what the user does and what
//! the runtime does (`docs/adr/0011-ui-security-boundary.md`). The runtime
//! is played by an environment that keeps the truth in a model of its own,
//! written here: identities, their contact records, the conversations with
//! their messages and statuses, requests and invitations. It runs the
//! commands the model issues against that truth and answers them at once,
//! late, in another order, never, or with handles of another identity or
//! record, as the sequence says. It changes the truth by itself (messages
//! arrive and are acknowledged, requests arrive, contacts accept, a record
//! is made again for the same person, an identity goes away) and raises the
//! events of the changes, or loses them; and it raises events that name
//! anything, deleted records and other identities included.
//!
//! After every step:
//!
//! - commands are numbered in order; a command of the user names exactly
//!   what the action named (for a confirmation, what its dialog captured);
//!   answers and events make the model issue reads only, of identities it
//!   lists and conversations it knows;
//! - the identities listed, and what is shown of each identity, are exactly
//!   an answer the truth gave for that identity, never mixed with another;
//!   a message is shown with a status and a text the truth gave for that
//!   exact handle, and once shown delivered it is never shown queued again;
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
//! `Missed` is raised and every read is answered: the model then shows
//! exactly the truth, and the conversation shown is read.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::{BTreeMap, BTreeSet};

use monolith_ui_model::{
    Command, CommandId, ContactView, ConversationView, Deferral, Dialog, Failure, IdentitySnapshot,
    IdentityView, InvitationView, MessageView, Model, NoticeKind, Outcome, Phase, ProfileOnDisk,
    RequestMode, RequestView, RuntimeEvent, Selection, Service, Slice, Status, TorCheck,
    UiContactId, UiIdentityId, UiInvitationId, UiMessageId, UiRequestId, Update,
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

/// A snapshot of an identity that holds nothing.
fn empty_snapshot(owner: UiIdentityId) -> IdentitySnapshot {
    IdentitySnapshot {
        identity: owner,
        contacts: Vec::new(),
        conversations: Vec::new(),
        requests: Vec::new(),
        invitations: Vec::new(),
        service: Service::Stopped,
        request_mode: RequestMode::Closed,
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

/// A contact record: a contact while `current`; after that an ended
/// conversation while it holds messages, and nothing once it holds none.
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
}

impl Record {
    fn exists(&self) -> bool {
        self.current || !self.messages.is_empty()
    }

    fn contact(&self) -> ContactView {
        ContactView {
            id: self.id,
            name: self.name.clone(),
            fingerprint: fingerprint(self.person),
            full_fingerprint: format!("{} 2222 3333", fingerprint(self.person)),
            accepted: self.accepted,
            verified: self.verified,
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

#[derive(Clone, Debug)]
struct Identity {
    id: UiIdentityId,
    name: Option<String>,
    records: Vec<Record>,
    requests: Vec<Request>,
    invitations: Vec<Invitation>,
    service: Service,
    mode: RequestMode,
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
            service: self.service,
            request_mode: self.mode,
        }
    }

    fn record(&self, contact: UiContactId) -> Option<&Record> {
        self.records.iter().find(|found| found.id == contact)
    }
}

/// What a slice shows of its identity, as a snapshot gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Part {
    contacts: Vec<ContactView>,
    conversations: Vec<ConversationView>,
    requests: Vec<RequestView>,
    invitations: Vec<InvitationView>,
    mode: RequestMode,
}

impl Part {
    fn of(snapshot: &IdentitySnapshot) -> Self {
        Self {
            contacts: snapshot.contacts.clone(),
            conversations: snapshot.conversations.clone(),
            requests: snapshot.requests.clone(),
            invitations: snapshot.invitations.clone(),
            mode: snapshot.request_mode,
        }
    }

    fn shown(slice: &Slice) -> Option<Self> {
        Some(Self {
            contacts: slice.contacts.clone(),
            conversations: slice.conversations.clone(),
            requests: slice.requests.clone(),
            invitations: slice.invitations.clone(),
            mode: slice.request_mode?,
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
    messages_ever: Vec<UiMessageId>,
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
    /// Alice and Bob. Alice has Carol, with a conversation, Dan, and Erin
    /// who has not accepted yet, an invitation and a request through it.
    /// Bob has Frank, the same person as Carol, with a conversation, an
    /// ended conversation, an invitation and a request.
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
            messages_ever: Vec::new(),
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
        env.add_record(alice, 1, true, Some("Dan".to_owned()));
        env.add_record(alice, 2, false, Some("Erin".to_owned()));
        let invitation = env.add_invitation(alice, Some("Friends".to_owned()));
        env.add_request(alice, 3, Some(invitation));
        let bob = env.add_identity(
            Some("Bob".to_owned()),
            Service::Publishing,
            RequestMode::Open,
        );
        let frank = env.add_record(bob, 0, true, Some("Frank".to_owned()));
        env.add_message(frank, Status::Delivered, TEXTS[2]);
        let ended = env.add_record(bob, 3, true, None);
        env.add_message(ended, Status::Received, TEXTS[0]);
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

    /// Records that are no contact and hold no message are gone.
    fn tidy(&mut self, owner: UiIdentityId) {
        if let Some(found) = self.find_mut(owner) {
            found.records.retain(Record::exists);
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
            service,
            mode,
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
        });
        self.contacts_ever.push(id);
        id
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

    fn take_request(&mut self, request: UiRequestId) -> Option<Request> {
        let found = self.find_mut(request.identity())?;
        let index = found.requests.iter().position(|held| held.id == request)?;
        Some(found.requests.remove(index))
    }

    // ---- Running commands ----

    /// Runs `command` against the truth.
    fn execute(&mut self, command: &Command) -> Ran {
        let mut ran = Ran {
            outcome: Outcome::Done,
            tells: Tells::Nothing,
            events: Vec::new(),
        };
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
            Command::BlockContact(contact) | Command::RemoveContact(contact) => {
                self.change_contact(*contact, |found| found.current = false)
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
            Command::DeclineRequest(request) | Command::BlockRequest(request) => {
                if self.take_request(*request).is_some() {
                    Outcome::Done
                } else {
                    Outcome::Failed(Failure::NoSuchRequest)
                }
            }
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
        };
        ran
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
        Outcome::ContactAdded(contact)
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
                Outcome::ContactAdded(_),
            ) => (
                Outcome::ContactAdded(self.foreign_contact(*owner, k)),
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
        }
    }

    /// Takes the commands the model issued: they wait for an answer.
    fn take(&mut self, commands: Vec<(CommandId, Command)>) {
        for (id, _) in &commands {
            self.waiting.insert(*id);
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
            Fate::Fail => Ran {
                outcome: Outcome::Failed(Failure::Disk),
                tells: Tells::Nothing,
                events: Vec::new(),
            },
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
        }
    }

    /// An event of any kind about any handle, deleted records, other
    /// identities and handles never made included, that changes nothing in
    /// the truth. A service is reported in a state it was in.
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
        match kind % 9 {
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
            _ => {
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
        }
    }

    /// An outcome of any kind, for a number nothing waits for.
    fn any_outcome(&self, kind: u8, pick: u8) -> Outcome {
        let contact = choose(&self.contacts_ever, pick).unwrap();
        let owner = choose(&self.identities_ever, pick).unwrap();
        match kind % 12 {
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
            5 => Outcome::ContactAdded(contact),
            6 => Outcome::MessageQueued(UiMessageId::new(contact, 1)),
            7 => Outcome::RequestAccepted(contact),
            8 => Outcome::InvitationCreated {
                invitation: UiInvitationId::new(owner, 77),
                card: Zeroizing::new("invitation card".to_owned()),
            },
            9 => Outcome::Tor(TorCheck::NotRunning),
            10 => Outcome::IdentityCreated(owner),
            _ => Outcome::ProfileOpen {
                ephemeral: true,
                message_store_missing: true,
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
        3 => (local(), pick(), pick())
            .prop_map(|(local, pick, mode)| Step::Mode { local, pick, mode }),
        3 => Just(Step::CheckTor),
        48 => (newest(), fate(), notify())
            .prop_map(|(pick, fate, notify)| Step::Run { pick, fate, notify }),
        30 => Just(Step::RunAll),
        18 => pick().prop_map(Step::Deliver),
        6 => (pick(), pick()).prop_map(|(pick, kind)| Step::StaleAnswer { pick, kind }),
        36 => (change(), notify()).prop_map(|(change, notify)| Step::Change { change, notify }),
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
        };
        Self { kind, target }
    }

    /// What opening the dialog asks for.
    fn opens(self) -> Option<Write> {
        let command = match self.kind {
            "OwnCard" => "OwnCard",
            "ShowInvitation" => "InvitationCard",
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

fn check_slice(env: &Env, track: &mut Track, round: &Round, owner: UiIdentityId, slice: &Slice) {
    let known = known(slice);
    assert!(
        known.iter().all(|contact| contact.identity() == owner)
            && slice
                .requests
                .iter()
                .all(|view| view.id.identity() == owner)
            && slice
                .invitations
                .iter()
                .all(|view| view.id.identity() == owner)
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
            known.is_empty() && slice.requests.is_empty() && slice.invitations.is_empty(),
            "{owner:?} shows something before it was read"
        );
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
    for owner in env.identities_ever.iter().copied().chain([stranger()]) {
        let Some(slice) = model.slice(owner) else {
            continue;
        };
        assert!(listed.contains(&owner), "{owner:?} is held, not listed");
        check_slice(env, track, round, owner, slice);
    }
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
        }
        assert!(writes.len() <= 1, "one step issued {writes:?}");
        for write in &writes {
            assert!(
                round.allowed.contains(write),
                "{write:?} issued; the step allowed {:?}",
                round.allowed
            );
        }
        self.env.take(commands);
        self.model.take_notices();
        check(&self.model, &self.env, &mut self.track, &round);
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
        let before = format!("{:?}", self.model);
        self.model.apply(Update::Done {
            command: id,
            outcome,
        });
        assert_eq!(
            before,
            format!("{:?}", self.model),
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
            Step::Mode { local, pick, mode } => {
                let owner = self.pick_identity(local, pick);
                round.allowed.push(Write {
                    command: "SetRequestMode",
                    target: Target::Identity(owner),
                });
                self.model
                    .set_request_mode(owner, MODES[usize::from(mode) % MODES.len()]);
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
                        self.model.apply(Update::Event(event));
                    }
                }
            }
            Step::Missed => self.model.apply(Update::Event(RuntimeEvent::Missed)),
            Step::Spurious { kind, pick } => {
                let event = self.env.spurious(kind, pick);
                self.model.apply(Update::Event(event));
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
        if self.env.stopped {
            return;
        }
        self.quiesce();
        let round = self.begin();
        self.model.apply(Update::Event(RuntimeEvent::Missed));
        self.finish(round);
        self.quiesce();
        self.reconstructed();
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

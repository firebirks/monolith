//! The state of the client (`docs/adr/0011-ui-security-boundary.md`).
//!
//! Two kinds of state live here, kept apart:
//!
//! - what the runtime said: identities, and per identity its contacts,
//!   conversations, requests, invitations, the people it blocked, the
//!   state of its service, and the messages of the conversations that were
//!   read. It changes only when an answer to a query arrives, for the
//!   handles that query named; an event only marks a part as stale, and
//!   that part is asked again;
//! - what the user is doing: the identity and the conversation shown,
//!   drafts, the open dialog, notices. A dialog that confirms an action
//!   holds the handles it acts on from the moment it opens.
//!
//! Every command carries the handles it acts on and is remembered with
//! them until its answer comes; the answer is applied to those handles,
//! never to whatever is shown at that moment.

use std::collections::{BTreeMap, BTreeSet};

use core::fmt;
use core::ops::Range;

use zeroize::{Zeroize, Zeroizing};

use crate::data::{
    BlockedView, ContactView, ConversationView, IdentityView, InvitationView, MessageView,
    RequestMode, RequestView, RuntimeEvent, Service, Status, TorCheck,
};
use crate::failure::Failure;
use crate::ids::{
    UiBlockedId, UiContactId, UiIdentityId, UiInvitationId, UiMessageId, UiRequestId,
};
use crate::present::{self, Prepared};
use crate::prose::Prose;
use crate::protocol::{Command, CommandId, IdentitySnapshot, Outcome, ProfileOnDisk, Update};

/// How many messages a conversation reads at a time.
pub const PAGE: usize = 100;

/// The most messages of one conversation kept in memory.
pub const MAX_WINDOW: usize = 2_000;

/// What the user is told once someone is unblocked: the runtime keeps no
/// record of them, so they are not a contact again.
pub const UNBLOCKED: &str = "Unblocked. They become a contact again only if you add their card \
                             or accept a request from them.";

/// Where the client is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Waiting for the bridge to say whether a profile exists.
    Starting,
    /// No profile exists: the first start.
    Welcome,
    /// A profile exists and waits for its passphrase.
    Locked,
    /// A profile is being opened or created.
    Opening,
    /// A profile is open.
    Ready,
    /// The runtime stopped.
    Stopped,
}

/// What the top bar says of Tor, from the state of the services and the
/// last check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TorIndicator {
    /// Nothing known yet.
    Starting,
    /// Tor answers and the service is being published, or Tor is building a
    /// circuit.
    Connecting,
    /// The service of the identity shown is published.
    Connected,
    /// Tor cannot be used now.
    Unavailable,
}

/// What a notice is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    /// Something done.
    Success,
    /// Something to know.
    Info,
    /// Something that failed.
    Error,
}

/// A short notice for the user.
#[derive(Clone, PartialEq, Eq)]
pub struct Notice {
    /// What it is about.
    pub kind: NoticeKind,
    /// The sentence, with the names in it kept apart ([`Prose`]).
    pub text: Prose,
    /// An invitation the notice offers to revoke: its action asks whether
    /// to revoke it.
    pub revoke: Option<UiInvitationId>,
}

impl fmt::Debug for Notice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The sentence can name a contact.
        f.debug_struct("Notice")
            .field("kind", &self.kind)
            .field("revoke", &self.revoke)
            .finish_non_exhaustive()
    }
}

/// What a contact that has not accepted waits for (`PROTOCOL.md` section
/// 12.2, `DESIGN_QUESTIONS.md` P6-6 and P6-20).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waiting {
    /// Its card is an invitation and nothing was written to it: the
    /// request goes out with the first message.
    FirstMessage,
    /// Its card is an invitation and messages were written to it: the
    /// request goes out with them, and waits to be accepted.
    Acceptance,
    /// Its card is not an invitation: it has to add the card of this
    /// identity too.
    TheirCard,
}

/// What `contact` waits for, its conversation holding `messages`
/// messages; `None` once it has accepted.
pub fn waiting(contact: &ContactView, messages: usize) -> Option<Waiting> {
    if contact.accepted {
        None
    } else if !contact.invitation {
        Some(Waiting::TheirCard)
    } else if messages == 0 {
        Some(Waiting::FirstMessage)
    } else {
        Some(Waiting::Acceptance)
    }
}

/// What a sidebar row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RowState {
    /// A contact who has accepted.
    Contact,
    /// A contact who has not accepted yet.
    Waiting,
    /// A conversation whose contact was blocked or removed.
    Ended,
}

/// A row of the sidebar: a contact, a conversation, or both.
#[derive(Clone, PartialEq, Eq)]
pub struct SidebarRow {
    /// The record.
    pub contact: UiContactId,
    /// The name shown.
    pub name: String,
    /// The user gave the contact a name; without one it is named by its
    /// fingerprint.
    pub named: bool,
    /// The fingerprint, compact.
    pub fingerprint: String,
    /// What it is.
    pub state: RowState,
    /// What a contact that has not accepted waits for.
    pub waiting: Option<Waiting>,
    /// The contact was verified.
    pub verified: bool,
    /// The newest message, if any.
    pub latest: Option<MessageView>,
    /// The newest message in one line, at most [`present::PREVIEW_CHARS`]
    /// characters; empty without one.
    pub preview: String,
    /// Messages received while the conversation was not shown.
    pub unread: u32,
    /// Messages wait to be delivered.
    pub queued: usize,
    /// The name in lower case, to sort and filter by.
    key: String,
}

impl SidebarRow {
    /// Whether the name holds `filter`, given in lower case. Every name
    /// holds the empty filter.
    pub fn matches(&self, filter: &str) -> bool {
        self.key.contains(filter)
    }
}

impl fmt::Debug for SidebarRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SidebarRow")
            .field("contact", &self.contact)
            .field("state", &self.state)
            .field("waiting", &self.waiting)
            .field("verified", &self.verified)
            .field("latest", &self.latest)
            .field("unread", &self.unread)
            .field("queued", &self.queued)
            .finish_non_exhaustive()
    }
}

/// What the main pane of an identity shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    /// A conversation.
    Conversation(UiContactId),
    /// The waiting contact requests.
    Requests,
}

/// The messages of one conversation that were read, and the draft.
#[derive(Default)]
pub struct Thread {
    /// Messages, oldest first.
    pub messages: Vec<MessageView>,
    /// No older message exists.
    pub complete: bool,
    /// A read is under way.
    pub loading: bool,
    /// The window must be read again.
    stale: bool,
    /// The draft.
    pub draft: String,
    /// The draft is being queued by this command.
    pub sending: Option<CommandId>,
    /// Since the send under way began, the draft has started with the text
    /// it sends: only then does that text leave the draft once it is
    /// queued.
    draft_starts_with_sent: bool,
    /// Why the last send failed.
    pub send_error: Option<Failure>,
    /// Messages the user expanded past the line limit.
    pub expanded: BTreeSet<UiMessageId>,
    /// A received message could not be stored.
    pub refused: bool,
    /// The texts made ready to draw, each as it is shown now (expanded or
    /// not), kept while its message is in the window.
    prepared: BTreeMap<UiMessageId, Prepared>,
    /// Changes with the messages and with what is expanded.
    revision: u64,
}

impl Thread {
    /// The text of `message` ready to draw, once
    /// [`Model::prepare_messages`] made it.
    pub fn prepared(&self, message: UiMessageId) -> Option<&Prepared> {
        self.prepared.get(&message)
    }

    /// A number that changes whenever the messages change or one is
    /// expanded, and is never given to another state of any thread: the
    /// same number means the same messages, shown the same way.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}

impl fmt::Debug for Thread {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Counts, never the texts or the draft.
        f.debug_struct("Thread")
            .field("messages", &self.messages.len())
            .field("complete", &self.complete)
            .field("loading", &self.loading)
            .field("stale", &self.stale)
            .field("sending", &self.sending)
            .field("draft_starts_with_sent", &self.draft_starts_with_sent)
            .field("send_error", &self.send_error)
            .field("expanded", &self.expanded.len())
            .field("refused", &self.refused)
            .field("prepared", &self.prepared.len())
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

/// What is known of one identity.
#[derive(Default)]
pub struct Slice {
    /// Read at least once.
    pub loaded: bool,
    /// Its contacts.
    pub contacts: Vec<ContactView>,
    /// Its conversations.
    pub conversations: Vec<ConversationView>,
    /// Its requests.
    pub requests: Vec<RequestView>,
    /// Its invitations.
    pub invitations: Vec<InvitationView>,
    /// The people it blocked.
    pub blocked: Vec<BlockedView>,
    /// The state of its service.
    pub service: Option<Service>,
    /// Its request mode.
    pub request_mode: Option<RequestMode>,
    /// What the main pane shows.
    pub selection: Option<Selection>,
    /// Conversations that were read.
    pub threads: BTreeMap<UiContactId, Thread>,
    /// Unread counts, local to this run.
    pub unread: BTreeMap<UiContactId, u32>,
    /// Why delivery waits, per conversation, as the last attempt said.
    pub deferrals: BTreeMap<UiContactId, crate::data::Deferral>,
    /// When delivery is tried next, per conversation that has messages
    /// queued, in seconds since the epoch, as the last attempt said.
    pub retries: BTreeMap<UiContactId, u64>,
    /// Messages the runtime said it queued, each with the number the next
    /// command had then: a read asked before that may not count it. Kept
    /// until a read asked later answers.
    queued_since: BTreeMap<UiMessageId, u64>,
    stale: bool,
    loading: bool,
    /// The rows of the sidebar, made when the contacts, conversations or
    /// unread counts change.
    rows: Vec<SidebarRow>,
    /// Changes with the rows.
    rows_revision: u64,
}

impl Slice {
    /// Makes the rows of the sidebar again: every contact and every
    /// conversation, newest activity first, then by name. A row whose
    /// newest message reads as before keeps its preview.
    fn make_rows(&mut self, revision: u64) {
        let mut before: BTreeMap<UiContactId, (Option<MessageView>, String)> =
            core::mem::take(&mut self.rows)
                .into_iter()
                .map(|row| (row.contact, (row.latest, row.preview)))
                .collect();
        let written: BTreeMap<UiContactId, usize> = self
            .conversations
            .iter()
            .map(|conversation| (conversation.contact, conversation.messages))
            .collect();
        let row_of = |contact: UiContactId, name: String, fingerprint: &str| SidebarRow {
            contact,
            key: name.to_lowercase(),
            name,
            named: false,
            fingerprint: fingerprint.to_owned(),
            state: RowState::Ended,
            waiting: None,
            verified: false,
            latest: None,
            preview: String::new(),
            unread: 0,
            queued: 0,
        };
        let mut rows: BTreeMap<UiContactId, SidebarRow> = BTreeMap::new();
        for contact in &self.contacts {
            let mut row = row_of(
                contact.id,
                present::contact_name(contact),
                &contact.fingerprint,
            );
            row.state = if contact.accepted {
                RowState::Contact
            } else {
                RowState::Waiting
            };
            row.waiting = waiting(
                contact,
                written.get(&contact.id).copied().unwrap_or_default(),
            );
            row.verified = contact.verified;
            row.named = present::named(contact);
            rows.insert(contact.id, row);
        }
        for conversation in &self.conversations {
            let row = rows.entry(conversation.contact).or_insert_with(|| {
                row_of(
                    conversation.contact,
                    present::ended_contact_name(&conversation.fingerprint),
                    &conversation.fingerprint,
                )
            });
            if !conversation.active {
                row.state = RowState::Ended;
                row.waiting = None;
            }
            row.latest = conversation.latest.clone();
            row.queued = conversation.queued;
        }
        for (contact, count) in &self.unread {
            if let Some(row) = rows.get_mut(contact) {
                row.unread = *count;
            }
        }
        let mut rows: Vec<SidebarRow> = rows.into_values().collect();
        for row in &mut rows {
            let text = row.latest.as_ref().map(|message| message.text.as_str());
            row.preview = match before.remove(&row.contact) {
                Some((Some(old), preview)) if Some(old.text.as_str()) == text => preview,
                _ => text
                    .map(|text| present::preview(text, present::PREVIEW_CHARS))
                    .unwrap_or_default(),
            };
        }
        rows.sort_by(|a, b| {
            let time = |row: &SidebarRow| row.latest.as_ref().map_or(0, |message| message.time);
            time(b)
                .cmp(&time(a))
                .then_with(|| a.state.cmp(&b.state))
                .then_with(|| a.key.cmp(&b.key))
                .then_with(|| a.contact.cmp(&b.contact))
        });
        self.rows = rows;
        self.rows_revision = revision;
    }

    /// Puts the unread count of `contact` in its row; the order of the
    /// rows does not depend on it.
    fn unread_changed(&mut self, contact: UiContactId, revision: u64) {
        let count = self.unread.get(&contact).copied().unwrap_or(0);
        if let Some(row) = self.rows.iter_mut().find(|row| row.contact == contact) {
            row.unread = count;
            self.rows_revision = revision;
        }
    }

    /// How many messages wait to be delivered: those the conversations
    /// count, those queued since the read they come from was asked, and
    /// one for each message being queued. Those of an ended conversation
    /// are not counted: they are never sent (`DESIGN_QUESTIONS.md` P5-9).
    fn undelivered(&self) -> usize {
        let counted = self
            .conversations
            .iter()
            .filter(|conversation| conversation.active)
            .fold(0_usize, |sum, conversation| {
                sum.saturating_add(conversation.queued)
            });
        let current = |contact: UiContactId| self.contacts.iter().any(|view| view.id == contact);
        let since = self
            .queued_since
            .keys()
            .filter(|message| current(message.contact()))
            .count();
        let sending = self
            .threads
            .iter()
            .filter(|(contact, thread)| thread.sending.is_some() && current(**contact))
            .count();
        counted.saturating_add(since).saturating_add(sending)
    }
}

impl fmt::Debug for Slice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Counts and handles, never names, labels or introductions.
        f.debug_struct("Slice")
            .field("loaded", &self.loaded)
            .field("contacts", &self.contacts.len())
            .field("conversations", &self.conversations.len())
            .field("requests", &self.requests.len())
            .field("invitations", &self.invitations.len())
            .field("blocked", &self.blocked.len())
            .field("service", &self.service)
            .field("request_mode", &self.request_mode)
            .field("selection", &self.selection)
            .field("threads", &self.threads)
            .field("unread", &self.unread)
            .field("deferrals", &self.deferrals)
            .field("retries", &self.retries)
            .field("queued_since", &self.queued_since)
            .field("stale", &self.stale)
            .field("loading", &self.loading)
            .field("rows", &self.rows.len())
            .field("rows_revision", &self.rows_revision)
            .finish()
    }
}

/// Takes the next number of `counter`. Only equality of these numbers
/// matters, so wrapping round is no harm.
fn next(counter: &mut u64) -> u64 {
    *counter = counter.wrapping_add(1);
    *counter
}

/// A dialog. Each holds the handles it acts on from the moment it opens;
/// `busy` is the command its action started, while it runs.
pub enum Dialog {
    /// Make a local identity.
    NewIdentity {
        /// The name typed.
        name: String,
        /// The command under way.
        busy: Option<CommandId>,
        /// Why the last attempt failed.
        error: Option<Failure>,
    },
    /// Add a contact by its card.
    AddContact {
        /// The identity that adds it.
        identity: UiIdentityId,
        /// The card typed, pasted or read from a file.
        card: String,
        /// The name typed.
        name: String,
        /// The command under way.
        busy: Option<CommandId>,
        /// Why the last attempt failed.
        error: Option<Failure>,
        /// The contact added from a card that was not an invitation: the
        /// dialog stays to say that it has to add the card of this identity
        /// too. The card typed is wiped by then.
        added: Option<UiContactId>,
    },
    /// The card of an identity, to share.
    OwnCard {
        /// The identity.
        identity: UiIdentityId,
        /// The card, once read.
        card: Option<Zeroizing<String>>,
        /// The read under way.
        busy: Option<CommandId>,
        /// Why reading failed.
        error: Option<Failure>,
    },
    /// Make an invitation, then show it.
    NewInvitation {
        /// The identity.
        identity: UiIdentityId,
        /// The label typed.
        label: String,
        /// The invitation made, with its card.
        created: Option<(UiInvitationId, Zeroizing<String>)>,
        /// The command under way.
        busy: Option<CommandId>,
        /// Why the last attempt failed.
        error: Option<Failure>,
    },
    /// An invitation's card again.
    ShowInvitation {
        /// The invitation.
        invitation: UiInvitationId,
        /// The card, once read.
        card: Option<Zeroizing<String>>,
        /// The read under way.
        busy: Option<CommandId>,
        /// Why reading failed.
        error: Option<Failure>,
    },
    /// Confirm revoking an invitation.
    RevokeInvitation {
        /// The invitation.
        invitation: UiInvitationId,
        /// Also drop the requests it admitted.
        discard_requests: bool,
        /// The command under way.
        busy: Option<CommandId>,
        /// Why it failed.
        error: Option<Failure>,
    },
    /// A contact's details, with its name to edit.
    ContactDetails {
        /// The record.
        contact: UiContactId,
        /// The name being edited.
        name: String,
        /// The command under way.
        busy: Option<CommandId>,
        /// Why the last change failed.
        error: Option<Failure>,
    },
    /// Confirm blocking a contact.
    BlockContact {
        /// The record.
        contact: UiContactId,
        /// The command under way.
        busy: Option<CommandId>,
        /// Why it failed.
        error: Option<Failure>,
    },
    /// Confirm removing a contact.
    RemoveContact {
        /// The record.
        contact: UiContactId,
        /// The command under way.
        busy: Option<CommandId>,
        /// Why it failed.
        error: Option<Failure>,
    },
    /// Confirm deleting the messages of a conversation.
    DeleteConversation {
        /// The record.
        contact: UiContactId,
        /// The command under way.
        busy: Option<CommandId>,
        /// Why it failed.
        error: Option<Failure>,
    },
    /// Accept a request, naming the contact.
    AcceptRequest {
        /// The request.
        request: UiRequestId,
        /// The name typed, starting from the sender's chosen name.
        name: String,
        /// The command under way.
        busy: Option<CommandId>,
        /// Why it failed.
        error: Option<Failure>,
    },
    /// Confirm deleting a local identity.
    DeleteIdentity {
        /// The identity.
        identity: UiIdentityId,
        /// The command under way.
        busy: Option<CommandId>,
        /// Why it failed.
        error: Option<Failure>,
    },
    /// What is known about Tor.
    TorDetails,
}

impl Dialog {
    /// The command under way, if the dialog waits for one.
    pub fn busy(&self) -> Option<CommandId> {
        match self {
            Self::NewIdentity { busy, .. }
            | Self::AddContact { busy, .. }
            | Self::OwnCard { busy, .. }
            | Self::NewInvitation { busy, .. }
            | Self::ShowInvitation { busy, .. }
            | Self::RevokeInvitation { busy, .. }
            | Self::ContactDetails { busy, .. }
            | Self::BlockContact { busy, .. }
            | Self::RemoveContact { busy, .. }
            | Self::DeleteConversation { busy, .. }
            | Self::AcceptRequest { busy, .. }
            | Self::DeleteIdentity { busy, .. } => *busy,
            Self::TorDetails => None,
        }
    }

    /// The identity the dialog belongs to, if it belongs to one.
    pub fn identity(&self) -> Option<UiIdentityId> {
        match self {
            Self::AddContact { identity, .. }
            | Self::OwnCard { identity, .. }
            | Self::NewInvitation { identity, .. }
            | Self::DeleteIdentity { identity, .. } => Some(*identity),
            Self::ShowInvitation { invitation, .. } | Self::RevokeInvitation { invitation, .. } => {
                Some(invitation.identity())
            }
            Self::ContactDetails { contact, .. }
            | Self::BlockContact { contact, .. }
            | Self::RemoveContact { contact, .. }
            | Self::DeleteConversation { contact, .. } => Some(contact.identity()),
            Self::AcceptRequest { request, .. } => Some(request.identity()),
            Self::NewIdentity { .. } | Self::TorDetails => None,
        }
    }

    fn set_error(&mut self, failure: Failure) {
        match self {
            Self::NewIdentity { busy, error, .. }
            | Self::AddContact { busy, error, .. }
            | Self::OwnCard { busy, error, .. }
            | Self::NewInvitation { busy, error, .. }
            | Self::ShowInvitation { busy, error, .. }
            | Self::RevokeInvitation { busy, error, .. }
            | Self::ContactDetails { busy, error, .. }
            | Self::BlockContact { busy, error, .. }
            | Self::RemoveContact { busy, error, .. }
            | Self::DeleteConversation { busy, error, .. }
            | Self::AcceptRequest { busy, error, .. }
            | Self::DeleteIdentity { busy, error, .. } => {
                *busy = None;
                *error = Some(failure);
            }
            Self::TorDetails => {}
        }
    }

    /// Why the last attempt failed, if it did.
    fn error(&self) -> Option<Failure> {
        match self {
            Self::NewIdentity { error, .. }
            | Self::AddContact { error, .. }
            | Self::OwnCard { error, .. }
            | Self::NewInvitation { error, .. }
            | Self::ShowInvitation { error, .. }
            | Self::RevokeInvitation { error, .. }
            | Self::ContactDetails { error, .. }
            | Self::BlockContact { error, .. }
            | Self::RemoveContact { error, .. }
            | Self::DeleteConversation { error, .. }
            | Self::AcceptRequest { error, .. }
            | Self::DeleteIdentity { error, .. } => *error,
            Self::TorDetails => None,
        }
    }
}

impl Drop for Dialog {
    /// The card typed, pasted or read into "Add contact" can carry an
    /// invitation: it is wiped when the dialog goes, whether it was
    /// closed or its contact was added.
    fn drop(&mut self) {
        if let Self::AddContact { card, .. } = self {
            card.zeroize();
        }
    }
}

impl fmt::Debug for Dialog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Handles and states, never a name, a label or a card.
        let (kind, handle): (&str, Option<&dyn fmt::Debug>) = match self {
            Self::NewIdentity { .. } => ("NewIdentity", None),
            Self::AddContact { identity, .. } => ("AddContact", Some(identity)),
            Self::OwnCard { identity, .. } => ("OwnCard", Some(identity)),
            Self::NewInvitation { identity, .. } => ("NewInvitation", Some(identity)),
            Self::ShowInvitation { invitation, .. } => ("ShowInvitation", Some(invitation)),
            Self::RevokeInvitation { invitation, .. } => ("RevokeInvitation", Some(invitation)),
            Self::ContactDetails { contact, .. } => ("ContactDetails", Some(contact)),
            Self::BlockContact { contact, .. } => ("BlockContact", Some(contact)),
            Self::RemoveContact { contact, .. } => ("RemoveContact", Some(contact)),
            Self::DeleteConversation { contact, .. } => ("DeleteConversation", Some(contact)),
            Self::AcceptRequest { request, .. } => ("AcceptRequest", Some(request)),
            Self::DeleteIdentity { identity, .. } => ("DeleteIdentity", Some(identity)),
            Self::TorDetails => ("TorDetails", None),
        };
        let mut out = f.debug_struct(kind);
        if let Some(handle) = handle {
            out.field("on", handle);
        }
        match self {
            Self::OwnCard { card, .. } | Self::ShowInvitation { card, .. } => {
                out.field("card_read", &card.is_some());
            }
            Self::NewInvitation { created, .. } => {
                out.field(
                    "created",
                    &created.as_ref().map(|(invitation, _)| invitation),
                );
            }
            Self::RevokeInvitation {
                discard_requests, ..
            } => {
                out.field("discard_requests", discard_requests);
            }
            Self::AddContact { added, .. } => {
                out.field("added", added);
            }
            _ => {}
        }
        out.field("busy", &self.busy())
            .field("error", &self.error())
            .finish_non_exhaustive()
    }
}

/// How a request is answered without a dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Answer {
    Decline,
    Block,
}

impl Answer {
    /// What the user is told once it is done.
    const fn done(self) -> &'static str {
        match self {
            Self::Decline => "Request declined.",
            Self::Block => "Sender blocked.",
        }
    }
}

/// What a command in flight was for, with the handles it named.
enum Pending {
    /// A profile being opened, created or started without saving; if it
    /// fails, the client goes back to this phase: the unlock screen for a
    /// profile that exists, the first start for one being created.
    Profile(Phase),
    Identities,
    Identity(UiIdentityId),
    Messages(UiContactId),
    Earlier(UiContactId),
    Tor,
    /// A command started by the open dialog.
    Dialog,
    Send {
        contact: UiContactId,
        text: String,
    },
    Answer(UiRequestId, Answer),
    Unblock(UiBlockedId),
    RequestMode(UiIdentityId),
    Verified(UiContactId),
    Shutdown,
}

impl fmt::Debug for Pending {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Profile(back) => write!(f, "Profile({back:?})"),
            Self::Identities => f.write_str("Identities"),
            Self::Identity(identity) => write!(f, "Identity({identity:?})"),
            Self::Messages(contact) => write!(f, "Messages({contact:?})"),
            Self::Earlier(contact) => write!(f, "Earlier({contact:?})"),
            Self::Tor => f.write_str("Tor"),
            Self::Dialog => f.write_str("Dialog"),
            // The text sent is never printed.
            Self::Send { contact, .. } => write!(f, "Send({contact:?})"),
            Self::Answer(request, answer) => write!(f, "Answer({request:?}, {answer:?})"),
            Self::Unblock(blocked) => write!(f, "Unblock({blocked:?})"),
            Self::RequestMode(identity) => write!(f, "RequestMode({identity:?})"),
            Self::Verified(contact) => write!(f, "Verified({contact:?})"),
            Self::Shutdown => f.write_str("Shutdown"),
        }
    }
}

/// The state of the client. See the module documentation.
pub struct Model {
    phase: Phase,
    location: String,
    ephemeral: bool,
    message_store_missing: bool,
    profile_error: Option<Failure>,
    identities: Vec<IdentityView>,
    identities_loaded: bool,
    identities_stale: bool,
    identities_loading: bool,
    slices: BTreeMap<UiIdentityId, Slice>,
    current: Option<UiIdentityId>,
    tor: Option<TorCheck>,
    tor_checking: bool,
    tor_wanted: bool,
    storage_failed: bool,
    pending: BTreeMap<CommandId, Pending>,
    next: u64,
    outbox: Vec<(CommandId, Command)>,
    notices: Vec<Notice>,
    dialog: Option<Dialog>,
    /// How many dialogs were opened: the number of the open one.
    dialogs_opened: u64,
    /// An identity just made, shown once it is listed.
    pending_select: Option<UiIdentityId>,
    /// A contact just added or accepted, shown once its identity is read.
    pending_open: Option<UiContactId>,
    /// The time now as the window last said, in seconds since the epoch.
    now: u64,
    /// The last number given to a state of a thread or of the rows.
    revisions: u64,
    /// Conversations whose messages were deleted, or whose contact or
    /// identity went, since the window last asked.
    forgotten: Vec<UiContactId>,
}

impl fmt::Debug for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Handles, counts and states: no name, card, draft, text or
        // notice, and not the data directory.
        f.debug_struct("Model")
            .field("phase", &self.phase)
            .field("ephemeral", &self.ephemeral)
            .field("message_store_missing", &self.message_store_missing)
            .field("profile_error", &self.profile_error)
            .field("identities", &self.identities)
            .field("identities_loaded", &self.identities_loaded)
            .field("identities_stale", &self.identities_stale)
            .field("identities_loading", &self.identities_loading)
            .field("slices", &self.slices)
            .field("current", &self.current)
            .field("tor", &self.tor)
            .field("tor_checking", &self.tor_checking)
            .field("tor_wanted", &self.tor_wanted)
            .field("storage_failed", &self.storage_failed)
            .field("pending", &self.pending)
            .field("next", &self.next)
            .field("outbox", &self.outbox)
            .field("notices", &self.notices.len())
            .field("dialog", &self.dialog)
            .field("dialogs_opened", &self.dialogs_opened)
            .field("pending_select", &self.pending_select)
            .field("pending_open", &self.pending_open)
            .field("now", &self.now)
            .field("revisions", &self.revisions)
            .field("forgotten", &self.forgotten)
            .finish_non_exhaustive()
    }
}

impl Default for Model {
    fn default() -> Self {
        Self::new()
    }
}

impl Model {
    /// The state at start.
    pub fn new() -> Self {
        Self {
            phase: Phase::Starting,
            location: String::new(),
            ephemeral: false,
            message_store_missing: false,
            profile_error: None,
            identities: Vec::new(),
            identities_loaded: false,
            identities_stale: false,
            identities_loading: false,
            slices: BTreeMap::new(),
            current: None,
            tor: None,
            tor_checking: false,
            tor_wanted: false,
            storage_failed: false,
            pending: BTreeMap::new(),
            next: 0,
            outbox: Vec::new(),
            notices: Vec::new(),
            dialog: None,
            dialogs_opened: 0,
            pending_select: None,
            pending_open: None,
            now: 0,
            revisions: 0,
            forgotten: Vec::new(),
        }
    }

    fn issue(&mut self, command: Command, pending: Pending) -> CommandId {
        let id = CommandId::new(self.next);
        self.next = self.next.saturating_add(1);
        self.pending.insert(id, pending);
        self.outbox.push((id, command));
        id
    }

    fn notice(&mut self, kind: NoticeKind, text: impl Into<Prose>) {
        self.notices.push(Notice {
            kind,
            text: text.into(),
            revoke: None,
        });
    }

    /// Tells the user that a command failed, unless the window says it
    /// ([`Failure::said_in_place`]).
    fn failed(&mut self, failure: Failure) {
        if failure.said_in_place() {
            self.notice(NoticeKind::Error, failure.message());
        }
    }

    /// Notes that the runtime queued `message`: it waits to be delivered
    /// even before a read of its identity counts it.
    fn known_queued(&mut self, message: UiMessageId) {
        let next = self.next;
        if let Some(slice) = self.slices.get_mut(&message.contact().identity()) {
            slice.queued_since.insert(message, next);
        }
    }

    // ---- What is shown ----

    /// Where the client is.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The data directory, for display.
    pub fn location(&self) -> &str {
        &self.location
    }

    /// Nothing is saved in this session.
    pub fn ephemeral(&self) -> bool {
        self.ephemeral
    }

    /// The messages from before were not found when the profile opened.
    pub fn message_store_missing(&self) -> bool {
        self.message_store_missing
    }

    /// Why opening the profile failed.
    pub fn profile_error(&self) -> Option<Failure> {
        self.profile_error
    }

    /// A store failed: nothing more is saved until Monolith starts again.
    pub fn storage_failed(&self) -> bool {
        self.storage_failed
    }

    /// The local identities, sorted by name.
    pub fn identities(&self) -> &[IdentityView] {
        &self.identities
    }

    /// The identities were read at least once.
    pub fn identities_loaded(&self) -> bool {
        self.identities_loaded
    }

    /// The identity shown.
    pub fn current(&self) -> Option<UiIdentityId> {
        self.current
    }

    /// The identity shown, as listed.
    pub fn current_identity(&self) -> Option<&IdentityView> {
        let current = self.current?;
        self.identities
            .iter()
            .find(|identity| identity.id == current)
    }

    /// What is known of `identity`.
    pub fn slice(&self, identity: UiIdentityId) -> Option<&Slice> {
        self.slices.get(&identity)
    }

    /// What is known of the identity shown.
    pub fn current_slice(&self) -> Option<&Slice> {
        self.slices.get(&self.current?)
    }

    /// The contact of `contact`, if it is a current contact.
    pub fn contact(&self, contact: UiContactId) -> Option<&ContactView> {
        self.slices
            .get(&contact.identity())?
            .contacts
            .iter()
            .find(|view| view.id == contact)
    }

    /// The conversation of `contact`, if it has one.
    pub fn conversation(&self, contact: UiContactId) -> Option<&ConversationView> {
        self.slices
            .get(&contact.identity())?
            .conversations
            .iter()
            .find(|view| view.contact == contact)
    }

    /// The name of `contact`, whether it is still a contact or not.
    pub fn contact_name(&self, contact: UiContactId) -> String {
        if let Some(view) = self.contact(contact) {
            return present::contact_name(view);
        }
        match self.conversation(contact) {
            Some(view) => present::ended_contact_name(&view.fingerprint),
            None => "Contact".to_owned(),
        }
    }

    /// The thread of `contact`, if it was read.
    pub fn thread(&self, contact: UiContactId) -> Option<&Thread> {
        self.slices.get(&contact.identity())?.threads.get(&contact)
    }

    /// What `contact` waits for, if it is a contact that has not accepted.
    pub fn waiting(&self, contact: UiContactId) -> Option<Waiting> {
        let view = self.contact(contact)?;
        let messages = self
            .conversation(contact)
            .map_or(0, |conversation| conversation.messages);
        waiting(view, messages)
    }

    /// What the main pane of the identity shown shows.
    pub fn selection(&self) -> Option<Selection> {
        self.current_slice()?.selection
    }

    /// The open dialog.
    pub fn dialog(&self) -> Option<&Dialog> {
        self.dialog.as_ref()
    }

    /// The open dialog, to edit its fields.
    pub fn dialog_mut(&mut self) -> Option<&mut Dialog> {
        self.dialog.as_mut()
    }

    /// The number of the open dialog. Every dialog opened gets a new one,
    /// so that what one dialog asked for (a card file read, say) is never
    /// taken by another that opened later, even of the same kind and
    /// identity.
    pub fn dialog_instance(&self) -> Option<u64> {
        self.dialog.as_ref().map(|_| self.dialogs_opened)
    }

    /// The last check of Tor.
    pub fn tor_check(&self) -> Option<&TorCheck> {
        self.tor.as_ref()
    }

    /// A check of Tor is under way.
    pub fn tor_checking(&self) -> bool {
        self.tor_checking
    }

    /// What the top bar says of Tor.
    pub fn tor_indicator(&self) -> TorIndicator {
        let check = match &self.tor {
            Some(TorCheck::Ready { .. }) => Some(TorIndicator::Connected),
            Some(TorCheck::Connecting { .. }) => Some(TorIndicator::Connecting),
            Some(_) => Some(TorIndicator::Unavailable),
            None => None,
        };
        let service = self.current_slice().and_then(|slice| slice.service);
        match service {
            Some(Service::Available) => TorIndicator::Connected,
            Some(Service::Publishing) => match check {
                Some(TorIndicator::Unavailable) => TorIndicator::Unavailable,
                _ => TorIndicator::Connecting,
            },
            Some(Service::Unavailable | Service::NoKey | Service::Stopped) => {
                TorIndicator::Unavailable
            }
            None => check.unwrap_or(TorIndicator::Starting),
        }
    }

    /// The rows of the sidebar of the identity shown: every contact and
    /// every conversation, newest activity first, then by name. They are
    /// made when what they show changes, not when they are read.
    pub fn sidebar_rows(&self) -> &[SidebarRow] {
        self.current_slice()
            .map_or(&[], |slice| slice.rows.as_slice())
    }

    /// A copy of [`Self::sidebar_rows`].
    pub fn sidebar(&self) -> Vec<SidebarRow> {
        self.sidebar_rows().to_vec()
    }

    /// A number that changes whenever the rows of the sidebar of the
    /// identity shown change: the same number means the same rows.
    pub fn sidebar_revision(&self) -> u64 {
        self.current_slice().map_or(0, |slice| slice.rows_revision)
    }

    /// Takes the conversations whose messages were deleted, or whose
    /// contact or identity went, since the last call, so that the window
    /// forgets what it kept of them.
    pub fn take_forgotten(&mut self) -> Vec<UiContactId> {
        core::mem::take(&mut self.forgotten)
    }

    /// How many messages wait to be delivered, over every identity: those
    /// its conversations count, those the runtime queued since the read
    /// they come from was asked, and one for each message being queued.
    /// Those of an ended conversation are not counted: they are never sent
    /// (`DESIGN_QUESTIONS.md` P5-9).
    pub fn undelivered(&self) -> usize {
        self.slices.values().fold(0_usize, |sum, slice| {
            sum.saturating_add(slice.undelivered())
        })
    }

    /// Takes the notices raised since the last call.
    pub fn take_notices(&mut self) -> Vec<Notice> {
        core::mem::take(&mut self.notices)
    }

    /// Takes the commands to send to the bridge: those the user's actions
    /// made, and one read for every part of the state that is stale and
    /// is not being read already. Reads are coalesced: many events about
    /// one part make one read.
    pub fn take_commands(&mut self) -> Vec<(CommandId, Command)> {
        if self.phase == Phase::Ready {
            if self.identities_stale && !self.identities_loading {
                self.identities_stale = false;
                self.identities_loading = true;
                self.issue(Command::LoadIdentities, Pending::Identities);
            }
            let stale: Vec<UiIdentityId> = self
                .slices
                .iter()
                .filter(|(_, slice)| slice.stale && !slice.loading)
                .map(|(id, _)| *id)
                .collect();
            for identity in stale {
                if let Some(slice) = self.slices.get_mut(&identity) {
                    slice.stale = false;
                    slice.loading = true;
                }
                self.issue(Command::LoadIdentity(identity), Pending::Identity(identity));
            }
            let threads: Vec<(UiContactId, usize)> = self
                .slices
                .values()
                .flat_map(|slice| slice.threads.iter())
                .filter(|(_, thread)| thread.stale && !thread.loading)
                .map(|(contact, thread)| (*contact, thread.messages.len().clamp(PAGE, MAX_WINDOW)))
                .collect();
            for (contact, limit) in threads {
                if let Some(thread) = self.thread_mut(contact) {
                    thread.stale = false;
                    thread.loading = true;
                }
                self.issue(
                    Command::LoadMessages { contact, limit },
                    Pending::Messages(contact),
                );
            }
            if self.tor_wanted && !self.tor_checking {
                self.tor_wanted = false;
                self.tor_checking = true;
                self.issue(Command::CheckTor, Pending::Tor);
            }
        }
        core::mem::take(&mut self.outbox)
    }

    fn thread_mut(&mut self, contact: UiContactId) -> Option<&mut Thread> {
        self.slices
            .get_mut(&contact.identity())?
            .threads
            .get_mut(&contact)
    }

    fn mark_identity(&mut self, identity: UiIdentityId) {
        if let Some(slice) = self.slices.get_mut(&identity) {
            slice.stale = true;
        }
    }

    fn mark_thread(&mut self, contact: UiContactId) {
        if let Some(thread) = self.thread_mut(contact) {
            thread.stale = true;
        }
    }

    fn mark_everything(&mut self) {
        self.identities_stale = true;
        // What Tor did in the events missed is not known either.
        self.tor_wanted = true;
        for slice in self.slices.values_mut() {
            slice.stale = true;
            for thread in slice.threads.values_mut() {
                thread.stale = true;
            }
        }
    }

    // ---- What the user does ----

    /// Opens the profile with `passphrase`.
    pub fn unlock(&mut self, passphrase: Zeroizing<String>) {
        if !matches!(self.phase, Phase::Locked) {
            return;
        }
        self.phase = Phase::Opening;
        self.profile_error = None;
        self.issue(
            Command::OpenProfile { passphrase },
            Pending::Profile(Phase::Locked),
        );
    }

    /// Creates a profile under `passphrase`.
    pub fn create_profile(&mut self, passphrase: Zeroizing<String>) {
        if !matches!(self.phase, Phase::Welcome) {
            return;
        }
        if passphrase.trim().is_empty() {
            self.profile_error = Some(Failure::UnusablePassphrase);
            return;
        }
        self.phase = Phase::Opening;
        self.profile_error = None;
        self.issue(
            Command::CreateProfile { passphrase },
            Pending::Profile(Phase::Welcome),
        );
    }

    /// Starts a session that keeps nothing.
    pub fn start_without_saving(&mut self) {
        if !matches!(self.phase, Phase::Welcome | Phase::Locked) {
            return;
        }
        let back = self.phase;
        self.phase = Phase::Opening;
        self.profile_error = None;
        self.issue(Command::StartWithoutSaving, Pending::Profile(back));
    }

    /// Shows the identity `identity`, if it is one of the identities. Its
    /// conversations, requests and invitations are shown from then on; no
    /// state of the identity shown before is carried over.
    pub fn select_identity(&mut self, identity: UiIdentityId) {
        if !self.identities.iter().any(|view| view.id == identity) {
            return;
        }
        if self.current == Some(identity) {
            return;
        }
        self.current = Some(identity);
        // A dialog of another identity has nothing to do here.
        if self
            .dialog
            .as_ref()
            .and_then(Dialog::identity)
            .is_some_and(|owner| owner != identity)
        {
            self.dialog = None;
        }
        let slice = self.slices.entry(identity).or_default();
        if !slice.loaded && !slice.loading {
            slice.stale = true;
        }
    }

    /// Shows the conversation of `contact`, if it is one of the identity
    /// shown. Its messages are read if they were not.
    pub fn select_conversation(&mut self, contact: UiContactId) {
        if Some(contact.identity()) != self.current {
            return;
        }
        let known = self.contact(contact).is_some() || self.conversation(contact).is_some();
        if !known {
            return;
        }
        let Some(slice) = self.slices.get_mut(&contact.identity()) else {
            return;
        };
        slice.selection = Some(Selection::Conversation(contact));
        if slice.unread.remove(&contact).is_some() {
            slice.unread_changed(contact, next(&mut self.revisions));
        }
        let thread = slice.threads.entry(contact).or_default();
        if thread.messages.is_empty() && !thread.loading {
            thread.stale = true;
        }
    }

    /// Shows the requests of the identity shown.
    pub fn show_requests(&mut self) {
        if let Some(current) = self.current {
            if let Some(slice) = self.slices.get_mut(&current) {
                slice.selection = Some(Selection::Requests);
            }
        }
    }

    /// The draft of the conversation of `contact`, to edit, also while a
    /// message is being queued: what is queued is the text taken when it
    /// was sent, and once it is queued only that text leaves the draft, if
    /// the draft has started with it ever since ([`Self::draft_edited`]).
    pub fn draft_mut(&mut self, contact: UiContactId) -> Option<&mut String> {
        let thread = self.thread_mut(contact)?;
        thread.send_error = None;
        Some(&mut thread.draft)
    }

    /// Tells the model that the draft of `contact` was edited, as the
    /// composer reports each frame. Once the draft no longer starts with
    /// the text being queued, that text does not leave it when it is
    /// queued, also if the draft starts with it again later: it was typed
    /// again.
    pub fn draft_edited(&mut self, contact: UiContactId) {
        let Some(thread) = self
            .slices
            .get_mut(&contact.identity())
            .and_then(|slice| slice.threads.get_mut(&contact))
        else {
            return;
        };
        let sent = thread
            .sending
            .and_then(|command| match self.pending.get(&command) {
                Some(Pending::Send { text, .. }) => Some(text.as_str()),
                _ => None,
            });
        if sent.is_some_and(|text| !thread.draft.starts_with(text)) {
            thread.draft_starts_with_sent = false;
        }
    }

    /// Whether the conversation of `contact` can take a message now: its
    /// contact is current, and nothing is being queued.
    pub fn can_send(&self, contact: UiContactId) -> bool {
        let current = self.contact(contact).is_some();
        let sending = self
            .thread(contact)
            .is_some_and(|thread| thread.sending.is_some());
        current && !sending && !self.storage_failed
    }

    /// Queues the draft of `contact`. The draft stays until the runtime
    /// says the message is durably queued, and then loses the text sent,
    /// keeping what was typed after it, unless it stopped starting with
    /// that text meanwhile; if the runtime refuses, the draft stays as it
    /// is, typing included, and the reason is shown.
    pub fn send(&mut self, contact: UiContactId) {
        if !self.can_send(contact) {
            return;
        }
        let Some(text) = self.thread(contact).map(|thread| thread.draft.clone()) else {
            return;
        };
        if !present::sendable(&text) {
            if let Some(thread) = self.thread_mut(contact) {
                if text.len() > present::MAX_MESSAGE_BYTES {
                    thread.send_error = Some(Failure::TooLong);
                }
            }
            return;
        }
        let id = self.issue(
            Command::Send {
                contact,
                text: text.clone(),
            },
            Pending::Send { contact, text },
        );
        if let Some(thread) = self.thread_mut(contact) {
            thread.sending = Some(id);
            thread.send_error = None;
            thread.draft_starts_with_sent = true;
        }
    }

    /// Reads the messages before the oldest one shown of `contact`.
    pub fn load_earlier(&mut self, contact: UiContactId) {
        let Some(thread) = self.thread(contact) else {
            return;
        };
        if thread.complete || thread.loading || thread.messages.len() >= MAX_WINDOW {
            return;
        }
        let Some(before) = thread.messages.first().map(|message| message.id.sequence()) else {
            return;
        };
        if let Some(thread) = self.thread_mut(contact) {
            thread.loading = true;
        }
        self.issue(
            Command::LoadEarlier {
                contact,
                before,
                limit: PAGE,
            },
            Pending::Earlier(contact),
        );
    }

    /// Shows all of a collapsed message.
    pub fn expand(&mut self, message: UiMessageId) {
        let revision = next(&mut self.revisions);
        if let Some(thread) = self.thread_mut(message.contact()) {
            if thread.expanded.insert(message) {
                // Made again, whole, when it is drawn next.
                thread.prepared.remove(&message);
                thread.revision = revision;
            }
        }
    }

    /// Makes the texts of the messages of `contact` at `range` (positions
    /// in [`Thread::messages`]) ready to draw, those not made yet. Each is
    /// made once and kept until its message leaves the window or is
    /// expanded ([`Thread::prepared`]).
    pub fn prepare_messages(&mut self, contact: UiContactId, range: Range<usize>) {
        let Some(thread) = self.thread_mut(contact) else {
            return;
        };
        let Thread {
            messages,
            expanded,
            prepared,
            ..
        } = thread;
        let end = range.end.min(messages.len());
        let start = range.start.min(end);
        for message in messages.get(start..end).unwrap_or_default() {
            prepared
                .entry(message.id)
                .or_insert_with(|| present::prepare(&message.text, expanded.contains(&message.id)));
        }
    }

    /// Asks Tor how it is.
    pub fn check_tor(&mut self) {
        if self.phase == Phase::Ready {
            self.tor_wanted = true;
        }
    }

    /// Opens `dialog`, unless one is open and waits for a command.
    pub fn open(&mut self, dialog: Dialog) {
        if self.dialog.as_ref().and_then(Dialog::busy).is_some() {
            return;
        }
        let needs = match &dialog {
            Dialog::OwnCard { identity, .. } => Some(Command::OwnCard(*identity)),
            Dialog::ShowInvitation { invitation, .. } => Some(Command::InvitationCard(*invitation)),
            _ => None,
        };
        self.dialog = Some(dialog);
        // A u64 counted up by one per dialog opened is never exhausted.
        self.dialogs_opened = self.dialogs_opened.saturating_add(1);
        if let Some(command) = needs {
            let id = self.issue(command, Pending::Dialog);
            if let Some(Dialog::OwnCard { busy, .. } | Dialog::ShowInvitation { busy, .. }) =
                self.dialog.as_mut()
            {
                *busy = Some(id);
            }
        }
    }

    /// Closes the open dialog, unless its action is under way. What it
    /// held (a card, a name) goes with it.
    pub fn close(&mut self) {
        if self.dialog.as_ref().and_then(Dialog::busy).is_none() {
            self.dialog = None;
        }
    }

    /// Carries out the action of the open dialog on the handles it holds.
    pub fn confirm(&mut self) {
        let Some(dialog) = self.dialog.take() else {
            return;
        };
        if dialog.busy().is_some() {
            self.dialog = Some(dialog);
            return;
        }
        let mut dialog = dialog;
        let command = match &dialog {
            Dialog::NewIdentity { name, .. } => Some(Command::CreateIdentity {
                name: optional(name),
            }),
            Dialog::AddContact {
                identity,
                card,
                name,
                added: None,
                ..
            } => Some(Command::AddContact {
                identity: *identity,
                card: Zeroizing::new(card.trim().to_owned()),
                name: optional(name),
            }),
            Dialog::NewInvitation {
                identity,
                label,
                created: None,
                ..
            } => Some(Command::CreateInvitation {
                identity: *identity,
                label: optional(label),
            }),
            Dialog::RevokeInvitation {
                invitation,
                discard_requests,
                ..
            } => Some(Command::RevokeInvitation {
                invitation: *invitation,
                discard_requests: *discard_requests,
            }),
            Dialog::ContactDetails { contact, name, .. } => Some(Command::SetContactName {
                contact: *contact,
                name: optional(name),
            }),
            Dialog::BlockContact { contact, .. } => Some(Command::BlockContact(*contact)),
            Dialog::RemoveContact { contact, .. } => Some(Command::RemoveContact(*contact)),
            Dialog::DeleteConversation { contact, .. } => {
                Some(Command::DeleteConversation(*contact))
            }
            Dialog::AcceptRequest { request, name, .. } => Some(Command::AcceptRequest {
                request: *request,
                name: optional(name),
            }),
            Dialog::DeleteIdentity { identity, .. } => Some(Command::DeleteIdentity(*identity)),
            Dialog::OwnCard { .. }
            | Dialog::ShowInvitation { .. }
            | Dialog::NewInvitation { .. }
            | Dialog::AddContact { .. }
            | Dialog::TorDetails => None,
        };
        let Some(command) = command else {
            // Nothing to do: the dialog was only showing something.
            self.dialog = None;
            return;
        };
        let id = self.issue(command, Pending::Dialog);
        match &mut dialog {
            Dialog::NewIdentity { busy, error, .. }
            | Dialog::AddContact { busy, error, .. }
            | Dialog::NewInvitation { busy, error, .. }
            | Dialog::RevokeInvitation { busy, error, .. }
            | Dialog::ContactDetails { busy, error, .. }
            | Dialog::BlockContact { busy, error, .. }
            | Dialog::RemoveContact { busy, error, .. }
            | Dialog::DeleteConversation { busy, error, .. }
            | Dialog::AcceptRequest { busy, error, .. }
            | Dialog::DeleteIdentity { busy, error, .. } => {
                *busy = Some(id);
                *error = None;
            }
            Dialog::OwnCard { .. } | Dialog::ShowInvitation { .. } | Dialog::TorDetails => {}
        }
        self.dialog = Some(dialog);
    }

    /// Declines the request `request`, without a dialog of the model: the
    /// window asks first. Nothing is sent; the sender goes on the declined
    /// list of the identity, and later requests from them are dropped
    /// (`docs/PROTOCOL.md` section 12).
    pub fn decline(&mut self, request: UiRequestId) {
        self.answer(request, Answer::Decline);
    }

    /// Blocks the sender of the request `request`.
    pub fn block_sender(&mut self, request: UiRequestId) {
        self.answer(request, Answer::Block);
    }

    fn answer(&mut self, request: UiRequestId, answer: Answer) {
        let waiting = self
            .slices
            .get(&request.identity())
            .is_some_and(|slice| slice.requests.iter().any(|view| view.id == request));
        let answering = self.answering(request);
        if waiting && !answering {
            let command = match answer {
                Answer::Decline => Command::DeclineRequest(request),
                Answer::Block => Command::BlockRequest(request),
            };
            self.issue(command, Pending::Answer(request, answer));
        }
    }

    /// Whether an answer to `request` is under way.
    pub fn answering(&self, request: UiRequestId) -> bool {
        self.pending
            .values()
            .any(|pending| matches!(pending, Pending::Answer(asked, _) if *asked == request))
    }

    /// Unblocks `blocked`, without a dialog of the model: the window asks
    /// first, since nothing is kept of them afterwards to block them by
    /// (`DESIGN_QUESTIONS.md` P6-21). Asked only for someone listed, once.
    pub fn unblock(&mut self, blocked: UiBlockedId) {
        let listed = self
            .slices
            .get(&blocked.identity())
            .is_some_and(|slice| slice.blocked.iter().any(|view| view.id == blocked));
        if listed && !self.unblocking(blocked) {
            self.issue(Command::Unblock(blocked), Pending::Unblock(blocked));
        }
    }

    /// Whether unblocking `blocked` is under way.
    pub fn unblocking(&self, blocked: UiBlockedId) -> bool {
        self.pending
            .values()
            .any(|pending| matches!(pending, Pending::Unblock(asked) if *asked == blocked))
    }

    /// Sets the request mode of `identity`.
    pub fn set_request_mode(&mut self, identity: UiIdentityId, mode: RequestMode) {
        if self.slices.contains_key(&identity) {
            self.issue(
                Command::SetRequestMode { identity, mode },
                Pending::RequestMode(identity),
            );
        }
    }

    /// Records whether the user verified the fingerprint of `contact`.
    pub fn set_verified(&mut self, contact: UiContactId, verified: bool) {
        if self.contact(contact).is_some() {
            self.issue(
                Command::SetVerified { contact, verified },
                Pending::Verified(contact),
            );
        }
    }

    /// Stops the runtime before the window closes.
    pub fn shut_down(&mut self) {
        if self.phase == Phase::Ready {
            self.issue(Command::Shutdown, Pending::Shutdown);
        }
    }

    // ---- What the bridge says ----

    /// Sets the time now, in seconds since the epoch, before what the
    /// bridge said is applied: a delay the runtime gives is kept as the
    /// time it ends.
    pub fn set_time(&mut self, now: u64) {
        self.now = now;
    }

    /// Applies what the bridge said.
    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Started { profile, location } => {
                if self.phase == Phase::Starting {
                    self.location = location;
                    self.phase = match profile {
                        ProfileOnDisk::Exists => Phase::Locked,
                        ProfileOnDisk::Absent => Phase::Welcome,
                    };
                }
            }
            Update::Done { command, outcome } => self.done(command, outcome),
            Update::Event(event) => self.event(event),
            Update::Stopped => {
                self.phase = Phase::Stopped;
                self.dialog = None;
            }
        }
    }

    fn done(&mut self, command: CommandId, outcome: Outcome) {
        // An answer to no command in flight, or to one already answered,
        // changes nothing.
        let Some(pending) = self.pending.remove(&command) else {
            return;
        };
        match pending {
            Pending::Profile(back) => self.profile_done(back, outcome),
            Pending::Identities => {
                self.identities_loading = false;
                if let Outcome::Identities(list) = outcome {
                    self.set_identities(list);
                }
            }
            Pending::Identity(identity) => {
                if let Some(slice) = self.slices.get_mut(&identity) {
                    slice.loading = false;
                }
                match outcome {
                    Outcome::Identity(snapshot) if snapshot.identity == identity => {
                        self.set_snapshot(*snapshot);
                        // A read asked once a message was known to be
                        // queued counts it.
                        if let Some(slice) = self.slices.get_mut(&identity) {
                            slice.queued_since.retain(|_, next| *next > command.get());
                        }
                    }
                    Outcome::Failed(Failure::NoSuchIdentity) => {
                        self.identities_stale = true;
                    }
                    _ => {}
                }
            }
            Pending::Messages(contact) => {
                if let Some(thread) = self.thread_mut(contact) {
                    thread.loading = false;
                }
                if let Outcome::Messages {
                    contact: answered,
                    messages,
                    complete,
                } = outcome
                {
                    if answered == contact {
                        self.set_window(contact, messages, complete);
                    }
                }
            }
            Pending::Earlier(contact) => {
                if let Some(thread) = self.thread_mut(contact) {
                    thread.loading = false;
                }
                if let Outcome::Messages {
                    contact: answered,
                    messages,
                    complete,
                } = outcome
                {
                    if answered == contact {
                        self.prepend(contact, messages, complete);
                    }
                }
            }
            Pending::Tor => {
                self.tor_checking = false;
                if let Outcome::Tor(check) = outcome {
                    self.tor = Some(check);
                }
            }
            Pending::Dialog => self.dialog_done(command, outcome),
            Pending::Send { contact, text } => self.send_done(command, contact, &text, outcome),
            Pending::Answer(request, answer) => {
                self.mark_identity(request.identity());
                match outcome {
                    Outcome::Failed(failure) => self.failed(failure),
                    _ => self.notice(NoticeKind::Info, answer.done()),
                }
            }
            Pending::Unblock(blocked) => {
                self.mark_identity(blocked.identity());
                if let Outcome::Failed(failure) = outcome {
                    self.failed(failure);
                } else {
                    self.notice(NoticeKind::Success, UNBLOCKED);
                }
            }
            Pending::RequestMode(identity) => {
                if let Outcome::Failed(failure) = outcome {
                    self.failed(failure);
                }
                self.mark_identity(identity);
            }
            Pending::Verified(contact) => {
                if let Outcome::Failed(failure) = outcome {
                    self.failed(failure);
                }
                self.mark_identity(contact.identity());
            }
            Pending::Shutdown => {}
        }
    }

    fn profile_done(&mut self, back: Phase, outcome: Outcome) {
        match outcome {
            Outcome::ProfileOpen {
                ephemeral,
                message_store_missing,
            } => {
                self.phase = Phase::Ready;
                self.ephemeral = ephemeral;
                self.message_store_missing = message_store_missing;
                self.identities_stale = true;
                self.tor_wanted = true;
            }
            Outcome::Failed(failure) => {
                // Back to where it was asked, with the reason: an open
                // that fails, even for a passphrase too long to be one,
                // stays on the unlock screen of the profile that exists.
                self.profile_error = Some(failure);
                self.phase = back;
            }
            _ => {}
        }
    }

    fn set_identities(&mut self, mut list: Vec<IdentityView>) {
        list.sort_by(|a, b| {
            present::identity_name(a)
                .to_lowercase()
                .cmp(&present::identity_name(b).to_lowercase())
                .then_with(|| a.id.cmp(&b.id))
        });
        let present: BTreeSet<UiIdentityId> = list.iter().map(|view| view.id).collect();
        for (_, slice) in self.slices.iter().filter(|(id, _)| !present.contains(id)) {
            self.forgotten.extend(slice.threads.keys().copied());
        }
        // An identity that is gone takes everything of it with it.
        self.slices.retain(|id, _| present.contains(id));
        if self
            .dialog
            .as_ref()
            .and_then(Dialog::identity)
            .is_some_and(|owner| !present.contains(&owner))
            && self.dialog.as_ref().and_then(Dialog::busy).is_none()
        {
            self.dialog = None;
        }
        for id in &present {
            let slice = self.slices.entry(*id).or_default();
            if !slice.loaded && !slice.loading {
                slice.stale = true;
            }
        }
        self.identities = list;
        self.identities_loaded = true;
        let keep = self.current.filter(|current| present.contains(current));
        self.current = keep.or_else(|| self.identities.first().map(|view| view.id));
        if let Some(made) = self.pending_select {
            if present.contains(&made) {
                self.pending_select = None;
                self.current = Some(made);
            }
        }
    }

    fn set_snapshot(&mut self, snapshot: IdentitySnapshot) {
        let identity = snapshot.identity;
        // Only what belongs to the identity read is taken.
        let contacts: Vec<ContactView> = snapshot
            .contacts
            .into_iter()
            .filter(|view| view.id.identity() == identity)
            .collect();
        let conversations: Vec<ConversationView> = snapshot
            .conversations
            .into_iter()
            .filter(|view| view.contact.identity() == identity)
            .collect();
        let requests: Vec<RequestView> = snapshot
            .requests
            .into_iter()
            .filter(|view| view.id.identity() == identity)
            .collect();
        let invitations: Vec<InvitationView> = snapshot
            .invitations
            .into_iter()
            .filter(|view| view.id.identity() == identity)
            .collect();
        let blocked: Vec<BlockedView> = snapshot
            .blocked
            .into_iter()
            .filter(|view| view.id.identity() == identity)
            .collect();
        let Some(slice) = self.slices.get_mut(&identity) else {
            return;
        };
        let known: BTreeSet<UiContactId> = contacts
            .iter()
            .map(|view| view.id)
            .chain(conversations.iter().map(|view| view.contact))
            .collect();
        // Only a conversation with something to send is tried again.
        let queued: BTreeSet<UiContactId> = conversations
            .iter()
            .filter(|view| view.active && view.queued > 0)
            .map(|view| view.contact)
            .collect();
        // A contact removed or blocked, and a conversation that is gone,
        // leave nothing behind in the window either.
        let current: BTreeSet<UiContactId> = contacts.iter().map(|view| view.id).collect();
        let gone: BTreeSet<UiContactId> = slice
            .contacts
            .iter()
            .map(|view| view.id)
            .filter(|contact| !current.contains(contact))
            .chain(
                slice
                    .threads
                    .keys()
                    .copied()
                    .filter(|contact| !known.contains(contact)),
            )
            .collect();
        self.forgotten.extend(gone);
        slice.contacts = contacts;
        slice.conversations = conversations;
        slice.requests = requests;
        slice.invitations = invitations;
        slice.blocked = blocked;
        slice.service = Some(snapshot.service);
        slice.request_mode = Some(snapshot.request_mode);
        slice.loaded = true;
        // A conversation that is gone is not shown any more, and nothing
        // of it is kept.
        slice.threads.retain(|contact, _| known.contains(contact));
        slice.unread.retain(|contact, _| known.contains(contact));
        slice.deferrals.retain(|contact, _| known.contains(contact));
        slice.retries.retain(|contact, _| queued.contains(contact));
        slice.make_rows(next(&mut self.revisions));
        if let Some(Selection::Conversation(contact)) = slice.selection {
            if !known.contains(&contact) {
                slice.selection = None;
            }
        }
        // A contact just added or accepted is shown once it is known, and
        // only while its identity is the one shown.
        if let Some(contact) = self.pending_open {
            if contact.identity() == identity {
                self.pending_open = None;
                if known.contains(&contact) && self.current == Some(identity) {
                    self.select_conversation(contact);
                }
            }
        }
    }

    fn set_window(&mut self, contact: UiContactId, messages: Vec<MessageView>, complete: bool) {
        let revision = next(&mut self.revisions);
        let Some(thread) = self.thread_mut(contact) else {
            return;
        };
        let mut messages: Vec<MessageView> = messages
            .into_iter()
            .filter(|message| message.id.contact() == contact)
            .collect();
        messages.sort_by_key(|message| message.id.sequence());
        // A text prepared before is kept only for a message that reads the
        // same.
        let same: BTreeSet<UiMessageId> = {
            let before: BTreeMap<UiMessageId, &str> = thread
                .messages
                .iter()
                .map(|message| (message.id, message.text.as_str()))
                .collect();
            messages
                .iter()
                .filter(|message| before.get(&message.id) == Some(&message.text.as_str()))
                .map(|message| message.id)
                .collect()
        };
        thread.prepared.retain(|message, _| same.contains(message));
        thread.messages = messages;
        thread.complete = complete;
        let present: BTreeSet<UiMessageId> = thread.messages.iter().map(|m| m.id).collect();
        thread.expanded.retain(|message| present.contains(message));
        thread.revision = revision;
    }

    fn prepend(&mut self, contact: UiContactId, messages: Vec<MessageView>, complete: bool) {
        let revision = next(&mut self.revisions);
        let Some(thread) = self.thread_mut(contact) else {
            return;
        };
        thread.revision = revision;
        let oldest = thread.messages.first().map(|message| message.id.sequence());
        let mut older: Vec<MessageView> = messages
            .into_iter()
            .filter(|message| message.id.contact() == contact)
            .filter(|message| oldest.is_none_or(|oldest| message.id.sequence() < oldest))
            .collect();
        older.sort_by_key(|message| message.id.sequence());
        let room = MAX_WINDOW.saturating_sub(thread.messages.len());
        let skip = older.len().saturating_sub(room);
        let mut window: Vec<MessageView> = older.into_iter().skip(skip).collect();
        window.append(&mut thread.messages);
        thread.messages = window;
        thread.complete = complete;
    }

    fn send_done(
        &mut self,
        command: CommandId,
        contact: UiContactId,
        text: &str,
        outcome: Outcome,
    ) {
        let queued = matches!(&outcome, Outcome::MessageQueued(message)
            if message.contact() == contact);
        if let (true, Outcome::MessageQueued(message)) = (queued, &outcome) {
            self.known_queued(*message);
        }
        if let Some(thread) = self.thread_mut(contact) {
            if thread.sending == Some(command) {
                thread.sending = None;
            }
            let starts = core::mem::take(&mut thread.draft_starts_with_sent);
            if queued {
                // Cleared only now that the runtime holds the message, and
                // only of the text that was sent, if the draft has started
                // with it ever since: what was typed after it while it was
                // queued stays. A draft cleared, or edited within that
                // text, since it was sent stays as it is.
                if let Some(typed) = thread.draft.strip_prefix(text).filter(|_| starts) {
                    let kept = typed.trim_start().len();
                    let sent = thread.draft.len().saturating_sub(kept);
                    thread.draft.drain(..sent);
                }
                thread.stale = true;
            } else if let Outcome::Failed(failure) = outcome {
                thread.send_error = Some(failure);
            }
        }
        self.mark_identity(contact.identity());
    }

    /// What is said once `request` is accepted: if it came through an
    /// invitation that is active while the identity takes requests, that
    /// invitation, which keeps admitting requests until it is revoked, and
    /// a sentence that says so.
    fn accepted_through(&self, request: UiRequestId) -> Option<(UiInvitationId, Prose)> {
        let slice = self.slices.get(&request.identity())?;
        if slice.request_mode == Some(RequestMode::Closed) {
            return None;
        }
        let invitation = slice
            .requests
            .iter()
            .find(|view| view.id == request)?
            .invitation?;
        let label = slice
            .invitations
            .iter()
            .find(|view| view.id == invitation)?
            .label
            .as_deref()
            .filter(|label| !label.trim().is_empty());
        let said = Prose::said("Request accepted. Others who have ");
        let named = match label {
            Some(label) => said.and("your invitation ").name(label),
            None => said.and("the invitation it came through"),
        };
        Some((
            invitation,
            named.and(" can still send you requests until you revoke it."),
        ))
    }

    fn dialog_done(&mut self, command: CommandId, outcome: Outcome) {
        // Whether a contact just added had accepted already: then its card
        // changed nothing.
        let accepted_before = match &outcome {
            Outcome::ContactAdded { contact, .. } => {
                self.contact(*contact).is_some_and(|view| view.accepted)
            }
            _ => false,
        };
        // The invitation a request just accepted came through, offered to
        // be revoked.
        let mut through = match (&outcome, self.dialog.as_ref()) {
            (Outcome::RequestAccepted(_), Some(Dialog::AcceptRequest { request, .. })) => {
                self.accepted_through(*request)
            }
            _ => None,
        };
        let mut revoke = None;
        let Some(dialog) = self.dialog.as_mut() else {
            // The dialog is gone; what its command changed is read again.
            self.mark_everything_of_outcome(&outcome);
            return;
        };
        if dialog.busy() != Some(command) {
            self.mark_everything_of_outcome(&outcome);
            return;
        }
        if let Outcome::Failed(failure) = outcome {
            dialog.set_error(failure);
            return;
        }
        let mut close = true;
        let mut affected: Option<UiIdentityId> = dialog.identity();
        let mut message: Option<&'static str> = None;
        // What an answer names is taken only if it belongs to the identity
        // the dialog acts on; anything else closes the dialog, and that
        // identity is read again.
        match (dialog, outcome) {
            (Dialog::OwnCard { card, busy, .. }, Outcome::Card(text))
            | (Dialog::ShowInvitation { card, busy, .. }, Outcome::Card(text)) => {
                *card = Some(text);
                *busy = None;
                close = false;
            }
            (
                Dialog::NewInvitation {
                    identity,
                    created,
                    busy,
                    ..
                },
                Outcome::InvitationCreated { invitation, card },
            ) if invitation.identity() == *identity => {
                *created = Some((invitation, card));
                *busy = None;
                close = false;
                message = Some("Invitation created.");
            }
            (Dialog::NewIdentity { .. }, Outcome::IdentityCreated(identity)) => {
                self.identities_stale = true;
                affected = Some(identity);
                self.pending_select = Some(identity);
                message = Some("Identity created.");
            }
            (
                Dialog::AddContact {
                    identity,
                    card,
                    busy,
                    added,
                    ..
                },
                Outcome::ContactAdded {
                    contact,
                    invitation,
                },
            ) if contact.identity() == *identity => {
                affected = Some(contact.identity());
                self.pending_open = Some(contact);
                if invitation || accepted_before {
                    message = Some("Contact added.");
                } else {
                    // A request without an invitation is dropped unless the
                    // other side takes requests from anyone; it has to add
                    // this identity's card too (`PROTOCOL.md` section
                    // 12.2). The dialog stays to say so.
                    card.zeroize();
                    *busy = None;
                    *added = Some(contact);
                    close = false;
                }
            }
            (Dialog::ContactDetails { busy, .. }, Outcome::Done) => {
                *busy = None;
                close = false;
                message = Some("Saved.");
            }
            (Dialog::BlockContact { .. }, Outcome::Done) => message = Some("Contact blocked."),
            (Dialog::RemoveContact { .. }, Outcome::Done) => message = Some("Contact removed."),
            (Dialog::DeleteConversation { contact, .. }, Outcome::Done) => {
                let contact = *contact;
                // What was read goes; the conversation stays, with its
                // draft, and is read again.
                if let Some(thread) = self
                    .slices
                    .get_mut(&contact.identity())
                    .and_then(|slice| slice.threads.get_mut(&contact))
                {
                    thread.messages.clear();
                    thread.expanded.clear();
                    thread.prepared.clear();
                    thread.revision = next(&mut self.revisions);
                    thread.stale = true;
                }
                self.forgotten.push(contact);
                message = Some("Messages deleted.");
            }
            (Dialog::AcceptRequest { request, .. }, Outcome::RequestAccepted(contact))
                if contact.identity() == request.identity() =>
            {
                affected = Some(contact.identity());
                message = Some("Request accepted.");
                revoke = through.take();
                self.pending_open = Some(contact);
            }
            (Dialog::RevokeInvitation { .. }, Outcome::Revoked { .. }) => {
                message = Some("Invitation revoked.");
            }
            (Dialog::DeleteIdentity { .. }, Outcome::Done) => {
                self.identities_stale = true;
                message = Some("Identity deleted.");
            }
            _ => {}
        }
        if close {
            self.dialog = None;
        }
        if let Some(identity) = affected {
            self.mark_identity(identity);
        }
        match (message, revoke) {
            (Some(_), Some((invitation, text))) => self.notices.push(Notice {
                kind: NoticeKind::Success,
                text,
                revoke: Some(invitation),
            }),
            (Some(text), None) => self.notice(NoticeKind::Success, text),
            (None, _) => {}
        }
    }

    fn mark_everything_of_outcome(&mut self, outcome: &Outcome) {
        match outcome {
            Outcome::ContactAdded { contact, .. } | Outcome::RequestAccepted(contact) => {
                self.mark_identity(contact.identity());
            }
            Outcome::IdentityCreated(_) => self.identities_stale = true,
            _ => {}
        }
    }

    fn event(&mut self, event: RuntimeEvent) {
        if let RuntimeEvent::MessageQueued(message) = &event {
            self.known_queued(*message);
        }
        match event {
            RuntimeEvent::MessageQueued(message) | RuntimeEvent::MessageDelivered(message) => {
                self.mark_thread(message.contact());
                self.mark_identity(message.contact().identity());
            }
            RuntimeEvent::MessageReceived(message) => {
                let contact = message.contact();
                let shown = self.current == Some(contact.identity())
                    && self.selection() == Some(Selection::Conversation(contact));
                if let Some(slice) = self.slices.get_mut(&contact.identity()) {
                    if !shown {
                        let count = slice.unread.entry(contact).or_insert(0);
                        *count = count.saturating_add(1);
                        slice.unread_changed(contact, next(&mut self.revisions));
                    }
                }
                self.mark_thread(contact);
                self.mark_identity(contact.identity());
            }
            RuntimeEvent::MessageRefused(contact) => {
                if let Some(thread) = self.thread_mut(contact) {
                    thread.refused = true;
                }
            }
            RuntimeEvent::DeliveryDeferred {
                contact,
                reason,
                retry_in,
            } => {
                let next = self.now.saturating_add(retry_in);
                if let Some(slice) = self.slices.get_mut(&contact.identity()) {
                    slice.deferrals.insert(contact, reason);
                    slice.retries.insert(contact, next);
                }
                if reason == crate::data::Deferral::Tor {
                    self.tor_wanted = true;
                }
            }
            RuntimeEvent::Service { identity, state } => {
                if let Some(slice) = self.slices.get_mut(&identity) {
                    slice.service = Some(state);
                }
                if state == Service::Unavailable || state == Service::Available {
                    self.tor_wanted = true;
                }
            }
            RuntimeEvent::ContactAccepted(contact) => {
                self.mark_identity(contact.identity());
                // An event of another identity changes nothing shown (ADR
                // 0011, State): its contact shows as accepted there.
                if self.current() == Some(contact.identity()) {
                    let name = self.contact_name(contact);
                    self.notice(
                        NoticeKind::Info,
                        Prose::said("").name(&name).and(" accepted your request."),
                    );
                }
            }
            RuntimeEvent::RequestReceived { identity, queued } => {
                if queued {
                    self.mark_identity(identity);
                }
            }
            RuntimeEvent::IdentityClosed(_) => self.identities_stale = true,
            // The screens say it, once and for as long as it lasts; a
            // notice would say it again.
            RuntimeEvent::StorageFailed => self.storage_failed = true,
            RuntimeEvent::Missed => self.mark_everything(),
        }
    }
}

fn optional(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// The status a message is shown with: exactly the runtime's.
pub fn shown_status(message: &MessageView) -> Status {
    message.status
}

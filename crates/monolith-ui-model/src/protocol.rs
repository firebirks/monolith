//! What passes between the UI model and the bridge: commands that carry
//! the handles they act on, and updates that come back to those handles.
//! A command is numbered by the model; its result names that number, so
//! that it reaches the command it answers and nothing else.

use core::fmt;

use zeroize::Zeroizing;

use crate::data::{
    ContactView, ConversationView, IdentityView, InvitationView, MessageView, RequestMode,
    RequestView, RuntimeEvent, Service, TorCheck,
};
use crate::failure::Failure;
use crate::ids::{UiContactId, UiIdentityId, UiInvitationId, UiMessageId, UiRequestId};

/// The number of a command, given by the model, never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CommandId(u64);

impl CommandId {
    /// The command numbered `number`. Only the model numbers commands; an
    /// answer that names a number no command in flight has changes
    /// nothing.
    pub const fn new(number: u64) -> Self {
        Self(number)
    }

    /// The number.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Something the bridge is asked to do. Every command names what it acts
/// on; none acts on "whatever is selected".
#[derive(Clone, PartialEq, Eq)]
pub enum Command {
    /// Open the profile in the data directory with this passphrase.
    OpenProfile {
        /// The passphrase; wiped when dropped.
        passphrase: Zeroizing<String>,
    },
    /// Create a profile in the data directory under this passphrase.
    CreateProfile {
        /// The passphrase; wiped when dropped.
        passphrase: Zeroizing<String>,
    },
    /// Start a session that keeps nothing on disk.
    StartWithoutSaving,
    /// Read the local identities.
    LoadIdentities,
    /// Read everything about one identity: contacts, conversations,
    /// requests, invitations, the state of its service, its request mode.
    LoadIdentity(UiIdentityId),
    /// Read the newest `limit` messages of a conversation.
    LoadMessages {
        /// The conversation.
        contact: UiContactId,
        /// How many.
        limit: usize,
    },
    /// Read up to `limit` messages before the sequence `before`.
    LoadEarlier {
        /// The conversation.
        contact: UiContactId,
        /// Before this sequence.
        before: u64,
        /// How many.
        limit: usize,
    },
    /// Ask the system Tor how it is.
    CheckTor,
    /// Read the card of an identity.
    OwnCard(UiIdentityId),
    /// Read the card of an invitation again.
    InvitationCard(UiInvitationId),
    /// Make a local identity.
    CreateIdentity {
        /// Its local name.
        name: Option<String>,
    },
    /// Delete a local identity.
    DeleteIdentity(UiIdentityId),
    /// Import a card into an identity, and name the contact.
    AddContact {
        /// The identity.
        identity: UiIdentityId,
        /// The card, in text form.
        card: String,
        /// The local name for the contact.
        name: Option<String>,
    },
    /// Name a contact, or remove its name.
    SetContactName {
        /// The record.
        contact: UiContactId,
        /// The name.
        name: Option<String>,
    },
    /// Record whether the user verified a contact's fingerprint.
    SetVerified {
        /// The record.
        contact: UiContactId,
        /// Verified or not.
        verified: bool,
    },
    /// Block a contact.
    BlockContact(UiContactId),
    /// Remove a contact.
    RemoveContact(UiContactId),
    /// Delete the messages of a conversation from this device.
    DeleteConversation(UiContactId),
    /// Queue a message.
    Send {
        /// The conversation.
        contact: UiContactId,
        /// The text.
        text: String,
    },
    /// Accept a contact request, and name the new contact.
    AcceptRequest {
        /// The request.
        request: UiRequestId,
        /// The local name for the contact.
        name: Option<String>,
    },
    /// Decline a contact request.
    DeclineRequest(UiRequestId),
    /// Block the sender of a contact request.
    BlockRequest(UiRequestId),
    /// Make an invitation.
    CreateInvitation {
        /// The identity.
        identity: UiIdentityId,
        /// Its local label.
        label: Option<String>,
    },
    /// Revoke an invitation.
    RevokeInvitation {
        /// The invitation.
        invitation: UiInvitationId,
        /// Also drop the requests it admitted that still wait.
        discard_requests: bool,
    },
    /// Set which requests from strangers an identity considers.
    SetRequestMode {
        /// The identity.
        identity: UiIdentityId,
        /// The mode.
        mode: RequestMode,
    },
    /// Stop the runtime: sessions close, services are removed.
    Shutdown,
}

impl fmt::Debug for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Passphrases, texts, cards and names are never printed.
        match self {
            Self::OpenProfile { .. } => f.write_str("OpenProfile"),
            Self::CreateProfile { .. } => f.write_str("CreateProfile"),
            Self::StartWithoutSaving => f.write_str("StartWithoutSaving"),
            Self::LoadIdentities => f.write_str("LoadIdentities"),
            Self::LoadIdentity(identity) => write!(f, "LoadIdentity({identity:?})"),
            Self::LoadMessages { contact, limit } => {
                write!(f, "LoadMessages({contact:?}, {limit})")
            }
            Self::LoadEarlier {
                contact,
                before,
                limit,
            } => write!(f, "LoadEarlier({contact:?}, {before}, {limit})"),
            Self::CheckTor => f.write_str("CheckTor"),
            Self::OwnCard(identity) => write!(f, "OwnCard({identity:?})"),
            Self::InvitationCard(invitation) => write!(f, "InvitationCard({invitation:?})"),
            Self::CreateIdentity { .. } => f.write_str("CreateIdentity"),
            Self::DeleteIdentity(identity) => write!(f, "DeleteIdentity({identity:?})"),
            Self::AddContact { identity, .. } => write!(f, "AddContact({identity:?})"),
            Self::SetContactName { contact, .. } => write!(f, "SetContactName({contact:?})"),
            Self::SetVerified { contact, verified } => {
                write!(f, "SetVerified({contact:?}, {verified})")
            }
            Self::BlockContact(contact) => write!(f, "BlockContact({contact:?})"),
            Self::RemoveContact(contact) => write!(f, "RemoveContact({contact:?})"),
            Self::DeleteConversation(contact) => write!(f, "DeleteConversation({contact:?})"),
            Self::Send { contact, .. } => write!(f, "Send({contact:?})"),
            Self::AcceptRequest { request, .. } => write!(f, "AcceptRequest({request:?})"),
            Self::DeclineRequest(request) => write!(f, "DeclineRequest({request:?})"),
            Self::BlockRequest(request) => write!(f, "BlockRequest({request:?})"),
            Self::CreateInvitation { identity, .. } => {
                write!(f, "CreateInvitation({identity:?})")
            }
            Self::RevokeInvitation {
                invitation,
                discard_requests,
            } => write!(f, "RevokeInvitation({invitation:?}, {discard_requests})"),
            Self::SetRequestMode { identity, mode } => {
                write!(f, "SetRequestMode({identity:?}, {mode:?})")
            }
            Self::Shutdown => f.write_str("Shutdown"),
        }
    }
}

/// Everything known about one identity, read at one time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentitySnapshot {
    /// The identity.
    pub identity: UiIdentityId,
    /// Its requested and accepted contacts.
    pub contacts: Vec<ContactView>,
    /// Its conversations, ended ones included.
    pub conversations: Vec<ConversationView>,
    /// Its requests waiting for an answer.
    pub requests: Vec<RequestView>,
    /// Its active invitations.
    pub invitations: Vec<InvitationView>,
    /// The state of its Onion Service.
    pub service: Service,
    /// Its request mode.
    pub request_mode: RequestMode,
}

/// How a command ended.
#[derive(Clone, PartialEq, Eq)]
pub enum Outcome {
    /// It failed.
    Failed(Failure),
    /// It succeeded with nothing to report.
    Done,
    /// A profile is open.
    ProfileOpen {
        /// Nothing is saved.
        ephemeral: bool,
        /// The vault holds identities but their messages were not found.
        message_store_missing: bool,
    },
    /// The local identities.
    Identities(Vec<IdentityView>),
    /// One identity.
    Identity(Box<IdentitySnapshot>),
    /// Messages of a conversation, newest last.
    Messages {
        /// The conversation.
        contact: UiContactId,
        /// The messages.
        messages: Vec<MessageView>,
        /// No older message exists.
        complete: bool,
    },
    /// What Tor said.
    Tor(TorCheck),
    /// A card in text form. It can carry an invitation, so it is wiped
    /// when dropped.
    Card(Zeroizing<String>),
    /// An identity was made.
    IdentityCreated(UiIdentityId),
    /// A contact was added.
    ContactAdded(UiContactId),
    /// A message was durably queued.
    MessageQueued(UiMessageId),
    /// A request was accepted.
    RequestAccepted(UiContactId),
    /// An invitation was made.
    InvitationCreated {
        /// Which.
        invitation: UiInvitationId,
        /// Its card, wiped when dropped.
        card: Zeroizing<String>,
    },
    /// An invitation was revoked.
    Revoked {
        /// How many requests were discarded with it.
        discarded: usize,
    },
}

impl fmt::Debug for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed(failure) => write!(f, "Failed({failure:?})"),
            Self::Done => f.write_str("Done"),
            Self::ProfileOpen { ephemeral, .. } => write!(f, "ProfileOpen({ephemeral})"),
            Self::Identities(list) => write!(f, "Identities({})", list.len()),
            Self::Identity(snapshot) => write!(f, "Identity({:?})", snapshot.identity),
            Self::Messages {
                contact, messages, ..
            } => write!(f, "Messages({contact:?}, {})", messages.len()),
            Self::Tor(check) => write!(f, "Tor({check:?})"),
            Self::Card(_) => f.write_str("Card"),
            Self::IdentityCreated(identity) => write!(f, "IdentityCreated({identity:?})"),
            Self::ContactAdded(contact) => write!(f, "ContactAdded({contact:?})"),
            Self::MessageQueued(message) => write!(f, "MessageQueued({message:?})"),
            Self::RequestAccepted(contact) => write!(f, "RequestAccepted({contact:?})"),
            Self::InvitationCreated { invitation, .. } => {
                write!(f, "InvitationCreated({invitation:?})")
            }
            Self::Revoked { discarded } => write!(f, "Revoked({discarded})"),
        }
    }
}

/// Whether a profile exists where the bridge looks for one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileOnDisk {
    /// A vault exists in the data directory.
    Exists,
    /// None does.
    Absent,
}

/// What the bridge tells the model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    /// At start: whether a profile exists, and where.
    Started {
        /// Whether one exists.
        profile: ProfileOnDisk,
        /// The data directory, for display.
        location: String,
    },
    /// A command ended.
    Done {
        /// Which command.
        command: CommandId,
        /// How.
        outcome: Outcome,
    },
    /// The runtime raised an event.
    Event(RuntimeEvent),
    /// The runtime stopped.
    Stopped,
}

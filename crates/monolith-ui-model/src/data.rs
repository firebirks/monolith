//! What the runtime says, as the UI model keeps it: plain values the bridge
//! makes from the runtime's queries and events. Text from other people
//! (names, introductions, messages) is data; its `Debug` output is never
//! shown.

use core::fmt;

use crate::ids::{UiContactId, UiIdentityId, UiInvitationId, UiMessageId, UiRequestId};

/// A local identity.
#[derive(Clone, PartialEq, Eq)]
pub struct IdentityView {
    /// Which.
    pub id: UiIdentityId,
    /// Its local name; never sent.
    pub name: Option<String>,
    /// Its fingerprint, compact.
    pub fingerprint: String,
    /// Its fingerprint, full.
    pub full_fingerprint: String,
}

impl fmt::Debug for IdentityView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdentityView")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// A requested or accepted contact.
#[derive(Clone, PartialEq, Eq)]
pub struct ContactView {
    /// Which record.
    pub id: UiContactId,
    /// The name the user gave it; never sent.
    pub name: Option<String>,
    /// Its fingerprint, compact.
    pub fingerprint: String,
    /// Its fingerprint, full.
    pub full_fingerprint: String,
    /// Both sides accepted each other.
    pub accepted: bool,
    /// The user compared the fingerprint out of band.
    pub verified: bool,
}

impl fmt::Debug for ContactView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContactView")
            .field("id", &self.id)
            .field("accepted", &self.accepted)
            .field("verified", &self.verified)
            .finish_non_exhaustive()
    }
}

/// Where a message stands, as the runtime says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Status {
    /// Sent by this side and stored; not acknowledged.
    Queued,
    /// Acknowledged by the contact.
    Delivered,
    /// Received from the contact.
    Received,
}

impl Status {
    /// Whether this side sent the message.
    pub const fn outgoing(self) -> bool {
        matches!(self, Self::Queued | Self::Delivered)
    }
}

/// A message.
#[derive(Clone, PartialEq, Eq)]
pub struct MessageView {
    /// Which.
    pub id: UiMessageId,
    /// Where it stands.
    pub status: Status,
    /// Local time it was queued or received, in seconds since the epoch.
    pub time: u64,
    /// The text, as it was sent.
    pub text: String,
}

impl fmt::Debug for MessageView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageView")
            .field("id", &self.id)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// A conversation in the list of an identity.
#[derive(Clone, PartialEq, Eq)]
pub struct ConversationView {
    /// Its contact record.
    pub contact: UiContactId,
    /// The fingerprint of the contact, compact.
    pub fingerprint: String,
    /// Its record is a current contact; an ended conversation (the contact
    /// was blocked or removed) keeps what it holds and sends nothing.
    pub active: bool,
    /// Messages stored.
    pub messages: usize,
    /// Of those, queued.
    pub queued: usize,
    /// The newest message, if any.
    pub latest: Option<MessageView>,
}

impl fmt::Debug for ConversationView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConversationView")
            .field("contact", &self.contact)
            .field("active", &self.active)
            .field("messages", &self.messages)
            .finish_non_exhaustive()
    }
}

/// A contact request.
#[derive(Clone, PartialEq, Eq)]
pub struct RequestView {
    /// Which.
    pub id: UiRequestId,
    /// The sender's fingerprint, compact.
    pub fingerprint: String,
    /// The sender's fingerprint, full.
    pub full_fingerprint: String,
    /// The name the sender chose; its own words, possibly empty.
    pub display_name: String,
    /// The sender's introduction; its own words, possibly empty.
    pub introduction: String,
    /// The invitation that admitted it.
    pub invitation: Option<UiInvitationId>,
}

impl fmt::Debug for RequestView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestView")
            .field("id", &self.id)
            .field("invitation", &self.invitation)
            .finish_non_exhaustive()
    }
}

/// An active invitation.
#[derive(Clone, PartialEq, Eq)]
pub struct InvitationView {
    /// Which.
    pub id: UiInvitationId,
    /// Its local label.
    pub label: Option<String>,
}

impl fmt::Debug for InvitationView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InvitationView")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// The state of the Onion Service of an identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Service {
    /// Being published.
    Publishing,
    /// Published.
    Available,
    /// Not published; tried again later.
    Unavailable,
    /// The identity holds no key Tor could publish.
    NoKey,
    /// Stopped.
    Stopped,
}

/// Which contact requests from strangers an identity considers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RequestMode {
    /// Only with one of its invitations.
    Invitation,
    /// Any.
    Open,
    /// None.
    Closed,
}

/// What a check of the system Tor found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TorCheck {
    /// Working.
    Ready {
        /// The version.
        version: String,
        /// Older than recommended.
        outdated: bool,
    },
    /// Reachable, building a circuit.
    Connecting {
        /// Percent, if Tor said.
        progress: Option<u8>,
    },
    /// Nothing answers at the configured endpoints.
    NotRunning,
    /// The SOCKS endpoint does not work.
    SocksUnavailable,
    /// The control endpoint does not work.
    ControlUnavailable,
    /// The control endpoint refused authentication.
    AuthenticationFailed,
    /// Too old.
    TooOld {
        /// The version.
        version: String,
    },
}

/// Why a delivery attempt did not deliver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Deferral {
    /// Tor could not open a stream.
    Tor,
    /// The contact could not be reached or did not answer.
    Unreachable,
    /// The contact's card has no address to dial.
    NoAddress,
    /// No acknowledgement came in time; the message is sent again later.
    NoAcknowledgement,
    /// Monolith limits how many connections run at once.
    Busy,
}

/// What a runtime event said, in handles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeEvent {
    /// A message was queued.
    MessageQueued(UiMessageId),
    /// A message was received and stored.
    MessageReceived(UiMessageId),
    /// A message was acknowledged.
    MessageDelivered(UiMessageId),
    /// A received message could not be stored.
    MessageRefused(UiContactId),
    /// An attempt to deliver failed; the next follows after `retry_in`.
    DeliveryDeferred {
        /// The conversation.
        contact: UiContactId,
        /// Why.
        reason: Deferral,
        /// Seconds to the next attempt.
        retry_in: u64,
    },
    /// The Onion Service of an identity changed state.
    Service {
        /// Which identity.
        identity: UiIdentityId,
        /// Its state.
        state: Service,
    },
    /// A requested contact accepted the identity.
    ContactAccepted(UiContactId),
    /// A contact request arrived; queued or dropped.
    RequestReceived {
        /// The identity.
        identity: UiIdentityId,
        /// Queued for the user.
        queued: bool,
    },
    /// An identity closed.
    IdentityClosed(UiIdentityId),
    /// The vault or the message store failed.
    StorageFailed,
    /// Events were missed; everything is asked again.
    Missed,
}

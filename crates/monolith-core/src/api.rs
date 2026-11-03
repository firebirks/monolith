//! The commands and events a front end exchanges with the runtime
//! ([`crate::runtime::Runtime`]), as plain values.
//!
//! Nothing here holds authority: no local identity, no link, no session,
//! no key, no capability, no handle of the contact store. Identities,
//! contact records and messages are named by opaque identifiers that stay
//! the same across restarts and never name anything of another identity.
//! Text a peer sent is shown as text only; `Debug` prints none of it.
//!
//! Events are notifications. One that reports stored state (a message
//! queued, received or delivered, a contact accepted) is raised after that
//! state is durable. The others report the runtime, whose state is not
//! stored: sessions, services, a request held in memory, a refused
//! message, a deferred attempt, an identity that closed, a failed store.
//! A front end that misses some (a slow consumer is told how many it
//! missed) reads the current state again with the queries.

use core::fmt;
use core::time::Duration;

pub use monolith_storage::record::{ContactId, IdentityId};

/// The length of a fingerprint (`monolith_identity::FINGERPRINT_LEN`); a
/// handle made from one of another length does not compile.
const FINGERPRINT_LEN: usize = 32;

/// One message of a conversation: its local identity, its contact record,
/// its place in the conversation. Never the same for two messages of the
/// installation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MessageRef {
    /// The local identity.
    pub identity: IdentityId,
    /// The contact record of the conversation.
    pub contact: ContactId,
    /// The place of the message in its conversation.
    pub sequence: u64,
}

/// Where a message stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MessageStatus {
    /// Sent by the local side and stored; not acknowledged by the contact.
    Queued,
    /// Acknowledged by the contact: its Monolith holds it durably.
    Delivered,
    /// Received from the contact.
    Received,
}

/// A message, for display.
#[derive(Clone, PartialEq, Eq)]
pub struct MessageView {
    /// Which message.
    pub reference: MessageRef,
    /// Where it stands.
    pub status: MessageStatus,
    /// The local time it was queued or received, in seconds since the Unix
    /// epoch. For display only.
    pub time: u64,
    /// The text. Peer text is data, never markup (S15).
    pub text: String,
}

impl fmt::Debug for MessageView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageView")
            .field("reference", &self.reference)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// A conversation, for a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationSummary {
    /// The contact record it belongs to.
    pub contact: ContactId,
    /// The fingerprint of the remote identity, in its compact form.
    pub fingerprint: String,
    /// Its contact record is the current one of a requested or accepted
    /// contact. A conversation of a contact deleted or blocked is ended:
    /// what it holds stays, and its queue is never sent.
    pub active: bool,
    /// Messages stored.
    pub messages: usize,
    /// Of those, queued.
    pub queued: usize,
    /// The sequence of the newest message.
    pub latest: Option<u64>,
}

/// A local identity, for a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentitySummary {
    /// Which identity.
    pub identity: IdentityId,
    /// The fingerprint of its identity key, in its compact form.
    pub fingerprint: String,
    /// The same fingerprint in its full form, for comparing out of band.
    pub full_fingerprint: String,
    /// Its local label. Never sent.
    pub label: Option<String>,
}

/// A contact, for a list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactSummary {
    /// Its contact record.
    pub contact: ContactId,
    /// The fingerprint of its identity key, in its compact form.
    pub fingerprint: String,
    /// The same fingerprint in its full form, for comparing out of band.
    pub full_fingerprint: String,
    /// The local alias. Never sent.
    pub alias: Option<String>,
    /// Both sides accepted each other; until then the contact is requested.
    pub accepted: bool,
    /// The user compared the fingerprint out of band.
    pub verified: bool,
    /// The card held for the contact carries an invitation, and a request
    /// to it carries that invitation. Without one, a contact in the
    /// default request mode drops the request: it has to import the card
    /// of the local identity too (`docs/PROTOCOL.md` section 12.2).
    pub invitation: bool,
}

/// A contact card that was imported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AddedContact {
    /// The contact record of its identity.
    pub contact: ContactId,
    /// The card the record holds after the import carries an invitation,
    /// as [`ContactSummary::invitation`] says. For an accepted contact the
    /// card imported changes nothing (`docs/PROTOCOL.md` section 12.2).
    pub invitation: bool,
}

/// Why a message was not queued. Nothing was stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SendError {
    /// The text is empty, or breaks the text rules of `docs/PROTOCOL.md`
    /// section 9.
    InvalidText,
    /// The text is longer than `MAX_CHAT_TEXT_LEN` bytes.
    TooLong,
    /// No such local identity, or it is closed.
    UnknownIdentity,
    /// The record is not the current one of a requested or accepted
    /// contact of the identity.
    NotAContact,
    /// A limit of the queue is reached.
    QueueFull,
    /// The message store is full.
    StoreFull,
    /// The message store could not be written. Nothing more is stored until
    /// the process starts again.
    Storage,
    /// The runtime is shutting down: the send was refused, or cut short. A
    /// send cut short once the message had entered the state of the store,
    /// while it waited for the write, may still be written, by that write or
    /// a later one; the message is then queued after a restart, and no
    /// `MessageQueued` was raised for it.
    ShuttingDown,
    /// The random source failed: the message has no identifier. Nothing
    /// was stored, and the store goes on.
    Randomness,
}

impl fmt::Display for SendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidText => "invalid text",
            Self::TooLong => "text too long",
            Self::UnknownIdentity => "no such identity",
            Self::NotAContact => "not a contact",
            Self::QueueFull => "queue full",
            Self::StoreFull => "message store full",
            Self::Storage => "message store failed",
            Self::ShuttingDown => "shutting down",
            Self::Randomness => "random source failed",
        })
    }
}

impl core::error::Error for SendError {}

/// Why another command failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CommandError {
    /// No such local identity, or it is closed.
    UnknownIdentity,
    /// No such contact record, or it is no longer the current one.
    UnknownContact,
    /// A contact card that does not parse or verify.
    InvalidCard,
    /// The text of a label or an alias breaks the rules.
    InvalidText,
    /// A limit is reached.
    Full,
    /// The identity is blocked; unblock it first.
    Blocked,
    /// The card is one of the local identity.
    OwnIdentity,
    /// Tor did not do what was asked.
    Tor,
    /// The vault or the message store could not be written.
    Storage,
    /// The runtime is shutting down.
    ShuttingDown,
    /// No such contact request: it was answered, dropped, or belongs to
    /// another identity or to an instance of it from before.
    UnknownRequest,
    /// No such invitation: it was revoked, or belongs to another identity
    /// or to an instance of it from before.
    UnknownInvitation,
    /// No such blocked identity: it was unblocked, or the handle belongs
    /// to another identity or to an instance of it from before.
    NotBlocked,
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnknownIdentity => "no such identity",
            Self::UnknownContact => "no such contact",
            Self::InvalidCard => "invalid contact card",
            Self::InvalidText => "invalid text",
            Self::Full => "limit reached",
            Self::Blocked => "identity is blocked",
            Self::OwnIdentity => "card of the local identity",
            Self::Tor => "Tor failed",
            Self::Storage => "storage failed",
            Self::ShuttingDown => "shutting down",
            Self::UnknownRequest => "no such request",
            Self::UnknownInvitation => "no such invitation",
            Self::NotBlocked => "not blocked",
        })
    }
}

impl core::error::Error for CommandError {}

/// A contact request waiting for the user, as a front end names it: its
/// local identity and the request of the core, which is local to that
/// instance of the identity and is not kept across restarts. Only a query
/// makes one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RequestRef {
    /// The local identity the request was made to.
    pub identity: IdentityId,
    /// The instance of the identity and the number of the request in it.
    request: (u64, u64),
}

impl RequestRef {
    pub(crate) const fn new(identity: IdentityId, request: (u64, u64)) -> Self {
        Self { identity, request }
    }

    pub(crate) const fn request(&self) -> (u64, u64) {
        self.request
    }
}

/// An invitation of a local identity, as a front end names it: its local
/// identity and the invitation of the core, which is local to that
/// instance of the identity and is not kept across restarts. Only a query
/// or the command that makes the invitation makes one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct InvitationRef {
    /// The local identity that made the invitation.
    pub identity: IdentityId,
    /// The instance of the identity and the number of the invitation in it.
    invitation: (u64, u64),
}

impl InvitationRef {
    pub(crate) const fn new(identity: IdentityId, invitation: (u64, u64)) -> Self {
        Self {
            identity,
            invitation,
        }
    }

    pub(crate) const fn invitation(&self) -> (u64, u64) {
        self.invitation
    }
}

/// An identity a local identity blocked, as a front end names it: its
/// local identity, and the fingerprint blocked bound to that instance of
/// the identity, which is not kept across restarts. Only a query makes
/// one.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockedRef {
    /// The local identity that blocked it.
    pub identity: IdentityId,
    /// The instance of the identity and the fingerprint blocked.
    blocked: (u64, [u8; FINGERPRINT_LEN]),
}

impl BlockedRef {
    pub(crate) const fn new(identity: IdentityId, blocked: (u64, [u8; FINGERPRINT_LEN])) -> Self {
        Self { identity, blocked }
    }

    pub(crate) const fn blocked(&self) -> (u64, [u8; FINGERPRINT_LEN]) {
        self.blocked
    }
}

impl fmt::Debug for BlockedRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Not whom: a fingerprint is not printed.
        f.debug_struct("BlockedRef")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

/// A contact request, for display. The display name and the introduction
/// are the sender's own words: data, never a statement of who the sender
/// is.
#[derive(Clone, PartialEq, Eq)]
pub struct RequestSummary {
    /// Which request.
    pub request: RequestRef,
    /// The fingerprint of the sender's identity key, compact.
    pub fingerprint: String,
    /// The same fingerprint in its full form.
    pub full_fingerprint: String,
    /// The display name the sender chose; may be empty.
    pub display_name: String,
    /// The introduction the sender wrote; may be empty.
    pub introduction: String,
    /// The invitation that admitted the request, if any.
    pub invitation: Option<InvitationRef>,
}

impl fmt::Debug for RequestSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestSummary")
            .field("request", &self.request)
            .field("invitation", &self.invitation)
            .finish_non_exhaustive()
    }
}

/// An invitation of the active set, for a list.
#[derive(Clone, PartialEq, Eq)]
pub struct InvitationSummary {
    /// Which invitation.
    pub invitation: InvitationRef,
    /// Its local label. Never sent.
    pub label: Option<String>,
}

impl fmt::Debug for InvitationSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InvitationSummary")
            .field("invitation", &self.invitation)
            .finish_non_exhaustive()
    }
}

/// A blocked identity, for a list. The contact store keeps nothing of it
/// but its key: no name, no card.
#[derive(Clone, PartialEq, Eq)]
pub struct BlockedSummary {
    /// Which.
    pub blocked: BlockedRef,
    /// The fingerprint of its identity key, compact.
    pub fingerprint: String,
    /// The same fingerprint in its full form.
    pub full_fingerprint: String,
}

impl fmt::Debug for BlockedSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockedSummary")
            .field("blocked", &self.blocked)
            .finish_non_exhaustive()
    }
}

/// Which contact requests from strangers an identity considers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RequestMode {
    /// Only requests that carry one of its invitations. The default.
    Invitation,
    /// Any request; one that carries an invitation no longer active is
    /// dropped.
    Open,
    /// None.
    Closed,
}

/// What a check of the system Tor found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TorCheck {
    /// Both endpoints work and Tor has a circuit.
    Ready {
        /// The version Tor reported.
        version: String,
        /// Older than the release Monolith recommends.
        outdated: bool,
    },
    /// Both endpoints work, but Tor has no circuit yet.
    Connecting {
        /// Bootstrap progress in percent, if Tor said.
        progress: Option<u8>,
    },
    /// Nothing answers at the configured endpoints: Tor is not running, or
    /// is configured elsewhere.
    NotRunning,
    /// The control endpoint does not answer, and the SOCKS endpoint is not
    /// tried without it (`SocksProof::ControlListeners`): Tor is not
    /// running, or its control port is not at the configured endpoint.
    NoControl,
    /// The SOCKS endpoint does not work: no peer can be reached.
    SocksUnavailable,
    /// The control endpoint does not work: no Onion Service can be
    /// published.
    ControlUnavailable,
    /// The control endpoint refused Monolith's authentication.
    AuthenticationFailed,
    /// The Tor answering is older than Monolith supports.
    TooOld {
        /// The version Tor reported.
        version: String,
    },
}

/// Why a delivery attempt did not deliver. The message stays queued.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DeferReason {
    /// Tor or its SOCKS endpoint did not open a stream.
    Tor,
    /// The handshake failed or timed out.
    Handshake,
    /// The contact's card has no address confirmed for dialing.
    NoEndpoint,
    /// No session budget was free.
    Budget,
    /// No acknowledgement came in time; delivery is uncertain.
    AckTimeout,
    /// The session ended with messages left.
    SessionEnded,
}

/// Where the Onion Service of an identity stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ServiceState {
    /// Being published.
    Publishing,
    /// Published; peers can connect.
    Available,
    /// Not published; another attempt follows after a delay.
    Unavailable,
    /// The identity holds no key Tor could publish.
    NoKey,
    /// Stopped at shutdown.
    Stopped,
}

/// Why a received message was not taken in. It was not acknowledged; the
/// sender keeps it and tries again later.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RefusedReason {
    /// The conversation or the message store is full.
    StoreFull,
    /// Reserved; not raised in this version. A message the message store
    /// could not write is reported by [`Event::StorageFailed`].
    Storage,
}

/// The length of a transfer identifier on the wire.
const TRANSFER_ID_LEN: usize = 16;

/// A file transfer, as a front end names it: its local identity, its
/// contact record, its direction and its identifier on the wire. The same
/// across restarts; never the same for two transfers of the installation.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TransferRef {
    /// The local identity.
    pub identity: IdentityId,
    /// The contact record.
    pub contact: ContactId,
    /// The contact offers the file; otherwise the local identity does.
    pub incoming: bool,
    /// The identifier the offerer drew.
    transfer: [u8; TRANSFER_ID_LEN],
}

impl TransferRef {
    pub(crate) const fn new(
        identity: IdentityId,
        contact: ContactId,
        incoming: bool,
        transfer: [u8; TRANSFER_ID_LEN],
    ) -> Self {
        Self {
            identity,
            contact,
            incoming,
            transfer,
        }
    }

    pub(crate) const fn transfer(&self) -> [u8; TRANSFER_ID_LEN] {
        self.transfer
    }
}

impl fmt::Debug for TransferRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransferRef")
            .field("identity", &self.identity)
            .field("contact", &self.contact)
            .field("incoming", &self.incoming)
            .finish_non_exhaustive()
    }
}

/// Where a transfer stands (`docs/adr/0013-file-transfer.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransferStatus {
    /// Outgoing: staged and offered; the contact has not accepted it.
    Waiting,
    /// Outgoing: the contact accepted it; it is being sent.
    Sending,
    /// Outgoing: the contact acknowledged, with an authenticated message,
    /// that it holds this exact file, verified. Not that anyone saved,
    /// opened or read it.
    Completed,
    /// Incoming: offered; the user has not decided.
    Offered,
    /// Incoming: accepted; it is being received.
    Receiving,
    /// Incoming: the whole file is staged and verified, and can be saved.
    /// Not that it was saved or opened.
    Received,
    /// Incoming: the offer was not decided in time.
    Expired,
    /// The receiver rejected the offer.
    Rejected,
    /// Either side cancelled it; [`TransferView::reason`] says who.
    Cancelled,
    /// It failed; [`TransferView::reason`] says why.
    Failed,
}

/// Why a transfer failed, or who cancelled it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransferReason {
    /// The staged data could not be written or read.
    Storage,
    /// The file received did not match its digest.
    Digest,
    /// No progress for `TRANSFER_STALL_LIMIT`.
    Stalled,
    /// The contact no longer offers the file.
    OffererGone,
    /// Cancelled here.
    ByUs,
    /// Cancelled by the contact.
    ByPeer,
    /// Cancelled because the contact was blocked or deleted.
    ContactEnded,
}

/// A transfer, for display. The name of an incoming one is the contact's
/// text: data, never a path or markup.
#[derive(Clone, PartialEq, Eq)]
pub struct TransferView {
    /// Which transfer.
    pub reference: TransferRef,
    /// Where it stands.
    pub status: TransferStatus,
    /// Why it failed or who cancelled it.
    pub reason: Option<TransferReason>,
    /// The declared size in bytes.
    pub size: u64,
    /// The filename of the offer.
    pub name: String,
    /// Outgoing: the bytes the contact acknowledged holding; incoming: the
    /// bytes staged durably. `None` while it is not known: after the
    /// installation opened again, or after a write that failed, until the
    /// transfer goes on and the staged file or the contact says it again.
    /// Never a 0 in its place: what was held is not lost.
    pub bytes: Option<u64>,
    /// The local time the transfer was made, in seconds since the Unix
    /// epoch. For display only.
    pub created: u64,
    /// Incoming: received data waits because the disk was full or the
    /// staged file was held by another program; it goes on later or when
    /// the user resumes it.
    pub held: bool,
    /// Incoming: the file was saved.
    pub saved: bool,
}

impl fmt::Debug for TransferView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TransferView")
            .field("reference", &self.reference)
            .field("status", &self.status)
            .field("reason", &self.reason)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

/// Why a command on a transfer failed. Nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransferError {
    /// No such local identity, or it is closed.
    UnknownIdentity,
    /// The record is not the current one of an accepted contact.
    NotAContact,
    /// No such transfer, or not one this command applies to now.
    UnknownTransfer,
    /// The file is larger than `MAX_TRANSFER_SIZE`.
    TooLarge,
    /// Too many offers wait for an answer, for the contact or the identity.
    TooManyWaiting,
    /// Too many transfers are being received, for the contact or the
    /// identity.
    TooManyActive,
    /// The files staged would take more than their limit.
    StagingFull,
    /// The list of transfers is full: nothing can be removed to make room.
    ListFull,
    /// The disk has not the room the file needs with its margin.
    DiskFull,
    /// The file could not be read, or did not give exactly its size.
    Source,
    /// Another file is being staged or saved.
    Busy,
    /// The transfer store or a staged file could not be written. A failed
    /// transfer store stores nothing more until the process starts again.
    Storage,
    /// The random source failed.
    Randomness,
    /// The runtime is shutting down.
    ShuttingDown,
}

impl fmt::Display for TransferError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnknownIdentity => "no such identity",
            Self::NotAContact => "not an accepted contact",
            Self::UnknownTransfer => "no such transfer",
            Self::TooLarge => "file too large",
            Self::TooManyWaiting => "too many offers waiting",
            Self::TooManyActive => "too many transfers being received",
            Self::StagingFull => "staging full",
            Self::ListFull => "list of transfers full",
            Self::DiskFull => "not enough disk space",
            Self::Source => "file could not be read",
            Self::Busy => "another file is being staged or saved",
            Self::Storage => "storage failed",
            Self::Randomness => "random source failed",
            Self::ShuttingDown => "shutting down",
        })
    }
}

impl core::error::Error for TransferError {}

/// How a save of a received file ended that wrote the file
/// (`docs/adr/0014-transfer-staging.md`, "Saving a received file").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SaveOutcome {
    /// The file is whole under the name chosen, and durable: the staged
    /// copy is gone.
    Saved,
    /// The file is whole under the name chosen, but the folder could not
    /// be synced or the temporary name removed: a crash now could lose it,
    /// and a file named `monolith-partial-...` may be left beside it. The
    /// staged copy stays, so the file can be saved again.
    Unconfirmed,
}

/// Why a received file was not saved. Nothing was written under the name
/// chosen, nothing was replaced, and the staged copy stays unless it is
/// damaged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SaveError {
    /// The transfer cannot be saved now.
    Transfer(TransferError),
    /// Something has the name chosen already.
    Exists,
    /// Not a place to save a file: not an absolute path, not a folder of
    /// this computer, not a plain file name, or a folder that cannot give
    /// a file its name without the risk of replacing another.
    Refused,
    /// The staged copy did not read back as it was received, and the
    /// transfer failed with the reason storage; or the save was cancelled.
    Source,
    /// The disk has no room left.
    Full,
    /// The disk failed.
    Io,
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transfer(error) => error.fmt(f),
            Self::Exists => f.write_str("a file of that name exists"),
            Self::Refused => f.write_str("not a place a file can be saved"),
            Self::Source => f.write_str("the received file does not read back whole"),
            Self::Full => f.write_str("not enough disk space"),
            Self::Io => f.write_str("disk failure"),
        }
    }
}

impl core::error::Error for SaveError {}

/// Something that happened: stored state once it is durable, or a change
/// of the runtime (see the module documentation).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// A message was queued.
    MessageQueued(MessageRef),
    /// A message was received and stored.
    MessageReceived(MessageRef),
    /// A queued message was acknowledged by its contact.
    MessageDelivered(MessageRef),
    /// A received message was not taken in.
    MessageRefused {
        /// The local identity.
        identity: IdentityId,
        /// The contact record.
        contact: ContactId,
        /// Why.
        reason: RefusedReason,
    },
    /// An attempt to deliver the queue of a conversation failed; the next
    /// follows after `retry_in`. Raised only when it does: not for a
    /// record that ended, an identity that closed or a store that failed.
    DeliveryDeferred {
        /// The local identity.
        identity: IdentityId,
        /// The contact record.
        contact: ContactId,
        /// Why.
        reason: DeferReason,
        /// The delay before the next attempt.
        retry_in: Duration,
    },
    /// A session that can carry the queue of a contact record exists.
    SessionAvailable {
        /// The local identity.
        identity: IdentityId,
        /// The contact record.
        contact: ContactId,
    },
    /// The last session of a contact record ended.
    SessionUnavailable {
        /// The local identity.
        identity: IdentityId,
        /// The contact record.
        contact: ContactId,
    },
    /// The Onion Service of an identity changed state.
    Service {
        /// The local identity.
        identity: IdentityId,
        /// Its new state.
        state: ServiceState,
    },
    /// A requested contact accepted the local identity.
    ContactAccepted {
        /// The local identity.
        identity: IdentityId,
        /// The contact record.
        contact: ContactId,
    },
    /// A contact request from a stranger arrived; how many wait is a
    /// query.
    RequestReceived {
        /// The local identity.
        identity: IdentityId,
        /// It was queued for the user, or dropped by the request policy.
        queued: bool,
    },
    /// A local identity was closed: deleted, or its installation closed.
    /// Raised when it closes, which for a deletion is before the vault
    /// commit; if that commit fails, [`Event::StorageFailed`] follows and
    /// a restart shows the identity.
    IdentityClosed {
        /// Which.
        identity: IdentityId,
    },
    /// The vault, the message store or the transfer store failed: nothing
    /// more is made durable, stored or acknowledged, and no session or
    /// attempt goes on, until the process starts again. Raised once.
    StorageFailed,
    /// A contact offered a file, and the offer was recorded.
    TransferOffered(TransferRef),
    /// A transfer changed state, durably.
    TransferChanged(TransferRef),
    /// More of a transfer is acknowledged (outgoing) or staged durably
    /// (incoming): `bytes` in all. Not stored.
    TransferProgressed {
        /// Which transfer.
        reference: TransferRef,
        /// The bytes acknowledged or staged.
        bytes: u64,
    },
    /// A save of a received file was under way when the process ended:
    /// a file named `monolith-partial-` and 16 hexadecimal digits may be
    /// left in the folder it went to. Raised when the runtime starts.
    SaveInterrupted(TransferRef),
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use monolith_storage::record::RECORD_ID_LEN;

    #[test]
    fn a_message_prints_no_text() {
        let view = MessageView {
            reference: MessageRef {
                identity: IdentityId::from_bytes([1; RECORD_ID_LEN]).unwrap(),
                contact: ContactId::from_bytes([2; RECORD_ID_LEN]).unwrap(),
                sequence: 3,
            },
            status: MessageStatus::Received,
            time: 4,
            text: "very secret words".to_owned(),
        };
        let text = format!("{view:?} {view:#?}");
        assert!(!text.contains("secret"), "{text}");
    }
}

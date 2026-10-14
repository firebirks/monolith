//! The handles of the UI model (`docs/adr/0011-ui-security-boundary.md`).
//!
//! They are presentation handles: holding one allows nothing, and a
//! command that carries one is checked by the runtime like any other. Each
//! names one thing of one instance of one local identity, and never
//! another: an identity handle holds its durable identifier, a contact
//! handle holds its identity handle and the durable identifier of its
//! record, a message handle holds its contact handle and its place in the
//! conversation. Requests, invitations and blocked people, whose
//! identifiers in the core do not outlive the process, get handles from the
//! bridge that are never reused. No handle comes from a list position, a
//! name or loop order.

use core::fmt;

/// The length of a durable identifier of an identity or a contact record.
pub const RECORD_ID_LEN: usize = 16;

/// Hex of the first bytes, enough to tell handles apart in a log.
fn short(bytes: &[u8]) -> String {
    bytes
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A local identity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UiIdentityId([u8; RECORD_ID_LEN]);

impl UiIdentityId {
    /// The handle of the identity whose durable identifier is `bytes`.
    pub const fn from_bytes(bytes: [u8; RECORD_ID_LEN]) -> Self {
        Self(bytes)
    }

    /// The durable identifier, for the bridge.
    pub const fn bytes(&self) -> [u8; RECORD_ID_LEN] {
        self.0
    }
}

impl fmt::Debug for UiIdentityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UiIdentityId({})", short(&self.0))
    }
}

/// A contact record of a local identity.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UiContactId {
    identity: UiIdentityId,
    record: [u8; RECORD_ID_LEN],
}

impl UiContactId {
    /// The handle of the record `record` of `identity`.
    pub const fn new(identity: UiIdentityId, record: [u8; RECORD_ID_LEN]) -> Self {
        Self { identity, record }
    }

    /// The identity it belongs to.
    pub const fn identity(&self) -> UiIdentityId {
        self.identity
    }

    /// The durable identifier of the record, for the bridge.
    pub const fn record(&self) -> [u8; RECORD_ID_LEN] {
        self.record
    }
}

impl fmt::Debug for UiContactId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "UiContactId({}/{})",
            short(&self.identity.0),
            short(&self.record)
        )
    }
}

/// A message of a conversation.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UiMessageId {
    contact: UiContactId,
    sequence: u64,
}

impl UiMessageId {
    /// The message at `sequence` in the conversation of `contact`.
    pub const fn new(contact: UiContactId, sequence: u64) -> Self {
        Self { contact, sequence }
    }

    /// The conversation it belongs to.
    pub const fn contact(&self) -> UiContactId {
        self.contact
    }

    /// Its place in the conversation.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
}

impl fmt::Debug for UiMessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UiMessageId({:?}#{})", self.contact, self.sequence)
    }
}

/// A contact request of a local identity. The number is given by the
/// bridge, once, and never reused in the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UiRequestId {
    identity: UiIdentityId,
    handle: u64,
}

impl UiRequestId {
    /// The request numbered `handle` by the bridge, of `identity`.
    pub const fn new(identity: UiIdentityId, handle: u64) -> Self {
        Self { identity, handle }
    }

    /// The identity the request was made to.
    pub const fn identity(&self) -> UiIdentityId {
        self.identity
    }

    /// The number the bridge gave it.
    pub const fn handle(&self) -> u64 {
        self.handle
    }
}

/// An invitation of a local identity. The number is given by the bridge,
/// once, and never reused in the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UiInvitationId {
    identity: UiIdentityId,
    handle: u64,
}

impl UiInvitationId {
    /// The invitation numbered `handle` by the bridge, of `identity`.
    pub const fn new(identity: UiIdentityId, handle: u64) -> Self {
        Self { identity, handle }
    }

    /// The identity that made the invitation.
    pub const fn identity(&self) -> UiIdentityId {
        self.identity
    }

    /// The number the bridge gave it.
    pub const fn handle(&self) -> u64 {
        self.handle
    }
}

/// Someone a local identity blocked. The number is given by the bridge,
/// once, and never reused in the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UiBlockedId {
    identity: UiIdentityId,
    handle: u64,
}

impl UiBlockedId {
    /// The blocked person numbered `handle` by the bridge, of `identity`.
    pub const fn new(identity: UiIdentityId, handle: u64) -> Self {
        Self { identity, handle }
    }

    /// The identity that blocked them.
    pub const fn identity(&self) -> UiIdentityId {
        self.identity
    }

    /// The number the bridge gave it.
    pub const fn handle(&self) -> u64 {
        self.handle
    }
}

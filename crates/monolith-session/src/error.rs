//! Error type of this crate.

use core::fmt;

use monolith_protocol::ProtocolError;

/// Why a handshake or a session operation failed.
///
/// The variants are for local use: logging, counting, and what the user is
/// told. None of them is ever sent to a peer, and none carries data a peer
/// controls. Towards the peer every failure looks the same: the stream is
/// closed and nothing is sent (`docs/PROTOCOL.md` section 4.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SessionError {
    /// The handshake failed or the peer violated the protocol. The session
    /// is over and the stream must be closed without sending anything. The
    /// category inside says what was wrong, for local use.
    Protocol(ProtocolError),
    /// The local card cannot be made from the local keys: key separation
    /// forbids the combination, or the signed card does not verify.
    LocalCard,
    /// The card to dial is a card of the local identity.
    OwnIdentity,
    /// The bytes given as a transport private key are all zero.
    InvalidSecretKey,
    /// The random source of the operating system reported an error.
    Randomness,
    /// The session is over. Nothing can be sent or received on it.
    Closed,
    /// The local side tried to send a message that may not be sent in the
    /// current state of the session. The session is not affected.
    NotPermitted,
    /// The local side tried to send a message that cannot be put on the
    /// wire: a card that is not its own, an invitation capability that is
    /// not the one for this peer, or a body that breaks a rule of its
    /// type. The session is not affected.
    InvalidMessage,
    /// The message does not fit in what the session has left: its age
    /// limit is reached, or the frame would leave no room for a Close. A
    /// smaller message may still fit until
    /// [`crate::AuthenticatedSession::limit_reached`] says that nothing
    /// but Close does. The session is not affected.
    Expired,
    /// The Noise library refused an operation that cannot fail for the
    /// inputs this crate gives it. The handshake or session is over.
    Internal,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Protocol(error) => write!(f, "session ended: {error}"),
            Self::LocalCard => f.write_str("local card cannot be made from the local keys"),
            Self::OwnIdentity => f.write_str("the card to dial is the local identity"),
            Self::InvalidSecretKey => f.write_str("transport private key is all zero"),
            Self::Randomness => f.write_str("random source failed"),
            Self::Closed => f.write_str("session is closed"),
            Self::NotPermitted => f.write_str("message not permitted in this state"),
            Self::InvalidMessage => f.write_str("message cannot be sent"),
            Self::Expired => f.write_str("session limit reached"),
            Self::Internal => f.write_str("session library failure"),
        }
    }
}

impl core::error::Error for SessionError {}

impl From<ProtocolError> for SessionError {
    fn from(error: ProtocolError) -> Self {
        Self::Protocol(error)
    }
}

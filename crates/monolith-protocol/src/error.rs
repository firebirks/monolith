//! Protocol error categories.

use core::fmt;

/// Why input from a peer was rejected.
///
/// The variants carry no peer-controlled data, so an error can be logged or
/// counted without leaking message content. A session that produces one is
/// closed; the peer is not told which error occurred.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProtocolError {
    /// The preamble did not carry the expected magic value.
    BadPreamble,
    /// The peer speaks a protocol major version this build does not.
    UnsupportedVersion,
    /// The cryptographic handshake failed.
    HandshakeFailed,
    /// The identity proof was missing, malformed or did not verify.
    AuthenticationFailed,
    /// The proven identity differs from the one pinned for this contact.
    IdentityMismatch,
    /// A frame length was outside the permitted range.
    FrameLengthOutOfRange,
    /// A frame failed authenticated decryption.
    FrameAuthenticationFailed,
    /// Frame padding was missing or not zero.
    BadPadding,
    /// The message type code is not assigned.
    UnknownMessageType,
    /// The message type is not legal in the current session state.
    MessageNotPermitted,
    /// A message body was shorter or longer than its type allows.
    BadMessageLength,
    /// A variable-size field declared a length above its limit.
    FieldTooLong,
    /// A text field was not valid UTF-8.
    InvalidUtf8,
    /// A text field contained a forbidden character.
    ForbiddenCharacter,
    /// A field held a value that its type does not allow.
    InvalidValue,
    /// A signature did not verify.
    BadSignature,
    /// An endpoint binding did not have a greater epoch than the pinned one.
    StaleEpoch,
    /// The peer exceeded a rate or count limit.
    LimitExceeded,
    /// The session reached its lifetime, frame or byte limit.
    SessionExpired,
    /// A timeout elapsed.
    TimedOut,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BadPreamble => "bad preamble",
            Self::UnsupportedVersion => "unsupported protocol version",
            Self::HandshakeFailed => "handshake failed",
            Self::AuthenticationFailed => "authentication failed",
            Self::IdentityMismatch => "identity mismatch",
            Self::FrameLengthOutOfRange => "frame length out of range",
            Self::FrameAuthenticationFailed => "frame authentication failed",
            Self::BadPadding => "bad padding",
            Self::UnknownMessageType => "unknown message type",
            Self::MessageNotPermitted => "message not permitted in this state",
            Self::BadMessageLength => "bad message length",
            Self::FieldTooLong => "field too long",
            Self::InvalidUtf8 => "invalid UTF-8",
            Self::ForbiddenCharacter => "forbidden character",
            Self::InvalidValue => "invalid value",
            Self::BadSignature => "bad signature",
            Self::StaleEpoch => "stale endpoint epoch",
            Self::LimitExceeded => "limit exceeded",
            Self::SessionExpired => "session expired",
            Self::TimedOut => "timed out",
        })
    }
}

impl core::error::Error for ProtocolError {}

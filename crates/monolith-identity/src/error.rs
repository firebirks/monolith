//! Error type of this crate.

use core::fmt;

/// Why a key, signature or encoded value was rejected.
///
/// The variants carry no input data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IdentityError {
    /// The bytes are not a valid key for the purpose. See `docs/PROTOCOL.md`
    /// section 10.1.
    InvalidKey,
    /// The signature does not verify under strict rules.
    InvalidSignature,
    /// An endpoint epoch of zero.
    InvalidEpoch,
    /// The text is not canonical base32, or decodes to more than allowed.
    InvalidBase32,
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidKey => "invalid key",
            Self::InvalidSignature => "invalid signature",
            Self::InvalidEpoch => "invalid endpoint epoch",
            Self::InvalidBase32 => "invalid base32",
        })
    }
}

impl core::error::Error for IdentityError {}

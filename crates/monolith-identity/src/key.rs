//! Identity key and fingerprint types.

use core::fmt;

use crate::redact::REDACTED;

/// Length in bytes of an Ed25519 public key.
pub const IDENTITY_PUBLIC_KEY_LEN: usize = 32;

/// Length in bytes of an identity fingerprint.
pub const FINGERPRINT_LEN: usize = 32;

/// The canonical Monolith identity: an Ed25519 public key.
///
/// Holding a value of this type says nothing about whether the bytes are a
/// valid curve point. Validation is part of Phase 1 and will be the only way
/// to construct the type once it exists.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct IdentityPublicKey([u8; IDENTITY_PUBLIC_KEY_LEN]);

impl IdentityPublicKey {
    /// Wraps raw key bytes without validating them.
    pub const fn from_bytes_unchecked(bytes: [u8; IDENTITY_PUBLIC_KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// Returns the raw key bytes.
    pub const fn as_bytes(&self) -> &[u8; IDENTITY_PUBLIC_KEY_LEN] {
        &self.0
    }
}

impl fmt::Debug for IdentityPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IdentityPublicKey({REDACTED})")
    }
}

/// Fingerprint of an identity public key, as defined in `docs/PROTOCOL.md`.
///
/// The fingerprint is what users compare out of band. It is derived from the
/// key and nothing else.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint([u8; FINGERPRINT_LEN]);

impl Fingerprint {
    /// Wraps raw fingerprint bytes.
    pub const fn from_bytes(bytes: [u8; FINGERPRINT_LEN]) -> Self {
        Self(bytes)
    }

    /// Returns the raw fingerprint bytes.
    pub const fn as_bytes(&self) -> &[u8; FINGERPRINT_LEN] {
        &self.0
    }
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({REDACTED})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_output_does_not_contain_key_bytes() {
        let key = IdentityPublicKey::from_bytes_unchecked([0xAB; IDENTITY_PUBLIC_KEY_LEN]);
        assert_eq!(format!("{key:?}"), "IdentityPublicKey([redacted])");
        assert_eq!(format!("{key:#?}"), "IdentityPublicKey([redacted])");
    }

    #[test]
    fn debug_output_does_not_contain_fingerprint_bytes() {
        let fingerprint = Fingerprint::from_bytes([0xCD; FINGERPRINT_LEN]);
        assert_eq!(format!("{fingerprint:?}"), "Fingerprint([redacted])");
        assert_eq!(format!("{fingerprint:#?}"), "Fingerprint([redacted])");
    }
}

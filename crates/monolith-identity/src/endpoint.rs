//! Onion endpoint naming types.

use core::fmt;

use crate::redact::REDACTED;

/// Length in bytes of an Onion Service v3 public key.
pub const ONION_SERVICE_KEY_LEN: usize = 32;

/// The Ed25519 public key that names an Onion Service v3 endpoint.
///
/// Monolith stores and signs the 32-byte key, not the textual address. The
/// `.onion` name is derived from the key when it is needed, so a stored
/// endpoint can never carry a wrong checksum or version byte.
///
/// An endpoint is where an identity can be reached. It is not the identity.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct OnionServiceKey([u8; ONION_SERVICE_KEY_LEN]);

impl OnionServiceKey {
    /// Wraps raw key bytes without validating them.
    pub const fn from_bytes_unchecked(bytes: [u8; ONION_SERVICE_KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// Returns the raw key bytes.
    pub const fn as_bytes(&self) -> &[u8; ONION_SERVICE_KEY_LEN] {
        &self.0
    }
}

impl fmt::Debug for OnionServiceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OnionServiceKey({REDACTED})")
    }
}

/// Monotonic counter attached to an identity's endpoint binding.
///
/// A signed endpoint binding replaces a pinned one only if its epoch is
/// strictly greater. See `docs/PROTOCOL.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EndpointEpoch(u64);

impl EndpointEpoch {
    /// Wraps an epoch number.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the epoch number.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns true if `candidate` may replace an endpoint pinned at `self`.
    pub const fn is_superseded_by(self, candidate: Self) -> bool {
        candidate.0 > self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_strictly_greater_epoch_supersedes() {
        let pinned = EndpointEpoch::new(5);
        assert!(pinned.is_superseded_by(EndpointEpoch::new(6)));
        assert!(!pinned.is_superseded_by(EndpointEpoch::new(5)));
        assert!(!pinned.is_superseded_by(EndpointEpoch::new(4)));
        assert!(!EndpointEpoch::new(u64::MAX).is_superseded_by(EndpointEpoch::new(0)));
    }

    #[test]
    fn debug_output_does_not_contain_key_bytes() {
        let key = OnionServiceKey::from_bytes_unchecked([0xEF; ONION_SERVICE_KEY_LEN]);
        assert_eq!(format!("{key:?}"), "OnionServiceKey([redacted])");
        assert_eq!(format!("{key:#?}"), "OnionServiceKey([redacted])");
    }
}

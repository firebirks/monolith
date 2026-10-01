//! Onion endpoint naming types.

use core::fmt;

use crate::IdentityError;
use crate::key::decode_key;
use crate::redact::REDACTED;

/// Length in bytes of an Onion Service v3 public key.
pub const ONION_SERVICE_KEY_LEN: usize = 32;

/// The Ed25519 public key that names an Onion Service v3 endpoint.
///
/// Monolith stores and signs the 32-byte key, not the textual address. The
/// `.onion` name is derived from the key when it is needed, so a stored
/// endpoint can never carry a wrong checksum or version byte.
///
/// A value of this type has passed the checks of `docs/PROTOCOL.md` section
/// 10.1 for onion service keys: it is a valid key and has no torsion
/// component, which is the test Tor applies to the key in an onion address.
///
/// An endpoint is where an identity can be reached. It is not the identity.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct OnionServiceKey([u8; ONION_SERVICE_KEY_LEN]);

impl OnionServiceKey {
    /// Validates 32 bytes as an onion service key.
    pub fn from_bytes(bytes: &[u8; ONION_SERVICE_KEY_LEN]) -> Result<Self, IdentityError> {
        let key = decode_key(bytes)?;
        if !key.to_edwards().is_torsion_free() {
            return Err(IdentityError::InvalidKey);
        }
        Ok(Self(*bytes))
    }

    /// Returns the key bytes.
    pub const fn as_bytes(&self) -> &[u8; ONION_SERVICE_KEY_LEN] {
        &self.0
    }
}

impl fmt::Debug for OnionServiceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OnionServiceKey({REDACTED})")
    }
}

/// Counter attached to an identity's endpoint set. Always at least 1.
///
/// A signed endpoint set replaces a pinned one only if its epoch is strictly
/// greater. See `docs/PROTOCOL.md` section 11.4.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EndpointEpoch(u64);

impl EndpointEpoch {
    /// The first epoch.
    pub const FIRST: Self = Self(1);

    /// Wraps an epoch number. Zero is not an epoch.
    pub const fn new(value: u64) -> Result<Self, IdentityError> {
        if value == 0 {
            Err(IdentityError::InvalidEpoch)
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the epoch number.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the epoch after this one, or `None` at the end of the range.
    pub const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    /// Returns true if `candidate` may replace an endpoint set pinned at
    /// `self`.
    pub const fn is_superseded_by(self, candidate: Self) -> bool {
        candidate.0 > self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IdentityPublicKey, IdentitySecretKey};

    fn honest_key(seed: u8) -> [u8; 32] {
        *IdentitySecretKey::from_seed(&[seed; 32])
            .public_key()
            .as_bytes()
    }

    /// Adds the point of order 2 to an honest key. The result is a valid
    /// point that is not of small order but has a torsion component.
    // Point addition on the curve, not integer arithmetic.
    #[allow(clippy::arithmetic_side_effects)]
    fn key_with_torsion(seed: u8) -> [u8; 32] {
        let mut order_two = [0xff_u8; 32];
        order_two[0] = 0xec;
        order_two[31] = 0x7f;
        let torsion = ed25519_dalek::VerifyingKey::from_bytes(&order_two)
            .unwrap()
            .to_edwards();
        let honest = ed25519_dalek::VerifyingKey::from_bytes(&honest_key(seed))
            .unwrap()
            .to_edwards();
        (honest + torsion).compress().to_bytes()
    }

    #[test]
    fn accepts_an_honestly_generated_key() {
        let bytes = honest_key(1);
        assert_eq!(
            OnionServiceKey::from_bytes(&bytes).unwrap().as_bytes(),
            &bytes
        );
    }

    #[test]
    fn rejects_a_key_with_a_torsion_component() {
        let bytes = key_with_torsion(1);
        // The same bytes are acceptable as an identity key: they are a
        // canonical point that is not of small order.
        assert!(IdentityPublicKey::from_bytes(&bytes).is_ok());
        assert_eq!(
            OnionServiceKey::from_bytes(&bytes).err(),
            Some(IdentityError::InvalidKey)
        );
    }

    #[test]
    fn rejects_the_neutral_element() {
        let mut bytes = [0_u8; 32];
        bytes[0] = 1;
        assert_eq!(
            OnionServiceKey::from_bytes(&bytes).err(),
            Some(IdentityError::InvalidKey)
        );
    }

    #[test]
    fn zero_is_not_an_epoch() {
        assert_eq!(EndpointEpoch::new(0), Err(IdentityError::InvalidEpoch));
        assert_eq!(EndpointEpoch::new(1), Ok(EndpointEpoch::FIRST));
    }

    #[test]
    fn only_a_strictly_greater_epoch_supersedes() {
        let pinned = EndpointEpoch::new(5).unwrap();
        assert!(pinned.is_superseded_by(EndpointEpoch::new(6).unwrap()));
        assert!(!pinned.is_superseded_by(EndpointEpoch::new(5).unwrap()));
        assert!(!pinned.is_superseded_by(EndpointEpoch::new(4).unwrap()));
        let last = EndpointEpoch::new(u64::MAX).unwrap();
        assert!(!last.is_superseded_by(EndpointEpoch::FIRST));
    }

    #[test]
    fn next_stops_at_the_end_of_the_range() {
        assert_eq!(
            EndpointEpoch::FIRST.next(),
            Some(EndpointEpoch::new(2).unwrap())
        );
        assert_eq!(EndpointEpoch::new(u64::MAX).unwrap().next(), None);
    }

    #[test]
    fn debug_output_does_not_contain_key_bytes() {
        let key = OnionServiceKey::from_bytes(&honest_key(1)).unwrap();
        assert_eq!(format!("{key:?}"), "OnionServiceKey([redacted])");
        assert_eq!(format!("{key:#?}"), "OnionServiceKey([redacted])");
    }
}

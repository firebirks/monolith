//! The textual name of an Onion Service v3 endpoint.
//!
//! Monolith stores and signs the 32-byte key ([`OnionServiceKey`]). Tor
//! names a service by its ServiceID, the 56 base32 characters before
//! `.onion`, and a SOCKS request carries the full `.onion` hostname. This
//! module converts between the two, as the onion service specification
//! defines it:
//!
//! ```text
//! onion_address = base32(PUBKEY | CHECKSUM | VERSION) + ".onion"
//! CHECKSUM      = SHA3-256(".onion checksum" | PUBKEY | VERSION)[:2]
//! VERSION       = 0x03
//! ```
//!
//! Every ServiceID that comes from outside, including the one Tor returns
//! for a service Monolith published, is parsed here: version, checksum and
//! the key rules of `docs/PROTOCOL.md` section 10.1. Nothing else in the
//! workspace builds or reads an onion name.

use core::fmt;

use sha3::{Digest, Sha3_256};

use crate::redact::REDACTED;
use crate::{IdentityError, ONION_SERVICE_KEY_LEN, OnionServiceKey, base32};

/// Length of a ServiceID: 35 bytes in base32 without padding.
pub const SERVICE_ID_LEN: usize = 56;

/// Length of the hostname `<ServiceID>.onion`.
pub const ONION_HOSTNAME_LEN: usize = SERVICE_ID_LEN + ONION_SUFFIX.len();

/// The suffix of an onion hostname.
const ONION_SUFFIX: &str = ".onion";

/// The version byte of an Onion Service v3 address.
const VERSION: u8 = 0x03;

/// The prefix of the checksum input.
const CHECKSUM_PREFIX: &[u8] = b".onion checksum";

/// Length of the decoded address: key, checksum, version.
const DECODED_LEN: usize = ONION_SERVICE_KEY_LEN + 2 + 1;

/// The two checksum bytes of an address for `key`.
fn checksum(key: &[u8; ONION_SERVICE_KEY_LEN]) -> [u8; 2] {
    let digest = Sha3_256::new()
        .chain_update(CHECKSUM_PREFIX)
        .chain_update(key)
        .chain_update([VERSION])
        .finalize();
    let mut out = [0_u8; 2];
    for (slot, byte) in out.iter_mut().zip(digest.iter()) {
        *slot = *byte;
    }
    out
}

/// The ServiceID of an Onion Service v3: 56 lower-case base32 characters.
///
/// A value of this type was built from a validated key, so it always names
/// a valid service. Its `Debug` output shows nothing: an onion address is
/// not logged. It has no `Display`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServiceId([u8; SERVICE_ID_LEN]);

impl ServiceId {
    /// Returns the ServiceID that names `key`.
    pub fn from_key(key: &OnionServiceKey) -> Self {
        let mut decoded = [0_u8; DECODED_LEN];
        let checksum = checksum(key.as_bytes());
        let parts = key
            .as_bytes()
            .iter()
            .chain(checksum.iter())
            .chain(core::iter::once(&VERSION));
        for (slot, byte) in decoded.iter_mut().zip(parts) {
            *slot = *byte;
        }
        let mut id = [0_u8; SERVICE_ID_LEN];
        for (slot, byte) in id.iter_mut().zip(base32::encode(&decoded).bytes()) {
            *slot = byte.to_ascii_lowercase();
        }
        Self(id)
    }

    /// Parses a ServiceID and returns the key it names.
    ///
    /// Exactly 56 characters of the lower-case base32 alphabet, which Tor
    /// produces; upper case is refused. The version byte must be 3, the
    /// checksum must match, and the key must pass the rules of
    /// `docs/PROTOCOL.md` section 10.1.
    pub fn parse(text: &[u8]) -> Result<OnionServiceKey, IdentityError> {
        if text.len() != SERVICE_ID_LEN
            || !text
                .iter()
                .all(|byte| byte.is_ascii_lowercase() || (b'2'..=b'7').contains(byte))
        {
            return Err(IdentityError::InvalidOnionAddress);
        }
        let text = core::str::from_utf8(text).map_err(|_| IdentityError::InvalidOnionAddress)?;
        let decoded =
            base32::decode(text, DECODED_LEN).map_err(|_| IdentityError::InvalidOnionAddress)?;
        let (key, rest) = decoded
            .split_first_chunk::<ONION_SERVICE_KEY_LEN>()
            .ok_or(IdentityError::InvalidOnionAddress)?;
        let [first, second] = checksum(key);
        if rest != [first, second, VERSION] {
            return Err(IdentityError::InvalidOnionAddress);
        }
        OnionServiceKey::from_bytes(key)
    }

    /// Returns the ServiceID as text.
    pub fn as_str(&self) -> &str {
        // Built from the base32 alphabet only.
        core::str::from_utf8(&self.0).unwrap_or_default()
    }

    /// Returns the hostname `<ServiceID>.onion`, as a SOCKS request carries
    /// it.
    pub fn hostname(&self) -> String {
        let mut name = String::with_capacity(ONION_HOSTNAME_LEN);
        name.push_str(self.as_str());
        name.push_str(ONION_SUFFIX);
        name
    }
}

impl fmt::Debug for ServiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ServiceId({REDACTED})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IdentitySecretKey;

    /// The example address of the onion service specification, section
    /// "Encoding onion addresses". Its checksum and version were checked
    /// with Python's hashlib, independently of this crate.
    const SPEC_ADDRESS: &str = "pg6mmjiyjmcrsslvykfwnntlaru7p5svn6y2ymmju6nubxndf4pscryd";

    fn key(seed: u8) -> OnionServiceKey {
        let public = IdentitySecretKey::from_seed(&[seed; 32]).public_key();
        OnionServiceKey::from_bytes(public.as_bytes()).unwrap()
    }

    #[test]
    fn a_key_round_trips_through_its_service_id() {
        for seed in [1_u8, 7, 0x42, 0xff] {
            let key = key(seed);
            let id = ServiceId::from_key(&key);
            assert_eq!(id.as_str().len(), SERVICE_ID_LEN);
            assert!(
                id.as_str()
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            );
            assert_eq!(ServiceId::parse(id.as_str().as_bytes()), Ok(key));
            assert_eq!(id.hostname(), format!("{}.onion", id.as_str()));
            assert_eq!(id.hostname().len(), ONION_HOSTNAME_LEN);
        }
    }

    #[test]
    fn the_example_of_the_specification_parses_and_encodes_back() {
        let key = ServiceId::parse(SPEC_ADDRESS.as_bytes()).unwrap();
        assert_eq!(ServiceId::from_key(&key).as_str(), SPEC_ADDRESS);
    }

    #[test]
    fn a_wrong_checksum_version_case_or_length_is_refused() {
        let good = ServiceId::from_key(&key(7));
        let text = good.as_str().as_bytes().to_vec();

        // Every single character changed to another one of the alphabet.
        for index in 0..SERVICE_ID_LEN {
            let mut changed = text.clone();
            changed[index] = if changed[index] == b'a' { b'b' } else { b'a' };
            assert!(ServiceId::parse(&changed).is_err(), "position {index}");
        }
        // Upper case, a suffix, a short or a long id, an empty one.
        assert!(ServiceId::parse(text.to_ascii_uppercase().as_slice()).is_err());
        assert!(ServiceId::parse(good.hostname().as_bytes()).is_err());
        assert!(ServiceId::parse(&text[..55]).is_err());
        let mut long = text.clone();
        long.push(b'a');
        assert!(ServiceId::parse(&long).is_err());
        assert!(ServiceId::parse(b"").is_err());
        // Characters outside the alphabet.
        for bad in [b'0', b'1', b'8', b'=', b'.', b' ', 0xff] {
            let mut changed = text.clone();
            changed[10] = bad;
            assert!(ServiceId::parse(&changed).is_err(), "{bad:#x}");
        }
    }

    #[test]
    fn a_version_other_than_three_is_refused_even_with_a_matching_checksum() {
        let key = key(7);
        for version in [0_u8, 2, 4, 0xff] {
            let digest = Sha3_256::new()
                .chain_update(CHECKSUM_PREFIX)
                .chain_update(key.as_bytes())
                .chain_update([version])
                .finalize();
            let mut decoded = key.as_bytes().to_vec();
            decoded.extend_from_slice(&digest[..2]);
            decoded.push(version);
            let text = base32::encode(&decoded).to_ascii_lowercase();
            assert_eq!(
                ServiceId::parse(text.as_bytes()),
                Err(IdentityError::InvalidOnionAddress),
                "version {version}"
            );
        }
    }

    #[test]
    fn an_address_of_an_invalid_key_is_refused() {
        // The identity element, with a correct checksum and version: the
        // address is well formed, the key is not valid.
        let mut identity_element = [0_u8; 32];
        identity_element[0] = 1;
        let mut decoded = identity_element.to_vec();
        decoded.extend_from_slice(&checksum(&identity_element));
        decoded.push(VERSION);
        let text = base32::encode(&decoded).to_ascii_lowercase();
        assert_eq!(
            ServiceId::parse(text.as_bytes()),
            Err(IdentityError::InvalidKey)
        );
    }

    #[test]
    fn debug_output_shows_no_address() {
        let id = ServiceId::from_key(&key(7));
        assert_eq!(format!("{id:?}"), "ServiceId([redacted])");
    }
}

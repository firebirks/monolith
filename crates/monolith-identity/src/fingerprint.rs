//! Identity fingerprints.

use core::fmt;

use sha2::{Digest, Sha256};

use crate::redact::REDACTED;
use crate::{IdentityPublicKey, base32};

/// Length in bytes of a fingerprint.
pub const FINGERPRINT_LEN: usize = 32;

/// Number of fingerprint bytes shown in the compact form.
const COMPACT_LEN: usize = 15;

/// Characters per group in the textual forms.
const GROUP_LEN: usize = 4;

/// Domain separation prefix of the fingerprint hash.
const PREFIX: &[u8; 23] = b"MONOLITH-FINGERPRINT-V1";

/// Key type byte for Ed25519.
const KEY_TYPE_ED25519: u8 = 0x01;

/// Fingerprint of an identity, as defined in `docs/PROTOCOL.md` section 10:
/// `SHA-256("MONOLITH-FINGERPRINT-V1" || 0x01 || identity_public_key)`.
///
/// This is what users compare out of band.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint([u8; FINGERPRINT_LEN]);

impl Fingerprint {
    /// Computes the fingerprint of an identity.
    pub fn of(identity: &IdentityPublicKey) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(PREFIX);
        hasher.update([KEY_TYPE_ED25519]);
        hasher.update(identity.as_bytes());
        Self(hasher.finalize().into())
    }

    /// Returns the fingerprint bytes.
    pub const fn as_bytes(&self) -> &[u8; FINGERPRINT_LEN] {
        &self.0
    }

    /// Returns the full form: 52 base32 characters in 13 groups of 4,
    /// separated by single spaces.
    pub fn to_full_text(&self) -> String {
        grouped(&base32::encode(&self.0))
    }

    /// Returns the compact form: the first 15 bytes as 24 base32 characters
    /// in 6 groups of 4. It has 120 bits and is for places where space is
    /// short. Verification screens show the full form.
    pub fn to_compact_text(&self) -> String {
        let (head, _) = self.0.split_at(COMPACT_LEN);
        grouped(&base32::encode(head))
    }
}

fn grouped(text: &str) -> String {
    let mut out = String::with_capacity(text.len().saturating_add(text.len() / GROUP_LEN));
    for (index, character) in text.chars().enumerate() {
        if index > 0 && index % GROUP_LEN == 0 {
            out.push(' ');
        }
        out.push(character);
    }
    out
}

impl fmt::Debug for Fingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Fingerprint({REDACTED})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IdentitySecretKey;

    fn identity(seed: u8) -> IdentityPublicKey {
        IdentitySecretKey::from_seed(&[seed; 32]).public_key()
    }

    #[test]
    fn known_answer() {
        // Identity key of the RFC 8032 test vector 1 seed.
        let seed: [u8; 32] = [
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
            0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
            0x1c, 0xae, 0x7f, 0x60,
        ];
        let fingerprint = Fingerprint::of(&IdentitySecretKey::from_seed(&seed).public_key());
        assert_eq!(fingerprint.to_full_text(), KNOWN_FULL);
        assert_eq!(fingerprint.to_compact_text(), KNOWN_COMPACT);
    }

    const KNOWN_FULL: &str = "YIRR UZHO JELC AIYD AIKQ AHCH XAGT EYDA AX3D VLUH 6WO3 ZSYJ BYYQ";
    const KNOWN_COMPACT: &str = "YIRR UZHO JELC AIYD AIKQ AHCH";

    #[test]
    fn full_form_has_thirteen_groups_of_four() {
        let text = Fingerprint::of(&identity(1)).to_full_text();
        let groups: Vec<&str> = text.split(' ').collect();
        assert_eq!(groups.len(), 13);
        assert!(groups.iter().all(|group| group.len() == 4));
        assert_eq!(text.len(), 52 + 12);
    }

    #[test]
    fn compact_form_has_six_groups_of_four() {
        let text = Fingerprint::of(&identity(1)).to_compact_text();
        let groups: Vec<&str> = text.split(' ').collect();
        assert_eq!(groups.len(), 6);
        assert!(groups.iter().all(|group| group.len() == 4));
    }

    #[test]
    fn compact_form_is_a_prefix_of_the_full_form() {
        let fingerprint = Fingerprint::of(&identity(1));
        assert!(
            fingerprint
                .to_full_text()
                .starts_with(&fingerprint.to_compact_text())
        );
    }

    #[test]
    fn different_identities_have_different_fingerprints() {
        assert_ne!(Fingerprint::of(&identity(1)), Fingerprint::of(&identity(2)));
    }

    #[test]
    fn text_uses_only_the_base32_alphabet_and_spaces() {
        let text = Fingerprint::of(&identity(3)).to_full_text();
        assert!(
            text.chars()
                .all(|c| c == ' ' || c.is_ascii_uppercase() || ('2'..='7').contains(&c))
        );
    }

    #[test]
    fn debug_output_shows_no_fingerprint() {
        assert_eq!(
            format!("{:?}", Fingerprint::of(&identity(1))),
            "Fingerprint([redacted])"
        );
    }
}

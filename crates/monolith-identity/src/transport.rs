//! Transport keys.

use core::cmp::Ordering;
use core::fmt;

use ed25519_dalek::VerifyingKey;

use crate::IdentityError;
use crate::redact::REDACTED;

/// Length in bytes of an X25519 public key.
pub const TRANSPORT_PUBLIC_KEY_LEN: usize = 32;

/// The field prime 2^255 - 19, little-endian.
const FIELD_PRIME: [u8; 32] = [
    0xed, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
    0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f,
];

/// The points of small order on Curve25519 and its twist, as canonical
/// u-coordinates: 0, 1, the two points of order 8, and p - 1. They are
/// listed in `docs/PROTOCOL.md` section 10.2. Every other encoding of one
/// of them is not canonical.
const SMALL_ORDER: [[u8; 32]; 5] = [
    [0; 32],
    [
        0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ],
    [
        0xe0, 0xeb, 0x7a, 0x7c, 0x3b, 0x41, 0xb8, 0xae, 0x16, 0x56, 0xe3, 0xfa, 0xf1, 0x9f, 0xc4,
        0x6a, 0xda, 0x09, 0x8d, 0xeb, 0x9c, 0x32, 0xb1, 0xfd, 0x86, 0x62, 0x05, 0x16, 0x5f, 0x49,
        0xb8, 0x00,
    ],
    [
        0x5f, 0x9c, 0x95, 0xbc, 0xa3, 0x50, 0x8c, 0x24, 0xb1, 0xd0, 0xb1, 0x55, 0x9c, 0x83, 0xef,
        0x5b, 0x04, 0x44, 0x5c, 0xc4, 0x58, 0x1c, 0x8e, 0x86, 0xd8, 0x22, 0x4e, 0xdd, 0xd0, 0x9f,
        0x11, 0x57,
    ],
    [
        0xec, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x7f,
    ],
];

/// Returns true if the bytes, read as a little-endian integer, are less
/// than the field prime. The top bit of the last byte is then zero.
fn is_canonical(bytes: &[u8; 32]) -> bool {
    // Compare from the most significant byte down.
    for (byte, limit) in bytes.iter().zip(FIELD_PRIME.iter()).rev() {
        match byte.cmp(limit) {
            Ordering::Less => return true,
            Ordering::Greater => return false,
            Ordering::Equal => {}
        }
    }
    false
}

/// The X25519 public key with which an identity authenticates its sessions.
///
/// A value of this type has passed the checks of `docs/PROTOCOL.md` section
/// 10.2: it is the canonical encoding of a field element and it is not a
/// point of small order. There is no way to construct one from unchecked
/// bytes. The same rule holds for the ephemeral keys of a handshake.
///
/// A transport key is not an identity. A contact card, signed by the
/// identity key, says which transport key belongs to which identity.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransportPublicKey([u8; TRANSPORT_PUBLIC_KEY_LEN]);

impl TransportPublicKey {
    /// Validates 32 bytes as an X25519 public key.
    pub fn from_bytes(bytes: &[u8; TRANSPORT_PUBLIC_KEY_LEN]) -> Result<Self, IdentityError> {
        if !is_canonical(bytes) || SMALL_ORDER.contains(bytes) {
            return Err(IdentityError::InvalidKey);
        }
        Ok(Self(*bytes))
    }

    /// Returns the key bytes.
    pub const fn as_bytes(&self) -> &[u8; TRANSPORT_PUBLIC_KEY_LEN] {
        &self.0
    }

    /// Returns true if this key is the Montgomery form of the Ed25519
    /// public key `edwards`: u = (1 + y) / (1 - y).
    ///
    /// Key separation forbids that coincidence. An identity key or an onion
    /// service key that was carried over to the other curve form must not
    /// serve as a transport key (`docs/PROTOCOL.md` section 11.2). Bytes
    /// that are not a point of the Edwards curve have no Montgomery form
    /// and give false.
    pub fn is_montgomery_form_of(&self, edwards: &[u8; 32]) -> bool {
        VerifyingKey::from_bytes(edwards).is_ok_and(|key| key.to_montgomery().to_bytes() == self.0)
    }
}

impl fmt::Debug for TransportPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TransportPublicKey({REDACTED})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IdentitySecretKey;

    /// The public keys of RFC 7748 section 6.1.
    const RFC7748_ALICE: [u8; 32] = [
        0x85, 0x20, 0xf0, 0x09, 0x89, 0x30, 0xa7, 0x54, 0x74, 0x8b, 0x7d, 0xdc, 0xb4, 0x3e, 0xf7,
        0x5a, 0x0d, 0xbf, 0x3a, 0x0d, 0x26, 0x38, 0x1a, 0xf4, 0xeb, 0xa4, 0xa9, 0x8e, 0xaa, 0x9b,
        0x4e, 0x6a,
    ];
    const RFC7748_BOB: [u8; 32] = [
        0xde, 0x9e, 0xdb, 0x7d, 0x7b, 0x7d, 0xc1, 0xb4, 0xd3, 0x5b, 0x61, 0xc2, 0xec, 0xe4, 0x35,
        0x37, 0x3f, 0x83, 0x43, 0xc8, 0x5b, 0x78, 0x67, 0x4d, 0xad, 0xfc, 0x7e, 0x14, 0x6f, 0x88,
        0x2b, 0x4f,
    ];

    /// Test vector 1 of RFC 8032 section 7.1, and the Montgomery form of
    /// its public key as `docs/PROTOCOL.md` section 16.1 gives it. That
    /// value was computed by an implementation written independently of
    /// this crate.
    const RFC8032_SEED: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];
    const RFC8032_MONTGOMERY: [u8; 32] = [
        0xd8, 0x5e, 0x07, 0xec, 0x22, 0xb0, 0xad, 0x88, 0x15, 0x37, 0xc2, 0xf4, 0x4d, 0x66, 0x2d,
        0x1a, 0x14, 0x3c, 0xf8, 0x30, 0xc5, 0x7a, 0xca, 0x43, 0x05, 0xd8, 0x5c, 0x7a, 0x90, 0xf6,
        0xb6, 0x2e,
    ];

    #[test]
    fn accepts_the_rfc_7748_keys_and_the_base_point() {
        for bytes in [RFC7748_ALICE, RFC7748_BOB] {
            assert_eq!(
                TransportPublicKey::from_bytes(&bytes).unwrap().as_bytes(),
                &bytes
            );
        }
        let mut base_point = [0_u8; 32];
        base_point[0] = 9;
        assert!(TransportPublicKey::from_bytes(&base_point).is_ok());
    }

    #[test]
    fn rejects_every_point_of_small_order() {
        for bytes in SMALL_ORDER {
            assert_eq!(
                TransportPublicKey::from_bytes(&bytes),
                Err(IdentityError::InvalidKey)
            );
        }
        // All five are distinct and canonical, so it is the list that
        // rejects them and not the encoding rule.
        for (index, bytes) in SMALL_ORDER.iter().enumerate() {
            assert!(is_canonical(bytes), "entry {index}");
            assert_eq!(
                SMALL_ORDER.iter().position(|other| other == bytes),
                Some(index)
            );
        }
    }

    #[test]
    fn the_small_order_list_is_the_one_in_the_specification() {
        let hex =
            |bytes: &[u8; 32]| -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() };
        let listed: Vec<String> = SMALL_ORDER.iter().map(hex).collect();
        assert_eq!(
            listed,
            [
                "0000000000000000000000000000000000000000000000000000000000000000",
                "0100000000000000000000000000000000000000000000000000000000000000",
                "e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b800",
                "5f9c95bca3508c24b1d0b1559c83ef5b04445cc4581c8e86d8224eddd09f1157",
                "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
            ]
        );
    }

    #[test]
    fn rejects_values_that_are_not_below_the_field_prime() {
        // p itself, and p + k up to the largest value that fits 255 bits.
        for low in 0xed..=0xff_u8 {
            let mut bytes = [0xff_u8; 32];
            bytes[0] = low;
            bytes[31] = 0x7f;
            assert_eq!(
                TransportPublicKey::from_bytes(&bytes),
                Err(IdentityError::InvalidKey),
                "low byte {low:#x}"
            );
        }
        // p - 2 is the largest canonical value that is not of small order.
        let mut largest = [0xff_u8; 32];
        largest[0] = 0xeb;
        largest[31] = 0x7f;
        assert!(TransportPublicKey::from_bytes(&largest).is_ok());
    }

    #[test]
    fn rejects_every_key_with_the_top_bit_set() {
        // RFC 7748 would mask the bit and accept these as second encodings
        // of valid keys. Monolith does not.
        for mut bytes in [RFC7748_ALICE, RFC7748_BOB, [0; 32], [0xff; 32]] {
            bytes[31] |= 0x80;
            assert_eq!(
                TransportPublicKey::from_bytes(&bytes),
                Err(IdentityError::InvalidKey)
            );
        }
    }

    #[test]
    fn the_comparison_with_the_prime_looks_at_every_byte() {
        // Equal to p in all bytes but one, which is lower: canonical. The
        // same byte higher is only possible in the last byte.
        for index in 0..32 {
            let mut bytes = FIELD_PRIME;
            bytes[index] -= 1;
            assert!(is_canonical(&bytes), "byte {index} lowered");
        }
        assert!(!is_canonical(&FIELD_PRIME));
        let mut above = FIELD_PRIME;
        above[0] += 1;
        assert!(!is_canonical(&above));
        // A higher byte above a lower one below still decides by the
        // more significant byte.
        let mut mixed = FIELD_PRIME;
        mixed[31] = 0x80;
        mixed[0] = 0;
        assert!(!is_canonical(&mixed));
        let mut mixed = [0xff_u8; 32];
        mixed[31] = 0x7e;
        assert!(is_canonical(&mixed));
    }

    #[test]
    fn montgomery_form_matches_the_published_vector() {
        let identity = IdentitySecretKey::from_seed(&RFC8032_SEED).public_key();
        let converted = TransportPublicKey::from_bytes(&RFC8032_MONTGOMERY).unwrap();
        assert!(converted.is_montgomery_form_of(identity.as_bytes()));

        // Any other key is not the Montgomery form of that identity, and
        // the converted key is not the form of any other identity.
        let other = TransportPublicKey::from_bytes(&RFC7748_ALICE).unwrap();
        assert!(!other.is_montgomery_form_of(identity.as_bytes()));
        let stranger = IdentitySecretKey::from_seed(&[7; 32]).public_key();
        assert!(!converted.is_montgomery_form_of(stranger.as_bytes()));
    }

    #[test]
    fn a_point_and_its_negative_have_the_same_montgomery_form() {
        // The sign bit of an Edwards encoding does not enter the
        // u-coordinate, so the check also catches the negated key.
        let identity = IdentitySecretKey::from_seed(&RFC8032_SEED).public_key();
        let mut negated = *identity.as_bytes();
        negated[31] ^= 0x80;
        let converted = TransportPublicKey::from_bytes(&RFC8032_MONTGOMERY).unwrap();
        assert!(converted.is_montgomery_form_of(&negated));
    }

    #[test]
    fn bytes_that_are_not_a_point_have_no_montgomery_form() {
        let mut not_a_point = [0_u8; 32];
        not_a_point[0] = 2;
        let key = TransportPublicKey::from_bytes(&RFC7748_ALICE).unwrap();
        assert!(!key.is_montgomery_form_of(&not_a_point));
    }

    #[test]
    fn debug_output_does_not_contain_key_bytes() {
        let key = TransportPublicKey::from_bytes(&RFC7748_ALICE).unwrap();
        assert_eq!(format!("{key:?}"), "TransportPublicKey([redacted])");
    }
}

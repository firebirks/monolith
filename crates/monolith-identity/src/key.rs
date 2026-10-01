//! Identity keys and signatures.

use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};

use ed25519_dalek::{Signer, SigningKey, VerifyingKey};

use crate::IdentityError;
use crate::redact::REDACTED;

/// Length in bytes of an Ed25519 public key.
pub const IDENTITY_PUBLIC_KEY_LEN: usize = 32;

/// Length in bytes of the seed an identity secret key is built from.
pub const IDENTITY_SEED_LEN: usize = 32;

/// Length in bytes of an Ed25519 signature.
pub const SIGNATURE_LEN: usize = 64;

/// Decodes 32 bytes as an Ed25519 public key under the rules of
/// `docs/PROTOCOL.md` section 10.1. A key is valid only when all three hold:
///
/// 1. the compressed Edwards encoding decompresses to a point;
/// 2. the point is torsion-free;
/// 3. the point is not of small order.
///
/// Neither of the last two is enough alone. The identity element is
/// torsion-free, so only the third condition rejects it. A point with a
/// torsion component that is not itself of small order passes the third,
/// so only the second rejects it. Together they say that the key is a
/// point of the prime-order subgroup other than the identity element,
/// which is what every honestly generated key is.
///
/// This is stricter than Ed25519 verification, which accepts any point that
/// decompresses. Monolith generates its own identity keys and needs no
/// compatibility with keys made elsewhere. The same rule serves the key in
/// an onion address, where the second condition is the test Tor applies.
///
/// A valid key has exactly one encoding. On this curve every non-canonical
/// encoding of a point is a point of small order or one with a torsion
/// component, so the conditions above already reject it. The comparison of
/// the encoding below is kept as a direct statement of that property; it
/// does not change which keys are valid.
pub(crate) fn decode_key(bytes: &[u8; 32]) -> Result<VerifyingKey, IdentityError> {
    let key = VerifyingKey::from_bytes(bytes).map_err(|_| IdentityError::InvalidKey)?;
    let point = key.to_edwards();
    if !point.is_torsion_free() || point.is_small_order() {
        return Err(IdentityError::InvalidKey);
    }
    if point.compress().to_bytes() != *bytes {
        return Err(IdentityError::InvalidKey);
    }
    Ok(key)
}

/// The canonical Monolith identity: a validated Ed25519 public key.
///
/// A value of this type has passed the checks of `docs/PROTOCOL.md` section
/// 10.1. There is no way to construct one from unchecked bytes.
#[derive(Clone, Copy)]
pub struct IdentityPublicKey(VerifyingKey);

impl IdentityPublicKey {
    /// Validates 32 bytes as an identity key.
    pub fn from_bytes(bytes: &[u8; IDENTITY_PUBLIC_KEY_LEN]) -> Result<Self, IdentityError> {
        decode_key(bytes).map(Self)
    }

    /// Returns the key bytes.
    pub fn as_bytes(&self) -> &[u8; IDENTITY_PUBLIC_KEY_LEN] {
        self.0.as_bytes()
    }

    /// Verifies a signature by this identity under strict rules: the scalar
    /// must be canonical and the R component must not be of small order.
    ///
    /// The caller is responsible for domain separation: `message` must be
    /// the signed bytes of one specific structure, prefix included.
    pub fn verify(&self, message: &[u8], signature: &Signature) -> Result<(), IdentityError> {
        let signature = ed25519_dalek::Signature::from_bytes(&signature.0);
        self.0
            .verify_strict(message, &signature)
            .map_err(|_| IdentityError::InvalidSignature)
    }
}

impl PartialEq for IdentityPublicKey {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for IdentityPublicKey {}

impl Hash for IdentityPublicKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

/// Identity keys are ordered by their bytes. The duplicate-session rule of
/// `docs/PROTOCOL.md` section 14 depends on this order.
impl Ord for IdentityPublicKey {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

impl PartialOrd for IdentityPublicKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Debug for IdentityPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IdentityPublicKey({REDACTED})")
    }
}

/// An Ed25519 signature.
///
/// Holding one says nothing about its validity; see
/// [`IdentityPublicKey::verify`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Signature([u8; SIGNATURE_LEN]);

impl Signature {
    /// Wraps signature bytes.
    pub const fn from_bytes(bytes: [u8; SIGNATURE_LEN]) -> Self {
        Self(bytes)
    }

    /// Returns the signature bytes.
    pub const fn as_bytes(&self) -> &[u8; SIGNATURE_LEN] {
        &self.0
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature({REDACTED})")
    }
}

/// The secret half of an identity.
///
/// The key material is erased when the value is dropped. The type has no
/// way to export it and prints nothing about it.
pub struct IdentitySecretKey(SigningKey);

impl IdentitySecretKey {
    /// Builds the secret key from a 32-byte seed.
    ///
    /// The seed must come from a cryptographically secure random source. The
    /// caller keeps responsibility for its own copy of the seed.
    pub fn from_seed(seed: &[u8; IDENTITY_SEED_LEN]) -> Self {
        Self(SigningKey::from_bytes(seed))
    }

    /// Returns the public identity that belongs to this key.
    pub fn public_key(&self) -> IdentityPublicKey {
        IdentityPublicKey(self.0.verifying_key())
    }

    /// Signs a message.
    ///
    /// The caller is responsible for domain separation: `message` must be
    /// the signed bytes of one specific structure, prefix included. Ed25519
    /// signatures are deterministic, so the same key and message always give
    /// the same signature.
    pub fn sign(&self, message: &[u8]) -> Signature {
        Signature(self.0.sign(message).to_bytes())
    }
}

impl fmt::Debug for IdentitySecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IdentitySecretKey({REDACTED})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test vector 1 of RFC 8032 section 7.1.
    const RFC8032_SEED: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];
    const RFC8032_PUBLIC: [u8; 32] = [
        0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07,
        0x3a, 0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07,
        0x51, 0x1a,
    ];
    const RFC8032_SIGNATURE_OF_EMPTY: [u8; 64] = [
        0xe5, 0x56, 0x43, 0x00, 0xc3, 0x60, 0xac, 0x72, 0x90, 0x86, 0xe2, 0xcc, 0x80, 0x6e, 0x82,
        0x8a, 0x84, 0x87, 0x7f, 0x1e, 0xb8, 0xe5, 0xd9, 0x74, 0xd8, 0x73, 0xe0, 0x65, 0x22, 0x49,
        0x01, 0x55, 0x5f, 0xb8, 0x82, 0x15, 0x90, 0xa3, 0x3b, 0xac, 0xc6, 0x1e, 0x39, 0x70, 0x1c,
        0xf9, 0xb4, 0x6b, 0xd2, 0x5b, 0xf5, 0xf0, 0x59, 0x5b, 0xbe, 0x24, 0x65, 0x51, 0x41, 0x43,
        0x8e, 0x7a, 0x10, 0x0b,
    ];

    /// The neutral element: y = 1.
    const IDENTITY_ELEMENT: [u8; 32] = {
        let mut bytes = [0_u8; 32];
        bytes[0] = 1;
        bytes
    };

    /// The point of order 2: y = p - 1.
    const ORDER_TWO: [u8; 32] = {
        let mut bytes = [0xff_u8; 32];
        bytes[0] = 0xec;
        bytes[31] = 0x7f;
        bytes
    };

    /// y = p, a second encoding of y = 0. Not canonical.
    const NON_CANONICAL_ZERO: [u8; 32] = {
        let mut bytes = [0xff_u8; 32];
        bytes[0] = 0xed;
        bytes[31] = 0x7f;
        bytes
    };

    #[test]
    fn matches_the_rfc_8032_test_vector() {
        let secret = IdentitySecretKey::from_seed(&RFC8032_SEED);
        assert_eq!(secret.public_key().as_bytes(), &RFC8032_PUBLIC);
        assert_eq!(secret.sign(b"").as_bytes(), &RFC8032_SIGNATURE_OF_EMPTY);
    }

    #[test]
    fn verifies_its_own_signatures() {
        let secret = IdentitySecretKey::from_seed(&[7; 32]);
        let public = secret.public_key();
        let signature = secret.sign(b"message");
        assert_eq!(public.verify(b"message", &signature), Ok(()));
        assert_eq!(
            public.verify(b"massage", &signature),
            Err(IdentityError::InvalidSignature)
        );
    }

    #[test]
    fn rejects_a_signature_by_another_key() {
        let signature = IdentitySecretKey::from_seed(&[7; 32]).sign(b"message");
        let other = IdentitySecretKey::from_seed(&[8; 32]).public_key();
        assert_eq!(
            other.verify(b"message", &signature),
            Err(IdentityError::InvalidSignature)
        );
    }

    #[test]
    fn rejects_every_single_bit_flip_in_a_signature() {
        let secret = IdentitySecretKey::from_seed(&[7; 32]);
        let public = secret.public_key();
        let good = *secret.sign(b"message").as_bytes();
        for byte in 0..SIGNATURE_LEN {
            for bit in 0..8 {
                let mut bad = good;
                bad[byte] ^= 1 << bit;
                assert!(
                    public
                        .verify(b"message", &Signature::from_bytes(bad))
                        .is_err(),
                    "byte {byte} bit {bit}"
                );
            }
        }
    }

    #[test]
    fn public_key_bytes_round_trip() {
        let public = IdentitySecretKey::from_seed(&[7; 32]).public_key();
        let again = IdentityPublicKey::from_bytes(public.as_bytes()).unwrap();
        assert_eq!(public, again);
    }

    /// The eight points of small order, in canonical encoding: orders 1, 2,
    /// 4, 4, 8, 8, 8, 8.
    const SMALL_ORDER: [[u8; 32]; 8] = [
        IDENTITY_ELEMENT,
        ORDER_TWO,
        [0; 32],
        {
            let mut bytes = [0_u8; 32];
            bytes[31] = 0x80;
            bytes
        },
        [
            0x26, 0xe8, 0x95, 0x8f, 0xc2, 0xb2, 0x27, 0xb0, 0x45, 0xc3, 0xf4, 0x89, 0xf2, 0xef,
            0x98, 0xf0, 0xd5, 0xdf, 0xac, 0x05, 0xd3, 0xc6, 0x33, 0x39, 0xb1, 0x38, 0x02, 0x88,
            0x6d, 0x53, 0xfc, 0x05,
        ],
        [
            0x26, 0xe8, 0x95, 0x8f, 0xc2, 0xb2, 0x27, 0xb0, 0x45, 0xc3, 0xf4, 0x89, 0xf2, 0xef,
            0x98, 0xf0, 0xd5, 0xdf, 0xac, 0x05, 0xd3, 0xc6, 0x33, 0x39, 0xb1, 0x38, 0x02, 0x88,
            0x6d, 0x53, 0xfc, 0x85,
        ],
        [
            0xc7, 0x17, 0x6a, 0x70, 0x3d, 0x4d, 0xd8, 0x4f, 0xba, 0x3c, 0x0b, 0x76, 0x0d, 0x10,
            0x67, 0x0f, 0x2a, 0x20, 0x53, 0xfa, 0x2c, 0x39, 0xcc, 0xc6, 0x4e, 0xc7, 0xfd, 0x77,
            0x92, 0xac, 0x03, 0x7a,
        ],
        [
            0xc7, 0x17, 0x6a, 0x70, 0x3d, 0x4d, 0xd8, 0x4f, 0xba, 0x3c, 0x0b, 0x76, 0x0d, 0x10,
            0x67, 0x0f, 0x2a, 0x20, 0x53, 0xfa, 0x2c, 0x39, 0xcc, 0xc6, 0x4e, 0xc7, 0xfd, 0x77,
            0x92, 0xac, 0x03, 0xfa,
        ],
    ];

    /// y = 2. No point of the curve has this y coordinate.
    const NOT_A_POINT: [u8; 32] = {
        let mut bytes = [0_u8; 32];
        bytes[0] = 2;
        bytes
    };

    /// Decompresses bytes with the curve library alone, without the checks
    /// of this crate.
    fn decompress(bytes: &[u8; 32]) -> Option<VerifyingKey> {
        VerifyingKey::from_bytes(bytes).ok()
    }

    #[test]
    fn key_validity_malformed_compressed_point() {
        // Decompression itself fails, so there is no point to examine.
        assert!(decompress(&NOT_A_POINT).is_none());
        assert_eq!(
            IdentityPublicKey::from_bytes(&NOT_A_POINT).err(),
            Some(IdentityError::InvalidKey)
        );
    }

    #[test]
    fn key_validity_small_order_point() {
        for bytes in SMALL_ORDER {
            let decoded = decompress(&bytes).unwrap().to_edwards();
            assert!(decoded.is_small_order(), "the constant is what it claims");
            assert_eq!(
                IdentityPublicKey::from_bytes(&bytes).err(),
                Some(IdentityError::InvalidKey)
            );
        }
        // All eight are distinct.
        for (index, bytes) in SMALL_ORDER.iter().enumerate() {
            assert_eq!(
                SMALL_ORDER.iter().position(|other| other == bytes),
                Some(index)
            );
        }
    }

    #[test]
    fn key_validity_identity_point() {
        // The identity element is in the prime-order subgroup, so the
        // torsion test alone lets it through. The small-order test is what
        // rejects it.
        let decoded = decompress(&IDENTITY_ELEMENT).unwrap().to_edwards();
        assert!(decoded.is_torsion_free());
        assert!(decoded.is_small_order());
        assert_eq!(
            IdentityPublicKey::from_bytes(&IDENTITY_ELEMENT).err(),
            Some(IdentityError::InvalidKey)
        );
    }

    #[test]
    fn key_validity_point_with_a_torsion_component() {
        // An honest key plus each point of small order other than the
        // identity element. None of the sums is of small order, so the
        // small-order test alone lets them through. The torsion test is
        // what rejects them.
        let honest = decompress(
            IdentitySecretKey::from_seed(&[7; 32])
                .public_key()
                .as_bytes(),
        )
        .unwrap()
        .to_edwards();
        for torsion in SMALL_ORDER.iter().skip(1) {
            let mixed = honest + decompress(torsion).unwrap().to_edwards();
            assert!(!mixed.is_small_order());
            assert!(!mixed.is_torsion_free());
            let bytes = mixed.compress().to_bytes();
            assert_eq!(
                IdentityPublicKey::from_bytes(&bytes).err(),
                Some(IdentityError::InvalidKey)
            );
        }
    }

    #[test]
    fn key_validity_generated_point() {
        for seed in [[0_u8; 32], [7; 32], [0xff; 32], RFC8032_SEED] {
            let public = IdentitySecretKey::from_seed(&seed).public_key();
            let decoded = decompress(public.as_bytes()).unwrap().to_edwards();
            assert!(decoded.is_torsion_free());
            assert!(!decoded.is_small_order());
            assert_eq!(decoded.compress().to_bytes(), *public.as_bytes());
            assert_eq!(IdentityPublicKey::from_bytes(public.as_bytes()), Ok(public));
        }
    }

    #[test]
    fn rejects_non_canonical_encodings() {
        assert_eq!(
            IdentityPublicKey::from_bytes(&NON_CANONICAL_ZERO).err(),
            Some(IdentityError::InvalidKey)
        );
        // Every encoding with y >= p, with either sign bit. On this curve
        // each of them is also a point of small order or with a torsion
        // component, or no point at all, so more than one check rejects
        // them. None may be accepted.
        for k in 0..=18_u8 {
            for top in [0x7f_u8, 0xff] {
                let mut bytes = [0xff_u8; 32];
                bytes[0] = 0xed + k;
                bytes[31] = top;
                assert_eq!(
                    IdentityPublicKey::from_bytes(&bytes).err(),
                    Some(IdentityError::InvalidKey),
                    "y = p + {k}, top byte {top:#x}"
                );
            }
        }
        // x = 0 with the sign bit set: a second encoding of the neutral
        // element.
        let mut negative_zero = IDENTITY_ELEMENT;
        negative_zero[31] = 0x80;
        assert!(IdentityPublicKey::from_bytes(&negative_zero).is_err());
    }

    #[test]
    fn verification_is_strict() {
        // A signature whose R component is the neutral element. Plain
        // Ed25519 verification accepts it for this key and message; strict
        // verification must not.
        const KEY: [u8; 32] = [
            0xea, 0x4a, 0x6c, 0x63, 0xe2, 0x9c, 0x52, 0x0a, 0xbe, 0xf5, 0x50, 0x7b, 0x13, 0x2e,
            0xc5, 0xf9, 0x95, 0x47, 0x76, 0xae, 0xbe, 0xbe, 0x7b, 0x92, 0x42, 0x1e, 0xea, 0x69,
            0x14, 0x46, 0xd2, 0x2c,
        ];
        const SIGNATURE: [u8; 64] = [
            0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0xd4, 0xc5, 0x32, 0x31, 0x31, 0xff, 0x96, 0x83, 0xcf, 0xb3,
            0xd3, 0x01, 0x97, 0xc7, 0x86, 0xc9, 0x04, 0x3f, 0x10, 0x52, 0xd6, 0xc2, 0xd5, 0x29,
            0x3f, 0x19, 0xd6, 0xf3, 0x6b, 0x91, 0x11, 0x04,
        ];
        let public = IdentitySecretKey::from_seed(&[7; 32]).public_key();
        assert_eq!(public.as_bytes(), &KEY);

        use ed25519_dalek::Verifier;
        let lenient = ed25519_dalek::VerifyingKey::from_bytes(&KEY).unwrap();
        assert!(
            lenient
                .verify(
                    b"message",
                    &ed25519_dalek::Signature::from_bytes(&SIGNATURE)
                )
                .is_ok(),
            "the vector is only meaningful if plain verification accepts it"
        );
        assert_eq!(
            public.verify(b"message", &Signature::from_bytes(SIGNATURE)),
            Err(IdentityError::InvalidSignature)
        );
    }

    #[test]
    fn rejects_bytes_that_are_not_a_point() {
        // Roughly half of all 32-byte strings do not decode to a point.
        let mut rejected = 0;
        for i in 0..64_u8 {
            let mut bytes = [0_u8; 32];
            bytes[0] = i;
            bytes[1] = 0x55;
            if IdentityPublicKey::from_bytes(&bytes).is_err() {
                rejected += 1;
            }
        }
        assert!(rejected > 8, "{rejected}");
    }

    #[test]
    fn keys_are_ordered_by_bytes() {
        let a = IdentitySecretKey::from_seed(&[1; 32]).public_key();
        let b = IdentitySecretKey::from_seed(&[2; 32]).public_key();
        assert_eq!(a.cmp(&b), a.as_bytes().cmp(b.as_bytes()));
        assert_eq!(a.cmp(&a), Ordering::Equal);
    }

    #[test]
    fn debug_output_shows_no_key_material() {
        let secret = IdentitySecretKey::from_seed(&[7; 32]);
        assert_eq!(format!("{secret:?}"), "IdentitySecretKey([redacted])");
        assert_eq!(
            format!("{:?}", secret.public_key()),
            "IdentityPublicKey([redacted])"
        );
        assert_eq!(format!("{:?}", secret.sign(b"x")), "Signature([redacted])");
    }
}

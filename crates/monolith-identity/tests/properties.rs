//! Property tests of the identity crate.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use monolith_identity::{
    Fingerprint, IdentityPublicKey, IdentitySecretKey, OnionServiceKey, Signature, base32,
};
use proptest::collection::vec;
use proptest::prelude::*;

/// The definition of a valid key in `docs/PROTOCOL.md` section 10.1, stated
/// with the curve operations directly: a point, canonically encoded, not of
/// small order, without a torsion component.
fn is_valid_key(bytes: &[u8; 32]) -> bool {
    let Ok(key) = ed25519_dalek::VerifyingKey::from_bytes(bytes) else {
        return false;
    };
    let point = key.to_edwards();
    point.compress().to_bytes() == *bytes && !point.is_small_order() && point.is_torsion_free()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn base32_round_trips(data in vec(any::<u8>(), 0..200)) {
        let text = base32::encode(&data);
        prop_assert_eq!(text.len(), base32::encoded_len(data.len()).unwrap());
        prop_assert_eq!(base32::decode(&text, data.len()).unwrap(), data.clone());
        prop_assert_eq!(base32::decode(&text.to_ascii_lowercase(), data.len()).unwrap(), data);
    }

    #[test]
    fn accepted_base32_is_canonical(text in "[A-Za-z2-7]{0,64}") {
        // Whatever decodes must encode back to the same text, in upper case.
        // Text with non-zero trailing bits or an impossible length does not
        // decode at all.
        if let Ok(bytes) = base32::decode(&text, 64) {
            prop_assert_eq!(base32::encode(&bytes), text.to_ascii_uppercase());
        }
    }

    #[test]
    fn base32_decoder_respects_its_limit(text in "[A-Z2-7]{0,64}", limit in 0..40_usize) {
        if let Ok(bytes) = base32::decode(&text, limit) {
            prop_assert!(bytes.len() <= limit);
        }
    }

    #[test]
    fn arbitrary_text_does_not_panic_the_base32_decoder(text in "\\PC{0,80}") {
        let _ = base32::decode(&text, 64);
    }

    #[test]
    fn generated_keys_are_valid_for_both_purposes(seed in any::<[u8; 32]>()) {
        let public = IdentitySecretKey::from_seed(&seed).public_key();
        prop_assert_eq!(IdentityPublicKey::from_bytes(public.as_bytes()).unwrap(), public);
        prop_assert!(OnionServiceKey::from_bytes(public.as_bytes()).is_ok());
    }

    #[test]
    fn key_validity_follows_the_definition(
        arbitrary in any::<[u8; 32]>(),
        seed in any::<[u8; 32]>(),
        bit in 0..256_usize,
    ) {
        // Two kinds of input: arbitrary bytes, about half of which decode
        // to a point and one in eight of those to a point in the prime-order
        // subgroup, and a valid key with one bit flipped, which stays close
        // to the valid encodings. Both key types accept a string exactly
        // when the definition of PROTOCOL.md section 10.1 holds for it.
        let mut flipped = *IdentitySecretKey::from_seed(&seed).public_key().as_bytes();
        flipped[bit / 8] ^= 1 << (bit % 8);
        for bytes in [arbitrary, flipped] {
            let expected = is_valid_key(&bytes);
            prop_assert_eq!(IdentityPublicKey::from_bytes(&bytes).is_ok(), expected);
            prop_assert_eq!(OnionServiceKey::from_bytes(&bytes).is_ok(), expected);
        }
    }

    #[test]
    fn signatures_verify_only_for_the_signed_message(
        seed in any::<[u8; 32]>(),
        message in vec(any::<u8>(), 0..200),
        other in vec(any::<u8>(), 0..200),
    ) {
        let secret = IdentitySecretKey::from_seed(&seed);
        let public = secret.public_key();
        let signature = secret.sign(&message);
        prop_assert!(public.verify(&message, &signature).is_ok());
        if other != message {
            prop_assert!(public.verify(&other, &signature).is_err());
        }
    }

    #[test]
    fn arbitrary_signatures_do_not_verify(
        seed in any::<[u8; 32]>(),
        first in any::<[u8; 32]>(),
        second in any::<[u8; 32]>(),
        message in vec(any::<u8>(), 0..64),
    ) {
        let public = IdentitySecretKey::from_seed(&seed).public_key();
        let mut bytes = [0_u8; 64];
        bytes[..32].copy_from_slice(&first);
        bytes[32..].copy_from_slice(&second);
        prop_assert!(public.verify(&message, &Signature::from_bytes(bytes)).is_err());
    }

    #[test]
    fn fingerprints_differ_between_identities(a in any::<[u8; 32]>(), b in any::<[u8; 32]>()) {
        let first = IdentitySecretKey::from_seed(&a).public_key();
        let second = IdentitySecretKey::from_seed(&b).public_key();
        prop_assume!(first != second);
        prop_assert_ne!(Fingerprint::of(&first), Fingerprint::of(&second));
        prop_assert_eq!(Fingerprint::of(&first).to_full_text().len(), 64);
        prop_assert_eq!(Fingerprint::of(&first).to_compact_text().len(), 29);
    }
}

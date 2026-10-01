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
    Fingerprint, IdentityPublicKey, IdentitySecretKey, OnionServiceKey, Signature,
    TransportPublicKey, base32,
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

/// The definition of a valid X25519 key in `docs/PROTOCOL.md` section 10.2,
/// stated with integers: the value is below 2^255 - 19 and is none of the
/// five points of small order.
fn is_valid_transport_key(bytes: &[u8; 32]) -> bool {
    let low = u128::from_le_bytes(bytes[..16].try_into().unwrap());
    let high = u128::from_le_bytes(bytes[16..].try_into().unwrap());
    let prime_high = (1_u128 << 127) - 1;
    let prime_low = u128::MAX - 18;
    let canonical = high < prime_high || (high == prime_high && low < prime_low);
    let small_order = [
        (0, 0),
        (0, 1),
        (
            0x00b8_495f_1605_6286_fdb1_329c_eb8d_09da,
            0x6ac4_9ff1_fae3_5616_aeb8_413b_7c7a_ebe0,
        ),
        (
            0x5711_9fd0_dd4e_22d8_868e_1c58_c45c_4404,
            0x5bef_839c_55b1_d0b1_248c_50a3_bc95_9c5f,
        ),
        (prime_high, prime_low - 1),
    ];
    canonical && !small_order.contains(&(high, low))
}

fn unhex(text: &str) -> [u8; 32] {
    let bytes: Vec<u8> = (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
        .collect();
    bytes.try_into().unwrap()
}

/// Ed25519 public keys and their Montgomery form u = (1 + y) / (1 - y),
/// computed with integer arithmetic outside this crate. They are
/// the keys of `docs/PROTOCOL.md` section 16.1.
const MONTGOMERY_FORMS: [(&str, &str); 4] = [
    (
        "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
        "d85e07ec22b0ad881537c2f44d662d1a143cf830c57aca4305d85c7a90f6b62e",
    ),
    (
        "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c",
        "25c704c594b88afc00a76b69d1ed2b984d7e22550f3ed0802d04fbcd07d38d47",
    ),
    (
        "8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394",
        "60346e7c911a5f6ba154129174cafe75b294ac3bbd5549632f48cec6266f8410",
    ),
    (
        "ed4928c628d1c2c6eae90338905995612959273a5c63f93636c14614ac8737d1",
        "75e270df2952c57ba8367ba8618c178f9fe50db2799d304e74e918d985686146",
    ),
];

#[test]
fn montgomery_forms_match_the_independent_values() {
    for (index, (edwards, montgomery)) in MONTGOMERY_FORMS.iter().enumerate() {
        let edwards = unhex(edwards);
        let transport = TransportPublicKey::from_bytes(&unhex(montgomery)).unwrap();
        assert!(transport.is_montgomery_form_of(&edwards), "pair {index}");
        // And of no other key in the table.
        for (other, (other_edwards, _)) in MONTGOMERY_FORMS.iter().enumerate() {
            assert_eq!(
                transport.is_montgomery_form_of(&unhex(other_edwards)),
                other == index,
                "pair {index} against key {other}"
            );
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn transport_key_validity_follows_the_definition(
        arbitrary in any::<[u8; 32]>(),
        low in any::<u8>(),
        top in any::<u8>(),
        small in 0..5_usize,
        bit in 0..256_usize,
    ) {
        // Arbitrary bytes are almost always below the prime, so the
        // boundary gets inputs of its own: values near 2^255 - 19, and each
        // point of small order with one bit flipped.
        let mut near_prime = [0xff_u8; 32];
        near_prime[0] = low;
        near_prime[31] = top;
        let points = [
            "0000000000000000000000000000000000000000000000000000000000000000",
            "0100000000000000000000000000000000000000000000000000000000000000",
            "e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b800",
            "5f9c95bca3508c24b1d0b1559c83ef5b04445cc4581c8e86d8224eddd09f1157",
            "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
        ];
        let exact = unhex(points[small]);
        let mut flipped = exact;
        flipped[bit / 8] ^= 1 << (bit % 8);
        prop_assert!(!is_valid_transport_key(&exact));
        for bytes in [arbitrary, near_prime, exact, flipped] {
            let accepted = TransportPublicKey::from_bytes(&bytes);
            prop_assert_eq!(accepted.is_ok(), is_valid_transport_key(&bytes));
            if let Ok(key) = accepted {
                prop_assert_eq!(key.as_bytes(), &bytes);
            }
        }
    }

    #[test]
    fn a_transport_key_is_the_montgomery_form_of_at_most_the_converted_key(
        seed in any::<[u8; 32]>(),
        other in any::<[u8; 32]>(),
    ) {
        let edwards = ed25519_dalek::VerifyingKey::from_bytes(
            IdentitySecretKey::from_seed(&seed).public_key().as_bytes(),
        )
        .unwrap();
        let converted = edwards.to_montgomery().to_bytes();
        let key = TransportPublicKey::from_bytes(&converted).unwrap();
        prop_assert!(key.is_montgomery_form_of(edwards.as_bytes()));
        let stranger = IdentitySecretKey::from_seed(&other).public_key();
        prop_assume!(stranger.as_bytes() != edwards.as_bytes());
        prop_assert!(!key.is_montgomery_form_of(stranger.as_bytes()));
        // Arbitrary bytes are a point about half of the time, and never
        // the one that converts to this key.
        prop_assert!(!key.is_montgomery_form_of(&other) || other == *edwards.as_bytes());
    }

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

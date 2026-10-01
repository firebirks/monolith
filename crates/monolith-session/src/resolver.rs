//! The cryptographic primitives behind the Noise library.
//!
//! `snow` runs the Noise state machine and takes its primitives through
//! four traits. This module fills them for the one suite Monolith uses,
//! `25519_ChaChaPoly_SHA256`, from `x25519-dalek`, `chacha20poly1305`,
//! `sha2` and the random source of the operating system. Why this is done
//! here and not with the resolver that comes with `snow` is recorded in
//! `docs/adr/0002-session-protocol.md`, F-R1.
//!
//! There is no cryptographic construction in this file. Every function
//! hands its arguments to one library call. Two things are decided here:
//!
//! - Which public keys reach a Diffie-Hellman operation. A key that is not
//!   canonical or is of small order is refused, and so is an all-zero
//!   result (`docs/PROTOCOL.md` sections 4.2 and 10.2, rule F5). This is
//!   the only place where the rule is applied, and every key of a
//!   handshake passes through it: the ephemeral keys of messages 1 and 2,
//!   the responder's transport key from the pinned card, and the
//!   initiator's transport key from message 3.
//! - What is kept in memory. A private key lives in a type that erases it
//!   on drop, and so do the cipher keys.
//!
//! Some of the traits have no way to report a failure. Where a function
//! cannot do its job it leaves no usable output, so that the handshake or
//! the frame fails at the peer or at the next step; nothing here panics.

use chacha20poly1305::{AeadInOut, ChaCha20Poly1305, KeyInit};
use monolith_identity::TransportPublicKey;
use sha2::{Digest, Sha256};
use snow::params::{CipherChoice, DHChoice, HashChoice};
use snow::resolvers::CryptoResolver;
use snow::types::{Cipher, Dh, Hash, Random};
use x25519_dalek::{PublicKey, SharedSecret, StaticSecret};
use zeroize::Zeroizing;

/// Length of an X25519 key and of an X25519 result.
const DH_LEN: usize = 32;

/// Length of a ChaCha20-Poly1305 tag.
const TAG_LEN: usize = 16;

/// Length of a SHA-256 output.
const HASH_LEN: usize = 32;

/// Block length of SHA-256.
const HASH_BLOCK_LEN: usize = 64;

/// Where the ephemeral key of a handshake comes from.
#[derive(Clone, Copy)]
enum Entropy {
    /// The random source of the operating system. The only source in a
    /// build that is not a test or fuzz build.
    OperatingSystem,
    /// Fixed bytes, for known-answer tests and for fuzz targets, which
    /// need a handshake that runs the same way twice.
    #[cfg(any(test, fuzzing))]
    Fixed([u8; DH_LEN]),
}

/// The resolver that is handed to `snow` for one handshake.
#[derive(Clone, Copy)]
pub(crate) struct Resolver {
    entropy: Entropy,
}

impl Resolver {
    /// A resolver that takes randomness from the operating system.
    pub(crate) const fn new() -> Self {
        Self {
            entropy: Entropy::OperatingSystem,
        }
    }

    /// A resolver whose random source returns the given bytes, so that the
    /// ephemeral key of the handshake is fixed. It does not exist outside
    /// test and fuzz builds.
    #[cfg(any(test, fuzzing))]
    pub(crate) const fn with_fixed_ephemeral(secret: [u8; DH_LEN]) -> Self {
        Self {
            entropy: Entropy::Fixed(secret),
        }
    }
}

impl CryptoResolver for Resolver {
    fn resolve_rng(&self) -> Option<Box<dyn Random>> {
        Some(match self.entropy {
            Entropy::OperatingSystem => Box::new(OsRandom),
            #[cfg(any(test, fuzzing))]
            Entropy::Fixed(bytes) => Box::new(FixedRandom(bytes)),
        })
    }

    fn resolve_dh(&self, choice: &DHChoice) -> Option<Box<dyn Dh>> {
        matches!(choice, DHChoice::Curve25519).then(|| Box::new(X25519::empty()) as Box<dyn Dh>)
    }

    fn resolve_hash(&self, choice: &HashChoice) -> Option<Box<dyn Hash>> {
        matches!(choice, HashChoice::SHA256)
            .then(|| Box::new(Sha256Hash(Sha256::new())) as Box<dyn Hash>)
    }

    fn resolve_cipher(&self, choice: &CipherChoice) -> Option<Box<dyn Cipher>> {
        matches!(choice, CipherChoice::ChaChaPoly)
            .then(|| Box::new(ChaChaPoly { cipher: None }) as Box<dyn Cipher>)
    }
}

/// The random source of the operating system.
struct OsRandom;

impl Random for OsRandom {
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), snow::Error> {
        getrandom::fill(dest).map_err(|_| snow::Error::Rng)
    }
}

/// A source that repeats fixed bytes. Test and fuzz builds only.
#[cfg(any(test, fuzzing))]
struct FixedRandom([u8; DH_LEN]);

#[cfg(any(test, fuzzing))]
impl Random for FixedRandom {
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), snow::Error> {
        for (slot, byte) in dest.iter_mut().zip(self.0.iter().cycle()) {
            *slot = *byte;
        }
        Ok(())
    }
}

/// One X25519 key pair of a handshake: the transport key or the ephemeral
/// key of the local side.
struct X25519 {
    /// Erased when the handshake state that owns this object is dropped.
    secret: Option<StaticSecret>,
    public: [u8; DH_LEN],
}

impl X25519 {
    const fn empty() -> Self {
        Self {
            secret: None,
            public: [0; DH_LEN],
        }
    }

    fn install(&mut self, bytes: &[u8; DH_LEN]) {
        let secret = StaticSecret::from(*bytes);
        self.public = PublicKey::from(&secret).to_bytes();
        self.secret = Some(secret);
    }
}

impl Dh for X25519 {
    fn name(&self) -> &'static str {
        "25519"
    }

    fn pub_len(&self) -> usize {
        DH_LEN
    }

    fn priv_len(&self) -> usize {
        DH_LEN
    }

    fn set(&mut self, privkey: &[u8]) {
        // The trait cannot refuse a key. A slice of the wrong length
        // leaves the object without one, and every operation that needs it
        // fails afterwards.
        self.secret = None;
        self.public = [0; DH_LEN];
        if let Ok(bytes) = <&[u8; DH_LEN]>::try_from(privkey) {
            self.install(bytes);
        }
    }

    fn generate(&mut self, rng: &mut dyn Random) -> Result<(), snow::Error> {
        self.secret = None;
        self.public = [0; DH_LEN];
        let mut bytes = Zeroizing::new([0_u8; DH_LEN]);
        rng.try_fill_bytes(bytes.as_mut_slice())?;
        self.install(&bytes);
        Ok(())
    }

    fn pubkey(&self) -> &[u8] {
        &self.public
    }

    fn privkey(&self) -> &[u8] {
        match &self.secret {
            Some(secret) => secret.as_bytes(),
            None => &[],
        }
    }

    fn dh(&self, pubkey: &[u8], out: &mut [u8]) -> Result<(), snow::Error> {
        let secret = self.secret.as_ref().ok_or(snow::Error::Dh)?;
        // `snow` passes its whole key buffer, which is longer than a key.
        // The key is at the front.
        let their: &[u8; DH_LEN] = pubkey.first_chunk().ok_or(snow::Error::Dh)?;
        let shared = diffie_hellman(secret, their).ok_or(snow::Error::Dh)?;
        out.first_chunk_mut::<DH_LEN>()
            .ok_or(snow::Error::Dh)?
            .copy_from_slice(shared.as_bytes());
        Ok(())
    }
}

/// X25519 between a local private key and a public key that came from
/// outside, under rule F5: the public key must be a valid key in the sense
/// of `docs/PROTOCOL.md` section 10.2, and the result must not be all
/// zeros.
fn diffie_hellman(secret: &StaticSecret, their: &[u8; DH_LEN]) -> Option<SharedSecret> {
    let their = TransportPublicKey::from_bytes(their).ok()?;
    contributory(secret, their.as_bytes())
}

/// X25519, refusing an all-zero result. With a valid public key the result
/// is never zero; this is the second half of rule F5 and does not rely on
/// the first.
fn contributory(secret: &StaticSecret, their: &[u8; DH_LEN]) -> Option<SharedSecret> {
    let shared = secret.diffie_hellman(&PublicKey::from(*their));
    shared.was_contributory().then_some(shared)
}

/// The nonce of a Noise message for ChaCha20-Poly1305: 32 zero bits and
/// the 64-bit counter in little-endian order.
fn nonce(counter: u64) -> [u8; 12] {
    let mut bytes = [0_u8; 12];
    for (slot, byte) in bytes.iter_mut().skip(4).zip(counter.to_le_bytes()) {
        *slot = byte;
    }
    bytes
}

/// One direction of a Noise cipher state.
struct ChaChaPoly {
    /// The cipher with its key. Erased on drop.
    cipher: Option<ChaCha20Poly1305>,
}

impl Cipher for ChaChaPoly {
    fn name(&self) -> &'static str {
        "ChaChaPoly"
    }

    fn set(&mut self, key: &[u8; 32]) {
        self.cipher = Some(ChaCha20Poly1305::new(key.into()));
    }

    fn encrypt(&self, counter: u64, authtext: &[u8], plaintext: &[u8], out: &mut [u8]) -> usize {
        // The trait cannot report a failure. Without room for the result
        // nothing is written. If the cipher refuses, the output is zeros,
        // so that no plaintext stays in the buffer; the peer rejects such
        // a message.
        let Some(target) = plaintext
            .len()
            .checked_add(TAG_LEN)
            .and_then(|total| out.get_mut(..total))
        else {
            return 0;
        };
        let total = target.len();
        let (body, tag) = target.split_at_mut(plaintext.len());
        body.copy_from_slice(plaintext);
        let sealed = self.cipher.as_ref().and_then(|cipher| {
            cipher
                .encrypt_inout_detached(&nonce(counter).into(), authtext, (&mut *body).into())
                .ok()
        });
        match sealed {
            Some(computed) => tag.copy_from_slice(&computed),
            None => {
                body.fill(0);
                tag.fill(0);
            }
        }
        total
    }

    fn decrypt(
        &self,
        counter: u64,
        authtext: &[u8],
        ciphertext: &[u8],
        out: &mut [u8],
    ) -> Result<usize, snow::Error> {
        let (body, tag) = ciphertext
            .split_last_chunk::<TAG_LEN>()
            .ok_or(snow::Error::Decrypt)?;
        let target = out.get_mut(..body.len()).ok_or(snow::Error::Decrypt)?;
        let cipher = self.cipher.as_ref().ok_or(snow::Error::Decrypt)?;
        target.copy_from_slice(body);
        let opened = cipher.decrypt_inout_detached(
            &nonce(counter).into(),
            authtext,
            (&mut *target).into(),
            &(*tag).into(),
        );
        if opened.is_err() {
            // Nothing of a message that failed authentication is left
            // where the caller could read it.
            target.fill(0);
            return Err(snow::Error::Decrypt);
        }
        Ok(body.len())
    }
}

/// SHA-256 for the handshake hash and, through the HMAC and HKDF that
/// `snow` builds from it, for the key schedule.
struct Sha256Hash(Sha256);

impl Hash for Sha256Hash {
    fn name(&self) -> &'static str {
        "SHA256"
    }

    fn block_len(&self) -> usize {
        HASH_BLOCK_LEN
    }

    fn hash_len(&self) -> usize {
        HASH_LEN
    }

    fn reset(&mut self) {
        self.0 = Sha256::new();
    }

    fn input(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    fn result(&mut self, out: &mut [u8]) {
        let digest = self.0.finalize_reset();
        for (slot, byte) in out.iter_mut().zip(digest.iter()) {
            *slot = *byte;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{hex, unhex};

    const NAME: &str = "Noise_XK_25519_ChaChaPoly_SHA256";

    /// RFC 7748 section 6.1.
    const ALICE_SECRET: &str = "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a";
    const ALICE_PUBLIC: &str = "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a";
    const BOB_SECRET: &str = "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb";
    const BOB_PUBLIC: &str = "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f";
    const SHARED: &str = "4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742";

    /// The points of small order, as `docs/PROTOCOL.md` section 10.2 lists
    /// them.
    const SMALL_ORDER: [&str; 5] = [
        "0000000000000000000000000000000000000000000000000000000000000000",
        "0100000000000000000000000000000000000000000000000000000000000000",
        "e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b800",
        "5f9c95bca3508c24b1d0b1559c83ef5b04445cc4581c8e86d8224eddd09f1157",
        "ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f",
    ];

    fn key32(text: &str) -> [u8; 32] {
        unhex(text).try_into().unwrap()
    }

    fn dh_with(secret: &str) -> X25519 {
        let mut dh = X25519::empty();
        dh.set(&unhex(secret));
        dh
    }

    #[test]
    fn the_resolver_serves_the_one_suite_and_nothing_else() {
        let resolver = Resolver::new();
        let params: snow::params::NoiseParams = NAME.parse().unwrap();
        let dh = resolver.resolve_dh(&params.dh).unwrap();
        assert_eq!((dh.name(), dh.pub_len(), dh.priv_len()), ("25519", 32, 32));
        let cipher = resolver.resolve_cipher(&params.cipher).unwrap();
        assert_eq!(cipher.name(), "ChaChaPoly");
        let hash = resolver.resolve_hash(&params.hash).unwrap();
        assert_eq!(
            (hash.name(), hash.block_len(), hash.hash_len()),
            ("SHA256", 64, 32)
        );
        assert!(resolver.resolve_rng().is_some());

        // The names snow builds from the three objects are the protocol
        // name of the specification.
        assert_eq!(
            format!("Noise_XK_{}_{}_{}", dh.name(), cipher.name(), hash.name()),
            NAME
        );

        assert!(resolver.resolve_dh(&DHChoice::Curve448).is_none());
        assert!(resolver.resolve_cipher(&CipherChoice::AESGCM).is_none());
        for other in [HashChoice::SHA512, HashChoice::Blake2s, HashChoice::Blake2b] {
            assert!(resolver.resolve_hash(&other).is_none());
        }
        // A handshake of another suite cannot be built on this resolver.
        for other in [
            "Noise_XK_25519_AESGCM_SHA256",
            "Noise_XK_25519_ChaChaPoly_BLAKE2s",
            "Noise_XK_448_ChaChaPoly_SHA256",
        ] {
            let params: snow::params::NoiseParams = other.parse().unwrap();
            let built = snow::Builder::with_resolver(params, Box::new(Resolver::new()))
                .local_private_key(&[1; 32])
                .unwrap()
                .remote_public_key(&key32(ALICE_PUBLIC))
                .unwrap()
                .build_initiator();
            assert!(built.is_err(), "{other}");
        }
    }

    #[test]
    fn x25519_matches_rfc_7748() {
        let alice = dh_with(ALICE_SECRET);
        let bob = dh_with(BOB_SECRET);
        assert_eq!(hex(alice.pubkey()), ALICE_PUBLIC);
        assert_eq!(hex(bob.pubkey()), BOB_PUBLIC);
        assert_eq!(hex(alice.privkey()), ALICE_SECRET);

        let mut out = [0_u8; 32];
        alice.dh(&unhex(BOB_PUBLIC), &mut out).unwrap();
        assert_eq!(hex(&out), SHARED);
        let mut out = [0_u8; 32];
        bob.dh(&unhex(ALICE_PUBLIC), &mut out).unwrap();
        assert_eq!(hex(&out), SHARED);
    }

    #[test]
    fn dh_takes_the_key_from_the_front_of_a_longer_buffer() {
        // snow hands over its 56-byte key buffer.
        let alice = dh_with(ALICE_SECRET);
        let mut buffer = unhex(BOB_PUBLIC);
        buffer.extend_from_slice(&[0xAA; 24]);
        let mut out = [0_u8; 56];
        alice.dh(&buffer, &mut out).unwrap();
        assert_eq!(hex(&out[..32]), SHARED);
        assert_eq!(&out[32..], &[0; 24]);
    }

    #[test]
    fn dh_refuses_every_point_of_small_order() {
        let alice = dh_with(ALICE_SECRET);
        for point in SMALL_ORDER {
            let mut out = [0x55_u8; 32];
            assert_eq!(
                alice.dh(&unhex(point), &mut out),
                Err(snow::Error::Dh),
                "{point}"
            );
            assert_eq!(out, [0x55; 32], "nothing is written for a refused key");
        }
    }

    #[test]
    fn dh_refuses_keys_that_are_not_canonical() {
        let alice = dh_with(ALICE_SECRET);
        let mut out = [0_u8; 32];

        // A valid key with the top bit set. RFC 7748 would mask the bit.
        let mut top_bit = key32(BOB_PUBLIC);
        top_bit[31] |= 0x80;
        assert_eq!(alice.dh(&top_bit, &mut out), Err(snow::Error::Dh));

        // Values from the field prime upwards. Reduced, the first two are
        // the small-order points 0 and 1, the third is the valid key 2.
        for low in [0xed_u8, 0xee, 0xef] {
            let mut bytes = [0xff_u8; 32];
            bytes[0] = low;
            bytes[31] = 0x7f;
            assert_eq!(alice.dh(&bytes, &mut out), Err(snow::Error::Dh), "{low:#x}");
        }
        // The same small-order points with the top bit set.
        for point in SMALL_ORDER {
            let mut bytes = key32(point);
            bytes[31] |= 0x80;
            assert_eq!(alice.dh(&bytes, &mut out), Err(snow::Error::Dh), "{point}");
        }
        assert_eq!(out, [0; 32]);
    }

    #[test]
    fn an_all_zero_result_is_refused_on_its_own() {
        // The second half of rule F5, without the key check in front of
        // it: X25519 with a point of small order gives zero, whatever the
        // private key is, and that result is not handed out.
        let secret = StaticSecret::from(key32(ALICE_SECRET));
        for point in SMALL_ORDER {
            let raw = secret.diffie_hellman(&PublicKey::from(key32(point)));
            assert_eq!(raw.as_bytes(), &[0; 32], "{point}");
            assert!(contributory(&secret, &key32(point)).is_none(), "{point}");
        }
        let good = contributory(&secret, &key32(BOB_PUBLIC)).unwrap();
        assert_eq!(hex(good.as_bytes()), SHARED);
    }

    #[test]
    fn dh_fails_without_a_key_and_with_short_buffers() {
        let empty = X25519::empty();
        let mut out = [0_u8; 32];
        assert_eq!(empty.dh(&unhex(BOB_PUBLIC), &mut out), Err(snow::Error::Dh));
        assert_eq!(empty.privkey(), &[] as &[u8]);
        assert_eq!(empty.pubkey(), &[0; 32]);

        // A private key of the wrong length is not installed, and removes
        // a key that was there.
        for len in [0, 31, 33, 64] {
            let mut dh = dh_with(ALICE_SECRET);
            dh.set(&vec![7; len]);
            assert_eq!(
                dh.dh(&unhex(BOB_PUBLIC), &mut out),
                Err(snow::Error::Dh),
                "{len}"
            );
            assert_eq!(dh.pubkey(), &[0; 32]);
        }

        let alice = dh_with(ALICE_SECRET);
        assert_eq!(
            alice.dh(&unhex(BOB_PUBLIC)[..31], &mut out),
            Err(snow::Error::Dh)
        );
        assert_eq!(
            alice.dh(&unhex(BOB_PUBLIC), &mut out[..31]),
            Err(snow::Error::Dh)
        );
    }

    #[test]
    fn generated_keys_come_from_the_random_source() {
        let mut fixed = FixedRandom(key32(ALICE_SECRET));
        let mut dh = X25519::empty();
        dh.generate(&mut fixed).unwrap();
        assert_eq!(hex(dh.pubkey()), ALICE_PUBLIC);

        // From the operating system: two keys differ, and each has a
        // valid public key.
        let mut first = X25519::empty();
        let mut second = X25519::empty();
        first.generate(&mut OsRandom).unwrap();
        second.generate(&mut OsRandom).unwrap();
        assert_ne!(first.pubkey(), second.pubkey());
        assert_ne!(first.privkey(), second.privkey());
        for dh in [&first, &second] {
            assert!(TransportPublicKey::from_bytes(dh.pubkey().try_into().unwrap()).is_ok());
        }

        // A source that fails leaves no key behind.
        struct Broken;
        impl Random for Broken {
            fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), snow::Error> {
                Err(snow::Error::Rng)
            }
        }
        let mut dh = dh_with(ALICE_SECRET);
        assert_eq!(dh.generate(&mut Broken), Err(snow::Error::Rng));
        assert_eq!(dh.privkey(), &[] as &[u8]);
        assert_eq!(dh.pubkey(), &[0; 32]);
    }

    #[test]
    fn the_operating_system_source_fills_the_whole_buffer() {
        let mut source = OsRandom;
        let mut first = [0_u8; 64];
        let mut second = [0_u8; 64];
        source.try_fill_bytes(&mut first).unwrap();
        source.try_fill_bytes(&mut second).unwrap();
        assert_ne!(first, second);
        assert_ne!(first[..32], [0; 32]);
        assert_ne!(first[32..], [0; 32]);
    }

    #[test]
    fn the_fixed_source_repeats_its_bytes() {
        let mut source = FixedRandom(key32(ALICE_SECRET));
        let mut out = [0_u8; 40];
        source.try_fill_bytes(&mut out).unwrap();
        assert_eq!(hex(&out[..32]), ALICE_SECRET);
        assert_eq!(out[32..], out[..8]);
    }

    #[test]
    fn the_nonce_is_four_zero_bytes_and_the_counter_in_little_endian() {
        assert_eq!(nonce(0), [0; 12]);
        assert_eq!(nonce(1), [0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            nonce(0x0102_0304_0506_0708),
            [0, 0, 0, 0, 8, 7, 6, 5, 4, 3, 2, 1]
        );
        assert_eq!(
            nonce(u64::MAX),
            [0, 0, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255]
        );
    }

    fn cipher_with_test_key() -> ChaChaPoly {
        let mut cipher = ChaChaPoly { cipher: None };
        let key: [u8; 32] = core::array::from_fn(|index| u8::try_from(index).unwrap());
        cipher.set(&key);
        cipher
    }

    #[test]
    fn the_cipher_matches_the_independent_vectors() {
        // Key 00 01 .. 1f. Computed by an implementation of RFC 8439
        // written independently of this crate, with the Noise nonce.
        let cipher = cipher_with_test_key();

        let mut out = [0_u8; 64];
        let len = cipher.encrypt(5, b"header", b"Monolith frame cipher test", &mut out);
        assert_eq!(len, 42);
        assert_eq!(
            hex(&out[..len]),
            "6e67f865294c3356abcc4b2948d010c0f74eeb208678756d0741cdf3cf50099cf1c0f4f49bbdbb82eda0"
        );
        assert_eq!(&out[len..], &[0; 22], "nothing is written past the tag");

        let mut out = [0_u8; 16];
        assert_eq!(cipher.encrypt(0, b"", b"", &mut out), 16);
        assert_eq!(hex(&out), "10324f800a160bd9a1794255be7ec29d");

        // A counter above 32 bits: all eight bytes of it enter the nonce.
        let mut out = [0_u8; 80];
        assert_eq!(cipher.encrypt((1 << 32) + 7, b"", &[0; 64], &mut out), 80);
        assert_eq!(
            hex(&out),
            concat!(
                "dc9ff0e83538476ce89fba0ba5a6fd24140adda69385cb250362f4f232169d8d",
                "f8d0133a15a90b5bfcd426f31900cede33456c97c14ac2d82ece75e98d13d193",
                "407612f92a4e06ce6076ed1ef2a852fd",
            )
        );
    }

    #[test]
    fn the_cipher_opens_what_it_sealed_and_nothing_else() {
        let cipher = cipher_with_test_key();
        let plaintext = b"Monolith frame cipher test";
        let mut sealed = [0_u8; 42];
        assert_eq!(cipher.encrypt(5, b"header", plaintext, &mut sealed), 42);

        let mut out = [0_u8; 26];
        assert_eq!(cipher.decrypt(5, b"header", &sealed, &mut out), Ok(26));
        assert_eq!(&out, plaintext);

        // Another counter, other associated data, another key.
        let mut out = [0x55_u8; 26];
        assert_eq!(
            cipher.decrypt(6, b"header", &sealed, &mut out),
            Err(snow::Error::Decrypt)
        );
        assert_eq!(out, [0; 26], "a rejected message leaves nothing readable");
        assert_eq!(
            cipher.decrypt(5, b"headex", &sealed, &mut out),
            Err(snow::Error::Decrypt)
        );
        let mut other = ChaChaPoly { cipher: None };
        other.set(&[9; 32]);
        assert_eq!(
            other.decrypt(5, b"header", &sealed, &mut out),
            Err(snow::Error::Decrypt)
        );

        // Every single bit of the ciphertext and of the tag.
        for index in 0..sealed.len() {
            for bit in 0..8 {
                let mut damaged = sealed;
                damaged[index] ^= 1 << bit;
                assert_eq!(
                    cipher.decrypt(5, b"header", &damaged, &mut out),
                    Err(snow::Error::Decrypt),
                    "byte {index} bit {bit}"
                );
            }
        }
    }

    #[test]
    fn the_cipher_handles_short_buffers_without_output() {
        let cipher = cipher_with_test_key();
        // No room for the tag: nothing is written.
        let mut out = [0x55_u8; 30];
        assert_eq!(cipher.encrypt(0, b"", &[1; 16], &mut out), 0);
        assert_eq!(out, [0x55; 30]);

        // Shorter than a tag, and an output that cannot hold the plaintext.
        let mut out = [0_u8; 26];
        assert_eq!(
            cipher.decrypt(0, b"", &[0; 15], &mut out),
            Err(snow::Error::Decrypt)
        );
        let mut sealed = [0_u8; 42];
        cipher.encrypt(5, b"", &[2; 26], &mut sealed);
        assert_eq!(
            cipher.decrypt(5, b"", &sealed, &mut out[..25]),
            Err(snow::Error::Decrypt)
        );
        assert_eq!(cipher.decrypt(5, b"", &sealed, &mut out), Ok(26));
    }

    #[test]
    fn a_cipher_without_a_key_produces_no_ciphertext_and_reads_none() {
        let unkeyed = ChaChaPoly { cipher: None };
        let mut out = [0x55_u8; 32];
        // The output is zeros, not the plaintext.
        assert_eq!(unkeyed.encrypt(0, b"", &[7; 16], &mut out), 32);
        assert_eq!(out, [0; 32]);
        let mut plain = [0_u8; 16];
        assert_eq!(
            unkeyed.decrypt(0, b"", &out, &mut plain),
            Err(snow::Error::Decrypt)
        );
    }

    #[test]
    fn sha256_matches_the_standard_vectors() {
        let mut hash = Sha256Hash(Sha256::new());
        let mut out = [0_u8; 32];
        hash.input(b"abc");
        hash.result(&mut out);
        assert_eq!(
            hex(&out),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // The state is fresh after a result, and after a reset.
        hash.result(&mut out);
        assert_eq!(
            hex(&out),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        hash.input(b"discarded");
        hash.reset();
        hash.input(b"a");
        hash.input(b"bc");
        hash.result(&mut out);
        assert_eq!(
            hex(&out),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // snow passes a 64-byte buffer; only the digest is written.
        let mut wide = [0xAA_u8; 64];
        hash.input(b"abc");
        hash.result(&mut wide);
        assert_eq!(&wide[..32], &out);
        assert_eq!(&wide[32..], &[0xAA; 32]);
    }

    #[test]
    fn the_hmac_snow_builds_from_the_hash_matches_rfc_4231() {
        // Test case 1 and test case 2 of RFC 4231. The Noise key schedule
        // is this HMAC, so the block and output lengths reported above
        // have to be right.
        let mut hash = Sha256Hash(Sha256::new());
        let mut out = [0_u8; 32];
        hash.hmac(&[0x0b; 20], b"Hi There", &mut out);
        assert_eq!(
            hex(&out),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        hash.hmac(b"Jefe", b"what do ya want for nothing?", &mut out);
        assert_eq!(
            hex(&out),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }
}

//! Identity and endpoint naming types.
//!
//! A Monolith identity is an Ed25519 public key. It is distinct from the
//! Onion Service keys that name the endpoints where the identity can be
//! reached. See `docs/adr/0001-identity-model.md` and `docs/PROTOCOL.md`
//! sections 10 and 11.
//!
//! This crate validates keys, signs and verifies, and derives fingerprints.
//! It does not generate keys: a secret key is built from 32 bytes that the
//! caller supplies. Key generation arrives with the randomness source in a
//! later phase.

pub mod base32;
pub mod redact;

mod endpoint;
mod error;
mod fingerprint;
mod key;

pub use endpoint::{EndpointEpoch, ONION_SERVICE_KEY_LEN, OnionServiceKey};
pub use error::IdentityError;
pub use fingerprint::{FINGERPRINT_LEN, Fingerprint};
pub use key::{
    IDENTITY_PUBLIC_KEY_LEN, IDENTITY_SEED_LEN, IdentityPublicKey, IdentitySecretKey,
    SIGNATURE_LEN, Signature,
};

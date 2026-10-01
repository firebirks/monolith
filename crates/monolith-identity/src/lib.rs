//! Identity and endpoint naming types.
//!
//! A Monolith identity is an Ed25519 public key. It is distinct from the
//! Onion Service key that names the endpoint where the identity can currently
//! be reached. See `docs/adr/0001-identity-model.md`.
//!
//! This crate holds only type definitions for now. Key generation, signing,
//! verification and address validation arrive in Phase 1 and must follow
//! `docs/CRYPTOGRAPHY.md` and `docs/PROTOCOL.md`.

pub mod redact;

mod endpoint;
mod key;

pub use endpoint::{EndpointEpoch, ONION_SERVICE_KEY_LEN, OnionServiceKey};
pub use key::{FINGERPRINT_LEN, Fingerprint, IDENTITY_PUBLIC_KEY_LEN, IdentityPublicKey};

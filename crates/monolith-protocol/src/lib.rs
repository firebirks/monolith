//! Wire protocol types, limits and session state.
//!
//! The protocol is specified in `docs/PROTOCOL.md`, which defines the
//! intended behavior; code in this crate follows it. A disagreement between
//! the two is a bug to resolve, not a choice either side wins silently.
//!
//! This crate is the protocol core. It does no I/O and no session
//! cryptography: it encodes and decodes, validates, and decides. Every
//! decoder takes bytes and returns a value or a [`ProtocolError`]; none of
//! them allocates from a length a peer declared before checking it, and
//! none of them panics on any input.
//!
//! What is here:
//!
//! - [`limits`]: every size, count, rate and timeout.
//! - [`codec`]: bounded readers and writers for the fixed-layout encoding.
//! - [`text`]: validated text fields.
//! - [`card`]: contact cards, their signed bytes and their text form.
//! - [`credential`]: which transport key stands for a contact, and how a
//!   successor takes over.
//! - [`body`]: message bodies.
//! - [`frame`]: the padded frame format and the bounded outer framing.
//! - [`session`]: session states, the standing of a peer, contact
//!   confirmation.
//! - [`duplicate`]: the duplicate-session rule.
//!
//! What is not here: the handshake and frame encryption. They are in the
//! session crate, which drives [`session::Session`] with what the handshake
//! established. See `docs/adr/0002-session-protocol.md`.

// Tests build their own inputs and do arithmetic and indexing on them.
#![cfg_attr(test, allow(clippy::arithmetic_side_effects))]

pub mod body;
pub mod card;
pub mod codec;
pub mod credential;
pub mod duplicate;
pub mod frame;
pub mod limits;
pub mod session;
pub mod text;

mod error;
mod message;
mod state;

pub use error::ProtocolError;
pub use message::MessageType;
pub use state::SessionState;

//! Authenticated encrypted peer sessions.
//!
//! This crate is the session layer of Monolith: the handshake that
//! authenticates two peers to each other, and the encrypted frames that
//! follow it. It is specified in `docs/PROTOCOL.md` sections 3 to 7 and
//! `docs/CRYPTOGRAPHY.md`; the decision behind it is
//! `docs/adr/0002-session-protocol.md`. If code and documents disagree,
//! the documents win and the code is wrong.
//!
//! The handshake is `Noise_XK_25519_ChaChaPoly_SHA256`, run by the `snow`
//! library. Every call to that library is in this crate, and nothing in
//! this crate implements a step of Noise, a key derivation or a cipher.
//! What Monolith adds is the binding between the keys Noise authenticates
//! and Monolith identities, through the contact card.
//!
//! How it is used:
//!
//! - The local side is a [`LocalParty`]: its contact card and the secret
//!   half of its transport key. [`LocalParty::issue`] signs the card from
//!   the local identity key; a card from outside cannot become the local
//!   one.
//! - To dial a contact: [`HandshakeInitiator::start`] with the pinned card
//!   of that contact, then [`HandshakeInitiator::read_message_2`], then
//!   [`OutboundPeer::admit`].
//! - To answer a stream: [`HandshakeResponder::new`], then
//!   [`HandshakeResponder::read_message_1`], then
//!   [`HandshakeResponderFinal::read_message_3`], then
//!   [`InboundPeer::admit`] with what the local side holds about the
//!   identity that was authenticated.
//! - Both ends then hold an [`AuthenticatedSession`], which encrypts
//!   messages into frames and decrypts frames into messages.
//!
//! Properties that are built into the shape of the types:
//!
//! - No frame can be sent or received before the handshake is complete and
//!   the peer is authenticated: only [`AuthenticatedSession`] has functions
//!   for frames, and only a completed handshake produces one.
//! - A session that was dialed for one identity can never be established
//!   with another. There is no option to continue with an unknown peer.
//! - Authentication and authorization are separate. The handshake says
//!   who the peer is, for every peer with a valid card. What the peer may
//!   do follows from the local record of that identity, afterwards.
//! - Nothing does I/O and nothing reads a clock. Bytes and time are
//!   arguments.
//! - Secrets are not printed. Every type here has a `Debug` output that
//!   shows no key, no identity and no message content.
//!
//! Randomness comes from the operating system. A handshake with fixed
//! ephemeral keys exists only in test builds and in fuzz builds of this
//! crate; no feature of a production build enables it.

// Tests build their own inputs and do arithmetic and indexing on them.
#![cfg_attr(test, allow(clippy::arithmetic_side_effects))]

mod error;
mod handshake;
mod key;
mod resolver;
mod session;

#[cfg(test)]
mod testing;
#[cfg(test)]
mod tests;

pub use error::SessionError;
pub use handshake::{
    HandshakeInitiator, HandshakeResponder, HandshakeResponderFinal, InboundPeer, MessageBuffer,
    OutboundPeer,
};
pub use key::{LocalParty, TRANSPORT_SECRET_KEY_LEN, TransportSecretKey};
pub use session::{AuthenticatedSession, Received};

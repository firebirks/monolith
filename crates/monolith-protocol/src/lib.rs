//! Wire protocol types, limits and session state.
//!
//! The protocol is specified in `docs/PROTOCOL.md`. Code in this crate must
//! follow that document; if the two disagree, the document wins and the code
//! is wrong.
//!
//! Phase 0 contains the limits table, the message type registry and the
//! session state gate. Framing, encoding, decoding and the cryptographic
//! session are not implemented yet.

pub mod limits;

mod error;
mod message;
mod state;

pub use error::ProtocolError;
pub use message::MessageType;
pub use state::SessionState;

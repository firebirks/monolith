//! Desktop front end.
//!
//! Placeholder. No toolkit has been chosen; the candidates and the criteria
//! are in `docs/adr/0006-gui-toolkit.md`. Work here starts in Phase 8, after
//! the protocol core is stable.
//!
//! Whatever toolkit is chosen, this crate is an adapter: it turns user input
//! into `monolith-core` commands and draws `monolith-core` events. It holds
//! no protocol state and makes no network or storage calls of its own.

pub use monolith_core::{IdentityLifetime, PeerState, TorState, TrustLevel};

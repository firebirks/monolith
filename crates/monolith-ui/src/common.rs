//! What several screens share: how the state of Tor is said, and the seed
//! of an avatar.

use monolith_ui_kit::tor_status::TorState;
use monolith_ui_model::{Model, TorIndicator};

/// The state of Tor for the top bar, and its words.
pub(crate) fn tor_state(model: &Model) -> (TorState, &'static str) {
    match model.tor_indicator() {
        TorIndicator::Starting => (TorState::Starting, "Starting"),
        TorIndicator::Connecting => (TorState::Connecting, "Connecting to Tor"),
        TorIndicator::Connected => (TorState::Connected, "Connected through Tor"),
        TorIndicator::Unavailable => (TorState::Unavailable, "Tor unavailable"),
    }
}

/// The seed of the avatar of someone: their fingerprint, which stays the
/// same for the same person in every record.
pub(crate) fn seed(fingerprint: &str) -> &[u8] {
    fingerprint.as_bytes()
}

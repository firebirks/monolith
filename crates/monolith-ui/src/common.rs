//! What several screens share: how the state of Tor is said, and the seed
//! of an avatar.

use monolith_ui_kit::tor_status::TorState;
use monolith_ui_model::{Model, TorIndicator};

/// The state of Tor for a top bar, and its words. Each names Tor, so that
/// next to a contact's name it is not read as whether they are online.
pub(crate) fn tor_state(model: &Model) -> (TorState, &'static str) {
    match model.tor_indicator() {
        TorIndicator::Starting => (TorState::Starting, "Checking Tor"),
        TorIndicator::Connecting => (TorState::Connecting, "Connecting to Tor"),
        TorIndicator::Connected => (TorState::Connected, "Tor connected"),
        TorIndicator::Unavailable => (TorState::Unavailable, "Tor unavailable"),
    }
}

/// The seed of the avatar of someone: their short fingerprint, which stays
/// the same for the same person in every record and on every screen.
pub(crate) fn seed(fingerprint: &str) -> &[u8] {
    fingerprint.as_bytes()
}

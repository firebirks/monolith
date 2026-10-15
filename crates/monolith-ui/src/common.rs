//! What several screens share: how the state of Tor is said, what a
//! contact that has not accepted waits for, and the seed of an avatar.

use monolith_ui_kit::tor_status::TorState;
use monolith_ui_model::{Model, TorCheck, TorIndicator, Waiting};

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

/// What the last check of Tor found, and what to do about it: the same
/// words in the Tor details and on the Tor page of the settings.
pub(crate) fn tor_words(check: Option<&TorCheck>) -> String {
    match check {
        None => "Tor has not been checked yet.".to_owned(),
        Some(TorCheck::Ready { version, .. }) => format!("Tor {version} is running."),
        Some(TorCheck::Connecting {
            progress: Some(percent),
        }) => format!("Tor is starting up ({percent}%)."),
        Some(TorCheck::Connecting { progress: None }) => "Tor is starting up.".to_owned(),
        Some(TorCheck::NotRunning) => "Tor is not running, or not at the endpoints of the \
                                       connection settings. Open Tor Browser and leave it \
                                       open, or start the Tor service, then press Check again."
            .to_owned(),
        Some(TorCheck::SocksUnavailable) => "Monolith cannot use the SOCKS port of Tor. Check \
                                             the SOCKS endpoint in the connection settings."
            .to_owned(),
        Some(TorCheck::ControlUnavailable) => "Monolith cannot reach the control port of Tor. \
                                               Check the control endpoint in the connection \
                                               settings."
            .to_owned(),
        Some(TorCheck::AuthenticationFailed) => "Tor refused Monolith access to its control \
                                                 port. Check the cookie file in the \
                                                 connection settings."
            .to_owned(),
        Some(TorCheck::TooOld { version }) => {
            format!("Tor {version} is too old. Install a newer version of Tor.")
        }
    }
}

/// What is said of a Tor that runs but is older than recommended.
pub(crate) fn tor_outdated(check: Option<&TorCheck>) -> Option<String> {
    match check {
        Some(TorCheck::Ready {
            version,
            outdated: true,
        }) => Some(format!(
            "Tor {version} is older than recommended. Update it when you can."
        )),
        _ => None,
    }
}

/// What a contact that has not accepted waits for, in a few words: the
/// same under its name, in the list and in its details.
pub(crate) fn waiting_words(waiting: Waiting) -> &'static str {
    match waiting {
        Waiting::FirstMessage => "Request not sent yet",
        Waiting::Acceptance => "Waiting for them to accept your request",
        Waiting::TheirCard => "Waiting for them to add your card",
    }
}

/// The seed of the avatar of someone: their short fingerprint, which stays
/// the same for the same person in every record and on every screen.
pub(crate) fn seed(fingerprint: &str) -> &[u8] {
    fingerprint.as_bytes()
}

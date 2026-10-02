//! Application core.
//!
//! The core owns contacts, sessions, queues and policy. Front ends send it
//! commands and receive typed events through a bounded queue; they hold no
//! protocol logic of their own. The core reaches the network only through a
//! `monolith_tor::TorBackend` and reaches the disk only through
//! `monolith-storage`. See `docs/ARCHITECTURE.md`.
//!
//! Phase 0 defines the states a front end must be able to tell apart. The
//! command and event types follow with the features that need them.

pub mod budget;
pub mod link;

/// State of the Tor connection as shown to the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TorState {
    /// Tor cannot be reached. Nothing is sent or received.
    Disconnected,
    /// Tor is reachable and bootstrapping.
    Connecting,
    /// Tor is ready; the Onion Service is being published.
    PublishingService,
    /// The Onion Service is published and peers can connect.
    ServiceAvailable,
}

/// State of one peer as shown to the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PeerState {
    /// No session and no connection attempt in progress.
    Offline,
    /// A transport stream is being opened through Tor.
    Connecting,
    /// The cryptographic handshake and identity proof are in progress.
    Handshaking,
    /// The peer proved its identity; the contact relationship is not
    /// confirmed on this session.
    AuthenticatedUnknown,
    /// A confirmed session with an accepted contact.
    Online,
    /// The peer presented an identity that differs from the pinned one. The
    /// session was refused and the user must be told.
    IdentityMismatch,
}

/// How far the user has gone in checking a contact's identity.
///
/// This is local information. It is never sent to the peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrustLevel {
    /// The identity key was pinned from a contact card or a request and has
    /// not been compared out of band.
    Pinned,
    /// The user compared the fingerprint out of band and marked it verified.
    Verified,
}

/// Whether the local identity outlives the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IdentityLifetime {
    /// The identity and its contacts disappear when the process exits.
    Ephemeral,
    /// The identity is kept in the encrypted vault.
    Persistent,
}

impl From<monolith_storage::StateMode> for IdentityLifetime {
    fn from(mode: monolith_storage::StateMode) -> Self {
        match mode {
            monolith_storage::StateMode::Ephemeral => Self::Ephemeral,
            monolith_storage::StateMode::Persistent => Self::Persistent,
        }
    }
}

/// Maps a session state to what the user is shown for that peer.
pub const fn peer_state_for(session: monolith_protocol::SessionState) -> PeerState {
    use monolith_protocol::SessionState;

    match session {
        SessionState::Connecting => PeerState::Connecting,
        SessionState::CryptoHandshake | SessionState::IdentityAuth => PeerState::Handshaking,
        SessionState::AuthenticatedUnknown => PeerState::AuthenticatedUnknown,
        SessionState::AuthenticatedContact => PeerState::Online,
        SessionState::Closing | SessionState::Closed => PeerState::Offline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use monolith_protocol::SessionState;

    #[test]
    fn a_peer_is_online_only_in_an_authenticated_contact_session() {
        let states = [
            SessionState::Connecting,
            SessionState::CryptoHandshake,
            SessionState::IdentityAuth,
            SessionState::AuthenticatedUnknown,
            SessionState::AuthenticatedContact,
            SessionState::Closing,
            SessionState::Closed,
        ];
        for state in states {
            let online = peer_state_for(state) == PeerState::Online;
            assert_eq!(online, state == SessionState::AuthenticatedContact);
        }
    }
}

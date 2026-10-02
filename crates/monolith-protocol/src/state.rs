//! Session states.

/// State of one peer session, as defined in `docs/PROTOCOL.md`.
///
/// A session only moves forward and passes through every state up to
/// `AuthenticatedUnknown` in order. Application messages are accepted in
/// `AuthenticatedContact` and nowhere else; [`crate::MessageType`] encodes
/// which message is legal in which state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SessionState {
    /// The transport stream is being established.
    Connecting,
    /// The three handshake messages are being exchanged.
    CryptoHandshake,
    /// The handshake is complete and the peer's card passed its checks.
    /// The identity is being bound to the session and the standing of the
    /// peer applied. No message is exchanged in this state.
    IdentityAuth,
    /// The peer's identity is authenticated. Whether the two sides are
    /// contacts has not been confirmed on this session.
    AuthenticatedUnknown,
    /// Both sides hold each other as accepted contacts and have said so on
    /// this session.
    AuthenticatedContact,
    /// A Close message was sent; the stream is being shut down. A Close
    /// that is received leads straight to `Closed`.
    Closing,
    /// The session is over. No further input is processed.
    Closed,
}

impl SessionState {
    /// Returns true if the session may move from `self` to `next`.
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Connecting, Self::CryptoHandshake | Self::Closed)
                | (Self::CryptoHandshake, Self::IdentityAuth | Self::Closed)
                | (
                    Self::IdentityAuth,
                    Self::AuthenticatedUnknown | Self::Closed
                )
                | (
                    Self::AuthenticatedUnknown,
                    Self::AuthenticatedContact | Self::Closing | Self::Closed
                )
                | (Self::AuthenticatedContact, Self::Closing | Self::Closed)
                | (Self::Closing, Self::Closed)
        )
    }

    /// Returns true if the peer's identity is authenticated in this state.
    pub const fn is_authenticated(self) -> bool {
        matches!(
            self,
            Self::AuthenticatedUnknown | Self::AuthenticatedContact
        )
    }
}

#[cfg(test)]
mod tests {
    use super::SessionState::{
        self, AuthenticatedContact, AuthenticatedUnknown, Closed, Closing, Connecting,
        CryptoHandshake, IdentityAuth,
    };

    const ALL: [SessionState; 7] = [
        Connecting,
        CryptoHandshake,
        IdentityAuth,
        AuthenticatedUnknown,
        AuthenticatedContact,
        Closing,
        Closed,
    ];

    fn rank(state: SessionState) -> u8 {
        match state {
            Connecting => 0,
            CryptoHandshake => 1,
            IdentityAuth => 2,
            AuthenticatedUnknown => 3,
            AuthenticatedContact => 4,
            Closing => 5,
            Closed => 6,
        }
    }

    #[test]
    fn sessions_never_move_backwards_or_stay_in_place() {
        for from in ALL {
            for to in ALL {
                if from.can_transition_to(to) {
                    assert!(rank(to) > rank(from), "{from:?} -> {to:?}");
                }
            }
        }
    }

    #[test]
    fn closed_is_terminal() {
        for to in ALL {
            assert!(!Closed.can_transition_to(to));
        }
    }

    #[test]
    fn every_live_state_can_reach_closed() {
        for from in ALL {
            if from != Closed {
                assert!(from.can_transition_to(Closed), "{from:?}");
            }
        }
    }

    #[test]
    fn authentication_cannot_be_skipped() {
        assert!(!Connecting.can_transition_to(IdentityAuth));
        assert!(!Connecting.can_transition_to(AuthenticatedContact));
        assert!(!CryptoHandshake.can_transition_to(AuthenticatedUnknown));
        assert!(!CryptoHandshake.can_transition_to(AuthenticatedContact));
        assert!(!IdentityAuth.can_transition_to(AuthenticatedContact));
    }

    #[test]
    fn contact_state_is_reached_only_through_confirmation() {
        for from in ALL {
            let expected = from == AuthenticatedUnknown;
            assert_eq!(
                from.can_transition_to(AuthenticatedContact),
                expected,
                "{from:?}"
            );
        }
    }

    #[test]
    fn only_the_two_authenticated_states_count_as_authenticated() {
        for state in ALL {
            let expected = matches!(state, AuthenticatedUnknown | AuthenticatedContact);
            assert_eq!(state.is_authenticated(), expected, "{state:?}");
        }
    }
}

//! Message type registry.

use crate::SessionState;

/// Message types of protocol version 1.
///
/// The numeric codes are part of the wire format; see `docs/PROTOCOL.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MessageType {
    /// Binds a Monolith identity to the encrypted session.
    AuthProof,
    /// Announces that the sender is closing the session.
    Close,
    /// Keepalive probe.
    Ping,
    /// Answer to a Ping.
    Pong,
    /// Request to become a contact.
    ContactRequest,
    /// Tells the peer that the sender holds it as an accepted contact. Sent
    /// at the start of every session between contacts.
    ContactAccept,
    /// Chat text.
    ChatMessage,
    /// Confirms that a chat message was received and validated.
    MessageAck,
    /// Display name and profile text of the sender.
    Profile,
    /// New signed endpoint binding of the sender.
    EndpointUpdate,
    /// Offer to send a file.
    FileOffer,
    /// Accepts a file offer.
    FileAccept,
    /// Declines a file offer.
    FileReject,
    /// One piece of file data.
    FileChunk,
    /// Ends a transfer and carries the digest of the file.
    FileComplete,
    /// Aborts a transfer from either side.
    FileAbort,
}

impl MessageType {
    /// Every message type, in code order.
    pub const ALL: [Self; 16] = [
        Self::AuthProof,
        Self::Close,
        Self::Ping,
        Self::Pong,
        Self::ContactRequest,
        Self::ContactAccept,
        Self::ChatMessage,
        Self::MessageAck,
        Self::Profile,
        Self::EndpointUpdate,
        Self::FileOffer,
        Self::FileAccept,
        Self::FileReject,
        Self::FileChunk,
        Self::FileComplete,
        Self::FileAbort,
    ];

    /// Returns the wire code of this message type.
    pub const fn code(self) -> u16 {
        match self {
            Self::AuthProof => 0x0001,
            Self::Close => 0x0002,
            Self::Ping => 0x0003,
            Self::Pong => 0x0004,
            Self::ContactRequest => 0x0010,
            Self::ContactAccept => 0x0011,
            Self::ChatMessage => 0x0020,
            Self::MessageAck => 0x0021,
            Self::Profile => 0x0030,
            Self::EndpointUpdate => 0x0031,
            Self::FileOffer => 0x0040,
            Self::FileAccept => 0x0041,
            Self::FileReject => 0x0042,
            Self::FileChunk => 0x0043,
            Self::FileComplete => 0x0044,
            Self::FileAbort => 0x0045,
        }
    }

    /// Returns the message type for a wire code, if the code is assigned.
    pub const fn from_code(code: u16) -> Option<Self> {
        Some(match code {
            0x0001 => Self::AuthProof,
            0x0002 => Self::Close,
            0x0003 => Self::Ping,
            0x0004 => Self::Pong,
            0x0010 => Self::ContactRequest,
            0x0011 => Self::ContactAccept,
            0x0020 => Self::ChatMessage,
            0x0021 => Self::MessageAck,
            0x0030 => Self::Profile,
            0x0031 => Self::EndpointUpdate,
            0x0040 => Self::FileOffer,
            0x0041 => Self::FileAccept,
            0x0042 => Self::FileReject,
            0x0043 => Self::FileChunk,
            0x0044 => Self::FileComplete,
            0x0045 => Self::FileAbort,
            _ => return None,
        })
    }

    /// Returns true if a peer may send this message while the session is in
    /// `state`.
    ///
    /// This is the gate behind invariant S19: nothing but an identity proof
    /// is accepted before authentication, and until a session is confirmed
    /// as a contact session the peer can do nothing but ask or confirm.
    pub const fn may_be_received_in(self, state: SessionState) -> bool {
        match state {
            SessionState::Connecting
            | SessionState::CryptoHandshake
            | SessionState::Closing
            | SessionState::Closed => false,
            SessionState::IdentityAuth => matches!(self, Self::AuthProof),
            SessionState::AuthenticatedUnknown => {
                matches!(
                    self,
                    Self::ContactRequest | Self::ContactAccept | Self::Close
                )
            }
            // ContactRequest and ContactAccept may still arrive after
            // confirmation when messages cross. They are validated like any
            // other message and then change nothing.
            SessionState::AuthenticatedContact => !matches!(self, Self::AuthProof),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for message in MessageType::ALL {
            assert_eq!(MessageType::from_code(message.code()), Some(message));
        }
    }

    #[test]
    fn codes_are_unique_and_ordered() {
        for pair in MessageType::ALL.windows(2) {
            assert!(pair[0].code() < pair[1].code());
        }
    }

    #[test]
    fn unassigned_codes_are_rejected() {
        let assigned: Vec<u16> = MessageType::ALL.iter().map(|m| m.code()).collect();
        for code in 0..=u16::MAX {
            if !assigned.contains(&code) {
                assert_eq!(MessageType::from_code(code), None);
            }
        }
    }

    #[test]
    fn nothing_is_accepted_before_the_encrypted_channel_exists() {
        for message in MessageType::ALL {
            assert!(!message.may_be_received_in(SessionState::Connecting));
            assert!(!message.may_be_received_in(SessionState::CryptoHandshake));
        }
    }

    #[test]
    fn only_an_identity_proof_is_accepted_before_authentication() {
        for message in MessageType::ALL {
            let expected = message == MessageType::AuthProof;
            assert_eq!(
                message.may_be_received_in(SessionState::IdentityAuth),
                expected
            );
        }
    }

    #[test]
    fn unknown_peers_cannot_send_application_data() {
        let allowed = [
            MessageType::ContactRequest,
            MessageType::ContactAccept,
            MessageType::Close,
        ];
        for message in MessageType::ALL {
            assert_eq!(
                message.may_be_received_in(SessionState::AuthenticatedUnknown),
                allowed.contains(&message)
            );
        }
    }

    #[test]
    fn an_identity_proof_is_accepted_only_once() {
        assert!(!MessageType::AuthProof.may_be_received_in(SessionState::AuthenticatedUnknown));
        assert!(!MessageType::AuthProof.may_be_received_in(SessionState::AuthenticatedContact));
    }

    #[test]
    fn nothing_is_accepted_while_closing_or_closed() {
        for message in MessageType::ALL {
            assert!(!message.may_be_received_in(SessionState::Closing));
            assert!(!message.may_be_received_in(SessionState::Closed));
        }
    }
}

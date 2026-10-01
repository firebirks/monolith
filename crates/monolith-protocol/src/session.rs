//! Session logic: states and contact confirmation.
//!
//! [`Session`] is the state machine of one peer session, as specified in
//! `docs/PROTOCOL.md` sections 6.4, 7 and 12. It does no I/O and no
//! cryptography. The caller tells it what happened (the stream is up, the
//! handshake finished, both identities are proven, a message of some type
//! arrived) and it answers with what to do ([`Action`]).
//!
//! Two properties are built into its shape:
//!
//! - It decides from the message type and the local [`Standing`] of the peer
//!   alone. Whether the user has verified a contact out of band is not an
//!   input, so it cannot influence anything a peer observes.
//! - For a peer that is not a contact, what is sent back does not depend on
//!   why it is not a contact. Unknown, declined and blocked identities take
//!   the same path; the only difference is an internal action that the peer
//!   cannot see. See `docs/PROTOCOL.md` section 12.1.

use crate::{MessageType, ProtocolError, SessionState};

/// What the local side holds about the identity a peer has proven.
///
/// A contact that the user deleted has no record and is [`Standing::None`],
/// like an identity that was never seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Standing {
    /// No record of this identity.
    None,
    /// The user declined a request from this identity.
    Declined,
    /// The user blocked this identity.
    Blocked,
    /// The user imported this identity's card and has not seen an acceptance.
    Requested,
    /// An accepted contact.
    Accepted,
}

impl Standing {
    /// Returns true for the standings that are counted against the contact
    /// session budget: identities the user chose to have as contacts.
    pub const fn is_contact_record(self) -> bool {
        matches!(self, Self::Requested | Self::Accepted)
    }
}

/// Something the caller has to do in response to an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// Send a ContactAccept.
    SendContactAccept,
    /// Send a ContactRequest.
    SendContactRequest,
    /// Send a Close and then close the stream.
    SendClose,
    /// Close the stream without sending anything.
    Disconnect,
    /// Record the peer as an accepted contact.
    MarkAccepted,
    /// Decide whether the contact request that just arrived goes into the
    /// pending queue. Nothing about the decision is sent to the peer.
    ConsiderRequest,
    /// The session is now a confirmed contact session.
    Confirmed,
    /// Hand the message to the application.
    Deliver,
    /// Hand the identity proof to the authentication layer.
    VerifyIdentityProof,
}

impl Action {
    /// Returns true if the peer can observe this action: something is sent
    /// or the stream is closed. The other actions are local bookkeeping.
    pub const fn is_visible_to_peer(self) -> bool {
        matches!(
            self,
            Self::SendContactAccept | Self::SendContactRequest | Self::SendClose | Self::Disconnect
        )
    }
}

/// The state machine of one session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    state: SessionState,
    standing: Standing,
    proof_received: bool,
}

impl Default for Session {
    fn default() -> Self {
        Self::new()
    }
}

impl Session {
    /// Creates a session in `Connecting`.
    pub const fn new() -> Self {
        Self {
            state: SessionState::Connecting,
            standing: Standing::None,
            proof_received: false,
        }
    }

    /// Returns the current state.
    pub const fn state(&self) -> SessionState {
        self.state
    }

    /// Returns the standing of the peer. Meaningful once the session is
    /// authenticated.
    pub const fn standing(&self) -> Standing {
        self.standing
    }

    fn transition(&mut self, next: SessionState) -> Result<(), ProtocolError> {
        if self.state.can_transition_to(next) {
            self.state = next;
            Ok(())
        } else {
            Err(ProtocolError::MessageNotPermitted)
        }
    }

    /// The transport stream is established.
    pub fn stream_established(&mut self) -> Result<(), ProtocolError> {
        if self.state != SessionState::Connecting {
            return Err(ProtocolError::MessageNotPermitted);
        }
        self.transition(SessionState::CryptoHandshake)
    }

    /// The cryptographic handshake finished and the encrypted channel exists.
    pub fn handshake_completed(&mut self) -> Result<(), ProtocolError> {
        if self.state != SessionState::CryptoHandshake {
            return Err(ProtocolError::MessageNotPermitted);
        }
        self.transition(SessionState::IdentityAuth)
    }

    /// Both identity proofs are verified. `standing` is what the local side
    /// holds about the identity the peer proved.
    ///
    /// The session enters `AuthenticatedUnknown`. The returned actions are
    /// the first message of `docs/PROTOCOL.md` section 6.4.
    pub fn identities_proven(&mut self, standing: Standing) -> Result<Vec<Action>, ProtocolError> {
        if self.state != SessionState::IdentityAuth {
            return Err(ProtocolError::MessageNotPermitted);
        }
        self.transition(SessionState::AuthenticatedUnknown)?;
        self.standing = standing;
        Ok(match standing {
            Standing::Accepted => vec![Action::SendContactAccept],
            Standing::Requested => vec![Action::SendContactRequest],
            Standing::None | Standing::Declined | Standing::Blocked => Vec::new(),
        })
    }

    /// A message of type `message_type` arrived and passed the frame checks.
    ///
    /// Returns what to do. An error is a protocol violation: the session is
    /// over and the stream must be closed without sending anything.
    pub fn receive(&mut self, message_type: MessageType) -> Result<Vec<Action>, ProtocolError> {
        // Frames that arrive after Close was sent or received are discarded.
        if matches!(self.state, SessionState::Closing | SessionState::Closed) {
            return Ok(Vec::new());
        }
        if !message_type.may_be_received_in(self.state) {
            self.state = SessionState::Closed;
            return Err(ProtocolError::MessageNotPermitted);
        }
        match self.state {
            SessionState::IdentityAuth => self.receive_in_identity_auth(),
            SessionState::AuthenticatedUnknown => self.receive_unconfirmed(message_type),
            SessionState::AuthenticatedContact => Ok(self.receive_confirmed(message_type)),
            // The gate above lets nothing through in the remaining states.
            SessionState::Connecting
            | SessionState::CryptoHandshake
            | SessionState::Closing
            | SessionState::Closed => {
                self.state = SessionState::Closed;
                Err(ProtocolError::MessageNotPermitted)
            }
        }
    }

    fn receive_in_identity_auth(&mut self) -> Result<Vec<Action>, ProtocolError> {
        // The gate admits only AuthProof here, and only one of them.
        if self.proof_received {
            self.state = SessionState::Closed;
            return Err(ProtocolError::MessageNotPermitted);
        }
        self.proof_received = true;
        Ok(vec![Action::VerifyIdentityProof])
    }

    fn receive_unconfirmed(
        &mut self,
        message_type: MessageType,
    ) -> Result<Vec<Action>, ProtocolError> {
        if message_type == MessageType::Close {
            self.state = SessionState::Closed;
            return Ok(vec![Action::Disconnect]);
        }
        let is_request = message_type == MessageType::ContactRequest;
        match self.standing {
            Standing::Accepted => {
                if is_request {
                    // The peer does not know yet that it was accepted.
                    // ContactAccept was sent when the session authenticated.
                    Ok(Vec::new())
                } else {
                    self.transition(SessionState::AuthenticatedContact)?;
                    Ok(vec![Action::Confirmed])
                }
            }
            Standing::Requested => {
                self.standing = Standing::Accepted;
                if is_request {
                    // Both sides asked. Confirmed once the peer's
                    // ContactAccept arrives.
                    Ok(vec![Action::MarkAccepted, Action::SendContactAccept])
                } else {
                    self.transition(SessionState::AuthenticatedContact)?;
                    Ok(vec![
                        Action::MarkAccepted,
                        Action::SendContactAccept,
                        Action::Confirmed,
                    ])
                }
            }
            Standing::None | Standing::Declined | Standing::Blocked => {
                // One path for every identity that is not a contact. The
                // standing only decides whether the request is looked at,
                // which the peer cannot see.
                self.transition(SessionState::Closing)?;
                let mut actions = Vec::with_capacity(2);
                if is_request && self.standing == Standing::None {
                    actions.push(Action::ConsiderRequest);
                }
                actions.push(Action::SendClose);
                Ok(actions)
            }
        }
    }

    fn receive_confirmed(&mut self, message_type: MessageType) -> Vec<Action> {
        match message_type {
            MessageType::Close => {
                self.state = SessionState::Closed;
                vec![Action::Disconnect]
            }
            // These can still arrive when messages cross. They change nothing.
            MessageType::ContactRequest | MessageType::ContactAccept => Vec::new(),
            _ => vec![Action::Deliver],
        }
    }

    /// Returns true if the local side may send a message of this type now.
    pub const fn may_send(&self, message_type: MessageType) -> bool {
        match self.state {
            SessionState::IdentityAuth => matches!(message_type, MessageType::AuthProof),
            SessionState::AuthenticatedUnknown => match message_type {
                MessageType::Close => true,
                MessageType::ContactAccept => matches!(self.standing, Standing::Accepted),
                MessageType::ContactRequest => matches!(self.standing, Standing::Requested),
                _ => false,
            },
            SessionState::AuthenticatedContact => !matches!(
                message_type,
                MessageType::AuthProof | MessageType::ContactRequest
            ),
            SessionState::Connecting
            | SessionState::CryptoHandshake
            | SessionState::Closing
            | SessionState::Closed => false,
        }
    }

    /// The local side ends the session on purpose: the user quits, a limit
    /// is reached, the session lost a duplicate resolution, or an
    /// unconfirmed session timed out.
    pub fn close(&mut self) -> Vec<Action> {
        match self.state {
            SessionState::AuthenticatedUnknown | SessionState::AuthenticatedContact => {
                self.state = SessionState::Closing;
                vec![Action::SendClose]
            }
            SessionState::Closing | SessionState::Closed => Vec::new(),
            SessionState::Connecting
            | SessionState::CryptoHandshake
            | SessionState::IdentityAuth => {
                self.state = SessionState::Closed;
                vec![Action::Disconnect]
            }
        }
    }

    /// The user blocked the peer of this session.
    ///
    /// The peer sees the same Close as for any other end of a session.
    pub fn peer_blocked(&mut self) -> Vec<Action> {
        self.standing = Standing::Blocked;
        self.close()
    }

    /// The stream is gone, for whatever reason.
    pub fn stream_closed(&mut self) {
        self.state = SessionState::Closed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NON_CONTACTS: [Standing; 3] = [Standing::None, Standing::Declined, Standing::Blocked];

    fn authenticated(standing: Standing) -> (Session, Vec<Action>) {
        let mut session = Session::new();
        session.stream_established().unwrap();
        session.handshake_completed().unwrap();
        let first = session.identities_proven(standing).unwrap();
        (session, first)
    }

    fn visible(actions: &[Action]) -> Vec<Action> {
        actions
            .iter()
            .copied()
            .filter(|action| action.is_visible_to_peer())
            .collect()
    }

    /// Everything the peer can observe of a session with the given standing
    /// when it sends the given messages: the visible actions after
    /// authentication and after each message, and the final state.
    fn transcript(
        standing: Standing,
        messages: &[MessageType],
    ) -> (Vec<Vec<Action>>, Result<(), ProtocolError>, SessionState) {
        let (mut session, first) = authenticated(standing);
        let mut seen = vec![visible(&first)];
        let mut outcome = Ok(());
        for message in messages {
            match session.receive(*message) {
                Ok(actions) => seen.push(visible(&actions)),
                Err(error) => {
                    outcome = Err(error);
                    break;
                }
            }
        }
        (seen, outcome, session.state())
    }

    #[test]
    fn states_are_entered_in_order() {
        let mut session = Session::new();
        assert_eq!(session.state(), SessionState::Connecting);
        assert!(session.handshake_completed().is_err());
        assert!(session.identities_proven(Standing::Accepted).is_err());
        session.stream_established().unwrap();
        assert!(session.stream_established().is_err());
        assert!(session.identities_proven(Standing::Accepted).is_err());
        session.handshake_completed().unwrap();
        assert_eq!(session.state(), SessionState::IdentityAuth);
        session.identities_proven(Standing::None).unwrap();
        assert_eq!(session.state(), SessionState::AuthenticatedUnknown);
        assert!(session.identities_proven(Standing::Accepted).is_err());
    }

    #[test]
    fn nothing_is_accepted_before_the_identity_proof() {
        // T-PROTO-STATE.
        for message in MessageType::ALL {
            let mut session = Session::new();
            assert!(
                session.receive(message).is_err(),
                "{message:?} in Connecting"
            );
            assert_eq!(session.state(), SessionState::Closed);

            let mut session = Session::new();
            session.stream_established().unwrap();
            assert!(
                session.receive(message).is_err(),
                "{message:?} in CryptoHandshake"
            );
        }
    }

    #[test]
    fn only_one_identity_proof_is_accepted() {
        let mut session = Session::new();
        session.stream_established().unwrap();
        session.handshake_completed().unwrap();
        assert_eq!(
            session.receive(MessageType::AuthProof),
            Ok(vec![Action::VerifyIdentityProof])
        );
        assert!(session.receive(MessageType::AuthProof).is_err());
        assert_eq!(session.state(), SessionState::Closed);
    }

    #[test]
    fn first_message_depends_only_on_the_standing() {
        assert_eq!(
            authenticated(Standing::Accepted).1,
            vec![Action::SendContactAccept]
        );
        assert_eq!(
            authenticated(Standing::Requested).1,
            vec![Action::SendContactRequest]
        );
        for standing in NON_CONTACTS {
            assert_eq!(authenticated(standing).1, Vec::new());
        }
    }

    #[test]
    fn accepted_contacts_confirm_with_contact_accept() {
        let (mut session, _) = authenticated(Standing::Accepted);
        assert_eq!(
            session.receive(MessageType::ContactAccept),
            Ok(vec![Action::Confirmed])
        );
        assert_eq!(session.state(), SessionState::AuthenticatedContact);
        assert_eq!(
            session.receive(MessageType::ChatMessage),
            Ok(vec![Action::Deliver])
        );
    }

    #[test]
    fn a_request_from_an_accepted_peer_changes_nothing() {
        let (mut session, _) = authenticated(Standing::Accepted);
        assert_eq!(session.receive(MessageType::ContactRequest), Ok(Vec::new()));
        assert_eq!(session.state(), SessionState::AuthenticatedUnknown);
        assert_eq!(
            session.receive(MessageType::ContactAccept),
            Ok(vec![Action::Confirmed])
        );
    }

    #[test]
    fn a_requested_peer_that_accepts_becomes_a_contact() {
        // T-CONTACT-5.
        let (mut session, _) = authenticated(Standing::Requested);
        assert_eq!(
            session.receive(MessageType::ContactAccept),
            Ok(vec![
                Action::MarkAccepted,
                Action::SendContactAccept,
                Action::Confirmed
            ])
        );
        assert_eq!(session.state(), SessionState::AuthenticatedContact);
        assert_eq!(session.standing(), Standing::Accepted);
    }

    #[test]
    fn crossing_requests_accept_each_other() {
        // T-CONTACT-2. Both sides hold the other as requested and both sent
        // a ContactRequest.
        let (mut a, first_a) = authenticated(Standing::Requested);
        let (mut b, first_b) = authenticated(Standing::Requested);
        assert_eq!(first_a, vec![Action::SendContactRequest]);
        assert_eq!(first_b, vec![Action::SendContactRequest]);

        let from_a = a.receive(MessageType::ContactRequest).unwrap();
        let from_b = b.receive(MessageType::ContactRequest).unwrap();
        assert_eq!(
            from_a,
            vec![Action::MarkAccepted, Action::SendContactAccept]
        );
        assert_eq!(from_b, from_a);
        assert_eq!(a.state(), SessionState::AuthenticatedUnknown);

        assert_eq!(
            a.receive(MessageType::ContactAccept),
            Ok(vec![Action::Confirmed])
        );
        assert_eq!(
            b.receive(MessageType::ContactAccept),
            Ok(vec![Action::Confirmed])
        );
        assert_eq!(a.state(), SessionState::AuthenticatedContact);
        assert_eq!(b.state(), SessionState::AuthenticatedContact);
    }

    #[test]
    fn a_stranger_request_is_considered_and_answered_with_close() {
        let (mut session, _) = authenticated(Standing::None);
        assert_eq!(
            session.receive(MessageType::ContactRequest),
            Ok(vec![Action::ConsiderRequest, Action::SendClose])
        );
        assert_eq!(session.state(), SessionState::Closing);
        // Whatever follows is discarded.
        assert_eq!(session.receive(MessageType::ChatMessage), Ok(Vec::new()));
        assert_eq!(session.receive(MessageType::ContactRequest), Ok(Vec::new()));
    }

    #[test]
    fn requests_from_declined_and_blocked_peers_are_not_considered() {
        for standing in [Standing::Declined, Standing::Blocked] {
            let (mut session, _) = authenticated(standing);
            assert_eq!(
                session.receive(MessageType::ContactRequest),
                Ok(vec![Action::SendClose])
            );
        }
    }

    #[test]
    fn oracle_1_non_contacts_are_indistinguishable() {
        // T-ORACLE-1. Unknown, declined and blocked identities, and a deleted
        // former contact (which has no record and is Standing::None), see
        // the same thing for every behavior.
        let behaviors: [&[MessageType]; 7] = [
            &[],
            &[MessageType::ContactRequest],
            &[MessageType::ContactAccept],
            &[MessageType::Close],
            &[MessageType::ContactRequest, MessageType::ContactRequest],
            &[MessageType::ContactAccept, MessageType::ChatMessage],
            &[MessageType::ChatMessage],
        ];
        for behavior in behaviors {
            let reference = transcript(Standing::None, behavior);
            for standing in NON_CONTACTS {
                assert_eq!(
                    transcript(standing, behavior),
                    reference,
                    "{standing:?} {behavior:?}"
                );
            }
        }
    }

    #[test]
    fn oracle_1_the_only_difference_is_invisible() {
        // The request of an unknown peer is considered, that of a declined
        // or blocked peer is not. That action is not visible to the peer.
        assert!(!Action::ConsiderRequest.is_visible_to_peer());
        assert!(!Action::MarkAccepted.is_visible_to_peer());
        assert!(!Action::Confirmed.is_visible_to_peer());
        assert!(!Action::Deliver.is_visible_to_peer());
        assert!(Action::SendClose.is_visible_to_peer());
        assert!(Action::Disconnect.is_visible_to_peer());
    }

    #[test]
    fn oracle_2_verification_is_not_an_input() {
        // T-ORACLE-2. Standing has no notion of "verified", and Session
        // takes nothing else about the peer. This test fails to compile if a
        // variant is added without being listed here.
        let all = [
            Standing::None,
            Standing::Declined,
            Standing::Blocked,
            Standing::Requested,
            Standing::Accepted,
        ];
        for standing in all {
            match standing {
                Standing::None
                | Standing::Declined
                | Standing::Blocked
                | Standing::Requested
                | Standing::Accepted => {}
            }
        }
    }

    #[test]
    fn oracle_4_violations_emit_nothing() {
        // T-ORACLE-4. A message that is not legal in the state ends the
        // session with an error and no action, for every standing.
        let all = [
            Standing::None,
            Standing::Declined,
            Standing::Blocked,
            Standing::Requested,
            Standing::Accepted,
        ];
        for standing in all {
            for message in MessageType::ALL {
                let (mut session, _) = authenticated(standing);
                if message.may_be_received_in(SessionState::AuthenticatedUnknown) {
                    continue;
                }
                assert_eq!(
                    session.receive(message),
                    Err(ProtocolError::MessageNotPermitted),
                    "{standing:?} {message:?}"
                );
                assert_eq!(session.state(), SessionState::Closed);
            }
        }
    }

    #[test]
    fn oracle_5_nothing_but_confirmation_messages_before_confirmation() {
        // T-ORACLE-5. Before a session is confirmed the local side may send
        // only ContactAccept, ContactRequest and Close, and never produces a
        // Deliver action.
        let all = [
            Standing::None,
            Standing::Declined,
            Standing::Blocked,
            Standing::Requested,
            Standing::Accepted,
        ];
        for standing in all {
            let (session, _) = authenticated(standing);
            for message in MessageType::ALL {
                let allowed = matches!(
                    message,
                    MessageType::ContactAccept | MessageType::ContactRequest | MessageType::Close
                );
                if !allowed {
                    assert!(!session.may_send(message), "{standing:?} {message:?}");
                }
            }
            for message in [MessageType::ContactRequest, MessageType::ContactAccept] {
                let (mut session, _) = authenticated(standing);
                let actions = session.receive(message).unwrap();
                if session.state() != SessionState::AuthenticatedContact {
                    assert!(!actions.contains(&Action::Deliver));
                }
            }
        }
    }

    #[test]
    fn may_send_follows_the_standing_before_confirmation() {
        let (session, _) = authenticated(Standing::Accepted);
        assert!(session.may_send(MessageType::ContactAccept));
        assert!(!session.may_send(MessageType::ContactRequest));

        let (session, _) = authenticated(Standing::Requested);
        assert!(session.may_send(MessageType::ContactRequest));
        assert!(!session.may_send(MessageType::ContactAccept));

        for standing in NON_CONTACTS {
            let (session, _) = authenticated(standing);
            assert!(!session.may_send(MessageType::ContactAccept));
            assert!(!session.may_send(MessageType::ContactRequest));
            assert!(session.may_send(MessageType::Close));
        }
    }

    #[test]
    fn one_sided_contact_is_never_confirmed() {
        // T-CONTACT-4. One side holds the other as accepted, the other side
        // has no record (deleted, lost its state, restored a backup).
        let (mut keeper, first_keeper) = authenticated(Standing::Accepted);
        let (mut deleter, first_deleter) = authenticated(Standing::None);
        assert_eq!(first_keeper, vec![Action::SendContactAccept]);
        assert_eq!(first_deleter, Vec::new());

        // The deleter receives the ContactAccept and answers with Close.
        assert_eq!(
            deleter.receive(MessageType::ContactAccept),
            Ok(vec![Action::SendClose])
        );
        // The keeper receives Close. It was never confirmed and never
        // delivered or sent anything else.
        assert_eq!(
            keeper.receive(MessageType::Close),
            Ok(vec![Action::Disconnect])
        );
        assert_eq!(keeper.state(), SessionState::Closed);
        assert!(!keeper.may_send(MessageType::ChatMessage));
    }

    #[test]
    fn confirmed_sessions_ignore_late_confirmation_messages() {
        let (mut session, _) = authenticated(Standing::Accepted);
        session.receive(MessageType::ContactAccept).unwrap();
        assert_eq!(session.receive(MessageType::ContactAccept), Ok(Vec::new()));
        assert_eq!(session.receive(MessageType::ContactRequest), Ok(Vec::new()));
        assert!(session.receive(MessageType::AuthProof).is_err());
    }

    #[test]
    fn close_from_the_peer_ends_the_session() {
        for standing in [Standing::None, Standing::Accepted] {
            let (mut session, _) = authenticated(standing);
            assert_eq!(
                session.receive(MessageType::Close),
                Ok(vec![Action::Disconnect])
            );
            assert_eq!(session.state(), SessionState::Closed);
            assert_eq!(session.receive(MessageType::ChatMessage), Ok(Vec::new()));
        }
    }

    #[test]
    fn local_close_sends_close_only_on_an_authenticated_session() {
        let mut session = Session::new();
        assert_eq!(session.close(), vec![Action::Disconnect]);
        assert_eq!(session.state(), SessionState::Closed);
        assert_eq!(session.close(), Vec::new());

        let (mut session, _) = authenticated(Standing::None);
        assert_eq!(session.close(), vec![Action::SendClose]);
        assert_eq!(session.state(), SessionState::Closing);
        assert_eq!(session.close(), Vec::new());
        session.stream_closed();
        assert_eq!(session.state(), SessionState::Closed);
    }

    #[test]
    fn blocking_a_contact_looks_like_any_other_close() {
        // First half of T-ORACLE-7.
        let (mut blocked, _) = authenticated(Standing::Accepted);
        blocked.receive(MessageType::ContactAccept).unwrap();
        let (mut quit, _) = authenticated(Standing::Accepted);
        quit.receive(MessageType::ContactAccept).unwrap();

        assert_eq!(blocked.peer_blocked(), quit.close());
        assert_eq!(blocked.state(), quit.state());
        assert_eq!(blocked.standing(), Standing::Blocked);
    }

    #[test]
    fn the_session_never_moves_backwards() {
        // Drive sessions with every pair of messages and check that each
        // step is a legal transition or no transition at all.
        let standings = [
            Standing::None,
            Standing::Declined,
            Standing::Blocked,
            Standing::Requested,
            Standing::Accepted,
        ];
        for standing in standings {
            for first in MessageType::ALL {
                for second in MessageType::ALL {
                    let (mut session, _) = authenticated(standing);
                    for message in [first, second] {
                        let before = session.state();
                        let _ = session.receive(message);
                        let after = session.state();
                        assert!(
                            before == after || before.can_transition_to(after),
                            "{standing:?} {first:?} {second:?}: {before:?} -> {after:?}"
                        );
                    }
                }
            }
        }
    }
}

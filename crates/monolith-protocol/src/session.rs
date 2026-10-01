//! Session logic: states and contact confirmation.
//!
//! [`Session`] is the state machine of one peer session, as specified in
//! `docs/PROTOCOL.md` sections 6.4, 7 and 12. It does no I/O and no
//! cryptography. The caller tells it what happened (the stream is up, the
//! handshake finished, both identities are proven, a message arrived) and it
//! answers with what to do ([`Action`]).
//!
//! Properties that are built into its shape:
//!
//! - It takes decoded messages, so nothing reaches it that has not passed
//!   the frame and body decoders.
//! - It knows the local identity and, for a session the local side opened,
//!   the identity that was dialed. A proof that names the local identity, or
//!   on an outbound session any identity but the dialed one, ends the
//!   session.
//! - It knows the identity the peer proved. A card inside a message that was
//!   signed by any other identity is a violation, and that is checked before
//!   the standing of the peer is looked at.
//! - It decides from the message and the local [`Standing`] of the peer
//!   alone. Whether the user has verified a contact out of band is not an
//!   input, so it cannot influence anything a peer observes.
//! - For a peer that is not a contact, what is sent back does not depend on
//!   why it is not a contact. Unknown, declined and blocked identities take
//!   the same path; the only difference is an internal action that the peer
//!   cannot see. See `docs/PROTOCOL.md` section 12.1.
//!
//! After a Close was sent or received the caller stops reading from the
//! stream. Bytes still in flight are dropped; they are not fed to the frame
//! decoder, which refuses every frame in those states.

use monolith_identity::IdentityPublicKey;

use crate::body::Message;
use crate::duplicate::Initiator;
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
    /// Send a Close and then close the stream. Both are done before any
    /// action that follows this one in the same list is carried out, so
    /// that nothing the peer can time depends on the later actions.
    SendClose,
    /// Close the stream without sending anything.
    Disconnect,
    /// Record the peer as an accepted contact.
    MarkAccepted,
    /// Decide whether the contact request that just arrived goes into the
    /// pending queue. Nothing about the decision is sent to the peer, and
    /// the stream is already closed when it is taken.
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
    /// The local identity. A peer that claims it is refused.
    local: IdentityPublicKey,
    /// On a session the local side opened: the identity it dialed.
    expected: Option<IdentityPublicKey>,
    /// The identity named in the peer's AuthProof, until it is verified.
    claimed: Option<IdentityPublicKey>,
    /// The identity the peer proved.
    peer: Option<IdentityPublicKey>,
    /// A ContactRequest arrived before the session was confirmed.
    request_received: bool,
}

impl Session {
    const fn new(local: IdentityPublicKey, expected: Option<IdentityPublicKey>) -> Self {
        Self {
            state: SessionState::Connecting,
            standing: Standing::None,
            local,
            expected,
            claimed: None,
            peer: None,
            request_received: false,
        }
    }

    /// Creates a session for a stream that a peer opened. It starts in
    /// `Connecting`. `local` is the local identity.
    pub const fn inbound(local: IdentityPublicKey) -> Self {
        Self::new(local, None)
    }

    /// Creates a session for a stream the local side opened to reach the
    /// identity `expected`. It starts in `Connecting`.
    ///
    /// The session can only ever be authenticated as `expected`. An
    /// endpoint that proves any other identity ends it with
    /// [`ProtocolError::IdentityMismatch`], which the caller reports to the
    /// user and never resolves by itself (`docs/PROTOCOL.md` section 6.2).
    pub const fn outbound(local: IdentityPublicKey, expected: IdentityPublicKey) -> Self {
        Self::new(local, Some(expected))
    }

    /// Returns which side opened the session.
    pub const fn initiator(&self) -> Initiator {
        if self.expected.is_some() {
            Initiator::Local
        } else {
            Initiator::Remote
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

    /// Returns the identity the peer proved, once it has.
    pub const fn peer(&self) -> Option<&IdentityPublicKey> {
        self.peer.as_ref()
    }

    fn violation<T>(&mut self, error: ProtocolError) -> Result<T, ProtocolError> {
        self.state = SessionState::Closed;
        Err(error)
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

    /// The authentication layer verified the peer's identity proof, and the
    /// local proof has been sent.
    ///
    /// `peer` is the proven identity and `standing` what the local side
    /// holds about it. This fails unless an AuthProof was received on this
    /// session and named the same identity, so a caller cannot authenticate
    /// a session that never presented a proof. That failure ends the
    /// session.
    ///
    /// The session enters `AuthenticatedUnknown`. The returned actions are
    /// the first message of `docs/PROTOCOL.md` section 6.4.
    ///
    /// The order in which the two proofs are sent is not checked here. It
    /// belongs to the authentication layer, which is not decided.
    pub fn identities_proven(
        &mut self,
        peer: IdentityPublicKey,
        standing: Standing,
    ) -> Result<Vec<Action>, ProtocolError> {
        if self.state != SessionState::IdentityAuth {
            return Err(ProtocolError::MessageNotPermitted);
        }
        if self.claimed != Some(peer) {
            return self.violation(ProtocolError::AuthenticationFailed);
        }
        self.transition(SessionState::AuthenticatedUnknown)?;
        self.peer = Some(peer);
        self.standing = standing;
        Ok(match standing {
            Standing::Accepted => vec![Action::SendContactAccept],
            Standing::Requested => vec![Action::SendContactRequest],
            Standing::None | Standing::Declined | Standing::Blocked => Vec::new(),
        })
    }

    /// A message arrived. It has passed the frame checks and its body has
    /// been decoded.
    ///
    /// Returns what to do. An error is a protocol violation: the session is
    /// over and the stream must be closed without sending anything.
    pub fn receive(&mut self, message: &Message) -> Result<Vec<Action>, ProtocolError> {
        // After a Close was sent or received nothing is processed any more.
        if matches!(self.state, SessionState::Closing | SessionState::Closed) {
            return Ok(Vec::new());
        }
        let message_type = message.message_type();
        if !message_type.may_be_received_in(self.state) {
            return self.violation(ProtocolError::MessageNotPermitted);
        }
        // A card inside a message must be the sender's own. This is checked
        // before the standing of the peer is looked at, so the outcome is
        // the same for every peer.
        let foreign_card =
            card_identity(message).is_some_and(|signer| self.peer.as_ref() != Some(signer));
        if foreign_card {
            return self.violation(ProtocolError::IdentityMismatch);
        }
        match self.state {
            SessionState::IdentityAuth => self.receive_in_identity_auth(message),
            SessionState::AuthenticatedUnknown => self.receive_unconfirmed(message_type),
            SessionState::AuthenticatedContact => Ok(self.receive_confirmed(message_type)),
            // The gate above lets nothing through in the remaining states.
            SessionState::Connecting
            | SessionState::CryptoHandshake
            | SessionState::Closing
            | SessionState::Closed => self.violation(ProtocolError::MessageNotPermitted),
        }
    }

    fn receive_in_identity_auth(
        &mut self,
        message: &Message,
    ) -> Result<Vec<Action>, ProtocolError> {
        // The gate admits only AuthProof here, and only one of them.
        let Message::AuthProof(proof) = message else {
            return self.violation(ProtocolError::MessageNotPermitted);
        };
        if self.claimed.is_some() {
            return self.violation(ProtocolError::MessageNotPermitted);
        }
        if proof.identity == self.local {
            // Nobody else holds the local identity key. Whoever names it
            // cannot prove it, and a session with oneself is not a session.
            return self.violation(ProtocolError::AuthenticationFailed);
        }
        if self
            .expected
            .is_some_and(|expected| expected != proof.identity)
        {
            return self.violation(ProtocolError::IdentityMismatch);
        }
        self.claimed = Some(proof.identity);
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
        if is_request {
            // A request is made once per session. A second one would only
            // make the receiver verify another signature.
            if self.request_received {
                return self.violation(ProtocolError::MessageNotPermitted);
            }
            self.request_received = true;
        }
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
                // Close goes out first; whether the request is looked at
                // afterwards is local and cannot be seen by the peer.
                self.transition(SessionState::Closing)?;
                let mut actions = vec![Action::SendClose];
                if is_request && self.standing == Standing::None {
                    actions.push(Action::ConsiderRequest);
                }
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
            // A conforming peer does not send these after confirmation.
            // They change nothing.
            MessageType::ContactRequest | MessageType::ContactAccept => Vec::new(),
            _ => vec![Action::Deliver],
        }
    }

    /// Returns true if the local side may send a message of this type now.
    ///
    /// In `Closing` this is true for Close alone: the state is entered when
    /// the decision to close is taken, and the Close still has to be
    /// written. In `IdentityAuth` it is true for AuthProof; the order in
    /// which the two sides send their proofs belongs to the authentication
    /// layer, which is not decided.
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
            SessionState::Closing => matches!(message_type, MessageType::Close),
            SessionState::Connecting | SessionState::CryptoHandshake | SessionState::Closed => {
                false
            }
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

    /// The user deleted the contact, or withdrew a request, while this
    /// session was open. The peer sees the same Close as for any other end
    /// of a session.
    pub fn contact_removed(&mut self) -> Vec<Action> {
        self.standing = Standing::None;
        self.close()
    }

    /// The stream is gone, for whatever reason.
    pub fn stream_closed(&mut self) {
        self.state = SessionState::Closed;
    }
}

/// Returns the identity that signed the card inside a message, for the
/// messages that carry one.
fn card_identity(message: &Message) -> Option<&IdentityPublicKey> {
    match message {
        Message::ContactRequest(request) => Some(request.card.identity()),
        Message::EndpointUpdate(card) => Some(card.identity()),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! Sample messages and sessions for tests.

    use std::sync::LazyLock;

    use monolith_identity::{EndpointEpoch, IdentityPublicKey, IdentitySecretKey, OnionServiceKey};

    use super::{Action, Session, Standing};
    use crate::MessageType;
    use crate::body::{AuthProof, ContactRequest, FileChunk, Message, MessageId, TransferId};
    use crate::card::{ContactCard, EndpointSet};
    use crate::text::{ChatText, DisplayName, Filename, IntroductionText, ProfileText};

    /// Seed of the peer in session tests.
    pub(crate) const PEER: u8 = 0x51;

    /// Seed of the local identity in session tests.
    pub(crate) const LOCAL: u8 = 0x10;

    pub(crate) fn secret(seed: u8) -> IdentitySecretKey {
        IdentitySecretKey::from_seed(&[seed; 32])
    }

    pub(crate) fn identity(seed: u8) -> IdentityPublicKey {
        secret(seed).public_key()
    }

    pub(crate) fn card(seed: u8) -> ContactCard {
        let endpoint =
            OnionServiceKey::from_bytes(identity(seed.wrapping_add(1)).as_bytes()).unwrap();
        ContactCard::sign(
            &secret(seed),
            EndpointEpoch::FIRST,
            EndpointSet::single(endpoint),
            None,
        )
        .unwrap()
    }

    /// One message of every type as the peer would send it. Built once:
    /// signing is slow in unoptimized test builds.
    static FROM_PEER: LazyLock<Vec<Message>> = LazyLock::new(|| {
        MessageType::ALL
            .into_iter()
            .map(|message_type| sample(message_type, PEER))
            .collect()
    });

    static PEER_IDENTITY: LazyLock<IdentityPublicKey> = LazyLock::new(|| identity(PEER));
    static LOCAL_IDENTITY: LazyLock<IdentityPublicKey> = LazyLock::new(|| identity(LOCAL));

    /// The identity of the peer in session tests.
    pub(crate) fn peer() -> IdentityPublicKey {
        *PEER_IDENTITY
    }

    /// The local identity in session tests.
    pub(crate) fn local() -> IdentityPublicKey {
        *LOCAL_IDENTITY
    }

    /// A new session for a stream the peer opened.
    pub(crate) fn inbound() -> Session {
        Session::inbound(local())
    }

    /// One message of the given type. Cards inside it are signed by the
    /// identity of `signer`.
    pub(crate) fn sample(message_type: MessageType, signer: u8) -> Message {
        let transfer = TransferId::from_bytes([3; 16]);
        let id = MessageId::from_bytes([4; 16]);
        match message_type {
            MessageType::AuthProof => Message::AuthProof(Box::new(AuthProof {
                identity: identity(signer),
                features: 0,
                signature: secret(signer).sign(b"stands in for a real proof"),
            })),
            MessageType::Close => Message::Close,
            MessageType::Ping => Message::Ping([1; 8]),
            MessageType::Pong => Message::Pong([1; 8]),
            MessageType::ContactRequest => Message::ContactRequest(Box::new(ContactRequest {
                card: card(signer),
                invitation: None,
                display_name: DisplayName::new("Peer").unwrap(),
                introduction: IntroductionText::new("hello").unwrap(),
            })),
            MessageType::ContactAccept => Message::ContactAccept,
            MessageType::ChatMessage => Message::ChatMessage {
                id,
                text: ChatText::new("hi").unwrap(),
            },
            MessageType::MessageAck => Message::MessageAck(id),
            MessageType::Profile => Message::Profile {
                display_name: DisplayName::new("Peer").unwrap(),
                profile_text: ProfileText::new("").unwrap(),
            },
            MessageType::EndpointUpdate => Message::EndpointUpdate(Box::new(card(signer))),
            MessageType::FileOffer => Message::FileOffer {
                transfer,
                size: 1,
                filename: Filename::new("a.txt").unwrap(),
            },
            MessageType::FileAccept => Message::FileAccept(transfer),
            MessageType::FileReject => Message::FileReject(transfer),
            MessageType::FileChunk => {
                Message::FileChunk(FileChunk::new(transfer, vec![1]).unwrap())
            }
            MessageType::FileComplete => Message::FileComplete {
                transfer,
                digest: [5; 32],
            },
            MessageType::FileAbort => Message::FileAbort(transfer),
        }
    }

    /// A message of the given type as the peer of the session would send it.
    pub(crate) fn from_peer(message_type: MessageType) -> Message {
        FROM_PEER
            .iter()
            .find(|message| message.message_type() == message_type)
            .cloned()
            .unwrap()
    }

    /// A session that has gone through the handshake and received the peer's
    /// identity proof, but has not been told that the proof verified.
    pub(crate) fn awaiting_verification() -> Session {
        let mut session = inbound();
        session.stream_established().unwrap();
        session.handshake_completed().unwrap();
        assert_eq!(
            session.receive(&from_peer(MessageType::AuthProof)),
            Ok(vec![Action::VerifyIdentityProof])
        );
        session
    }

    /// A session in `AuthenticatedUnknown` with a peer of the given
    /// standing, and the actions that authentication produced.
    pub(crate) fn authenticated(standing: Standing) -> (Session, Vec<Action>) {
        let mut session = awaiting_verification();
        let first = session.identities_proven(peer(), standing).unwrap();
        (session, first)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{
        LOCAL, PEER, authenticated, awaiting_verification, from_peer, identity, inbound, local,
        peer, sample,
    };
    use std::collections::VecDeque;

    use super::*;
    use crate::card::InvitationCapability;

    const NON_CONTACTS: [Standing; 3] = [Standing::None, Standing::Declined, Standing::Blocked];

    const ALL_STANDINGS: [Standing; 5] = [
        Standing::None,
        Standing::Declined,
        Standing::Blocked,
        Standing::Requested,
        Standing::Accepted,
    ];

    /// Seed of an identity that is not the peer.
    const STRANGER: u8 = 0x77;

    fn visible(actions: &[Action]) -> Vec<Action> {
        actions
            .iter()
            .copied()
            .filter(|action| action.is_visible_to_peer())
            .collect()
    }

    type Transcript = (Vec<Vec<Action>>, Result<(), ProtocolError>, SessionState);

    /// Everything the peer can observe of a session with the given standing
    /// when it sends the given messages: the visible actions after
    /// authentication and after each message, the outcome, and the final
    /// state.
    fn transcript(standing: Standing, messages: &[Message]) -> Transcript {
        let (mut session, first) = authenticated(standing);
        let mut seen = vec![visible(&first)];
        let mut outcome = Ok(());
        for message in messages {
            match session.receive(message) {
                Ok(actions) => seen.push(visible(&actions)),
                Err(error) => {
                    outcome = Err(error);
                    break;
                }
            }
        }
        (seen, outcome, session.state())
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Side {
        A,
        B,
    }

    /// Two sessions connected to each other. `a` has the identity of seed
    /// `LOCAL` and opened the stream to `b`, which has the identity of seed
    /// `PEER`. What one side is told to send is built as a real message and
    /// delivered to the other, in order, as a stream would.
    struct Link {
        a: Session,
        b: Session,
        /// Every action each side was told to take, in order.
        a_actions: Vec<Action>,
        b_actions: Vec<Action>,
    }

    impl Link {
        /// Runs the two sessions until neither has anything left to send.
        /// Each side holds the other with the given standing.
        fn connect(a_holds_b: Standing, b_holds_a: Standing) -> Self {
            let mut link = Self {
                a: Session::outbound(local(), peer()),
                b: Session::inbound(peer()),
                a_actions: Vec::new(),
                b_actions: Vec::new(),
            };
            for session in [&mut link.a, &mut link.b] {
                session.stream_established().unwrap();
                session.handshake_completed().unwrap();
            }
            let mut wire = VecDeque::new();

            // The responder proves its identity first, then the initiator.
            assert_eq!(
                link.a.receive(&sample(MessageType::AuthProof, PEER)),
                Ok(vec![Action::VerifyIdentityProof])
            );
            let first = link.a.identities_proven(peer(), a_holds_b).unwrap();
            link.act(Side::A, first, &mut wire);
            assert_eq!(
                link.b.receive(&sample(MessageType::AuthProof, LOCAL)),
                Ok(vec![Action::VerifyIdentityProof])
            );
            let first = link.b.identities_proven(local(), b_holds_a).unwrap();
            link.act(Side::B, first, &mut wire);

            link.run(wire);
            link
        }

        /// Records what one side was told to do and puts what it sends on
        /// the wire.
        fn act(&mut self, side: Side, actions: Vec<Action>, wire: &mut VecDeque<(Side, Message)>) {
            let (session, seed, to) = match side {
                Side::A => (&self.a, LOCAL, Side::B),
                Side::B => (&self.b, PEER, Side::A),
            };
            for action in &actions {
                let message = match action {
                    Action::SendContactAccept => Message::ContactAccept,
                    Action::SendContactRequest => sample(MessageType::ContactRequest, seed),
                    Action::SendClose => Message::Close,
                    _ => continue,
                };
                assert!(session.may_send(message.message_type()), "{action:?}");
                wire.push_back((to, message));
            }
            match side {
                Side::A => self.a_actions.extend(actions),
                Side::B => self.b_actions.extend(actions),
            }
        }

        fn run(&mut self, mut wire: VecDeque<(Side, Message)>) {
            while let Some((to, message)) = wire.pop_front() {
                let session = match to {
                    Side::A => &mut self.a,
                    Side::B => &mut self.b,
                };
                let actions = session.receive(&message).unwrap();
                self.act(to, actions, &mut wire);
            }
            // A Close ends with the stream gone for both sides.
            let closed = self
                .a_actions
                .iter()
                .chain(&self.b_actions)
                .any(|action| matches!(action, Action::SendClose | Action::Disconnect));
            if closed {
                self.a.stream_closed();
                self.b.stream_closed();
            }
        }

        /// One side sends an application message on the running link.
        fn send(&mut self, from: Side, message: Message) {
            let (session, to) = match from {
                Side::A => (&self.a, Side::B),
                Side::B => (&self.b, Side::A),
            };
            assert!(session.may_send(message.message_type()));
            self.run(VecDeque::from([(to, message)]));
        }
    }

    #[test]
    fn crossing_requests_accept_each_other() {
        // T-CONTACT-2. Both users imported the other's card. Each side
        // sends a request, takes the other's request as the acceptance, and
        // the session is confirmed on both ends without a reconnect.
        let link = Link::connect(Standing::Requested, Standing::Requested);
        let expected = vec![
            Action::SendContactRequest,
            Action::MarkAccepted,
            Action::SendContactAccept,
            Action::Confirmed,
        ];
        assert_eq!(link.a_actions, expected);
        assert_eq!(link.b_actions, expected);
        for session in [&link.a, &link.b] {
            assert_eq!(session.state(), SessionState::AuthenticatedContact);
            assert_eq!(session.standing(), Standing::Accepted);
        }
    }

    #[test]
    fn a_request_to_an_accepting_side_confirms_both() {
        // T-CONTACT-5. The requester learns of the acceptance on this
        // session.
        let mut link = Link::connect(Standing::Requested, Standing::Accepted);
        assert_eq!(
            link.a_actions,
            vec![
                Action::SendContactRequest,
                Action::MarkAccepted,
                Action::SendContactAccept,
                Action::Confirmed
            ]
        );
        assert_eq!(
            link.b_actions,
            vec![Action::SendContactAccept, Action::Confirmed]
        );

        // Application data flows in both directions afterwards.
        link.send(Side::A, sample(MessageType::ChatMessage, LOCAL));
        assert_eq!(link.b_actions.last(), Some(&Action::Deliver));
        link.send(Side::B, sample(MessageType::EndpointUpdate, PEER));
        assert_eq!(link.a_actions.last(), Some(&Action::Deliver));
    }

    #[test]
    fn one_sided_contact_is_never_confirmed() {
        // T-CONTACT-4. One side holds the other as accepted, the other side
        // has no record (deleted, lost its state, restored a backup). In
        // either direction the keeper offers ContactAccept and gets a Close.
        let link = Link::connect(Standing::Accepted, Standing::None);
        assert_eq!(
            link.a_actions,
            vec![Action::SendContactAccept, Action::Disconnect]
        );
        assert_eq!(link.b_actions, vec![Action::SendClose]);

        let link = Link::connect(Standing::None, Standing::Accepted);
        assert_eq!(link.a_actions, vec![Action::SendClose]);
        assert_eq!(
            link.b_actions,
            vec![Action::SendContactAccept, Action::Disconnect]
        );
        assert_eq!(link.a.state(), SessionState::Closed);
        assert_eq!(link.b.state(), SessionState::Closed);
    }

    #[test]
    fn a_requester_cannot_tell_why_it_was_turned_away() {
        // T-ORACLE-1 from the other end of the stream. A real requester
        // runs against a side that does not know it, declined it or
        // blocked it. Everything the requester is told to do, and the state
        // it ends in, is the same in the three cases.
        let reference = Link::connect(Standing::Requested, Standing::None);
        assert_eq!(
            reference.a_actions,
            vec![Action::SendContactRequest, Action::Disconnect]
        );
        assert_eq!(
            reference.b_actions,
            vec![Action::SendClose, Action::ConsiderRequest]
        );
        for standing in [Standing::Declined, Standing::Blocked] {
            let link = Link::connect(Standing::Requested, standing);
            assert_eq!(link.a_actions, reference.a_actions, "{standing:?}");
            assert_eq!(link.a.state(), reference.a.state());
            assert_eq!(link.a.standing(), Standing::Requested);
            assert_eq!(link.b_actions, vec![Action::SendClose]);
        }
    }

    #[test]
    fn every_pair_of_standings_ends_consistently() {
        // A session is confirmed on both sides or on neither, and exactly
        // when each side holds the other as requested or accepted.
        for a_holds_b in ALL_STANDINGS {
            for b_holds_a in ALL_STANDINGS {
                let link = Link::connect(a_holds_b, b_holds_a);
                let pair = format!("{a_holds_b:?} / {b_holds_a:?}");
                let confirmed =
                    |session: &Session| session.state() == SessionState::AuthenticatedContact;
                let expected = a_holds_b.is_contact_record() && b_holds_a.is_contact_record();
                assert_eq!(confirmed(&link.a), expected, "{pair}");
                assert_eq!(confirmed(&link.b), expected, "{pair}");

                for (actions, held) in [(&link.a_actions, a_holds_b), (&link.b_actions, b_holds_a)]
                {
                    assert!(!actions.contains(&Action::Deliver), "{pair}");
                    assert_eq!(actions.contains(&Action::Confirmed), expected, "{pair}");
                    // Nobody becomes a contact that the user did not ask
                    // for, and nobody becomes one against the other side.
                    assert_eq!(
                        actions.contains(&Action::MarkAccepted),
                        expected && held == Standing::Requested,
                        "{pair}"
                    );
                }
            }
        }
    }

    #[test]
    fn states_are_entered_in_order() {
        let mut session = inbound();
        assert_eq!(session.state(), SessionState::Connecting);
        assert!(session.handshake_completed().is_err());
        assert!(
            session
                .identities_proven(peer(), Standing::Accepted)
                .is_err()
        );
        session.stream_established().unwrap();
        assert!(session.stream_established().is_err());
        session.handshake_completed().unwrap();
        assert_eq!(session.state(), SessionState::IdentityAuth);
        assert_eq!(session.peer(), None);
    }

    #[test]
    fn a_session_cannot_be_authenticated_without_a_proof() {
        let mut session = inbound();
        session.stream_established().unwrap();
        session.handshake_completed().unwrap();
        assert_eq!(
            session.identities_proven(peer(), Standing::Accepted),
            Err(ProtocolError::AuthenticationFailed)
        );
        assert_eq!(session.state(), SessionState::Closed);
        assert_eq!(session.peer(), None);
    }

    #[test]
    fn the_proven_identity_must_be_the_one_the_proof_named() {
        let mut session = awaiting_verification();
        assert_eq!(
            session.identities_proven(identity(STRANGER), Standing::Accepted),
            Err(ProtocolError::AuthenticationFailed)
        );
        // The failure ends the session. It cannot be tried again.
        assert_eq!(session.state(), SessionState::Closed);
        assert!(session.identities_proven(peer(), Standing::None).is_err());
        assert_eq!(session.peer(), None);

        let mut session = awaiting_verification();
        assert_eq!(
            session.identities_proven(peer(), Standing::None),
            Ok(Vec::new())
        );
        assert_eq!(session.peer(), Some(&peer()));
        assert_eq!(session.state(), SessionState::AuthenticatedUnknown);
        // Calling it again is refused and changes nothing.
        assert!(
            session
                .identities_proven(peer(), Standing::Accepted)
                .is_err()
        );
        assert_eq!(session.state(), SessionState::AuthenticatedUnknown);
        assert_eq!(session.standing(), Standing::None);
    }

    #[test]
    fn an_outbound_session_accepts_only_the_identity_it_dialed() {
        // PROTOCOL.md 6.2 step 2. The endpoint proves another identity,
        // which the local side even holds as an accepted contact. The
        // session ends; it does not become a session with that contact.
        let outbound = || {
            let mut session = Session::outbound(local(), peer());
            session.stream_established().unwrap();
            session.handshake_completed().unwrap();
            session
        };

        let mut session = outbound();
        assert_eq!(session.initiator(), Initiator::Local);
        assert_eq!(
            session.receive(&sample(MessageType::AuthProof, STRANGER)),
            Err(ProtocolError::IdentityMismatch)
        );
        assert_eq!(session.state(), SessionState::Closed);
        assert!(
            session
                .identities_proven(identity(STRANGER), Standing::Accepted)
                .is_err()
        );
        assert_eq!(session.peer(), None);

        let mut session = outbound();
        assert_eq!(
            session.receive(&from_peer(MessageType::AuthProof)),
            Ok(vec![Action::VerifyIdentityProof])
        );
        assert_eq!(
            session.identities_proven(peer(), Standing::Accepted),
            Ok(vec![Action::SendContactAccept])
        );

        // An inbound session takes whoever proves an identity.
        let mut session = inbound();
        assert_eq!(session.initiator(), Initiator::Remote);
        session.stream_established().unwrap();
        session.handshake_completed().unwrap();
        assert!(
            session
                .receive(&sample(MessageType::AuthProof, STRANGER))
                .is_ok()
        );
    }

    #[test]
    fn a_proof_that_names_the_local_identity_is_refused() {
        for mut session in [inbound(), Session::outbound(local(), local())] {
            session.stream_established().unwrap();
            session.handshake_completed().unwrap();
            assert_eq!(
                session.receive(&sample(MessageType::AuthProof, LOCAL)),
                Err(ProtocolError::AuthenticationFailed)
            );
            assert_eq!(session.state(), SessionState::Closed);
        }
    }

    #[test]
    fn nothing_is_accepted_before_the_identity_proof() {
        // T-PROTO-STATE.
        for message_type in MessageType::ALL {
            let message = from_peer(message_type);

            let mut session = inbound();
            assert!(session.receive(&message).is_err(), "{message_type:?}");
            assert_eq!(session.state(), SessionState::Closed);

            let mut session = inbound();
            session.stream_established().unwrap();
            assert!(session.receive(&message).is_err(), "{message_type:?}");
        }
    }

    #[test]
    fn only_one_identity_proof_is_accepted() {
        let mut session = awaiting_verification();
        assert!(session.receive(&from_peer(MessageType::AuthProof)).is_err());
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
            session.receive(&Message::ContactAccept),
            Ok(vec![Action::Confirmed])
        );
        assert_eq!(session.state(), SessionState::AuthenticatedContact);
        assert_eq!(
            session.receive(&from_peer(MessageType::ChatMessage)),
            Ok(vec![Action::Deliver])
        );
    }

    #[test]
    fn a_request_from_an_accepted_peer_changes_nothing() {
        let (mut session, _) = authenticated(Standing::Accepted);
        assert_eq!(
            session.receive(&from_peer(MessageType::ContactRequest)),
            Ok(Vec::new())
        );
        assert_eq!(session.state(), SessionState::AuthenticatedUnknown);
        assert_eq!(
            session.receive(&Message::ContactAccept),
            Ok(vec![Action::Confirmed])
        );
    }

    #[test]
    fn a_second_request_before_confirmation_is_a_violation() {
        for standing in [Standing::Accepted, Standing::Requested] {
            let (mut session, _) = authenticated(standing);
            let request = from_peer(MessageType::ContactRequest);
            assert!(session.receive(&request).is_ok());
            assert_eq!(
                session.receive(&request),
                Err(ProtocolError::MessageNotPermitted),
                "{standing:?}"
            );
            assert_eq!(session.state(), SessionState::Closed);
        }
    }

    #[test]
    fn a_requested_peer_that_accepts_becomes_a_contact() {
        // T-CONTACT-5.
        let (mut session, _) = authenticated(Standing::Requested);
        assert_eq!(
            session.receive(&Message::ContactAccept),
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
    fn a_stranger_request_is_answered_with_close_and_then_considered() {
        let (mut session, _) = authenticated(Standing::None);
        assert_eq!(
            session.receive(&from_peer(MessageType::ContactRequest)),
            Ok(vec![Action::SendClose, Action::ConsiderRequest])
        );
        assert_eq!(session.state(), SessionState::Closing);
        // Whatever follows is not processed.
        assert_eq!(
            session.receive(&from_peer(MessageType::ChatMessage)),
            Ok(Vec::new())
        );
        assert_eq!(
            session.receive(&from_peer(MessageType::ContactRequest)),
            Ok(Vec::new())
        );
    }

    #[test]
    fn requests_from_declined_and_blocked_peers_are_not_considered() {
        for standing in [Standing::Declined, Standing::Blocked] {
            let (mut session, _) = authenticated(standing);
            assert_eq!(
                session.receive(&from_peer(MessageType::ContactRequest)),
                Ok(vec![Action::SendClose])
            );
        }
    }

    #[test]
    fn a_request_with_a_card_of_another_identity_is_a_violation_for_everyone() {
        // PROTOCOL.md 8.3: the card in a ContactRequest must be the
        // sender's own. The outcome must not depend on the standing, or it
        // would tell the sender something about it.
        let foreign = sample(MessageType::ContactRequest, STRANGER);
        for standing in ALL_STANDINGS {
            let (mut session, _) = authenticated(standing);
            assert_eq!(
                session.receive(&foreign),
                Err(ProtocolError::IdentityMismatch),
                "{standing:?}"
            );
            assert_eq!(session.state(), SessionState::Closed);
            assert_eq!(
                session.standing(),
                standing,
                "a foreign card must not change the standing"
            );
        }
    }

    #[test]
    fn an_endpoint_update_with_a_card_of_another_identity_is_a_violation() {
        let (mut session, _) = authenticated(Standing::Accepted);
        session.receive(&Message::ContactAccept).unwrap();
        assert_eq!(
            session.receive(&from_peer(MessageType::EndpointUpdate)),
            Ok(vec![Action::Deliver])
        );
        assert_eq!(
            session.receive(&sample(MessageType::EndpointUpdate, STRANGER)),
            Err(ProtocolError::IdentityMismatch)
        );
        assert_eq!(session.state(), SessionState::Closed);
    }

    #[test]
    fn oracle_1_non_contacts_are_indistinguishable() {
        // T-ORACLE-1. Unknown, declined and blocked identities, and a deleted
        // former contact (which has no record and is Standing::None), see
        // the same thing for every behavior, including a request that
        // carries someone else's card.
        let request = from_peer(MessageType::ContactRequest);
        let with_invitation = {
            let Message::ContactRequest(mut inner) = request.clone() else {
                panic!("not a contact request");
            };
            inner.invitation = Some(InvitationCapability::from_bytes([9; 16]));
            Message::ContactRequest(inner)
        };
        let foreign = sample(MessageType::ContactRequest, STRANGER);
        let chat = from_peer(MessageType::ChatMessage);
        let behaviors: Vec<Vec<Message>> = vec![
            vec![],
            vec![request.clone()],
            // Whether an invitation is valid is decided after the Close,
            // outside the session. With or without one, the peer sees the
            // same.
            vec![with_invitation],
            vec![Message::ContactAccept],
            vec![Message::Close],
            vec![request.clone(), request.clone()],
            vec![Message::ContactAccept, chat.clone()],
            vec![chat],
            vec![foreign.clone()],
            vec![request, foreign],
        ];
        for behavior in &behaviors {
            let reference = transcript(Standing::None, behavior);
            for standing in NON_CONTACTS {
                assert_eq!(transcript(standing, behavior), reference, "{standing:?}");
            }
        }
    }

    #[test]
    fn oracle_1_the_only_difference_is_invisible() {
        // The request of an unknown peer is considered, that of a declined
        // or blocked peer is not. That action is not visible to the peer,
        // and it comes after the Close.
        assert!(!Action::ConsiderRequest.is_visible_to_peer());
        assert!(!Action::MarkAccepted.is_visible_to_peer());
        assert!(!Action::Confirmed.is_visible_to_peer());
        assert!(!Action::Deliver.is_visible_to_peer());
        assert!(!Action::VerifyIdentityProof.is_visible_to_peer());
        assert!(Action::SendClose.is_visible_to_peer());
        assert!(Action::SendContactAccept.is_visible_to_peer());
        assert!(Action::SendContactRequest.is_visible_to_peer());
        assert!(Action::Disconnect.is_visible_to_peer());
    }

    #[test]
    fn oracle_4_violations_emit_nothing() {
        // T-ORACLE-4. A message that is not legal in the state ends the
        // session with an error and no action, for every standing.
        for standing in ALL_STANDINGS {
            for message_type in MessageType::ALL {
                if message_type.may_be_received_in(SessionState::AuthenticatedUnknown) {
                    continue;
                }
                let (mut session, _) = authenticated(standing);
                assert_eq!(
                    session.receive(&from_peer(message_type)),
                    Err(ProtocolError::MessageNotPermitted),
                    "{standing:?} {message_type:?}"
                );
                assert_eq!(session.state(), SessionState::Closed);
            }
        }
    }

    #[test]
    fn oracle_5_nothing_but_confirmation_messages_before_confirmation() {
        // T-ORACLE-5. Before a session is confirmed the local side may send
        // only ContactAccept, ContactRequest and Close, and a message that
        // arrives then is never delivered to the application, not even the
        // one that confirms the session.
        for standing in ALL_STANDINGS {
            let (session, _) = authenticated(standing);
            for message_type in MessageType::ALL {
                let allowed = matches!(
                    message_type,
                    MessageType::ContactAccept | MessageType::ContactRequest | MessageType::Close
                );
                if !allowed {
                    assert!(
                        !session.may_send(message_type),
                        "{standing:?} {message_type:?}"
                    );
                }
            }
            for message_type in [
                MessageType::ContactRequest,
                MessageType::ContactAccept,
                MessageType::Close,
            ] {
                let (mut session, _) = authenticated(standing);
                let actions = session.receive(&from_peer(message_type)).unwrap();
                assert!(
                    !actions.contains(&Action::Deliver),
                    "{standing:?} {message_type:?}"
                );
            }
        }
    }

    #[test]
    fn may_send_in_every_state() {
        let each = |session: &Session| -> Vec<MessageType> {
            MessageType::ALL
                .into_iter()
                .filter(|message_type| session.may_send(*message_type))
                .collect()
        };

        let mut session = inbound();
        assert_eq!(each(&session), Vec::new(), "Connecting");
        session.stream_established().unwrap();
        assert_eq!(each(&session), Vec::new(), "CryptoHandshake");
        session.handshake_completed().unwrap();
        assert_eq!(each(&session), vec![MessageType::AuthProof], "IdentityAuth");

        let (session, _) = authenticated(Standing::Accepted);
        assert_eq!(
            each(&session),
            vec![MessageType::Close, MessageType::ContactAccept]
        );
        let (session, _) = authenticated(Standing::Requested);
        assert_eq!(
            each(&session),
            vec![MessageType::Close, MessageType::ContactRequest]
        );
        for standing in NON_CONTACTS {
            let (session, _) = authenticated(standing);
            assert_eq!(each(&session), vec![MessageType::Close], "{standing:?}");
        }

        let (mut session, _) = authenticated(Standing::Accepted);
        session.receive(&Message::ContactAccept).unwrap();
        let confirmed = each(&session);
        assert!(!confirmed.contains(&MessageType::AuthProof));
        assert!(!confirmed.contains(&MessageType::ContactRequest));
        assert_eq!(confirmed.len(), MessageType::ALL.len() - 2);

        assert_eq!(session.close(), vec![Action::SendClose]);
        assert_eq!(each(&session), vec![MessageType::Close], "Closing");
        session.stream_closed();
        assert_eq!(each(&session), Vec::new(), "Closed");
    }

    #[test]
    fn every_emitted_send_is_permitted_by_may_send() {
        // Whatever the session tells the caller to send, may_send allows at
        // that moment.
        let permitted = |session: &Session, actions: &[Action]| {
            for action in actions {
                let message_type = match action {
                    Action::SendContactAccept => MessageType::ContactAccept,
                    Action::SendContactRequest => MessageType::ContactRequest,
                    Action::SendClose => MessageType::Close,
                    _ => continue,
                };
                assert!(
                    session.may_send(message_type),
                    "{action:?} in {:?}",
                    session.state()
                );
            }
        };
        for standing in ALL_STANDINGS {
            let (session, first) = authenticated(standing);
            permitted(&session, &first);
            for message_type in [
                MessageType::ContactRequest,
                MessageType::ContactAccept,
                MessageType::Close,
            ] {
                let (mut session, _) = authenticated(standing);
                let actions = session.receive(&from_peer(message_type)).unwrap();
                permitted(&session, &actions);
                let actions = session.close();
                permitted(&session, &actions);
            }
            let (mut session, _) = authenticated(standing);
            let actions = session.peer_blocked();
            permitted(&session, &actions);
        }
    }

    #[test]
    fn what_one_side_may_send_the_other_side_accepts() {
        // For every pair of standings, before confirmation: a message that
        // one side may send is legal for the other side to receive.
        for sender_standing in ALL_STANDINGS {
            let (sender, _) = authenticated(sender_standing);
            for message_type in MessageType::ALL {
                if sender.may_send(message_type) {
                    assert!(
                        message_type.may_be_received_in(SessionState::AuthenticatedUnknown),
                        "{sender_standing:?} {message_type:?}"
                    );
                }
            }
        }
        // After confirmation: everything a confirmed side may send is legal
        // in a confirmed session, and ContactAccept and Close are also
        // legal for a peer that is not confirmed yet.
        let (mut sender, _) = authenticated(Standing::Accepted);
        sender.receive(&Message::ContactAccept).unwrap();
        for message_type in MessageType::ALL {
            if sender.may_send(message_type) {
                assert!(message_type.may_be_received_in(SessionState::AuthenticatedContact));
            }
        }
    }

    #[test]
    fn confirmed_sessions_ignore_late_confirmation_messages() {
        let (mut session, _) = authenticated(Standing::Accepted);
        session.receive(&Message::ContactAccept).unwrap();
        assert_eq!(session.receive(&Message::ContactAccept), Ok(Vec::new()));
        assert_eq!(
            session.receive(&from_peer(MessageType::ContactRequest)),
            Ok(Vec::new())
        );
        assert!(session.receive(&from_peer(MessageType::AuthProof)).is_err());
    }

    #[test]
    fn close_from_the_peer_ends_the_session() {
        for standing in [Standing::None, Standing::Accepted] {
            let (mut session, _) = authenticated(standing);
            assert_eq!(
                session.receive(&Message::Close),
                Ok(vec![Action::Disconnect])
            );
            assert_eq!(session.state(), SessionState::Closed);
            assert_eq!(
                session.receive(&from_peer(MessageType::ChatMessage)),
                Ok(Vec::new())
            );
        }
    }

    #[test]
    fn local_close_in_every_state() {
        let mut session = inbound();
        assert_eq!(session.close(), vec![Action::Disconnect]);
        assert_eq!(session.state(), SessionState::Closed);
        assert_eq!(session.close(), Vec::new());

        let mut session = inbound();
        session.stream_established().unwrap();
        assert_eq!(session.close(), vec![Action::Disconnect]);

        // Before the identities are proven nothing is sent, not even Close.
        let mut session = awaiting_verification();
        assert_eq!(session.close(), vec![Action::Disconnect]);
        assert_eq!(session.state(), SessionState::Closed);

        let (mut session, _) = authenticated(Standing::None);
        assert_eq!(session.close(), vec![Action::SendClose]);
        assert_eq!(session.state(), SessionState::Closing);
        assert_eq!(session.close(), Vec::new());
        session.stream_closed();
        assert_eq!(session.state(), SessionState::Closed);

        let (mut session, _) = authenticated(Standing::Accepted);
        session.receive(&Message::ContactAccept).unwrap();
        assert_eq!(session.close(), vec![Action::SendClose]);
        assert_eq!(session.state(), SessionState::Closing);
    }

    #[test]
    fn blocking_or_removing_a_contact_looks_like_any_other_close() {
        // First half of T-ORACLE-7.
        let confirmed = || {
            let (mut session, _) = authenticated(Standing::Accepted);
            session.receive(&Message::ContactAccept).unwrap();
            session
        };
        let mut quit = confirmed();
        let reference = (quit.close(), quit.state());

        let mut blocked = confirmed();
        assert_eq!((blocked.peer_blocked(), blocked.state()), reference);
        assert_eq!(blocked.standing(), Standing::Blocked);

        let mut removed = confirmed();
        assert_eq!((removed.contact_removed(), removed.state()), reference);
        assert_eq!(removed.standing(), Standing::None);

        // Nothing is delivered afterwards.
        assert_eq!(
            removed.receive(&from_peer(MessageType::ChatMessage)),
            Ok(Vec::new())
        );
    }

    #[test]
    fn the_session_never_moves_backwards() {
        // Drive sessions with every pair of messages and check that each
        // step is a legal transition or no transition at all.
        let messages: Vec<Message> = MessageType::ALL.into_iter().map(from_peer).collect();
        for standing in ALL_STANDINGS {
            for first in &messages {
                for second in &messages {
                    let (mut session, _) = authenticated(standing);
                    for message in [first, second] {
                        let before = session.state();
                        let result = session.receive(message);
                        let after = session.state();
                        assert!(
                            before == after || before.can_transition_to(after),
                            "{standing:?}: {before:?} -> {after:?}"
                        );
                        if result.is_ok_and(|actions| actions.contains(&Action::Deliver)) {
                            assert_eq!(before, SessionState::AuthenticatedContact);
                        }
                    }
                }
            }
        }
    }
}

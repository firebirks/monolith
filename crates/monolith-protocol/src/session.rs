//! Session logic: states and contact confirmation.
//!
//! [`Session`] is the state machine of one peer session, as specified in
//! `docs/PROTOCOL.md` sections 6, 7 and 12. It does no I/O and no
//! cryptography. The caller tells it what happened (the stream is up, the
//! handshake finished, the peer is authenticated, a message arrived) and it
//! answers with what to do ([`Action`]).
//!
//! The handshake itself is not here. It lives in the session crate, whose
//! `AuthenticatedSession` owns a `Session` and tells it that a peer is
//! authenticated when a handshake has completed. `Session` is a public
//! type, and code that builds one by hand can tell it anything; such a
//! session has no keys and can neither read nor write a frame. The
//! guarantee that a session exists only for a peer that completed a
//! handshake is that of `AuthenticatedSession`, which is what the rest of
//! Monolith holds.
//!
//! Properties that are built into its shape:
//!
//! - It takes decoded messages, so nothing reaches it that has not passed
//!   the frame and body decoders.
//! - It knows the local identity and, for a session the local side opened,
//!   the identity that was dialed. A peer that is the local identity, or on
//!   an outbound session any identity but the dialed one, ends the session.
//! - It knows the card that stands for the peer: the one the peer presented
//!   in the handshake, or the pinned one that was dialed. A card inside a
//!   message that does not belong to that peer is a violation, and that is
//!   checked before the standing of the peer is looked at.
//! - It decides from the message and the local [`Standing`] of the peer
//!   alone. Whether the user has verified a contact out of band is not an
//!   input, so it cannot influence anything a peer observes.
//! - For a peer that is not a contact, what is sent back does not depend on
//!   why it is not a contact. Unknown, declined and blocked identities, and
//!   contacts that presented a stale card, take the same path; the only
//!   difference is an internal action that the peer cannot see. See
//!   `docs/PROTOCOL.md` section 12.1.
//!
//! After a Close was sent or received the caller stops reading from the
//! stream. Bytes still in flight are dropped; they are not fed to the frame
//! decoder, which refuses every frame in those states.

use monolith_identity::IdentityPublicKey;

use crate::body::Message;
use crate::card::{ContactCard, InvitationCapability};
use crate::credential::{CredentialChange, Credentials};
use crate::duplicate::Initiator;
use crate::{MessageType, ProtocolError, SessionState};

/// The standing of an authenticated peer for one session.
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
    /// The identity is held as a requested or accepted contact, but the
    /// card that stands for it on this session does not stand for the
    /// contact: a card of another transport key than the active one that
    /// is older than the active card or than the authorized successor, a
    /// card that states the retired transport key, or a card with the
    /// epoch of the active card or of the authorized successor that states
    /// something else. An older card of the active
    /// key is not one of them. For this session the peer is not a contact
    /// (`docs/PROTOCOL.md` section 6.2). A session whose transport key was
    /// retired while it was open ends with this standing as well.
    StaleCard,
    /// The identity is held as a requested or accepted contact, and the
    /// peer proved a newer transport key that did not come through the
    /// active one. The key is held as the pending successor. For this
    /// session the peer is not a contact; it may become one after the user
    /// confirms the key (`docs/PROTOCOL.md` section 11.4).
    PendingSuccessor,
    /// The user imported this identity's card and has not seen an acceptance.
    Requested,
    /// An accepted contact.
    Accepted,
}

impl Standing {
    /// Returns true for the standings that are counted against the contact
    /// session budget: identities the user chose to have as contacts, on a
    /// session that can become a contact session.
    pub const fn is_contact_record(self) -> bool {
        matches!(self, Self::Requested | Self::Accepted)
    }

    /// Returns true if a peer with this standing may learn the local
    /// identity: it stands for an identity the local side holds as a
    /// requested or accepted contact. See
    /// [`Admission::may_learn_local_identity`].
    pub const fn may_learn_local_identity(self) -> bool {
        self.is_contact_record()
    }
}

/// What the local side holds about an identity. For a contact that
/// includes its [`Credentials`]: which transport key stands for it, and
/// its successors.
///
/// The record of a contact is borrowed mutably. Deciding the standing of
/// a session is also the moment a promoted successor or a newer card is
/// recorded, in the same call, so that the two cannot be separated. The
/// caller looks the record up, admits, and keeps what changed as one step
/// on its contact state, with nothing in between that waits.
#[derive(Debug)]
pub enum PeerRecord<'a> {
    /// No record of this identity.
    None,
    /// The user declined a request from this identity.
    Declined,
    /// The user blocked this identity.
    Blocked,
    /// The user imported a card of this identity and has not seen an
    /// acceptance.
    Requested(&'a mut Credentials),
    /// An accepted contact.
    Accepted(&'a mut Credentials),
}

/// The standing of a peer that presented a card in the handshake, and what
/// that card did to the credentials of the contact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Admission {
    /// The standing of the peer for this session.
    pub standing: Standing,
    /// For an identity held as a contact: what the card of the session did
    /// to its credentials. [`CredentialChange::Promoted`] means that
    /// sessions authenticated with the previous transport key no longer
    /// stand for the contact and have to be withdrawn.
    /// [`CredentialChange::Pending`] and [`CredentialChange::Conflict`]
    /// are for the user. Nothing of this is visible to the peer.
    pub change: Option<CredentialChange>,
}

impl PeerRecord<'_> {
    /// Decides the standing of a peer for one session and records what the
    /// card means for the contact. `card` is the card that stands for the
    /// peer: the card it presented in the handshake of an inbound session,
    /// or the card that an outbound session dialed. The handshake proved
    /// that the peer holds its transport key. This is the table of
    /// `docs/PROTOCOL.md` section 6.2; [`Credentials::admit`] does the
    /// comparison.
    ///
    /// The active key, with the active card or a newer one, and a proven
    /// authorized successor give the standing of the record. A newer key
    /// without continuity gives [`Standing::PendingSuccessor`]. An older or
    /// contradicting card, or the retired key, gives
    /// [`Standing::StaleCard`], whatever the record says.
    ///
    /// Fails with [`ProtocolError::IdentityMismatch`] if the record belongs
    /// to another identity than the card.
    pub fn admit(&mut self, card: &ContactCard) -> Result<Admission, ProtocolError> {
        let (credentials, as_recorded) = match self {
            Self::None => return Ok(Admission::without_record(Standing::None)),
            Self::Declined => return Ok(Admission::without_record(Standing::Declined)),
            Self::Blocked => return Ok(Admission::without_record(Standing::Blocked)),
            Self::Requested(credentials) => (&mut **credentials, Standing::Requested),
            Self::Accepted(credentials) => (&mut **credentials, Standing::Accepted),
        };
        let change = credentials.admit(card)?;
        let standing = match change {
            CredentialChange::Unchanged
            | CredentialChange::Superseded
            | CredentialChange::Advanced
            | CredentialChange::Promoted => as_recorded,
            CredentialChange::Pending => Standing::PendingSuccessor,
            CredentialChange::Conflict
            | CredentialChange::Stale
            | CredentialChange::Authorized
            | CredentialChange::NoContinuity => Standing::StaleCard,
        };
        Ok(Admission {
            standing,
            change: Some(change),
        })
    }

    /// Returns the capability a contact request to this identity carries,
    /// for a record that has one.
    pub const fn invitation(&self) -> Option<&InvitationCapability> {
        match self {
            Self::Requested(credentials) | Self::Accepted(credentials) => credentials.invitation(),
            Self::None | Self::Declined | Self::Blocked => None,
        }
    }

    /// The user imported `card` by hand for this identity. For a requested
    /// contact the card takes the place of the held one
    /// ([`Credentials::replace`]); for an accepted contact a new transport
    /// key is only held as the pending successor ([`Credentials::import`]).
    ///
    /// Fails with [`ProtocolError::InvalidValue`] for an identity that is
    /// not held as a contact: importing its card makes a new record, which
    /// is the caller's. Fails with [`ProtocolError::IdentityMismatch`] for
    /// a card of another identity.
    pub fn import(&mut self, card: ContactCard) -> Result<CredentialChange, ProtocolError> {
        match self {
            Self::Requested(credentials) => credentials.replace(card),
            Self::Accepted(credentials) => credentials.import(card),
            Self::None | Self::Declined | Self::Blocked => Err(ProtocolError::InvalidValue),
        }
    }
}

impl Admission {
    /// Returns true if the peer may learn the local identity: its proven
    /// transport key stands, as of this admission, for an identity the
    /// local side holds as a requested or accepted contact. That is the
    /// active key, with the active card, an older or a newer one, and an
    /// authorized successor this admission promoted. It is false for a
    /// pending or retired key, a contradicting card, and for an identity
    /// that is not held as a contact or is declined or blocked.
    ///
    /// An initiator writes message 3, which carries its identity and
    /// card, only when this is true (`docs/PROTOCOL.md` section 4.4).
    pub const fn may_learn_local_identity(&self) -> bool {
        self.standing.may_learn_local_identity()
    }

    const fn without_record(standing: Standing) -> Self {
        Self {
            standing,
            change: None,
        }
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
    /// The local identity. A peer that is this identity is refused.
    local: IdentityPublicKey,
    /// On a session the local side opened: the identity it dialed.
    expected: Option<IdentityPublicKey>,
    /// The card that stands for the authenticated peer. On an inbound
    /// session it is the card the peer presented in the handshake. On an
    /// outbound session it is the pinned card that was dialed.
    peer: Option<ContactCard>,
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
    /// The session can only ever be authenticated as `expected`. Any other
    /// identity ends it with [`ProtocolError::IdentityMismatch`], which the
    /// caller reports to the user and never resolves by itself
    /// (`docs/PROTOCOL.md` section 4.4).
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

    /// Returns the identity of the authenticated peer, once there is one.
    pub fn peer(&self) -> Option<&IdentityPublicKey> {
        self.peer.as_ref().map(ContactCard::identity)
    }

    /// Returns the card that stands for the authenticated peer: the card
    /// it presented in the handshake of an inbound session, or the pinned
    /// card that an outbound session dialed.
    pub const fn peer_card(&self) -> Option<&ContactCard> {
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

    /// The handshake authenticated the peer.
    ///
    /// `card` is the card that stands for the peer. On an inbound session
    /// it is the card the peer presented in the third handshake message,
    /// after the checks of `docs/PROTOCOL.md` section 4.4. On an outbound
    /// session it is the pinned card that was dialed: the responder proved
    /// that it holds the transport key of that card. `standing` is what the
    /// local side decided for this session (section 6.2).
    ///
    /// The session ends if the identity of the card is the local one, or,
    /// on an outbound session, any identity but the one that was dialed. An
    /// inbound card that carries an invitation capability ends it as well:
    /// the card of the handshake never has one.
    ///
    /// The session enters `AuthenticatedUnknown`. The returned actions are
    /// the first message of `docs/PROTOCOL.md` section 6.4.
    pub fn authenticated(
        &mut self,
        card: ContactCard,
        standing: Standing,
    ) -> Result<Vec<Action>, ProtocolError> {
        if self.state != SessionState::IdentityAuth {
            return Err(ProtocolError::MessageNotPermitted);
        }
        if *card.identity() == self.local {
            // Nobody else holds the local identity key, and a session with
            // oneself is not a session.
            return self.violation(ProtocolError::AuthenticationFailed);
        }
        match self.expected {
            Some(expected) => {
                if expected != *card.identity() {
                    return self.violation(ProtocolError::IdentityMismatch);
                }
            }
            None => {
                if card.invitation().is_some() {
                    return self.violation(ProtocolError::InvalidValue);
                }
            }
        }
        self.transition(SessionState::AuthenticatedUnknown)?;
        self.peer = Some(card);
        self.standing = standing;
        Ok(match standing {
            Standing::Accepted => vec![Action::SendContactAccept],
            Standing::Requested => vec![Action::SendContactRequest],
            Standing::None
            | Standing::Declined
            | Standing::Blocked
            | Standing::StaleCard
            | Standing::PendingSuccessor => Vec::new(),
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
        if !self.card_belongs_to_peer(message) {
            return self.violation(ProtocolError::IdentityMismatch);
        }
        match self.state {
            SessionState::AuthenticatedUnknown => self.receive_unconfirmed(message_type),
            SessionState::AuthenticatedContact => Ok(self.receive_confirmed(message_type)),
            // The gate above lets nothing through in the remaining states.
            SessionState::Connecting
            | SessionState::CryptoHandshake
            | SessionState::IdentityAuth
            | SessionState::Closing
            | SessionState::Closed => self.violation(ProtocolError::MessageNotPermitted),
        }
    }

    /// Checks the card inside a message against the authenticated peer.
    /// Messages without a card pass.
    ///
    /// The card in a ContactRequest states the identity key and the
    /// transport key that the handshake authenticated; from the initiator
    /// it is byte for byte the card of the third handshake message
    /// (`docs/PROTOCOL.md` section 8.3). The card in an EndpointUpdate is
    /// signed by the peer's identity and may state another transport key:
    /// that is how a new one is announced (section 8.8).
    fn card_belongs_to_peer(&self, message: &Message) -> bool {
        let card = match message {
            Message::ContactRequest(request) => &request.card,
            Message::EndpointUpdate(card) => &**card,
            _ => return true,
        };
        let Some(peer) = &self.peer else {
            return false;
        };
        if card.identity() != peer.identity() {
            return false;
        }
        match message {
            Message::ContactRequest(_) => match self.initiator() {
                Initiator::Remote => card == peer,
                Initiator::Local => card.transport() == peer.transport(),
            },
            _ => true,
        }
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
            // A request is made once per session.
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
            Standing::None
            | Standing::Declined
            | Standing::Blocked
            | Standing::StaleCard
            | Standing::PendingSuccessor => {
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
    /// written. Before the peer is authenticated nothing may be sent: the
    /// handshake messages are not messages of this kind.
    pub const fn may_send(&self, message_type: MessageType) -> bool {
        match self.state {
            SessionState::AuthenticatedUnknown => match message_type {
                MessageType::Close => true,
                MessageType::ContactAccept => matches!(self.standing, Standing::Accepted),
                MessageType::ContactRequest => matches!(self.standing, Standing::Requested),
                _ => false,
            },
            SessionState::AuthenticatedContact => {
                !matches!(message_type, MessageType::ContactRequest)
            }
            SessionState::Closing => matches!(message_type, MessageType::Close),
            SessionState::Connecting
            | SessionState::CryptoHandshake
            | SessionState::IdentityAuth
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

    /// The user deleted the contact, or withdrew a request, while this
    /// session was open. The peer sees the same Close as for any other end
    /// of a session.
    pub fn contact_removed(&mut self) -> Vec<Action> {
        self.standing = Standing::None;
        self.close()
    }

    /// The transport key this session was authenticated with is no longer
    /// the active one of the peer: a successor took over while the session
    /// was open ([`CredentialChange::Promoted`]). The session loses its
    /// standing and ends. Nothing it receives afterwards is delivered. The
    /// peer sees the same Close as for any other end of a session.
    pub fn withdraw(&mut self) -> Vec<Action> {
        self.standing = Standing::StaleCard;
        self.close()
    }

    /// The stream is gone, for whatever reason.
    pub fn stream_closed(&mut self) {
        self.state = SessionState::Closed;
    }
}

#[cfg(test)]
pub(crate) mod testing {
    //! Sample messages and sessions for tests.

    use std::sync::LazyLock;

    use monolith_identity::{
        EndpointEpoch, IdentityPublicKey, IdentitySecretKey, OnionServiceKey, TransportPublicKey,
    };

    use super::{Action, Session, Standing};
    use crate::MessageType;
    use crate::body::{ContactRequest, FileChunk, Message, MessageId, TransferId};
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

    /// A valid X25519 public key that depends on the seed.
    pub(crate) fn transport(seed: u8) -> TransportPublicKey {
        let mut bytes = [seed; 32];
        bytes[31] = 0x40;
        TransportPublicKey::from_bytes(&bytes).unwrap()
    }

    /// A card of the identity of `seed` with the given transport key, epoch
    /// and endpoint.
    pub(crate) fn card_with(
        seed: u8,
        transport_seed: u8,
        epoch: u64,
        endpoint_seed: u8,
    ) -> ContactCard {
        let endpoint = OnionServiceKey::from_bytes(identity(endpoint_seed).as_bytes()).unwrap();
        ContactCard::sign(
            &secret(seed),
            transport(transport_seed),
            EndpointEpoch::new(epoch).unwrap(),
            EndpointSet::single(endpoint),
            None,
        )
        .unwrap()
    }

    /// The card of the identity of `seed` as it is used throughout the
    /// tests: its own transport key, the first epoch, one endpoint.
    pub(crate) fn card(seed: u8) -> ContactCard {
        card_with(seed, seed, 1, seed.wrapping_add(1))
    }

    /// One message of every type as the peer would send it. Built once:
    /// signing is slow in unoptimized test builds.
    static FROM_PEER: LazyLock<Vec<Message>> = LazyLock::new(|| {
        MessageType::ALL
            .into_iter()
            .map(|message_type| sample(message_type, PEER))
            .collect()
    });

    static PEER_CARD: LazyLock<ContactCard> = LazyLock::new(|| card(PEER));
    static LOCAL_IDENTITY: LazyLock<IdentityPublicKey> = LazyLock::new(|| identity(LOCAL));

    /// The card of the peer in session tests.
    pub(crate) fn peer_card() -> ContactCard {
        PEER_CARD.clone()
    }

    /// The identity of the peer in session tests.
    pub(crate) fn peer() -> IdentityPublicKey {
        *PEER_CARD.identity()
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
            MessageType::Close => Message::Close,
            MessageType::Ping => Message::Ping([1; 8]),
            MessageType::Pong => Message::Pong([1; 8]),
            MessageType::ContactRequest => request_with(card(signer)),
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

    /// A contact request that carries the given card.
    pub(crate) fn request_with(card: ContactCard) -> Message {
        Message::ContactRequest(Box::new(ContactRequest {
            card,
            invitation: None,
            display_name: DisplayName::new("Peer").unwrap(),
            introduction: IntroductionText::new("hello").unwrap(),
        }))
    }

    /// A message of the given type as the peer of the session would send it.
    pub(crate) fn from_peer(message_type: MessageType) -> Message {
        FROM_PEER
            .iter()
            .find(|message| message.message_type() == message_type)
            .cloned()
            .unwrap()
    }

    /// An inbound session whose handshake finished and whose peer has not
    /// been authenticated yet.
    pub(crate) fn awaiting_authentication() -> Session {
        let mut session = inbound();
        session.stream_established().unwrap();
        session.handshake_completed().unwrap();
        session
    }

    /// A session in `AuthenticatedUnknown` with a peer of the given
    /// standing, and the actions that authentication produced.
    pub(crate) fn authenticated(standing: Standing) -> (Session, Vec<Action>) {
        let mut session = awaiting_authentication();
        let first = session.authenticated(peer_card(), standing).unwrap();
        (session, first)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{
        LOCAL, PEER, authenticated, awaiting_authentication, card, card_with, from_peer, identity,
        inbound, local, peer, peer_card, request_with, sample,
    };
    use std::collections::VecDeque;

    use super::*;
    use crate::card::InvitationCapability;

    const NON_CONTACTS: [Standing; 5] = [
        Standing::None,
        Standing::Declined,
        Standing::Blocked,
        Standing::StaleCard,
        Standing::PendingSuccessor,
    ];

    const ALL_STANDINGS: [Standing; 7] = [
        Standing::None,
        Standing::Declined,
        Standing::Blocked,
        Standing::StaleCard,
        Standing::PendingSuccessor,
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

            // The responder is authenticated first: the initiator dialed
            // its pinned card. Then the responder takes the card the
            // initiator presented.
            let first = link.a.authenticated(peer_card(), a_holds_b).unwrap();
            link.act(Side::A, first, &mut wire);
            let first = link.b.authenticated(card(LOCAL), b_holds_a).unwrap();
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
        // runs against a side that does not know it, declined it, blocked
        // it, or holds it as a contact and was shown a stale card.
        // Everything the requester is told to do, and the state it ends
        // in, is the same in all cases.
        let reference = Link::connect(Standing::Requested, Standing::None);
        assert_eq!(
            reference.a_actions,
            vec![Action::SendContactRequest, Action::Disconnect]
        );
        assert_eq!(
            reference.b_actions,
            vec![Action::SendClose, Action::ConsiderRequest]
        );
        for standing in [Standing::Declined, Standing::Blocked, Standing::StaleCard] {
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
    fn only_requested_and_accepted_count_as_contact_records() {
        for standing in ALL_STANDINGS {
            assert_eq!(
                standing.is_contact_record(),
                matches!(standing, Standing::Requested | Standing::Accepted),
                "{standing:?}"
            );
        }
    }

    #[test]
    fn states_are_entered_in_order() {
        let mut session = inbound();
        assert_eq!(session.state(), SessionState::Connecting);
        assert!(session.handshake_completed().is_err());
        assert!(
            session
                .authenticated(peer_card(), Standing::Accepted)
                .is_err()
        );
        session.stream_established().unwrap();
        assert!(session.stream_established().is_err());
        assert!(
            session
                .authenticated(peer_card(), Standing::Accepted)
                .is_err()
        );
        assert_eq!(session.state(), SessionState::CryptoHandshake);
        session.handshake_completed().unwrap();
        assert!(session.handshake_completed().is_err());
        assert_eq!(session.state(), SessionState::IdentityAuth);
        assert_eq!(session.peer(), None);
        assert_eq!(session.peer_card(), None);
    }

    #[test]
    fn a_session_is_authenticated_once() {
        let mut session = awaiting_authentication();
        assert_eq!(
            session.authenticated(peer_card(), Standing::None),
            Ok(Vec::new())
        );
        assert_eq!(session.peer(), Some(&peer()));
        assert_eq!(session.peer_card(), Some(&peer_card()));
        assert_eq!(session.state(), SessionState::AuthenticatedUnknown);
        // Calling it again is refused and changes nothing.
        for again in [peer_card(), card(STRANGER)] {
            assert_eq!(
                session.authenticated(again, Standing::Accepted),
                Err(ProtocolError::MessageNotPermitted)
            );
            assert_eq!(session.state(), SessionState::AuthenticatedUnknown);
            assert_eq!(session.standing(), Standing::None);
            assert_eq!(session.peer(), Some(&peer()));
        }
    }

    #[test]
    fn an_outbound_session_accepts_only_the_identity_it_dialed() {
        // PROTOCOL.md 4.4 and 7. The handshake layer reports another
        // identity than the one that was dialed, one that the local side
        // even holds as an accepted contact. The session ends; it does not
        // become a session with that contact.
        let outbound = || {
            let mut session = Session::outbound(local(), peer());
            session.stream_established().unwrap();
            session.handshake_completed().unwrap();
            session
        };

        let mut session = outbound();
        assert_eq!(session.initiator(), Initiator::Local);
        assert_eq!(
            session.authenticated(card(STRANGER), Standing::Accepted),
            Err(ProtocolError::IdentityMismatch)
        );
        assert_eq!(session.state(), SessionState::Closed);
        assert_eq!(session.peer(), None);
        // The failure ends the session. It cannot be tried again.
        assert!(
            session
                .authenticated(peer_card(), Standing::Accepted)
                .is_err()
        );
        assert_eq!(session.peer(), None);

        let mut session = outbound();
        assert_eq!(
            session.authenticated(peer_card(), Standing::Accepted),
            Ok(vec![Action::SendContactAccept])
        );
        assert_eq!(session.peer(), Some(&peer()));

        // An inbound session takes whoever the handshake authenticated.
        let mut session = awaiting_authentication();
        assert_eq!(session.initiator(), Initiator::Remote);
        assert!(
            session
                .authenticated(card(STRANGER), Standing::None)
                .is_ok()
        );
        assert_eq!(session.peer(), Some(&identity(STRANGER)));
    }

    #[test]
    fn the_local_identity_is_refused_as_a_peer() {
        for mut session in [inbound(), Session::outbound(local(), local())] {
            session.stream_established().unwrap();
            session.handshake_completed().unwrap();
            assert_eq!(
                session.authenticated(card(LOCAL), Standing::Accepted),
                Err(ProtocolError::AuthenticationFailed)
            );
            assert_eq!(session.state(), SessionState::Closed);
            assert_eq!(session.peer(), None);
        }
    }

    #[test]
    fn an_inbound_card_with_an_invitation_is_refused() {
        // The card of the third handshake message never carries a
        // capability. A pinned card that was dialed may.
        let with_invitation = |seed: u8| {
            let endpoint = monolith_identity::OnionServiceKey::from_bytes(
                identity(seed.wrapping_add(1)).as_bytes(),
            )
            .unwrap();
            ContactCard::sign(
                &testing::secret(seed),
                testing::transport(seed),
                monolith_identity::EndpointEpoch::FIRST,
                crate::card::EndpointSet::single(endpoint),
                Some(InvitationCapability::from_bytes([9; 16])),
            )
            .unwrap()
        };

        let mut session = awaiting_authentication();
        assert_eq!(
            session.authenticated(with_invitation(PEER), Standing::Accepted),
            Err(ProtocolError::InvalidValue)
        );
        assert_eq!(session.state(), SessionState::Closed);
        assert_eq!(session.peer(), None);

        let mut session = Session::outbound(local(), peer());
        session.stream_established().unwrap();
        session.handshake_completed().unwrap();
        assert_eq!(
            session.authenticated(with_invitation(PEER), Standing::Requested),
            Ok(vec![Action::SendContactRequest])
        );
    }

    #[test]
    fn nothing_is_accepted_before_authentication() {
        // T-PROTO-STATE. No message type is legal before the handshake
        // authenticated the peer, whatever stage the session is in.
        for message_type in MessageType::ALL {
            let message = from_peer(message_type);
            for steps in 0..=2 {
                let mut session = inbound();
                if steps >= 1 {
                    session.stream_established().unwrap();
                }
                if steps >= 2 {
                    session.handshake_completed().unwrap();
                }
                assert_eq!(
                    session.receive(&message),
                    Err(ProtocolError::MessageNotPermitted),
                    "{message_type:?} after {steps} steps"
                );
                assert_eq!(session.state(), SessionState::Closed);
                assert_eq!(session.peer(), None);
            }
        }
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
    fn requests_from_other_non_contacts_are_not_considered() {
        // A declined or blocked identity is not asked about again. A
        // contact that presented a stale card has a record already.
        for standing in [Standing::Declined, Standing::Blocked, Standing::StaleCard] {
            let (mut session, _) = authenticated(standing);
            assert_eq!(
                session.receive(&from_peer(MessageType::ContactRequest)),
                Ok(vec![Action::SendClose]),
                "{standing:?}"
            );
            assert_eq!(session.state(), SessionState::Closing);
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
    fn a_request_from_the_initiator_carries_the_card_of_the_handshake() {
        // PROTOCOL.md 8.3. The peer opened the session and presented
        // card(PEER). A request with any other card of the same identity,
        // validly signed, is a violation, whatever the standing: another
        // transport key, another epoch, another endpoint.
        let others = [
            card_with(PEER, 0x52, 1, PEER + 1),
            card_with(PEER, PEER, 2, PEER + 1),
            card_with(PEER, PEER, 1, PEER + 2),
        ];
        for other in others {
            assert_eq!(other.identity(), &peer());
            assert_ne!(other, peer_card());
            for standing in ALL_STANDINGS {
                let (mut session, _) = authenticated(standing);
                assert_eq!(
                    session.receive(&request_with(other.clone())),
                    Err(ProtocolError::IdentityMismatch),
                    "{standing:?}"
                );
                assert_eq!(session.state(), SessionState::Closed);
            }
        }
        // The card of the handshake itself is accepted.
        let (mut session, _) = authenticated(Standing::Requested);
        assert!(session.receive(&request_with(peer_card())).is_ok());
    }

    #[test]
    fn a_request_from_the_responder_states_the_authenticated_transport_key() {
        // The local side dialed card(PEER). The responder proved that it
        // holds the transport key of that card, so a request from it must
        // state that key. Its card may be a later one of the same identity
        // with the same key.
        let dialed = || {
            let mut session = Session::outbound(local(), peer());
            session.stream_established().unwrap();
            session.handshake_completed().unwrap();
            session
                .authenticated(peer_card(), Standing::Requested)
                .unwrap();
            session
        };

        let mut session = dialed();
        assert_eq!(
            session.receive(&request_with(card_with(PEER, 0x52, 1, PEER + 1))),
            Err(ProtocolError::IdentityMismatch)
        );
        assert_eq!(session.state(), SessionState::Closed);

        let mut session = dialed();
        assert_eq!(
            session.receive(&request_with(card_with(PEER, PEER, 2, PEER + 2))),
            Ok(vec![Action::MarkAccepted, Action::SendContactAccept])
        );

        let mut session = dialed();
        assert_eq!(
            session.receive(&request_with(card(STRANGER))),
            Err(ProtocolError::IdentityMismatch)
        );
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
    fn an_endpoint_update_may_announce_another_transport_key() {
        // PROTOCOL.md 8.8. The card is the sender's own and states a new
        // transport key under a greater epoch. That is not a violation;
        // what it means for the contact is decided by
        // Credentials::announce.
        let (mut session, _) = authenticated(Standing::Accepted);
        session.receive(&Message::ContactAccept).unwrap();
        let announcement = card_with(PEER, 0x52, 2, PEER + 3);
        assert_eq!(
            session.receive(&Message::EndpointUpdate(Box::new(announcement))),
            Ok(vec![Action::Deliver])
        );
        // The session keeps the card it was authenticated with.
        assert_eq!(session.peer_card(), Some(&peer_card()));
    }

    #[test]
    fn standing_of_an_inbound_peer_follows_the_record_and_the_card() {
        // The table of PROTOCOL.md 6.2.
        let active = card_with(PEER, PEER, 5, PEER + 1);
        let same = card_with(PEER, PEER, 5, PEER + 1);
        let new_key = card_with(PEER, 0x52, 6, PEER + 2);
        let moved = card_with(PEER, PEER, 6, PEER + 2);
        let other_key = card_with(PEER, 0x52, 5, PEER + 1);
        let other_endpoint = card_with(PEER, PEER, 5, PEER + 2);
        let older = card_with(PEER, PEER, 4, PEER + 1);
        let older_other_key = card_with(PEER, 0x52, 4, PEER + 1);

        // Without a contact record the card is not compared with anything.
        for mut record in [PeerRecord::None, PeerRecord::Declined, PeerRecord::Blocked] {
            let standing = match record {
                PeerRecord::None => Standing::None,
                PeerRecord::Declined => Standing::Declined,
                _ => Standing::Blocked,
            };
            for presented in [&same, &new_key, &other_key, &older] {
                assert_eq!(
                    record.admit(presented),
                    Ok(Admission {
                        standing,
                        change: None
                    })
                );
            }
            assert_eq!(record.invitation(), None);
        }

        for as_recorded in [Standing::Requested, Standing::Accepted] {
            let admit = |presented: &ContactCard| {
                let mut credentials = Credentials::new(active.clone());
                let mut record = if as_recorded == Standing::Requested {
                    PeerRecord::Requested(&mut credentials)
                } else {
                    PeerRecord::Accepted(&mut credentials)
                };
                let admission = record.admit(presented);
                (admission, credentials)
            };
            let expect = |standing, change| {
                Ok(Admission {
                    standing,
                    change: Some(change),
                })
            };
            assert_eq!(
                admit(&same).0,
                expect(as_recorded, CredentialChange::Unchanged)
            );
            // A newer card with the active key: the contact, and the card
            // is the active one from now on.
            let (admission, credentials) = admit(&moved);
            assert_eq!(admission, expect(as_recorded, CredentialChange::Advanced));
            assert_eq!(credentials.active(), &moved);
            // A newer key that did not come through the active one: not a
            // contact for this session, and the key is only pending.
            let (admission, credentials) = admit(&new_key);
            assert_eq!(
                admission,
                expect(Standing::PendingSuccessor, CredentialChange::Pending)
            );
            assert_eq!(credentials.active(), &active);
            assert_eq!(credentials.pending_successor(), Some(&new_key));
            // The stale-card rule.
            for presented in [&other_key, &other_endpoint] {
                assert_eq!(
                    admit(presented).0,
                    expect(Standing::StaleCard, CredentialChange::Conflict)
                );
            }
            // An older card of the active key is the contact; an older card
            // of another key is not.
            assert_eq!(
                admit(&older).0,
                expect(as_recorded, CredentialChange::Superseded)
            );
            assert_eq!(
                admit(&older_other_key).0,
                expect(Standing::StaleCard, CredentialChange::Stale)
            );
            // A record of another identity is the caller's mistake.
            assert_eq!(
                admit(&card(STRANGER)).0,
                Err(ProtocolError::IdentityMismatch)
            );
        }

        // A successor announced through the active key is proven and
        // keeps the standing of the record.
        let mut credentials = Credentials::new(active.clone());
        credentials.announce(&new_key, &active).unwrap();
        assert_eq!(
            PeerRecord::Accepted(&mut credentials).admit(&new_key),
            Ok(Admission {
                standing: Standing::Accepted,
                change: Some(CredentialChange::Promoted)
            })
        );
        assert_eq!(
            PeerRecord::Accepted(&mut credentials).admit(&active),
            Ok(Admission {
                standing: Standing::StaleCard,
                change: Some(CredentialChange::Stale)
            })
        );
    }

    #[test]
    fn importing_a_card_depends_on_the_relationship() {
        let held = card_with(PEER, PEER, 1, PEER + 1);
        let new_key = card_with(PEER, 0x52, 2, PEER + 1);
        let mut requested = Credentials::new(held.clone());
        assert_eq!(
            PeerRecord::Requested(&mut requested).import(new_key.clone()),
            Ok(CredentialChange::Promoted)
        );
        assert_eq!(requested.active(), &new_key);
        let mut accepted = Credentials::new(held.clone());
        assert_eq!(
            PeerRecord::Accepted(&mut accepted).import(new_key.clone()),
            Ok(CredentialChange::Pending)
        );
        assert_eq!(accepted.active(), &held);
        for mut record in [PeerRecord::None, PeerRecord::Declined, PeerRecord::Blocked] {
            assert_eq!(
                record.import(new_key.clone()),
                Err(ProtocolError::InvalidValue)
            );
        }
    }

    #[test]
    fn a_withdrawn_session_ends_and_delivers_nothing() {
        let (mut session, _) = authenticated(Standing::Accepted);
        session.receive(&Message::ContactAccept).unwrap();
        assert_eq!(session.state(), SessionState::AuthenticatedContact);
        assert_eq!(session.withdraw(), vec![Action::SendClose]);
        assert_eq!(session.standing(), Standing::StaleCard);
        assert_eq!(session.state(), SessionState::Closing);
        for message_type in MessageType::ALL {
            assert_eq!(session.receive(&from_peer(message_type)), Ok(Vec::new()));
            assert_eq!(
                session.may_send(message_type),
                message_type == MessageType::Close
            );
        }
        // Before confirmation as well.
        let (mut session, _) = authenticated(Standing::Requested);
        assert_eq!(session.withdraw(), vec![Action::SendClose]);
        assert_eq!(
            session.receive(&from_peer(MessageType::ChatMessage)),
            Ok(Vec::new())
        );
    }

    #[test]
    fn oracle_1_non_contacts_are_indistinguishable() {
        // T-ORACLE-1. Unknown, declined and blocked identities, a deleted
        // former contact (which has no record and is Standing::None), and a
        // contact that presented a stale card see the same thing for every
        // behavior, including a request that carries someone else's card.
        let request = from_peer(MessageType::ContactRequest);
        let with_invitation = {
            let Message::ContactRequest(mut inner) = request.clone() else {
                panic!("not a contact request");
            };
            inner.invitation = Some(InvitationCapability::from_bytes([9; 16]));
            Message::ContactRequest(inner)
        };
        let foreign = sample(MessageType::ContactRequest, STRANGER);
        let other_card = request_with(card_with(PEER, 0x52, 1, PEER + 1));
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
            vec![other_card],
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
        // The request of an unknown peer is considered, that of the other
        // peers that are not contacts is not. That action is not visible
        // to the peer, and it comes after the Close.
        assert!(!Action::ConsiderRequest.is_visible_to_peer());
        assert!(!Action::MarkAccepted.is_visible_to_peer());
        assert!(!Action::Confirmed.is_visible_to_peer());
        assert!(!Action::Deliver.is_visible_to_peer());
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
        assert_eq!(each(&session), Vec::new(), "IdentityAuth");

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
        assert!(!confirmed.contains(&MessageType::ContactRequest));
        assert_eq!(confirmed.len(), MessageType::ALL.len() - 1);

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
        assert_eq!(session.state(), SessionState::AuthenticatedContact);
        // A late request is still checked against the handshake.
        assert_eq!(
            session.receive(&request_with(card_with(PEER, 0x52, 1, PEER + 1))),
            Err(ProtocolError::IdentityMismatch)
        );
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

        // Before the peer is authenticated nothing is sent, not even Close.
        let mut session = awaiting_authentication();
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

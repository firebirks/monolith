//! The handshake: `Noise_XK_25519_ChaChaPoly_SHA256` with the checks of
//! `docs/PROTOCOL.md` section 4.
//!
//! Each step of the handshake is a type of its own, and each step consumes
//! the one before it:
//!
//! ```text
//! initiator   HandshakeInitiator --message 2--> OutboundPeer --record--> message 3, AuthenticatedSession
//! responder   HandshakeResponder --message 1--> HandshakeResponderFinal
//!                                --message 3--> InboundPeer  --record--> AuthenticatedSession
//! ```
//!
//! Message 3 carries the local identity and card. It exists only after the
//! admission of the responder allowed it: [`OutboundPeer::admit`] writes it
//! for a standing that may learn the local identity, and for any other
//! standing returns no message 3 and no session. No function returns it
//! before that decision.
//!
//! A handshake object has no function that sends or receives a frame, so
//! no application data can be exchanged before the peer is authenticated.
//! A step that fails is gone: there is nothing left to try again with, and
//! nothing is sent to the peer. Every message has a fixed size and is taken
//! as an array of exactly that size.
//!
//! This file contains no cryptography. The Noise library runs the
//! handshake; what is here is the choice of its parameters and the checks
//! that bind its result to Monolith identities (rules F1 to F5 of
//! `docs/adr/0002-session-protocol.md`).

use core::fmt;
use std::time::Instant;

use monolith_identity::IdentityPublicKey;
use monolith_identity::redact::REDACTED;
use monolith_protocol::ProtocolError;
use monolith_protocol::card::ContactCard;
use monolith_protocol::limits::{
    CONTACT_CARD_BASE_LEN, HANDSHAKE_MSG1_LEN, HANDSHAKE_MSG2_LEN, HANDSHAKE_MSG3_LEN,
    HANDSHAKE_PROLOGUE_LEN, HANDSHAKE_TIMEOUT, SESSION_LABEL_LEN,
};
use monolith_protocol::session::{Action, Admission, PeerRecord, Session};

use crate::resolver::Resolver;
use crate::session::{AuthenticatedSession, Established, SessionLimits};
use crate::{LocalParty, SessionError};

/// What admitting an authenticated peer yields: the session, what the
/// local side decided about the peer, and the first message to send.
pub type Admitted = (AuthenticatedSession, Admission, Vec<Action>);

/// Message 3 of a handshake: the local card, encrypted to the responder.
///
/// Only [`OutboundPeer::admit`] makes one, and only after the admission
/// decided that the responder may learn the local identity
/// (`docs/PROTOCOL.md` section 4.4). Its bytes are written to the stream
/// as they are.
pub struct Message3([u8; HANDSHAKE_MSG3_LEN]);

impl Message3 {
    /// The bytes to write.
    pub const fn as_bytes(&self) -> &[u8; HANDSHAKE_MSG3_LEN] {
        &self.0
    }
}

/// What the admission of a responder yields.
///
/// The granted variant holds a session and is much larger than the other.
/// A value is made once per dial and moved straight into the link, so it
/// is not boxed.
#[derive(Debug)]
#[allow(clippy::large_enum_variant)]
pub enum OutboundAdmission {
    /// The responder's proven key stands for an identity the local side
    /// holds as a requested or accepted contact: the session, the
    /// admission, the first message to send once message 3 is written,
    /// and message 3.
    Granted {
        /// The session. It may be used once message 3 is written.
        session: AuthenticatedSession,
        /// What the local side decided about the responder.
        admission: Admission,
        /// The first message to send.
        first: Vec<Action>,
        /// Message 3, to write before anything else.
        message_3: Message3,
    },
    /// The responder may not learn the local identity. No message 3 was
    /// made and no session exists; the stream is closed with nothing more
    /// sent. The admission says why, for the local side only.
    Refused(Admission),
}

/// The Noise protocol name of `docs/PROTOCOL.md` section 4.1. One suite;
/// nothing is negotiated.
const NOISE_PROTOCOL: &str = "Noise_XK_25519_ChaChaPoly_SHA256";

/// The label at the start of the prologue. It names the protocol and its
/// version; a peer with another label fails the first message.
const SESSION_LABEL: [u8; SESSION_LABEL_LEN] = *b"MONOLITH-SESSION-V1";

/// Builds the prologue: the label and the identity key of the responder.
///
/// The initiator passes the identity it dials, the responder its own. If
/// they differ the first message fails, which is what keeps a card that
/// names another party's transport key from producing a session (rule F2).
fn prologue(responder: &IdentityPublicKey) -> [u8; HANDSHAKE_PROLOGUE_LEN] {
    let mut bytes = [0_u8; HANDSHAKE_PROLOGUE_LEN];
    let content = SESSION_LABEL.iter().chain(responder.as_bytes());
    for (slot, byte) in bytes.iter_mut().zip(content) {
        *slot = *byte;
    }
    bytes
}

fn noise_builder<'a>(resolver: Resolver) -> Result<snow::Builder<'a>, SessionError> {
    let params = NOISE_PROTOCOL.parse().map_err(|_| SessionError::Internal)?;
    Ok(snow::Builder::with_resolver(params, Box::new(resolver)))
}

/// A failure of the Noise library while it builds a state or writes a
/// message. Neither depends on anything the peer sent.
fn local_failure(error: snow::Error) -> SessionError {
    match error {
        snow::Error::Rng => SessionError::Randomness,
        _ => SessionError::Internal,
    }
}

/// Fails once `HANDSHAKE_TIMEOUT` has passed since the handshake began.
fn check_deadline(started: Instant, now: Instant) -> Result<(), SessionError> {
    if now.saturating_duration_since(started) > HANDSHAKE_TIMEOUT {
        return Err(SessionError::Protocol(ProtocolError::TimedOut));
    }
    Ok(())
}

/// Reads the handshake hash and turns the handshake state into the
/// transport state. Called when the last message was written or read.
fn establish(
    noise: snow::HandshakeState,
    mut logic: Session,
    local_card: ContactCard,
    limits: SessionLimits,
    now: Instant,
) -> Result<Established, SessionError> {
    let hash: [u8; 32] = noise
        .get_handshake_hash()
        .try_into()
        .map_err(|_| SessionError::Internal)?;
    let noise = noise
        .into_transport_mode()
        .map_err(|_| SessionError::Internal)?;
    logic
        .handshake_completed()
        .map_err(|_| SessionError::Internal)?;
    Ok(Established {
        noise,
        logic,
        local_card,
        limits,
        at: now,
        hash,
    })
}

/// The initiator's side of a handshake that has sent message 1 and waits
/// for message 2.
pub struct HandshakeInitiator {
    noise: snow::HandshakeState,
    logic: Session,
    /// The pinned card that is dialed.
    remote: ContactCard,
    local_card: ContactCard,
    limits: SessionLimits,
    started: Instant,
}

impl HandshakeInitiator {
    /// Begins a handshake with the identity of the pinned card `remote`
    /// and returns message 1.
    ///
    /// The session can only ever be established with that identity: the
    /// responder has to hold the transport key that the card states, and
    /// has to be that identity by its own account. Message 1 carries
    /// nothing about the initiator.
    ///
    /// Fails with [`SessionError::OwnIdentity`] if the card is one of the
    /// local identity.
    pub fn start(
        local: &LocalParty,
        remote: &ContactCard,
        now: Instant,
    ) -> Result<(Self, [u8; HANDSHAKE_MSG1_LEN]), SessionError> {
        Self::start_with(local, remote, now, Resolver::new())
    }

    /// As [`Self::start`], with a fixed ephemeral key. For known-answer
    /// tests and fuzz targets; it does not exist in any other build.
    #[cfg(any(test, fuzzing))]
    #[doc(hidden)]
    pub fn start_with_ephemeral(
        local: &LocalParty,
        remote: &ContactCard,
        now: Instant,
        ephemeral: [u8; 32],
    ) -> Result<(Self, [u8; HANDSHAKE_MSG1_LEN]), SessionError> {
        Self::start_with(
            local,
            remote,
            now,
            Resolver::with_fixed_ephemeral(ephemeral),
        )
    }

    pub(crate) fn start_with(
        local: &LocalParty,
        remote: &ContactCard,
        now: Instant,
        resolver: Resolver,
    ) -> Result<(Self, [u8; HANDSHAKE_MSG1_LEN]), SessionError> {
        if remote.identity() == local.identity() {
            return Err(SessionError::OwnIdentity);
        }
        let prologue = prologue(remote.identity());
        let mut noise = noise_builder(resolver)?
            .local_private_key(local.transport().expose())
            .map_err(local_failure)?
            .remote_public_key(remote.transport().as_bytes())
            .map_err(local_failure)?
            .prologue(&prologue)
            .map_err(local_failure)?
            .build_initiator()
            .map_err(local_failure)?;

        let mut message = [0_u8; HANDSHAKE_MSG1_LEN];
        let written = noise
            .write_message(&[], &mut message)
            .map_err(local_failure)?;
        if written != HANDSHAKE_MSG1_LEN {
            return Err(SessionError::Internal);
        }

        let mut logic = Session::outbound(*local.identity(), *remote.identity());
        logic
            .stream_established()
            .map_err(|_| SessionError::Internal)?;
        let handshake = Self {
            noise,
            logic,
            remote: remote.clone(),
            local_card: local.card().clone(),
            limits: local.limits(),
            started: now,
        };
        Ok((handshake, message))
    }

    /// Returns the moment at which the handshake times out, or `None` if
    /// the clock cannot represent it.
    pub fn deadline(&self) -> Option<Instant> {
        self.started.checked_add(HANDSHAKE_TIMEOUT)
    }

    /// Takes message 2 and returns the authenticated responder.
    ///
    /// A message 2 that Noise accepts shows that its sender holds the
    /// transport key of the pinned card and used the pinned identity key
    /// in its prologue. Message 3, which carries the local transport key
    /// and the local card, is not made here: [`OutboundPeer::admit`] makes
    /// it, and only for a responder that may learn the local identity.
    ///
    /// Any failure is [`ProtocolError::IdentityMismatch`]: the endpoint did
    /// not prove the identity that was dialed. The caller reports it and
    /// never resolves it by itself; there is no way to continue with the
    /// peer as somebody else. A message that arrives after the handshake
    /// timeout is [`ProtocolError::TimedOut`].
    pub fn read_message_2(
        mut self,
        message: &[u8; HANDSHAKE_MSG2_LEN],
        now: Instant,
    ) -> Result<OutboundPeer, SessionError> {
        check_deadline(self.started, now)?;
        if self.noise.read_message(message, &mut []) != Ok(0) {
            return Err(SessionError::Protocol(ProtocolError::IdentityMismatch));
        }
        Ok(OutboundPeer {
            noise: self.noise,
            logic: self.logic,
            local_card: self.local_card,
            limits: self.limits,
            at: now,
            card: self.remote,
        })
    }
}

/// The responder's side of a handshake that waits for message 1.
pub struct HandshakeResponder {
    noise: snow::HandshakeState,
    logic: Session,
    local_card: ContactCard,
    limits: SessionLimits,
    started: Instant,
}

impl HandshakeResponder {
    /// Prepares to answer a handshake on a stream that a peer opened.
    pub fn new(local: &LocalParty, now: Instant) -> Result<Self, SessionError> {
        Self::new_with(local, now, Resolver::new())
    }

    /// As [`Self::new`], with a fixed ephemeral key. For known-answer
    /// tests and fuzz targets; it does not exist in any other build.
    #[cfg(any(test, fuzzing))]
    #[doc(hidden)]
    pub fn new_with_ephemeral(
        local: &LocalParty,
        now: Instant,
        ephemeral: [u8; 32],
    ) -> Result<Self, SessionError> {
        Self::new_with(local, now, Resolver::with_fixed_ephemeral(ephemeral))
    }

    pub(crate) fn new_with(
        local: &LocalParty,
        now: Instant,
        resolver: Resolver,
    ) -> Result<Self, SessionError> {
        let prologue = prologue(local.identity());
        let noise = noise_builder(resolver)?
            .local_private_key(local.transport().expose())
            .map_err(local_failure)?
            .prologue(&prologue)
            .map_err(local_failure)?
            .build_responder()
            .map_err(local_failure)?;
        let mut logic = Session::inbound(*local.identity());
        logic
            .stream_established()
            .map_err(|_| SessionError::Internal)?;
        Ok(Self {
            noise,
            logic,
            local_card: local.card().clone(),
            limits: local.limits(),
            started: now,
        })
    }

    /// Returns the moment at which the handshake times out, or `None` if
    /// the clock cannot represent it.
    pub fn deadline(&self) -> Option<Instant> {
        self.started.checked_add(HANDSHAKE_TIMEOUT)
    }

    /// Takes message 1 and returns message 2.
    ///
    /// A message 1 that Noise accepts shows that its sender knows the
    /// local transport public key and the local identity public key. Both
    /// are public data: this is not authentication of anybody. A sender
    /// without them gets no reply at all.
    ///
    /// Any failure is [`ProtocolError::HandshakeFailed`], or
    /// [`ProtocolError::TimedOut`] after the handshake timeout. Nothing is
    /// sent.
    pub fn read_message_1(
        mut self,
        message: &[u8; HANDSHAKE_MSG1_LEN],
        now: Instant,
    ) -> Result<(HandshakeResponderFinal, [u8; HANDSHAKE_MSG2_LEN]), SessionError> {
        check_deadline(self.started, now)?;
        if self.noise.read_message(message, &mut []) != Ok(0) {
            return Err(SessionError::Protocol(ProtocolError::HandshakeFailed));
        }

        let mut reply = [0_u8; HANDSHAKE_MSG2_LEN];
        let written = self
            .noise
            .write_message(&[], &mut reply)
            .map_err(local_failure)?;
        if written != HANDSHAKE_MSG2_LEN {
            return Err(SessionError::Internal);
        }
        let waiting = HandshakeResponderFinal {
            noise: self.noise,
            logic: self.logic,
            local_card: self.local_card,
            limits: self.limits,
            started: self.started,
        };
        Ok((waiting, reply))
    }
}

/// The responder's side of a handshake that has sent message 2 and waits
/// for message 3.
pub struct HandshakeResponderFinal {
    noise: snow::HandshakeState,
    logic: Session,
    local_card: ContactCard,
    limits: SessionLimits,
    started: Instant,
}

impl HandshakeResponderFinal {
    /// Returns the moment at which the handshake times out, or `None` if
    /// the clock cannot represent it.
    pub fn deadline(&self) -> Option<Instant> {
        self.started.checked_add(HANDSHAKE_TIMEOUT)
    }

    /// Takes message 3 and returns the authenticated peer.
    ///
    /// The checks are those of `docs/PROTOCOL.md` section 4.4, in that
    /// order: Noise accepts the message, which authenticates the
    /// initiator's transport key; the payload is a valid contact card; the
    /// transport key in the card is the one Noise authenticated; the
    /// identity in the card is not the local one. Nothing else is looked
    /// at here. Contacts and block lists play no part, so the handshake
    /// behaves the same for every initiator with a valid card.
    ///
    /// A message that Noise rejects is [`ProtocolError::HandshakeFailed`].
    /// A card that fails validation gives the error of the card decoder.
    /// A card that is valid and does not belong to the key holder, or is
    /// the local identity's, is [`ProtocolError::AuthenticationFailed`].
    /// The peer cannot tell these apart: nothing is sent.
    pub fn read_message_3(
        mut self,
        message: &[u8; HANDSHAKE_MSG3_LEN],
        now: Instant,
    ) -> Result<InboundPeer, SessionError> {
        check_deadline(self.started, now)?;
        let mut payload = [0_u8; CONTACT_CARD_BASE_LEN];
        if self.noise.read_message(message, &mut payload) != Ok(CONTACT_CARD_BASE_LEN) {
            return Err(SessionError::Protocol(ProtocolError::HandshakeFailed));
        }
        let card = ContactCard::decode(&payload)?;

        // The card says which identity vouches for a transport key. Noise
        // says who holds one. The peer is that identity only if the two
        // keys are the same (rule F3).
        let authenticated = self
            .noise
            .get_remote_static()
            .ok_or(SessionError::Internal)?;
        if authenticated != card.transport().as_bytes() {
            return Err(SessionError::Protocol(ProtocolError::AuthenticationFailed));
        }
        if card.identity() == self.local_card.identity() {
            return Err(SessionError::Protocol(ProtocolError::AuthenticationFailed));
        }

        let established = establish(self.noise, self.logic, self.local_card, self.limits, now)?;
        Ok(InboundPeer { established, card })
    }
}

/// A responder that proved the identity that was dialed. Message 3, which
/// completes the handshake, has not been made yet.
pub struct OutboundPeer {
    noise: snow::HandshakeState,
    logic: Session,
    local_card: ContactCard,
    limits: SessionLimits,
    /// When message 2 was taken.
    at: Instant,
    card: ContactCard,
}

impl OutboundPeer {
    /// Returns the pinned card that was dialed. Its identity is the
    /// identity of the peer.
    pub const fn card(&self) -> &ContactCard {
        &self.card
    }

    /// Decides whether the responder may learn the local identity, and only
    /// then makes message 3 and the session. `record` is what the local
    /// side holds about the identity that was dialed, as it is now: the
    /// caller looks it up after message 2, not before the dial.
    ///
    /// The standing follows from the record and from the card that was
    /// dialed, by the same decision as for an inbound peer
    /// (`docs/PROTOCOL.md` section 6.2), and what the card means for the
    /// contact is recorded in the same step. Normally the dialed card
    /// states the active key and the standing is that of the record. If
    /// the active key changed while the dial was in progress, the
    /// responder has proved a key that is retired, pending or never held,
    /// and the peer is not a contact.
    ///
    /// For a standing that may learn the local identity
    /// ([`Admission::may_learn_local_identity`]) the result is
    /// [`OutboundAdmission::Granted`] with message 3 and the session. For
    /// any other it is [`OutboundAdmission::Refused`]: no message 3 exists,
    /// the handshake state with its keys is dropped, and no session is
    /// made. What the card changed in the record stands either way; a
    /// pending successor is held for the user (`docs/PROTOCOL.md` section
    /// 4.4, check 3).
    ///
    /// Fails with [`ProtocolError::IdentityMismatch`] if the record is one
    /// of another identity.
    pub fn admit(mut self, mut record: PeerRecord<'_>) -> Result<OutboundAdmission, SessionError> {
        let admission = record.admit(&self.card)?;
        if !admission.may_learn_local_identity() {
            return Ok(OutboundAdmission::Refused(admission));
        }
        let mut message_3 = [0_u8; HANDSHAKE_MSG3_LEN];
        let card = self.local_card.encode();
        let written = self
            .noise
            .write_message(&card, &mut message_3)
            .map_err(local_failure)?;
        if written != HANDSHAKE_MSG3_LEN {
            return Err(SessionError::Internal);
        }
        let established = establish(
            self.noise,
            self.logic,
            self.local_card,
            self.limits,
            self.at,
        )?;
        // A request to this peer carries the invitation the user was given
        // for it, which the record keeps (`docs/PROTOCOL.md` section 8.3).
        let invitation = record.invitation().cloned();
        let (session, first) =
            AuthenticatedSession::new(established, self.card, admission.standing, invitation)?;
        Ok(OutboundAdmission::Granted {
            session,
            admission,
            first,
            message_3: Message3(message_3),
        })
    }
}

/// An initiator that presented a valid card of its own and proved that it
/// holds the transport key in it. The handshake is complete; the session
/// exists once the caller has looked up what it holds about that identity.
pub struct InboundPeer {
    established: Established,
    card: ContactCard,
}

impl InboundPeer {
    /// Returns the card the peer presented. Its identity is the identity
    /// of the peer.
    pub const fn card(&self) -> &ContactCard {
        &self.card
    }

    /// Creates the session. `record` is what the local side holds about
    /// the identity of [`Self::card`], as it is now.
    ///
    /// The standing of the peer follows from the record and from the card
    /// it presented (`docs/PROTOCOL.md` section 6.2), and what the card
    /// means for the contact is recorded in the same step: a newer card of
    /// the active key, a promoted successor, a pending one. A contact that
    /// presented a card older than the active one, one that contradicts
    /// it, the retired key, or a new key without continuity, is not a
    /// contact for this session. The returned [`Admission`] says what
    /// changed; with [`CredentialChange::Promoted`] the caller withdraws
    /// the sessions of the retired key before it does anything else with
    /// the contact. All of that is for the local side and is never visible
    /// to the peer. The returned actions are the first message to send, if
    /// any.
    ///
    /// Fails with [`ProtocolError::IdentityMismatch`] if the record is one
    /// of another identity.
    ///
    /// [`CredentialChange::Promoted`]: monolith_protocol::credential::CredentialChange::Promoted
    pub fn admit(self, mut record: PeerRecord<'_>) -> Result<Admitted, SessionError> {
        let admission = record.admit(&self.card)?;
        // The card of the handshake never carries an invitation. A request
        // to this peer carries the one in the card the user was given,
        // which is in the record.
        let invitation = record.invitation().cloned();
        let (session, actions) =
            AuthenticatedSession::new(self.established, self.card, admission.standing, invitation)?;
        Ok((session, admission, actions))
    }
}

macro_rules! redacted_debug {
    ($($name:ident),+) => {
        $(impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), REDACTED)
            }
        })+
    };
}

redacted_debug!(
    Message3,
    HandshakeInitiator,
    HandshakeResponder,
    HandshakeResponderFinal,
    OutboundPeer,
    InboundPeer
);

/// Collects the bytes of one handshake message from a stream that delivers
/// them in pieces.
///
/// It takes exactly `N` bytes and not one more, so that what follows the
/// message on the stream stays there for whoever reads next
/// (`docs/PROTOCOL.md` section 4.5).
pub struct MessageBuffer<const N: usize> {
    bytes: [u8; N],
    filled: usize,
}

impl<const N: usize> MessageBuffer<N> {
    /// Creates an empty buffer.
    pub const fn new() -> Self {
        Self {
            bytes: [0; N],
            filled: 0,
        }
    }

    /// Returns how many bytes are still missing.
    pub const fn missing(&self) -> usize {
        N.saturating_sub(self.filled)
    }

    /// Takes bytes from `input` until the message is complete. Returns how
    /// many were taken and the message, once all `N` bytes are there.
    pub fn feed(&mut self, input: &[u8]) -> (usize, Option<&[u8; N]>) {
        let taken = self.missing().min(input.len());
        let target = self.bytes.iter_mut().skip(self.filled);
        for (slot, byte) in target.zip(input.iter().take(taken)) {
            *slot = *byte;
        }
        self.filled = self.filled.saturating_add(taken);
        let message = (self.filled == N).then_some(&self.bytes);
        (taken, message)
    }
}

impl<const N: usize> Default for MessageBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> fmt::Debug for MessageBuffer<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageBuffer")
            .field("len", &N)
            .field("filled", &self.filled)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_protocol_name_and_the_label_are_those_of_the_specification() {
        assert_eq!(NOISE_PROTOCOL, "Noise_XK_25519_ChaChaPoly_SHA256");
        // The name is as long as a SHA-256 output, so the initial
        // handshake hash is the name itself.
        assert_eq!(NOISE_PROTOCOL.len(), 32);
        assert_eq!(&SESSION_LABEL, b"MONOLITH-SESSION-V1");
        assert_eq!(SESSION_LABEL.len(), 19);
    }

    #[test]
    fn the_prologue_is_the_label_and_the_responder_identity() {
        let identity = crate::testing::identity_secret(7).public_key();
        let bytes = prologue(&identity);
        assert_eq!(bytes.len(), 51);
        assert_eq!(&bytes[..19], b"MONOLITH-SESSION-V1");
        assert_eq!(&bytes[19..], identity.as_bytes());
    }

    #[test]
    fn a_message_buffer_takes_exactly_its_size() {
        let mut buffer = MessageBuffer::<5>::new();
        assert_eq!(buffer.missing(), 5);
        assert_eq!(buffer.feed(&[]), (0, None));
        assert_eq!(buffer.feed(&[1, 2]), (2, None));
        assert_eq!(buffer.missing(), 3);
        // More than is missing: the rest is not taken.
        assert_eq!(buffer.feed(&[3, 4, 5, 6, 7]), (3, Some(&[1, 2, 3, 4, 5])));
        assert_eq!(buffer.missing(), 0);
        // A full buffer takes nothing more.
        assert_eq!(buffer.feed(&[8, 9]), (0, Some(&[1, 2, 3, 4, 5])));

        let mut buffer = MessageBuffer::<3>::default();
        assert_eq!(buffer.feed(&[9, 8, 7]), (3, Some(&[9, 8, 7])));
        let mut buffer = MessageBuffer::<3>::new();
        for (index, byte) in [9_u8, 8, 7].iter().enumerate() {
            let (taken, message) = buffer.feed(core::slice::from_ref(byte));
            assert_eq!(taken, 1);
            assert_eq!(message.is_some(), index == 2);
        }
        assert_eq!(format!("{buffer:?}"), "MessageBuffer { len: 3, filled: 3 }");
    }
}

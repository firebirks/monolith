//! The handshake: `Noise_XK_25519_ChaChaPoly_SHA256` with the checks of
//! `docs/PROTOCOL.md` section 4.
//!
//! Each step of the handshake is a type of its own, and each step consumes
//! the one before it:
//!
//! ```text
//! initiator   HandshakeInitiator --message 2--> OutboundPeer --standing--> AuthenticatedSession
//! responder   HandshakeResponder --message 1--> HandshakeResponderFinal
//!                                --message 3--> InboundPeer  --record----> AuthenticatedSession
//! ```
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
use monolith_protocol::session::{Action, Admission, PeerRecord, Session, Standing};

use crate::resolver::Resolver;
use crate::session::{AuthenticatedSession, Established, SessionLimits};
use crate::{LocalParty, SessionError};

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

    fn start_with(
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

    /// Takes message 2 and returns the authenticated peer and message 3.
    ///
    /// A message 2 that Noise accepts shows that its sender holds the
    /// transport key of the pinned card and used the pinned identity key
    /// in its prologue. Only then is message 3 produced, which carries the
    /// local transport key and the local card, encrypted to that responder.
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
    ) -> Result<(OutboundPeer, [u8; HANDSHAKE_MSG3_LEN]), SessionError> {
        check_deadline(self.started, now)?;
        if self.noise.read_message(message, &mut []) != Ok(0) {
            return Err(SessionError::Protocol(ProtocolError::IdentityMismatch));
        }

        let mut reply = [0_u8; HANDSHAKE_MSG3_LEN];
        let card = self.local_card.encode();
        let written = self
            .noise
            .write_message(&card, &mut reply)
            .map_err(local_failure)?;
        if written != HANDSHAKE_MSG3_LEN {
            return Err(SessionError::Internal);
        }

        let established = establish(self.noise, self.logic, self.local_card, self.limits, now)?;
        let peer = OutboundPeer {
            established,
            card: self.remote,
        };
        Ok((peer, reply))
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

    fn new_with(
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

/// A responder that proved the identity that was dialed. The handshake is
/// complete; the session exists once the caller has said what the local
/// side holds about the peer.
pub struct OutboundPeer {
    established: Established,
    card: ContactCard,
}

impl OutboundPeer {
    /// Returns the pinned card that was dialed. Its identity is the
    /// identity of the peer.
    pub const fn card(&self) -> &ContactCard {
        &self.card
    }

    /// Creates the session. `standing` is the local record of the
    /// identity that was dialed. The returned actions are the first
    /// message to send, if any (`docs/PROTOCOL.md` section 6.4).
    pub fn admit(
        self,
        standing: Standing,
    ) -> Result<(AuthenticatedSession, Vec<Action>), SessionError> {
        AuthenticatedSession::new(self.established, self.card, standing)
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
    /// the identity of [`Self::card`].
    ///
    /// The standing of the peer follows from the record and from the card
    /// it presented (`docs/PROTOCOL.md` section 6.2): a contact that
    /// presented a card older than the pinned one, or one that contradicts
    /// it, is not a contact for this session. The returned [`Admission`]
    /// says how the card compares with the pinned one; that is for the
    /// local side and is never visible to the peer. The returned actions
    /// are the first message to send, if any.
    ///
    /// Fails with [`ProtocolError::IdentityMismatch`] if the record is one
    /// of another identity.
    pub fn admit(
        self,
        record: PeerRecord<'_>,
    ) -> Result<(AuthenticatedSession, Admission, Vec<Action>), SessionError> {
        let admission = record.admit(&self.card)?;
        let (session, actions) =
            AuthenticatedSession::new(self.established, self.card, admission.standing)?;
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

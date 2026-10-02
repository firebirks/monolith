//! An authenticated session: encrypted frames in both directions.
//!
//! [`AuthenticatedSession`] is what a completed handshake yields, and there
//! is no other way to obtain one. It owns the Noise transport state, the
//! frame decoder and the session logic of the protocol core, and it is the
//! only object that turns plaintext messages into frames and back.
//!
//! It does no I/O. The caller feeds it the bytes that arrive on the stream
//! and writes the bytes it returns. Time is an argument of every call that
//! needs it; the session never reads a clock.

use core::fmt;
use core::time::Duration;
use std::time::Instant;

use monolith_identity::IdentityPublicKey;
use monolith_protocol::body::Message;
use monolith_protocol::card::{ContactCard, InvitationCapability};
use monolith_protocol::duplicate::Initiator;
use monolith_protocol::frame::{
    FrameParams, OuterDecoder, decode_plaintext, encode_outer, encode_plaintext,
};
use monolith_protocol::limits::{
    FRAME_READ_TIMEOUT, IDLE_TIMEOUT, MAX_BYTES_PER_DIRECTION, MAX_FRAMES_PER_DIRECTION,
    MAX_SESSION_LIFETIME, MAX_SESSION_LIFETIME_WITH_TRANSFER, SESSION_CLOSE_GRACE,
    UNKNOWN_FIRST_MESSAGE_TIMEOUT, UNKNOWN_SESSION_TIMEOUT,
};
use monolith_protocol::session::{Action, Session, Standing};
use monolith_protocol::{MessageType, ProtocolError, SessionState};

use crate::SessionError;

/// The frame parameters of this build. They are not negotiated.
const PARAMS: FrameParams = FrameParams::PROVISIONAL;

/// When a session has to end, whichever limit is reached first
/// (`docs/PROTOCOL.md` section 7.1).
///
/// The limits are constants of the protocol: both ends of a session have to
/// agree on them, and nothing negotiates them. A product build has
/// [`SessionLimits::PROTOCOL`] and no way to choose others. Tests of this
/// crate run with lower values so that a limit can be reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SessionLimits {
    max_age: Duration,
    max_age_with_transfer: Duration,
    max_frames: u64,
    max_bytes: u64,
}

impl SessionLimits {
    /// The limits of the protocol: 24 hours, 48 hours while a file transfer
    /// is active, 2^32 frames and 2^40 ciphertext bytes in each direction.
    pub(crate) const PROTOCOL: Self = Self {
        max_age: MAX_SESSION_LIFETIME,
        max_age_with_transfer: MAX_SESSION_LIFETIME_WITH_TRANSFER,
        max_frames: MAX_FRAMES_PER_DIRECTION,
        max_bytes: MAX_BYTES_PER_DIRECTION,
    };

    /// Lower limits, for tests. Both ends of a test session get the same.
    #[cfg(test)]
    pub(crate) fn reduced(
        max_age: Duration,
        max_age_with_transfer: Duration,
        max_frames: u64,
        max_bytes: u64,
    ) -> Self {
        assert!(max_age <= MAX_SESSION_LIFETIME && max_age <= max_age_with_transfer);
        assert!(max_age_with_transfer <= MAX_SESSION_LIFETIME_WITH_TRANSFER);
        assert!((1..=MAX_FRAMES_PER_DIRECTION).contains(&max_frames));
        assert!((smallest_frame()..=MAX_BYTES_PER_DIRECTION).contains(&max_bytes));
        Self {
            max_age,
            max_age_with_transfer,
            max_frames,
            max_bytes,
        }
    }
}

/// The ciphertext length of the smallest frame, which is also the length
/// of a Close.
const fn smallest_frame() -> u64 {
    // One padding block and the overhead fit in 16 bits; see FrameParams.
    PARAMS.padding_block().saturating_add(PARAMS.overhead()) as u64
}

/// Frames and ciphertext bytes that went one way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Traffic {
    frames: u64,
    bytes: u64,
}

impl Traffic {
    fn count(&mut self, frame_len: u64) {
        self.frames = self.frames.saturating_add(1);
        self.bytes = self.bytes.saturating_add(frame_len);
    }
}

/// A message that arrived on a session, and what the session logic says
/// has to be done about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Received {
    /// The message. It has passed every check of the frame, of its body
    /// and of the session state.
    pub message: Message,
    /// What to do, in order. See [`Action`].
    pub actions: Vec<Action>,
}

/// What a deadline of [`AuthenticatedSession::deadline`] did to the session
/// when it passed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expiry {
    /// No deadline has passed. Nothing changed.
    Running,
    /// A frame that had begun did not complete within
    /// `FRAME_READ_TIMEOUT`, or no frame completed within `IDLE_TIMEOUT`.
    /// The session is over; nothing is sent and the stream is closed.
    Silent,
    /// The session reached its age limit, or stayed in
    /// `AuthenticatedUnknown` longer than its peer is given there. The
    /// session is over after this Close, if there is one to write.
    Close(Option<Vec<u8>>),
}

/// What a completed handshake hands over to a session.
pub(crate) struct Established {
    pub(crate) noise: snow::TransportState,
    pub(crate) logic: Session,
    pub(crate) local_card: ContactCard,
    pub(crate) limits: SessionLimits,
    pub(crate) at: Instant,
    pub(crate) hash: [u8; 32],
}

/// A session with an authenticated peer.
///
/// The only way to obtain one is to complete a handshake:
/// [`crate::OutboundPeer::admit`] or [`crate::InboundPeer::admit`]. The
/// peer's identity is fixed for the life of the session.
///
/// After any error from [`Self::receive`] the session is over: the stream
/// is closed without sending anything, and every later call fails.
///
/// The cipher keys are dropped, and with that erased, when the session
/// ends: when it fails, when a Close was received, when the local Close
/// was produced, or when the stream is reported closed. They do not wait
/// for the object to be dropped.
pub struct AuthenticatedSession {
    /// The two cipher states. `None` once the session has ended.
    noise: Option<snow::TransportState>,
    logic: Session,
    decoder: OuterDecoder,
    /// The card that stands for the peer. See [`Session::peer_card`].
    peer: ContactCard,
    /// The local card. A ContactRequest that is sent carries exactly this
    /// card, and an EndpointUpdate a card of this identity.
    local_card: ContactCard,
    /// The invitation capability that a ContactRequest to this peer
    /// carries: the one in the card of the peer that the user was given.
    invitation: Option<InvitationCapability>,
    limits: SessionLimits,
    established: Instant,
    transfer_active: bool,
    sent: Traffic,
    received: Traffic,
    hash: [u8; 32],
    /// A ContactAccept was sent on this session.
    accept_sent: bool,
    /// A ContactRequest was sent on this session.
    request_sent: bool,
    /// A Close frame was produced. Nothing is sent after it.
    close_sent: bool,
    /// The session ended with a violation or a failure.
    failed: bool,
    /// When the last complete frame arrived; at first, when the session
    /// was established.
    last_frame: Instant,
    /// When the frame that has begun and is not complete began.
    frame_started: Option<Instant>,
    /// A message from the peer was taken.
    heard: bool,
}

impl AuthenticatedSession {
    /// Turns a completed handshake into a session. `card` stands for the
    /// peer, `standing` is what the local side decided for it, and
    /// `invitation` is the capability a request to this peer has to carry.
    pub(crate) fn new(
        established: Established,
        card: ContactCard,
        standing: Standing,
        invitation: Option<InvitationCapability>,
    ) -> Result<(Self, Vec<Action>), SessionError> {
        let Established {
            noise,
            mut logic,
            local_card,
            limits,
            at,
            hash,
        } = established;
        let actions = logic.authenticated(card.clone(), standing)?;
        let session = Self {
            noise: Some(noise),
            logic,
            decoder: OuterDecoder::new(PARAMS),
            peer: card,
            local_card,
            invitation,
            limits,
            established: at,
            transfer_active: false,
            sent: Traffic::default(),
            received: Traffic::default(),
            hash,
            accept_sent: false,
            request_sent: false,
            close_sent: false,
            failed: false,
            last_frame: at,
            frame_started: None,
            heard: false,
        };
        Ok((session, actions))
    }

    /// Returns the state of the session.
    pub const fn state(&self) -> SessionState {
        self.logic.state()
    }

    /// Returns the standing of the peer.
    pub const fn standing(&self) -> Standing {
        self.logic.standing()
    }

    /// Returns the identity of the peer.
    pub const fn peer(&self) -> &IdentityPublicKey {
        self.peer.identity()
    }

    /// Returns the card that stands for the peer: the card it presented in
    /// the handshake of an inbound session, or the card that an outbound
    /// session dialed. Its transport key is the one the handshake proved,
    /// and with its identity and epoch it says which credential the
    /// session was authenticated with.
    pub const fn peer_card(&self) -> &ContactCard {
        &self.peer
    }

    /// Returns which side opened the session.
    pub const fn initiator(&self) -> Initiator {
        self.logic.initiator()
    }

    /// Returns the handshake hash of the session. Both sides hold the same
    /// value and no other session has it. It is not a secret. Nothing in
    /// version 1 of the protocol uses it.
    pub const fn handshake_hash(&self) -> &[u8; 32] {
        &self.hash
    }

    /// Returns true if a message of this type may be sent now.
    ///
    /// This is what the session logic allows in the current state, and
    /// three rules that depend on what was sent before:
    ///
    /// - Close is not sent with [`Self::send`]; see [`Self::close`].
    /// - A ContactRequest is sent once on a session (`docs/PROTOCOL.md`
    ///   section 6.4).
    /// - On a session that the peer's ContactAccept confirmed, nothing is
    ///   sent before the local ContactAccept. The peer takes application
    ///   messages only after it has seen that one.
    pub const fn may_send(&self, message_type: MessageType) -> bool {
        if self.failed || self.close_sent || !self.logic.may_send(message_type) {
            return false;
        }
        match message_type {
            MessageType::Close => false,
            MessageType::ContactRequest => !self.request_sent,
            MessageType::ContactAccept => true,
            _ => self.accept_sent,
        }
    }

    /// Tells the session whether a file transfer is active. While one is,
    /// the longer lifetime applies (`docs/PROTOCOL.md` section 7.1).
    ///
    /// A transfer that begins after the session has reached its ordinary
    /// lifetime does not extend it: the call returns false and changes
    /// nothing. Ending a transfer always succeeds.
    pub fn set_transfer_active(&mut self, active: bool, now: Instant) -> bool {
        if active && !self.transfer_active && self.age(now) >= self.limits.max_age {
            return false;
        }
        self.transfer_active = active;
        true
    }

    const fn lifetime(&self) -> Duration {
        if self.transfer_active {
            self.limits.max_age_with_transfer
        } else {
            self.limits.max_age
        }
    }

    fn age(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.established)
    }

    /// Returns the moment at which the session reaches its age limit, or
    /// `None` if the clock cannot represent it.
    pub fn expires_at(&self) -> Option<Instant> {
        self.established.checked_add(self.lifetime())
    }

    fn too_old(&self, now: Instant) -> bool {
        self.age(now) >= self.lifetime()
    }

    /// Returns true if another frame of `frame_len` ciphertext bytes may
    /// be sent and a Close still fits after it.
    fn has_room_to_send(&self, frame_len: u64) -> bool {
        let frames_after = self.sent.frames.saturating_add(1);
        let bytes_after = self
            .sent
            .bytes
            .saturating_add(frame_len)
            .saturating_add(smallest_frame());
        frames_after < self.limits.max_frames && bytes_after <= self.limits.max_bytes
    }

    /// Returns true if nothing but Close can be sent any more: the session
    /// has reached its age limit, or not even the smallest frame fits in
    /// what is left of its frame and byte limits. The caller then closes
    /// the session and connects again.
    pub fn limit_reached(&self, now: Instant) -> bool {
        self.too_old(now) || !self.has_room_to_send(smallest_frame())
    }

    /// Ends the session: no key is kept past this point.
    fn end(&mut self) {
        self.noise = None;
    }

    /// Returns true once the session is over: it failed, a Close was sent
    /// or received, or the stream was reported closed. Nothing can be
    /// sent or received on it any more, and it holds no key.
    pub const fn is_over(&self) -> bool {
        self.noise.is_none()
    }

    /// The deadlines of the session that end it, each with whether it ends
    /// it silently.
    fn deadlines(&self) -> [(Option<Instant>, bool); 6] {
        let unknown = self.logic.state() == SessionState::AuthenticatedUnknown;
        // The logic enters Closing only on a frame it received, and the
        // Close it decided then is due at once.
        let closing = self.logic.state() == SessionState::Closing;
        let first_message = unknown && !self.logic.standing().is_contact_record() && !self.heard;
        [
            (
                self.frame_started
                    .and_then(|started| started.checked_add(FRAME_READ_TIMEOUT)),
                true,
            ),
            (self.last_frame.checked_add(IDLE_TIMEOUT), true),
            (self.expires_at(), false),
            (
                unknown
                    .then(|| self.established.checked_add(UNKNOWN_SESSION_TIMEOUT))
                    .flatten(),
                false,
            ),
            (
                first_message
                    .then(|| self.established.checked_add(UNKNOWN_FIRST_MESSAGE_TIMEOUT))
                    .flatten(),
                false,
            ),
            (closing.then_some(self.last_frame), false),
        ]
    }

    /// Returns the moment by which something has to happen on the session
    /// for it to go on, or `None` once it is over. The caller waits for
    /// bytes from the peer until then, and calls [`Self::expire`] if none
    /// change it in time.
    ///
    /// The deadlines are those of `docs/RESOURCE_LIMITS.md` section 4:
    ///
    /// - a frame that has begun completes within `FRAME_READ_TIMEOUT` of
    ///   its first byte; later bytes of it do not move this;
    /// - a frame completes within `IDLE_TIMEOUT` of the last complete one;
    /// - the session ends at its age limit;
    /// - it leaves `AuthenticatedUnknown` within `UNKNOWN_SESSION_TIMEOUT`
    ///   of the handshake, and a peer that is not a contact sends its first
    ///   message within `UNKNOWN_FIRST_MESSAGE_TIMEOUT`;
    /// - a Close the session logic decided on a message it received
    ///   ([`Action::SendClose`]) is due at once, and the session ends with
    ///   it whether or not the caller closes it.
    pub fn deadline(&self) -> Option<Instant> {
        if self.is_over() || self.close_sent {
            return None;
        }
        self.deadlines()
            .into_iter()
            .filter_map(|(deadline, _)| deadline)
            .min()
    }

    /// Ends the session if a deadline of [`Self::deadline`] has passed at
    /// `now`. A frame that did not complete in time and a peer that sent
    /// nothing end it silently. The age limit, the time in
    /// `AuthenticatedUnknown` and a Close that is due end it with a Close,
    /// as the local side ends a session on purpose. Either way the session
    /// is over afterwards.
    pub fn expire(&mut self, now: Instant) -> Expiry {
        if self.is_over() || self.close_sent {
            return Expiry::Running;
        }
        let passed = |deadline: Option<Instant>| deadline.is_some_and(|deadline| now >= deadline);
        let deadlines = self.deadlines();
        if deadlines
            .iter()
            .any(|(deadline, silent)| *silent && passed(*deadline))
        {
            self.failed = true;
            self.logic.stream_closed();
            self.end();
            return Expiry::Silent;
        }
        if deadlines.iter().any(|(deadline, _)| passed(*deadline)) {
            let frame = self.close();
            // Nothing is read or written after this, Close or not.
            self.logic.stream_closed();
            self.end();
            return Expiry::Close(frame);
        }
        Expiry::Running
    }

    fn fail<T>(&mut self, error: ProtocolError) -> Result<T, SessionError> {
        self.failed = true;
        self.logic.stream_closed();
        self.end();
        Err(SessionError::Protocol(error))
    }

    /// Takes bytes that arrived on the stream.
    ///
    /// Returns how many bytes of `input` were consumed and, if they
    /// completed a frame, the message in it with the actions to take. A
    /// call never consumes past the end of one frame; the caller acts on
    /// the result and then feeds the rest. Input may be split at any byte.
    ///
    /// An error means the peer violated the protocol. The session is over:
    /// close the stream and send nothing. No error is ever reported to the
    /// peer.
    ///
    /// The peer is held to the limits of the protocol. Its frames are
    /// taken for a short while after the age limit, because the two sides
    /// do not start counting at the same moment and a Close needs time to
    /// arrive (`docs/PROTOCOL.md` section 7.1).
    ///
    /// After a Close was sent or received the remaining bytes of the
    /// stream are dropped without being decoded.
    pub fn receive(
        &mut self,
        input: &[u8],
        now: Instant,
    ) -> Result<(usize, Option<Received>), SessionError> {
        if self.failed {
            return Err(SessionError::Closed);
        }
        let state = self.logic.state();
        if matches!(state, SessionState::Closing | SessionState::Closed) {
            return Ok((input.len(), None));
        }
        let (used, frame) = match self.decoder.feed(input, state) {
            Ok(fed) => fed,
            Err(error) => return self.fail(error),
        };
        let Some(payload) = frame else {
            if self.frame_started.is_none() && self.decoder.in_frame() {
                self.frame_started = Some(now);
            }
            return Ok((used, None));
        };
        self.frame_started = None;
        self.last_frame = now;

        let frame_len = u64::try_from(payload.len()).unwrap_or(u64::MAX);
        if self.age(now) >= self.lifetime().saturating_add(SESSION_CLOSE_GRACE)
            || self.received.frames >= self.limits.max_frames
            || self.received.bytes.saturating_add(frame_len) > self.limits.max_bytes
        {
            return self.fail(ProtocolError::SessionExpired);
        }

        let mut plaintext = vec![0_u8; payload.len().saturating_sub(PARAMS.overhead())];
        let opened = match self.noise.as_mut() {
            Some(noise) => noise.read_message(&payload, &mut plaintext),
            None => return Err(SessionError::Closed),
        };
        if opened != Ok(plaintext.len()) {
            return self.fail(ProtocolError::FrameAuthenticationFailed);
        }
        self.received.count(frame_len);

        let message = match decode_plaintext(&PARAMS, state, &plaintext)
            .and_then(|(message_type, body)| Message::decode(message_type, body))
        {
            Ok(message) => message,
            Err(error) => return self.fail(error),
        };
        match self.logic.receive(&message) {
            Ok(actions) => {
                self.heard = true;
                if self.logic.state() == SessionState::Closed {
                    // The peer closed. Nothing more is read or written.
                    self.end();
                }
                Ok((used, Some(Received { message, actions })))
            }
            Err(error) => self.fail(error),
        }
    }

    /// Encrypts a message into a frame, with its length prefix, for the
    /// stream.
    ///
    /// Fails with [`SessionError::NotPermitted`] if the message may not be
    /// sent now ([`Self::may_send`]), with [`SessionError::InvalidMessage`]
    /// if it carries a card or an invitation capability that is not the
    /// one this session sends, and with [`SessionError::Expired`] if the
    /// message does not fit in what the session has left. In these three
    /// cases nothing was encrypted and the session is as it was.
    ///
    /// Close is not sent with this function; see [`Self::close`].
    pub fn send(&mut self, message: &Message, now: Instant) -> Result<Vec<u8>, SessionError> {
        if self.failed || self.close_sent || self.logic.state() == SessionState::Closed {
            return Err(SessionError::Closed);
        }
        let message_type = message.message_type();
        if !self.may_send(message_type) {
            return Err(SessionError::NotPermitted);
        }
        if !self.speaks_only_for_the_local_side(message) {
            return Err(SessionError::InvalidMessage);
        }
        let plaintext = plaintext_of(message)?;
        let frame_len = plaintext.len().saturating_add(PARAMS.overhead());
        let frame_len = u64::try_from(frame_len).unwrap_or(u64::MAX);
        if self.too_old(now) || !self.has_room_to_send(frame_len) {
            return Err(SessionError::Expired);
        }
        let frame = self.seal(&plaintext)?;
        match message_type {
            MessageType::ContactAccept => self.accept_sent = true,
            MessageType::ContactRequest => self.request_sent = true,
            _ => {}
        }
        Ok(frame)
    }

    /// A message that is sent makes claims only about the local identity,
    /// and presents only the invitation the user was given for this peer.
    ///
    /// The card in a ContactRequest is the local card, which is also the
    /// card of the handshake, and its invitation capability is the one in
    /// the card of the peer that is asked. The card in an EndpointUpdate
    /// is a card of the local identity (`docs/PROTOCOL.md` sections 8.3
    /// and 8.8).
    fn speaks_only_for_the_local_side(&self, message: &Message) -> bool {
        match message {
            Message::ContactRequest(request) => {
                request.card == self.local_card && request.invitation == self.invitation
            }
            Message::EndpointUpdate(card) => card.identity() == self.local_card.identity(),
            _ => true,
        }
    }

    /// Encrypts one frame plaintext and puts the length prefix in front.
    fn seal(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, SessionError> {
        let len = plaintext.len().saturating_add(PARAMS.overhead());
        let mut payload = vec![0_u8; len];
        let Some(noise) = self.noise.as_mut() else {
            return Err(SessionError::Closed);
        };
        let sealed = match noise.write_message(plaintext, &mut payload) {
            Ok(written) if written == len => encode_outer(&PARAMS, &payload).ok(),
            _ => None,
        };
        let Some(frame) = sealed else {
            // Not reached with the lengths the frame encoder produces. The
            // cipher state may have moved on, so the session cannot go on.
            self.failed = true;
            self.logic.stream_closed();
            self.end();
            return Err(SessionError::Internal);
        };
        self.sent.count(u64::try_from(len).unwrap_or(u64::MAX));
        Ok(frame)
    }

    /// Ends the session on purpose, or completes an end that the session
    /// logic decided ([`Action::SendClose`]).
    ///
    /// Returns the Close frame to write before the stream is closed, or
    /// `None` if there is nothing to send: the peer closed first, the
    /// session failed, or the Close was already produced. A Close is
    /// produced once and fits whatever limit the session has reached.
    #[must_use]
    pub fn close(&mut self) -> Option<Vec<u8>> {
        // The actions are implied by the state the logic is left in.
        let _ = self.logic.close();
        self.close_frame()
    }

    /// The user blocked the peer. The peer sees the same Close as for any
    /// other end of a session.
    #[must_use]
    pub fn block_peer(&mut self) -> Option<Vec<u8>> {
        let _ = self.logic.peer_blocked();
        self.close_frame()
    }

    /// The user deleted the contact or withdrew a request. The peer sees
    /// the same Close as for any other end of a session.
    #[must_use]
    pub fn remove_contact(&mut self) -> Option<Vec<u8>> {
        let _ = self.logic.contact_removed();
        self.close_frame()
    }

    /// The transport key this session was authenticated with is no longer
    /// the active credential of the peer: a successor took over
    /// (`Credentials::authorizes` in the protocol crate is false for
    /// [`Self::peer_card`]). The session loses its standing and ends:
    /// nothing it receives afterwards is delivered, nothing but this Close
    /// is sent. The peer sees the same Close as for any other end of a
    /// session.
    #[must_use]
    pub fn withdraw(&mut self) -> Option<Vec<u8>> {
        let _ = self.logic.withdraw();
        self.close_frame()
    }

    fn close_frame(&mut self) -> Option<Vec<u8>> {
        if self.failed || self.close_sent || self.logic.state() != SessionState::Closing {
            return None;
        }
        self.close_sent = true;
        let frame = plaintext_of(&Message::Close)
            .ok()
            .and_then(|plaintext| self.seal(&plaintext).ok());
        // The Close is the last frame. Nothing is read after it either.
        self.end();
        frame
    }

    /// The stream is gone, for whatever reason.
    pub fn stream_closed(&mut self) {
        self.logic.stream_closed();
        self.end();
    }

    /// Returns true while the session holds its cipher keys.
    #[cfg(test)]
    pub(crate) const fn holds_keys(&self) -> bool {
        self.noise.is_some()
    }

    /// Encrypts a message without asking whether it may be sent. Tests
    /// use it to play a peer that breaks the rules.
    #[cfg(test)]
    pub(crate) fn seal_unchecked(&mut self, message: &Message) -> Vec<u8> {
        let plaintext = plaintext_of(message).unwrap();
        self.seal(&plaintext).unwrap()
    }

    /// Encrypts arbitrary bytes as a frame. Tests use it to send frames
    /// that authenticate and are malformed inside.
    #[cfg(test)]
    pub(crate) fn seal_raw(&mut self, plaintext: &[u8]) -> Vec<u8> {
        self.seal(plaintext).unwrap()
    }
}

/// Builds the padded plaintext of a frame for a message.
fn plaintext_of(message: &Message) -> Result<Vec<u8>, SessionError> {
    let body = message
        .encode_body()
        .map_err(|_| SessionError::InvalidMessage)?;
    encode_plaintext(&PARAMS, message.message_type(), &body)
        .map_err(|_| SessionError::InvalidMessage)
}

impl fmt::Debug for AuthenticatedSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The state and the traffic counters. No key, no identity, no
        // message content.
        f.debug_struct("AuthenticatedSession")
            .field("state", &self.logic.state())
            .field("sent", &self.sent)
            .field("received", &self.received)
            .field("failed", &self.failed)
            .finish_non_exhaustive()
    }
}

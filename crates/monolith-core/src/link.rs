//! Tor streams to authenticated sessions.
//!
//! Tor answers "this stream reached that Onion Service". The handshake of
//! `monolith-session` answers "the other end holds the transport key that a
//! contact card binds to an identity". This module puts the second on top
//! of the first and adds nothing to either: a stream is a byte stream, and
//! nothing about it, the onion address included, counts as authentication.
//! What a peer may do afterwards is decided by its record, as before.
//!
//! The record is read when the handshake is over and not earlier. `dial`
//! and `answer` take an admission function, which they call once, when
//! the peer is authenticated: the dial budget, the Tor stream and the
//! handshake messages that authenticate it are behind it. The function
//! receives the authenticated peer and makes the session from the contact
//! state as it is at that moment, in one synchronous step. Nothing a dial
//! captured before it started can decide the standing of the peer.
//!
//! A session can lose its standing later: when a successor of the
//! transport key it was authenticated with takes over. The admission
//! function is given the [`Withdrawal`] of the link, keeps it with the
//! session in the same step, and the contact state uses it when the key
//! is retired. From then on the link delivers nothing and ends with a
//! Close.
//!
//! Every step is bounded: the handshake by `HANDSHAKE_TIMEOUT`, each frame
//! write by `FRAME_WRITE_TIMEOUT`, silence by `IDLE_TIMEOUT`. Reads go into
//! one fixed buffer, and nothing more is read until the session has taken
//! what is there, so a fast peer is slowed down by the reader and cannot
//! fill memory.

use core::fmt;
use core::future::{Future, poll_fn};
use core::pin::pin;
use core::task::Poll;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use monolith_protocol::ProtocolError;
use monolith_protocol::body::Message;
use monolith_protocol::card::ContactCard;
use monolith_protocol::limits::{
    FRAME_WRITE_TIMEOUT, HANDSHAKE_MSG1_LEN, HANDSHAKE_MSG2_LEN, HANDSHAKE_MSG3_LEN,
    HANDSHAKE_TIMEOUT, IDLE_TIMEOUT,
};
use monolith_protocol::session::{Action, Admission, Standing};
use tokio::sync::{Notify, OwnedSemaphorePermit};

use crate::budget::Budgets;
use monolith_session::{
    Admitted, AuthenticatedSession, HandshakeInitiator, HandshakeResponder, InboundPeer,
    LocalParty, MessageBuffer, OutboundPeer, Received, SessionError,
};
use monolith_tor::{IsolationGroup, TorBackend, TorError};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Size of the read buffer of a link.
const READ_CHUNK: usize = 4096;

/// Why a link could not be made or used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinkError {
    /// Tor could not open the stream.
    Tor(TorError),
    /// The handshake failed, or the peer broke the protocol.
    Session(SessionError),
    /// The stream ended or failed.
    Stream,
    /// A deadline passed: the handshake, a write, or the idle limit.
    TimedOut,
    /// The contact card names no endpoint that can be dialed.
    NoEndpoint,
    /// The peer is not a contact and the budget for such sessions is full.
    /// The stream was closed without a reply.
    Budget,
    /// The session was withdrawn: the transport key it was authenticated
    /// with is no longer the active one of the peer.
    Withdrawn,
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tor(error) => write!(f, "{error}"),
            Self::Session(error) => write!(f, "{error}"),
            Self::Stream => f.write_str("stream closed"),
            Self::TimedOut => f.write_str("timed out"),
            Self::NoEndpoint => f.write_str("no endpoint to dial"),
            Self::Budget => f.write_str("session budget full"),
            Self::Withdrawn => f.write_str("session withdrawn"),
        }
    }
}

impl core::error::Error for LinkError {}

impl From<SessionError> for LinkError {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

/// The moment the session layer is told. The session never reads a clock;
/// the core does, through tokio's clock so that tests can control it.
fn now() -> Instant {
    tokio::time::Instant::now().into_std()
}

/// The power to withdraw one link from outside the task that runs it.
///
/// The admission function of [`dial`] and [`answer`] receives it and keeps
/// it with the session, in the same step in which it decides the session.
/// It is handed out nowhere else, so it cannot be kept later, outside that
/// step. When the transport key the session was authenticated with is
/// retired, the contact state calls [`Self::withdraw`]. The link then
/// delivers nothing more: a frame still in its buffer is dropped, a link
/// that waits for the peer wakes up, and it sends a Close and shuts the
/// stream down. Clones refer to the same link.
///
/// [`Self::is_ended`] turns true when the link is dropped, or when it never
/// came to exist because `answer` refused the peer after the admission, so
/// that the contact state can forget the withdrawals of links that are
/// gone.
#[derive(Clone)]
pub struct Withdrawal(Arc<WithdrawalState>);

struct WithdrawalState {
    withdrawn: AtomicBool,
    ended: AtomicBool,
    wake: Notify,
}

impl Withdrawal {
    fn new() -> Self {
        Self(Arc::new(WithdrawalState {
            withdrawn: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            wake: Notify::new(),
        }))
    }

    fn end(&self) {
        self.0.ended.store(true, Ordering::SeqCst);
    }

    /// Returns true once the link is gone. Withdrawing it then changes
    /// nothing.
    pub fn is_ended(&self) -> bool {
        self.0.ended.load(Ordering::SeqCst)
    }

    /// Returns true if `other` refers to the same link.
    pub fn same_link(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Withdraws the session. It cannot be undone.
    pub fn withdraw(&self) {
        self.0.withdrawn.store(true, Ordering::SeqCst);
        // One task runs the link. If it is not waiting now, the permit is
        // kept and ends its next wait at once.
        self.0.wake.notify_one();
    }

    /// Returns true once the session was withdrawn.
    pub fn is_withdrawn(&self) -> bool {
        self.0.withdrawn.load(Ordering::SeqCst)
    }
}

impl fmt::Debug for Withdrawal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Withdrawal")
            .field("withdrawn", &self.is_withdrawn())
            .field("ended", &self.is_ended())
            .finish()
    }
}

/// An authenticated session over a stream.
pub struct Link<S> {
    stream: S,
    session: AuthenticatedSession,
    withdrawal: Withdrawal,
    buffer: Box<[u8; READ_CHUNK]>,
    /// The unread part of `buffer` is `start..end`.
    start: usize,
    end: usize,
}

/// A link with what the session logic said when it was made.
pub struct Established<S> {
    /// The link.
    pub link: Link<S>,
    /// How the peer's card compares with what is held of it.
    pub admission: Admission,
    /// The first message to send, if any.
    pub first: Vec<Action>,
    /// For a peer that is not a contact: its slot in the budget of
    /// `MAX_UNKNOWN_SESSIONS`, held for as long as this value is.
    pub unknown_slot: Option<OwnedSemaphorePermit>,
}

impl<S> fmt::Debug for Established<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Established")
            .field("session", &self.link.session)
            .field("first", &self.first)
            .finish_non_exhaustive()
    }
}

async fn write_all<S: AsyncWrite + Unpin>(stream: &mut S, bytes: &[u8]) -> Result<(), LinkError> {
    stream
        .write_all(bytes)
        .await
        .map_err(|_| LinkError::Stream)?;
    stream.flush().await.map_err(|_| LinkError::Stream)
}

/// Reads one handshake message of exactly `N` bytes, and not one byte
/// more: what follows belongs to the session.
async fn read_message<S: AsyncRead + Unpin, const N: usize>(
    stream: &mut S,
) -> Result<[u8; N], LinkError> {
    let mut buffer = MessageBuffer::<N>::new();
    let mut message = [0_u8; N];
    loop {
        let missing = buffer.missing();
        let slot = message.get_mut(..missing).ok_or(LinkError::Stream)?;
        let read = stream.read(slot).await.map_err(|_| LinkError::Stream)?;
        if read == 0 {
            return Err(LinkError::Stream);
        }
        let (_, complete) = buffer.feed(slot.get(..read).ok_or(LinkError::Stream)?);
        if let Some(complete) = complete {
            return Ok(*complete);
        }
    }
}

/// Dials the first endpoint of `card` through `backend` and runs the
/// initiator's handshake. `isolation` is that contact's isolation group. A
/// dial waits for a slot in the budget of `MAX_CONCURRENT_DIALS` and holds
/// it through the SOCKS negotiation and the handshake.
///
/// `admit` is called once, when message 2 has authenticated the responder
/// and before message 3, which carries the local identity and card, is
/// written. It looks up what the local side holds about the identity of
/// `card` as it is then, admits the peer with [`OutboundPeer::admit`],
/// keeps the [`Withdrawal`] with the session, and returns what
/// `admit` returned, all in one step on the contact state. The record is
/// never taken before the dial. If the key the responder proved is no
/// longer the one that stands for the contact, because it was retired or
/// is only pending, message 3 is not sent and the dial fails with
/// [`ProtocolError::IdentityMismatch`]: the local identity is not shown to
/// the holder of a key the contact has left (`docs/PROTOCOL.md` section
/// 4.4). A retirement after the admission, while message 3 is written,
/// reaches the session through its withdrawal.
///
/// `admit` runs inside the handshake deadline and must not block: it takes
/// the lock of the contact state, does its work and lets go. The caller
/// must not hold that lock, or a reference into the contact state, across
/// the `await` of `dial`; the function is where the state is read.
pub async fn dial<B, F>(
    backend: &B,
    budgets: &Budgets,
    local: &LocalParty,
    card: &ContactCard,
    isolation: &IsolationGroup,
    admit: F,
) -> Result<Established<B::Stream>, LinkError>
where
    B: TorBackend,
    F: FnOnce(OutboundPeer, &Withdrawal) -> Result<Admitted, SessionError>,
{
    let _slot = budgets.dial().await.ok_or(LinkError::Budget)?;
    let endpoint = *card.endpoints().first();
    let mut stream = backend
        .connect_onion(&endpoint, isolation)
        .await
        .map_err(LinkError::Tor)?;
    tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        let (initiator, message_1) = HandshakeInitiator::start(local, card, now())?;
        write_all(&mut stream, &message_1).await?;
        let message_2 = read_message::<_, HANDSHAKE_MSG2_LEN>(&mut stream).await?;
        let (outbound, message_3) = initiator.read_message_2(&message_2, now())?;
        // The responder is authenticated. The admission comes before the
        // local identity goes out in message 3.
        let withdrawal = Withdrawal::new();
        let (session, admission, first) = admit(outbound, &withdrawal)?;
        let mut link = Link::new(stream, session, withdrawal);
        if matches!(
            admission.standing,
            Standing::StaleCard | Standing::PendingSuccessor
        ) {
            return Err(LinkError::Session(SessionError::Protocol(
                ProtocolError::IdentityMismatch,
            )));
        }
        write_all(&mut link.stream, &message_3).await?;
        Ok(Established {
            link,
            admission,
            first,
            unknown_slot: None,
        })
    })
    .await
    .map_err(|_| LinkError::TimedOut)?
}

/// Runs the responder's handshake on an inbound stream. `admit` is called
/// once the handshake has authenticated the card the initiator presented,
/// with nothing left to wait for; it looks up the record of that identity
/// as it is then, admits the peer with [`InboundPeer::admit`], keeps the
/// [`Withdrawal`] with the session, and returns what `admit` returned, in
/// one step. A peer that is not a contact needs a slot in the budget of
/// `MAX_UNKNOWN_SESSIONS`; without one the stream is closed and nothing is
/// sent. The caller holds the inbound handshake slot from the accept loop
/// until this returns. `admit` is held to the same rules as for [`dial`].
///
/// The budget is applied after the admission, to the standing it decided
/// (`docs/PROTOCOL.md` section 6.2). When it refuses the peer, `admit` has
/// run and what it recorded stands, a pending successor included; the link
/// is never made and its [`Withdrawal`] reports [`Withdrawal::is_ended`].
pub async fn answer<S, F>(
    mut stream: S,
    budgets: &Budgets,
    local: &LocalParty,
    admit: F,
) -> Result<Established<S>, LinkError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce(InboundPeer, &Withdrawal) -> Result<Admitted, SessionError>,
{
    tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        let responder = HandshakeResponder::new(local, now())?;
        let message_1 = read_message::<_, HANDSHAKE_MSG1_LEN>(&mut stream).await?;
        let (waiting, message_2) = responder.read_message_1(&message_1, now())?;
        write_all(&mut stream, &message_2).await?;
        let message_3 = read_message::<_, HANDSHAKE_MSG3_LEN>(&mut stream).await?;
        let inbound = waiting.read_message_3(&message_3, now())?;
        // The last wait is behind. From here to the session nothing waits.
        let withdrawal = Withdrawal::new();
        let (session, admission, first) = admit(inbound, &withdrawal)?;
        let unknown_slot = if admission.standing.is_contact_record() {
            None
        } else {
            let Some(slot) = budgets.unknown_session() else {
                withdrawal.end();
                return Err(LinkError::Budget);
            };
            Some(slot)
        };
        Ok(Established {
            link: Link::new(stream, session, withdrawal),
            admission,
            first,
            unknown_slot,
        })
    })
    .await
    .map_err(|_| LinkError::TimedOut)?
}

impl<S: AsyncRead + AsyncWrite + Unpin> Link<S> {
    fn new(stream: S, session: AuthenticatedSession, withdrawal: Withdrawal) -> Self {
        Self {
            stream,
            session,
            withdrawal,
            buffer: Box::new([0_u8; READ_CHUNK]),
            start: 0,
            end: 0,
        }
    }

    /// The session.
    pub const fn session(&self) -> &AuthenticatedSession {
        &self.session
    }

    /// Returns true once the session was withdrawn. A message that was
    /// returned before stays the caller's to judge: what it would change in
    /// the contact state is applied under the lock of that state, and only
    /// while the session still stands for the contact
    /// (`Credentials::authorizes` for its card in the protocol crate).
    pub fn is_withdrawn(&self) -> bool {
        self.withdrawal.is_withdrawn()
    }

    /// Ends a withdrawn session: the session gives up its standing, the
    /// Close is written if it was not yet, and the stream is shut down, each
    /// within `FRAME_WRITE_TIMEOUT`.
    async fn end_withdrawn(&mut self) -> LinkError {
        if let Some(frame) = self.session.withdraw() {
            let _ = tokio::time::timeout(FRAME_WRITE_TIMEOUT, write_all(&mut self.stream, &frame))
                .await;
        }
        let _ = tokio::time::timeout(FRAME_WRITE_TIMEOUT, self.stream.shutdown()).await;
        LinkError::Withdrawn
    }

    /// Encrypts a message and writes it, within `FRAME_WRITE_TIMEOUT`.
    /// Nothing is sent on a withdrawn link.
    pub async fn send(&mut self, message: &Message) -> Result<(), LinkError> {
        if self.withdrawal.is_withdrawn() {
            return Err(self.end_withdrawn().await);
        }
        let frame = self.session.send(message, now())?;
        let written =
            tokio::time::timeout(FRAME_WRITE_TIMEOUT, write_all(&mut self.stream, &frame))
                .await
                .map_err(|_| LinkError::TimedOut)
                .and_then(|result| result);
        if written.is_err() {
            // Part of a frame may be on the stream: nothing more can follow.
            self.session.stream_closed();
        }
        written
    }

    /// Waits for the next message. Fails when the peer breaks the
    /// protocol, the stream ends, nothing arrives for `IDLE_TIMEOUT`, or
    /// the session is withdrawn. After a failure the session is over.
    ///
    /// A withdrawal is looked at before each frame is taken from the
    /// buffer and before every wait for the peer, and it ends a wait that
    /// is in progress. A frame that was not taken when the session was
    /// withdrawn is not delivered, and every call after the withdrawal
    /// fails with [`LinkError::Withdrawn`] at once.
    pub async fn receive(&mut self) -> Result<Received, LinkError> {
        loop {
            while self.start < self.end {
                if self.withdrawal.is_withdrawn() {
                    return Err(self.end_withdrawn().await);
                }
                let pending = self
                    .buffer
                    .get(self.start..self.end)
                    .ok_or(LinkError::Stream)?;
                let (used, received) = self.session.receive(pending, now())?;
                self.start = self.start.saturating_add(used).min(self.end);
                if let Some(received) = received {
                    return Ok(received);
                }
            }
            if self.withdrawal.is_withdrawn() {
                return Err(self.end_withdrawn().await);
            }
            // Everything read so far was taken; only now is more read, or
            // the wait ends because the session was withdrawn.
            let outcome = {
                let mut withdrawn = pin!(self.withdrawal.0.wake.notified());
                let mut read = pin!(tokio::time::timeout(
                    IDLE_TIMEOUT,
                    self.stream.read(self.buffer.as_mut_slice())
                ));
                poll_fn(|cx| {
                    if withdrawn.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(None);
                    }
                    read.as_mut().poll(cx).map(Some)
                })
                .await
            };
            let Some(read) = outcome else {
                return Err(self.end_withdrawn().await);
            };
            let read = read
                .map_err(|_| LinkError::TimedOut)?
                .map_err(|_| LinkError::Stream)?;
            if read == 0 {
                self.session.stream_closed();
                return Err(LinkError::Stream);
            }
            self.start = 0;
            self.end = read;
        }
    }

    /// Ends the session: writes the Close, if one is due, and shuts the
    /// stream down.
    pub async fn close(mut self) -> Result<(), LinkError> {
        if let Some(frame) = self.session.close() {
            tokio::time::timeout(FRAME_WRITE_TIMEOUT, write_all(&mut self.stream, &frame))
                .await
                .map_err(|_| LinkError::TimedOut)??;
        }
        let _ = self.stream.shutdown().await;
        Ok(())
    }
}

impl<S> Drop for Link<S> {
    fn drop(&mut self) {
        self.withdrawal.end();
    }
}

impl<S> fmt::Debug for Link<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Link")
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

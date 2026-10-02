//! Tor streams to authenticated sessions.
//!
//! Tor answers "this stream reached that Onion Service". The handshake of
//! `monolith-session` answers "the other end holds the transport key that a
//! contact card binds to an identity". This module puts the second on top
//! of the first and adds nothing to either: a stream is a byte stream, and
//! nothing about it, the onion address included, counts as authentication.
//! What a peer may do afterwards is decided by its record, as before.
//!
//! Every step is bounded: the handshake by `HANDSHAKE_TIMEOUT`, each frame
//! write by `FRAME_WRITE_TIMEOUT`, silence by `IDLE_TIMEOUT`. Reads go into
//! one fixed buffer, and nothing more is read until the session has taken
//! what is there, so a fast peer is slowed down by the reader and cannot
//! fill memory.

use core::fmt;
use std::time::Instant;

use monolith_protocol::body::Message;
use monolith_protocol::card::ContactCard;
use monolith_protocol::limits::{
    FRAME_WRITE_TIMEOUT, HANDSHAKE_MSG1_LEN, HANDSHAKE_MSG2_LEN, HANDSHAKE_MSG3_LEN,
    HANDSHAKE_TIMEOUT, IDLE_TIMEOUT,
};
use monolith_protocol::session::{Action, Admission, PeerRecord};
use monolith_session::{
    AuthenticatedSession, HandshakeInitiator, HandshakeResponder, LocalParty, MessageBuffer,
    Received, SessionError,
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
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tor(error) => write!(f, "{error}"),
            Self::Session(error) => write!(f, "{error}"),
            Self::Stream => f.write_str("stream closed"),
            Self::TimedOut => f.write_str("timed out"),
            Self::NoEndpoint => f.write_str("no endpoint to dial"),
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

/// An authenticated session over a stream.
pub struct Link<S> {
    stream: S,
    session: AuthenticatedSession,
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
/// initiator's handshake. `record` is what the local side holds about the
/// identity of `card`; `isolation` is that contact's isolation group.
pub async fn dial<B: TorBackend>(
    backend: &B,
    local: &LocalParty,
    card: &ContactCard,
    record: PeerRecord<'_>,
    isolation: &IsolationGroup,
) -> Result<Established<B::Stream>, LinkError> {
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
        write_all(&mut stream, &message_3).await?;
        let (session, admission, first) = outbound.admit(record)?;
        Ok(Established {
            link: Link::new(stream, session),
            admission,
            first,
        })
    })
    .await
    .map_err(|_| LinkError::TimedOut)?
}

/// Runs the responder's handshake on an inbound stream. `lookup` gives
/// the record the local side holds about the identity in the card the
/// initiator presented; it is asked only after the handshake has
/// authenticated that card.
pub async fn answer<'r, S, F>(
    mut stream: S,
    local: &LocalParty,
    lookup: F,
) -> Result<Established<S>, LinkError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce(&ContactCard) -> PeerRecord<'r>,
{
    tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        let responder = HandshakeResponder::new(local, now())?;
        let message_1 = read_message::<_, HANDSHAKE_MSG1_LEN>(&mut stream).await?;
        let (waiting, message_2) = responder.read_message_1(&message_1, now())?;
        write_all(&mut stream, &message_2).await?;
        let message_3 = read_message::<_, HANDSHAKE_MSG3_LEN>(&mut stream).await?;
        let inbound = waiting.read_message_3(&message_3, now())?;
        let record = lookup(inbound.card());
        let (session, admission, first) = inbound.admit(record)?;
        Ok(Established {
            link: Link::new(stream, session),
            admission,
            first,
        })
    })
    .await
    .map_err(|_| LinkError::TimedOut)?
}

impl<S: AsyncRead + AsyncWrite + Unpin> Link<S> {
    fn new(stream: S, session: AuthenticatedSession) -> Self {
        Self {
            stream,
            session,
            buffer: Box::new([0_u8; READ_CHUNK]),
            start: 0,
            end: 0,
        }
    }

    /// The session.
    pub const fn session(&self) -> &AuthenticatedSession {
        &self.session
    }

    /// Encrypts a message and writes it, within `FRAME_WRITE_TIMEOUT`.
    pub async fn send(&mut self, message: &Message) -> Result<(), LinkError> {
        let frame = self.session.send(message, now())?;
        tokio::time::timeout(FRAME_WRITE_TIMEOUT, write_all(&mut self.stream, &frame))
            .await
            .map_err(|_| LinkError::TimedOut)?
    }

    /// Waits for the next message. Fails when the peer breaks the
    /// protocol, the stream ends, or nothing arrives for `IDLE_TIMEOUT`.
    /// After a failure the session is over.
    pub async fn receive(&mut self) -> Result<Received, LinkError> {
        loop {
            while self.start < self.end {
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
            // Everything read so far was taken; only now is more read.
            let read =
                tokio::time::timeout(IDLE_TIMEOUT, self.stream.read(self.buffer.as_mut_slice()))
                    .await
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

impl<S> fmt::Debug for Link<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Link")
            .field("session", &self.session)
            .finish_non_exhaustive()
    }
}

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
//! and `answer` take the local identity the link is for, and admit the
//! peer through its contact store once, when the peer is authenticated:
//! the dial budget, the Tor stream and the handshake messages that
//! authenticate it are behind it. The admission makes the session from
//! the contact state as it is at that moment, in one synchronous step
//! under the lock of that contact. Nothing a dial captured before it
//! started can decide the standing of the peer, and no other function
//! decides it: there is no way to hand `dial` an admission of one's own.
//!
//! Message 3 carries the local identity. The session crate makes it only
//! when the admission lets the responder learn the local identity; `dial`
//! waits until the state that admission depended on is durable, and then
//! writes it unless the session was withdrawn in the meantime. A crash
//! before that point loses the admission and sends nothing.
//!
//! A session can lose its standing later: when a successor of the
//! transport key it was authenticated with takes over, or the contact is
//! blocked or deleted. The store keeps the [`Withdrawal`] of the link with
//! the session in the admission step, and uses it when the session no
//! longer stands. From then on the link delivers nothing and ends with a
//! Close.
//!
//! Every step is bounded: the handshake by `HANDSHAKE_TIMEOUT`, each frame
//! write by `FRAME_WRITE_TIMEOUT`, and everything after the handshake by
//! the deadlines of the session (`AuthenticatedSession::deadline`): a frame
//! has to complete within `FRAME_READ_TIMEOUT` of its first byte, the next
//! one within `IDLE_TIMEOUT` of the last, an unconfirmed session ends in
//! time, and every session at its age limit. A peer that sends a frame a
//! byte at a time moves none of these. Reads go into one fixed buffer, and
//! nothing more is read until the session has taken what is there, so a
//! fast peer is slowed down by the reader and cannot fill memory.
//!
//! A link that failed is over for good: after the end of the stream, a read
//! or write error, a deadline, a withdrawal, a violation or a Close, every
//! later call fails at once, without waiting for the stream again, and
//! nothing more is read or delivered. So is a link whose send was dropped
//! while it wrote (a deadline of the caller, an aborted task): the session
//! counted the frame as sent and the stream may hold part of it, so the
//! next call ends the link, without a Close, and fails.

use core::fmt;
use core::future::{Future, poll_fn};
use core::pin::{Pin, pin};
use core::task::Poll;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use std::time::Instant;

use monolith_protocol::SessionState;
use monolith_protocol::body::Message;
use monolith_protocol::card::ContactCard;
use monolith_protocol::limits::{
    FRAME_WRITE_TIMEOUT, HANDSHAKE_MSG1_LEN, HANDSHAKE_MSG2_LEN, HANDSHAKE_MSG3_LEN,
    HANDSHAKE_TIMEOUT,
};
use monolith_protocol::session::{Action, Admission};
use tokio::sync::Notify;

use crate::budget::{Budgets, ContactPermit};
use crate::identity::{LocalIdentity, SessionRef};
use crate::persist::CommitError;
use crate::strangers::StrangerSlot;
use monolith_session::{
    AuthenticatedSession, Expiry, HandshakeInitiator, HandshakeResponder, MessageBuffer,
    OutboundAdmission, Received, SessionError,
};
use monolith_tor::{TorBackend, TorError};
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
    /// A deadline passed: the handshake, a write, or one of the session
    /// (a frame, silence, an unconfirmed session, the age limit).
    TimedOut,
    /// The contact card names no endpoint that can be dialed.
    NoEndpoint,
    /// The peer is not a contact and the budget for such sessions is full,
    /// or the peer is a contact and the budget for contact sessions is.
    /// The stream was closed without a reply.
    Budget,
    /// The peer was a stranger that had not sent its first message, and
    /// its slot went to a newer stranger. The link ended with a Close.
    Evicted,
    /// The admission could not be made durable, so it was not used. The
    /// installation refuses everything until the process starts again.
    Storage(CommitError),
    /// The session was withdrawn: the transport key it was authenticated
    /// with is no longer the active one of the peer.
    Withdrawn,
    /// A dial authenticated the responder, but the key it proved may not
    /// learn the local identity: message 3 was not sent and no session was
    /// made. The admission says why, for the local side only; a conflict
    /// ([`monolith_protocol::credential::CredentialChange::Conflict`]) or a
    /// pending key is for the user. The peer sees the stream close, as for
    /// a failed handshake.
    Refused(Admission),
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
            Self::Evicted => f.write_str("stranger evicted for a newer one"),
            Self::Storage(error) => write!(f, "{error}"),
            Self::Refused(_) => f.write_str("peer may not learn the local identity"),
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
/// The contact store receives it in the admission of [`dial`] and
/// [`answer`] and keeps it with the session, in the same step in which it
/// decides the session. It is handed out nowhere else, so it cannot be
/// kept later, outside that step. When the transport key the session was
/// authenticated with is retired, or the record of the peer's identity is
/// deleted or blocked, the store calls [`Self::withdraw`]. The link then delivers
/// nothing more: a frame still in its buffer is dropped, a link that
/// waits for the peer wakes up, a write in progress ends, and the link
/// ends. A withdrawal seen before a send or a receive starts is ended with
/// a Close; one that comes while a send is under way, even before its
/// first byte, ends the link without one, as a failed stream. Clones refer
/// to the same link.
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
    /// The contact store that admitted the session, set once, by that
    /// store, in the admission step.
    owner: OnceLock<Weak<Owner>>,
}

/// What a contact store binds the withdrawals of its sessions to. Nothing
/// outside the store can make one or bind to one.
pub(crate) struct Owner;

impl Withdrawal {
    fn new() -> Self {
        Self(Arc::new(WithdrawalState {
            withdrawn: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            wake: Notify::new(),
            owner: OnceLock::new(),
        }))
    }

    /// Binds the session to the store of `owner`, in its admission. A
    /// session is admitted once; a later binding changes nothing.
    pub(crate) fn bind(&self, owner: &Arc<Owner>) {
        let _ = self.0.owner.set(Arc::downgrade(owner));
    }

    /// Returns true if the store of `owner` admitted the session.
    pub(crate) fn admitted_by(&self, owner: &Arc<Owner>) -> bool {
        self.0
            .owner
            .get()
            .is_some_and(|held| Weak::ptr_eq(held, &Arc::downgrade(owner)))
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

/// Ends the withdrawal of a link that is not made yet if the wait it
/// guards never returns: a deadline or an aborted task that drops the
/// future while the admission is made durable. The store then forgets the
/// session it tracked. Disarmed once the wait returns; from there on the
/// error paths end the withdrawal, and the link does when it is dropped.
struct EndIfCancelled(Option<Withdrawal>);

impl EndIfCancelled {
    fn new(withdrawal: &Withdrawal) -> Self {
        Self(Some(withdrawal.clone()))
    }

    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for EndIfCancelled {
    fn drop(&mut self) {
        if let Some(withdrawal) = self.0.take() {
            withdrawal.end();
        }
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
    /// For a peer that is not a contact: its slot in the budget of
    /// `MAX_UNKNOWN_SESSIONS` of the local identity, held until the session
    /// is over.
    unknown_slot: Option<StrangerSlot>,
    /// For a contact: its slot in the contact session budgets.
    contact_slot: Option<ContactPermit>,
    buffer: Box<[u8; READ_CHUNK]>,
    /// The unread part of `buffer` is `start..end`.
    start: usize,
    end: usize,
    /// The link has ended: the stream was shut down, or given the chance.
    finished: bool,
    /// A frame is being written. Still true when a call is entered, it
    /// means the call that wrote it was dropped part way: the session
    /// counted the frame as sent and the stream may hold part of it, so
    /// the link can only end.
    writing: bool,
    /// A message the session took and a receive has not returned yet,
    /// because that receive was dropped while it ended the link. The next
    /// receive returns it: the session counted it as received.
    taken: Option<Received>,
    /// A step a test runs when a message was taken from the buffer, before
    /// it is delivered, to place an eviction exactly there.
    #[cfg(test)]
    hook: Option<Box<dyn FnOnce() + Send>>,
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

/// Dials `card`, a card of a contact of `identity`, through `backend`, and
/// runs the initiator's handshake with the party of `identity` that
/// dials that contact, in the isolation group of the pair (S42, S45). A
/// dial waits for a slot in the budget of `MAX_CONCURRENT_DIALS` and holds
/// it through the SOCKS negotiation and the handshake.
///
/// When message 2 has authenticated the responder, the contact store of
/// `identity` admits it against the record of its identity as it is then,
/// and keeps the [`Withdrawal`] of the link with the session in the same
/// step. The record is never read before.
///
/// Message 3 exists only if the responder may learn the local identity:
/// its key stands for a contact the identity holds as requested or
/// accepted. Otherwise, for a key that was retired or is pending, a key
/// older than the announced successor, a contradicting card, or an
/// identity that was deleted, declined or blocked during the dial, the
/// dial fails with [`LinkError::Refused`], which carries the admission,
/// nothing more is sent and no session is made (`docs/PROTOCOL.md`
/// section 4.4). Before message 3 is written, the state the admission
/// depended on is made durable ([`LinkError::Storage`] if it cannot be),
/// and a slot of the contact session budgets is taken
/// ([`LinkError::Budget`] if none is free). What an admission recorded (a
/// pending successor, a conflict, a newer card) stands also when no
/// session follows, and is made durable before the refusal is returned. The withdrawal is looked at
/// before the write starts and while it is pending; a withdrawal ends the
/// dial with [`LinkError::Withdrawn`]. Bytes the stream accepted before
/// that cannot be called back.
pub async fn dial<B>(
    backend: &B,
    budgets: &Budgets,
    identity: &LocalIdentity,
    card: &ContactCard,
) -> Result<Established<B::Stream>, LinkError>
where
    B: TorBackend,
{
    let contact = *card.identity();
    // A deleted identity dials nobody: it has no party any more.
    let local = identity
        .dial_plan(&contact)
        .map_or_else(|| identity.answering_party(), |plan| Ok(plan.local))
        .map_err(|_| LinkError::Storage(CommitError::Failed))?;
    let isolation = identity.isolation(&contact).map_err(LinkError::Tor)?;
    let _slot = budgets.dial().await.ok_or(LinkError::Budget)?;
    // The identity may have been deleted while the dial waited: it opens
    // no stream then, and starts no handshake.
    if identity.is_deleted() {
        return Err(LinkError::Storage(CommitError::Failed));
    }
    let endpoint = *card.endpoints().first();
    let mut stream = backend
        .connect_onion(&endpoint, &isolation)
        .await
        .map_err(LinkError::Tor)?;
    if identity.is_deleted() {
        return Err(LinkError::Storage(CommitError::Failed));
    }
    tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        let (initiator, message_1) = HandshakeInitiator::start(&local, card, now())?;
        write_all(&mut stream, &message_1).await?;
        let message_2 = read_message::<_, HANDSHAKE_MSG2_LEN>(&mut stream).await?;
        let outbound = initiator.read_message_2(&message_2, now())?;
        // The responder is authenticated. The admission decides whether
        // message 3, which carries the local identity, is made at all.
        let withdrawal = Withdrawal::new();
        let (session, admission, first, message_3, depends) =
            match identity.admit_outbound(outbound, &withdrawal) {
                Ok((
                    OutboundAdmission::Granted {
                        session,
                        admission,
                        first,
                        message_3,
                    },
                    depends,
                )) => (session, admission, first, message_3, depends),
                Ok((OutboundAdmission::Refused(admission), depends)) => {
                    withdrawal.end();
                    keep_recorded(identity, depends).await?;
                    return Err(LinkError::Refused(admission));
                }
                Err(error) => {
                    withdrawal.end();
                    return Err(error.into());
                }
            };
        let Some(contact_slot) = budgets.contact_session(identity) else {
            withdrawal.end();
            keep_recorded(identity, depends).await?;
            return Err(LinkError::Budget);
        };
        let cancelled = EndIfCancelled::new(&withdrawal);
        let durable = identity.wait_durable(depends).await;
        cancelled.disarm();
        if let Err(error) = durable {
            withdrawal.end();
            return Err(LinkError::Storage(error));
        }
        let mut link = Link::new(stream, session, withdrawal, None, Some(contact_slot));
        link.write_unless_withdrawn(message_3.as_bytes()).await?;
        Ok(Established {
            link,
            admission,
            first,
        })
    })
    .await
    .map_err(|_| LinkError::TimedOut)?
}

/// Runs the responder's handshake on an inbound stream of the Onion
/// Service of `identity`, with the party that identity answers with. Once
/// the handshake has authenticated the card the initiator presented, with
/// nothing left to wait for, the contact store of `identity` admits it
/// against the record of its identity as it is then and keeps the
/// [`Withdrawal`] with the session, in one step. The caller holds the
/// inbound handshake slot from the accept loop until this returns.
///
/// The budgets are applied after the admission, to the standing it decided
/// (`docs/PROTOCOL.md` section 6.2): a contact takes a slot of the contact
/// session budgets, any other peer a slot for strangers, which may evict
/// the oldest silent stranger (`crate::strangers`). Without a slot the
/// stream is closed and nothing is sent; what the admission recorded
/// stands, a pending successor included, and is made durable before the
/// refusal is returned, and the link's [`Withdrawal`] reports
/// [`Withdrawal::is_ended`]. The established link is returned only
/// once the state the admission depended on is durable.
pub async fn answer<S>(
    mut stream: S,
    budgets: &Budgets,
    identity: &LocalIdentity,
) -> Result<Established<S>, LinkError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // A deleted identity answers nobody: it has no party any more.
    let local = identity
        .answering_party()
        .map_err(|_| LinkError::Storage(CommitError::Failed))?;
    tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        let responder = HandshakeResponder::new(&local, now())?;
        let message_1 = read_message::<_, HANDSHAKE_MSG1_LEN>(&mut stream).await?;
        let (waiting, message_2) = responder.read_message_1(&message_1, now())?;
        write_all(&mut stream, &message_2).await?;
        let message_3 = read_message::<_, HANDSHAKE_MSG3_LEN>(&mut stream).await?;
        let inbound = waiting.read_message_3(&message_3, now())?;
        // The last wait for the peer is behind. From here to the session
        // nothing waits for it.
        let withdrawal = Withdrawal::new();
        let ((session, admission, first), depends) = identity
            .admit_inbound(inbound, &withdrawal)
            .inspect_err(|_| withdrawal.end())?;
        let (unknown_slot, contact_slot) = if session.standing().is_contact_record() {
            let Some(slot) = budgets.contact_session(identity) else {
                withdrawal.end();
                keep_recorded(identity, depends).await?;
                return Err(LinkError::Budget);
            };
            (None, Some(slot))
        } else {
            let Some(slot) = identity.strangers.take() else {
                withdrawal.end();
                keep_recorded(identity, depends).await?;
                return Err(LinkError::Budget);
            };
            (Some(slot), None)
        };
        let cancelled = EndIfCancelled::new(&withdrawal);
        let durable = identity.wait_durable(depends).await;
        cancelled.disarm();
        if let Err(error) = durable {
            withdrawal.end();
            return Err(LinkError::Storage(error));
        }
        Ok(Established {
            link: Link::new(stream, session, withdrawal, unknown_slot, contact_slot),
            admission,
            first,
        })
    })
    .await
    .map_err(|_| LinkError::TimedOut)?
}

/// Makes durable what an admission that leads to no session recorded: a
/// pending successor, a conflict, a newer card. It stands, and the user is
/// shown it, so it is written like any other change; a restart does not
/// lose it.
async fn keep_recorded(identity: &LocalIdentity, depends: u64) -> Result<(), LinkError> {
    identity
        .wait_durable(depends)
        .await
        .map_err(LinkError::Storage)
}

/// What ended a wait for the peer.
enum Wake {
    Withdrawn,
    Evicted,
    Deadline,
    Read(std::io::Result<usize>),
}

impl<S: AsyncRead + AsyncWrite + Unpin> Link<S> {
    fn new(
        stream: S,
        session: AuthenticatedSession,
        withdrawal: Withdrawal,
        unknown_slot: Option<StrangerSlot>,
        contact_slot: Option<ContactPermit>,
    ) -> Self {
        Self {
            stream,
            session,
            withdrawal,
            unknown_slot,
            contact_slot,
            buffer: Box::new([0_u8; READ_CHUNK]),
            start: 0,
            end: 0,
            finished: false,
            writing: false,
            taken: None,
            #[cfg(test)]
            hook: None,
        }
    }

    /// Ends the link if a write of an earlier call was dropped part way.
    /// Returns the error for the call, or `None` if the link can go on.
    async fn interrupted(&mut self) -> Option<LinkError> {
        if !self.writing {
            return None;
        }
        self.writing = false;
        self.finish().await;
        Some(LinkError::Stream)
    }

    /// Writes `frame`, a frame or handshake message, in whole, and flushes
    /// it, within `FRAME_WRITE_TIMEOUT`. If the call is dropped while it
    /// writes, the link is over from the next call on.
    async fn write_frame(&mut self, frame: &[u8]) -> Result<(), LinkError> {
        self.writing = true;
        let written = tokio::time::timeout(FRAME_WRITE_TIMEOUT, write_all(&mut self.stream, frame))
            .await
            .map_err(|_| LinkError::TimedOut)
            .and_then(|result| result);
        self.writing = false;
        written
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

    /// Returns true once the link is over. Every later call fails.
    pub const fn is_over(&self) -> bool {
        self.session.is_over() || self.writing
    }

    /// Returns true while the link holds a slot of `MAX_UNKNOWN_SESSIONS`.
    /// It gives the slot back when the session is over.
    pub const fn holds_unknown_slot(&self) -> bool {
        self.unknown_slot.is_some()
    }

    /// Returns true while the link holds a slot of the contact session
    /// budgets. It gives the slot back when the session is over.
    pub const fn holds_contact_slot(&self) -> bool {
        self.contact_slot.is_some()
    }

    /// What the contact store needs to apply a message this link returned:
    /// the card of the peer, the local card, the withdrawal.
    pub fn session_ref(&self) -> SessionRef<'_> {
        SessionRef {
            peer: self.session.peer_card(),
            local: self.session.local_card(),
            withdrawal: &self.withdrawal,
        }
    }

    fn evicted(&self) -> bool {
        self.unknown_slot
            .as_ref()
            .is_some_and(|slot| slot.state().is_evicted())
    }

    /// The one way a link ends: the session is over, the slot goes back,
    /// and the stream is shut down within `FRAME_WRITE_TIMEOUT`. It does
    /// this once; afterwards it returns at once.
    async fn finish(&mut self) {
        self.session.stream_closed();
        self.unknown_slot = None;
        self.contact_slot = None;
        if self.finished {
            return;
        }
        self.finished = true;
        let _ = tokio::time::timeout(FRAME_WRITE_TIMEOUT, self.stream.shutdown()).await;
    }

    /// The error of a call on a link that is over.
    fn over(&mut self) -> LinkError {
        self.unknown_slot = None;
        self.contact_slot = None;
        if self.withdrawal.is_withdrawn() {
            LinkError::Withdrawn
        } else {
            LinkError::Session(SessionError::Closed)
        }
    }

    /// Returns true if a deadline of the session has passed at `now`.
    fn deadline_passed(&self, now: Instant) -> bool {
        self.session
            .deadline()
            .is_some_and(|deadline| now >= deadline)
    }

    /// Writes `bytes` unless the session is withdrawn before the write
    /// starts or while it is under way. The withdrawal is looked at before
    /// every call that hands bytes to the stream, the first included, so
    /// nothing is handed over once it is seen, also when the stream would
    /// take more at that moment. A withdrawal or a failed write ends the
    /// link without a Close: part of the bytes may be on the stream. A call
    /// that is dropped while this runs leaves the link to end at the next
    /// call ([`Self::interrupted`]).
    async fn write_unless_withdrawn(&mut self, bytes: &[u8]) -> Result<(), LinkError> {
        self.writing = true;
        let outcome = {
            let withdrawal = &self.withdrawal;
            let stream = &mut self.stream;
            let mut withdrawn = pin!(withdrawal.0.wake.notified());
            let mut written = 0_usize;
            poll_fn(|cx| {
                loop {
                    if withdrawal.is_withdrawn() || withdrawn.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(None);
                    }
                    let Some(rest) = bytes.get(written..).filter(|rest| !rest.is_empty()) else {
                        return Pin::new(&mut *stream)
                            .poll_flush(cx)
                            .map(|flushed| Some(flushed.map_err(|_| LinkError::Stream)));
                    };
                    match Pin::new(&mut *stream).poll_write(cx, rest) {
                        Poll::Ready(Ok(0) | Err(_)) => {
                            return Poll::Ready(Some(Err(LinkError::Stream)));
                        }
                        Poll::Ready(Ok(taken)) => {
                            written = written.saturating_add(taken.min(rest.len()));
                        }
                        Poll::Pending => return Poll::Pending,
                    }
                }
            })
            .await
        };
        self.writing = false;
        match outcome {
            Some(Ok(())) => Ok(()),
            Some(Err(error)) => {
                self.finish().await;
                Err(error)
            }
            None => {
                // The session gives up its standing; its Close is not
                // written after what may be part of a frame.
                let _ = self.session.withdraw();
                self.finish().await;
                Err(LinkError::Withdrawn)
            }
        }
    }

    /// Ends a withdrawn session: the session gives up its standing, the
    /// Close is written if it was not yet, within `FRAME_WRITE_TIMEOUT`,
    /// and the link ends.
    async fn end_withdrawn(&mut self) -> LinkError {
        if let Some(frame) = self.session.withdraw() {
            let _ = self.write_frame(&frame).await;
        }
        self.finish().await;
        LinkError::Withdrawn
    }

    /// Ends the session of a stranger that was evicted for a newer one:
    /// with the Close a stranger gets when its time is up, written within
    /// `FRAME_WRITE_TIMEOUT`.
    async fn end_evicted(&mut self) -> LinkError {
        if let Some(frame) = self.session.close() {
            let _ = self.write_frame(&frame).await;
        }
        self.finish().await;
        LinkError::Evicted
    }

    /// Ends a session whose deadline passed: with its Close, if the
    /// deadline calls for one, written within `FRAME_WRITE_TIMEOUT`.
    async fn end_expired(&mut self, now: Instant) -> LinkError {
        if let Expiry::Close(Some(frame)) = self.session.expire(now) {
            let _ = self.write_frame(&frame).await;
        }
        self.finish().await;
        LinkError::TimedOut
    }

    /// Encrypts a message and writes it, within `FRAME_WRITE_TIMEOUT`.
    /// Nothing is sent on a link that is over or withdrawn, or once a
    /// deadline of the session has passed. A failed write ends the link:
    /// part of a frame may be on the stream. A withdrawal while the frame
    /// is written ends the write and the link, without a Close. A message
    /// that may not be sent now fails with the error of the session and
    /// changes nothing.
    pub async fn send(&mut self, message: &Message) -> Result<(), LinkError> {
        if let Some(error) = self.interrupted().await {
            return Err(error);
        }
        if self.session.is_over() {
            return Err(self.over());
        }
        if self.withdrawal.is_withdrawn() {
            return Err(self.end_withdrawn().await);
        }
        let now = now();
        if self.deadline_passed(now) {
            return Err(self.end_expired(now).await);
        }
        let frame = match self.session.send(message, now) {
            Ok(frame) => frame,
            Err(error) => {
                if self.session.is_over() {
                    self.finish().await;
                }
                return Err(error.into());
            }
        };
        if let Ok(written) =
            tokio::time::timeout(FRAME_WRITE_TIMEOUT, self.write_unless_withdrawn(&frame)).await
        {
            return written;
        }
        // The write was given up here, not dropped by the caller: the link
        // ends now.
        self.writing = false;
        self.finish().await;
        Err(LinkError::TimedOut)
    }

    /// Completes an end the session decided on a message it received. The
    /// Close that is due after the first message of a peer that is not a
    /// contact ([`Action::SendClose`]) is written within
    /// `FRAME_WRITE_TIMEOUT`, and a link whose session is over, because
    /// that Close went out or the peer closed, ends at once and gives back
    /// its slot.
    async fn complete_end(&mut self) {
        if self.session.state() == SessionState::Closing {
            if let Some(frame) = self.session.close() {
                let _ = self.write_frame(&frame).await;
            }
        }
        if self.session.is_over() {
            self.finish().await;
        }
    }

    /// Waits for the next message. Fails when the peer breaks the
    /// protocol, the stream ends or fails, a deadline of the session
    /// passes, or the session is withdrawn. After a failure the link is
    /// over, and every later call fails at once.
    ///
    /// A withdrawal and the deadlines are looked at before each frame is
    /// taken from the buffer and before every wait for the peer, and they
    /// end a wait that is in progress. A frame that was not taken when the
    /// session was withdrawn is not delivered.
    ///
    /// When a message ends the session, the link ends before it is
    /// returned: a Close the session logic decided ([`Action::SendClose`])
    /// has been written, and after a Close from the peer the stream is shut
    /// down. Either way the slot for strangers is back. A receive that is
    /// dropped after it took a message, while it ends the link, does not
    /// lose the message: the next receive returns it, and the one after
    /// fails.
    pub async fn receive(&mut self) -> Result<Received, LinkError> {
        if let Some(received) = self.taken.take() {
            return Ok(received);
        }
        if let Some(error) = self.interrupted().await {
            return Err(error);
        }
        loop {
            if self.session.is_over() {
                return Err(self.over());
            }
            if self.withdrawal.is_withdrawn() {
                return Err(self.end_withdrawn().await);
            }
            if self.evicted() {
                return Err(self.end_evicted().await);
            }
            let now = now();
            if self.deadline_passed(now) {
                return Err(self.end_expired(now).await);
            }
            if self.start < self.end {
                let pending = self
                    .buffer
                    .get(self.start..self.end)
                    .ok_or(LinkError::Stream)?;
                match self.session.receive(pending, now) {
                    Ok((used, received)) => {
                        self.start = self.start.saturating_add(used).min(self.end);
                        if let Some(received) = received {
                            #[cfg(test)]
                            if let Some(step) = self.hook.take() {
                                step();
                            }
                            // A stranger's first message and its eviction
                            // race on one state; an evicted stranger's
                            // message is not delivered.
                            if self
                                .unknown_slot
                                .as_ref()
                                .is_some_and(|slot| !slot.state().heard())
                            {
                                return Err(self.end_evicted().await);
                            }
                            // Kept until it is returned: a receive dropped
                            // while it ends the link does not lose it.
                            self.taken = Some(received);
                            self.complete_end().await;
                            return self.taken.take().ok_or(LinkError::Stream);
                        }
                    }
                    Err(error) => {
                        self.finish().await;
                        return Err(error.into());
                    }
                }
                continue;
            }
            // Everything read so far was taken; only now is more read, or
            // the wait ends: the session was withdrawn or a deadline came.
            let deadline = self.session.deadline().map(tokio::time::Instant::from_std);
            let wake = {
                let mut withdrawn = pin!(self.withdrawal.0.wake.notified());
                let stranger = self.unknown_slot.as_ref().map(|slot| slot.state().clone());
                let mut evicted = pin!(async {
                    match stranger {
                        Some(state) => state.evicted().await,
                        None => core::future::pending::<()>().await,
                    }
                });
                let mut timer = pin!(async {
                    match deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => core::future::pending::<()>().await,
                    }
                });
                let mut read = pin!(self.stream.read(self.buffer.as_mut_slice()));
                poll_fn(|cx| {
                    if withdrawn.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(Wake::Withdrawn);
                    }
                    if evicted.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(Wake::Evicted);
                    }
                    if timer.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(Wake::Deadline);
                    }
                    read.as_mut().poll(cx).map(Wake::Read)
                })
                .await
            };
            match wake {
                Wake::Withdrawn => return Err(self.end_withdrawn().await),
                Wake::Evicted => return Err(self.end_evicted().await),
                Wake::Deadline => return Err(self.end_expired(self::now()).await),
                Wake::Read(Ok(0) | Err(_)) => {
                    self.finish().await;
                    return Err(LinkError::Stream);
                }
                Wake::Read(Ok(read)) => {
                    self.start = 0;
                    self.end = read;
                }
            }
        }
    }

    /// Ends the session: writes the Close, if one is due, and shuts the
    /// stream down, each within `FRAME_WRITE_TIMEOUT`. On a link that is
    /// over already it returns at once.
    pub async fn close(mut self) -> Result<(), LinkError> {
        if let Some(error) = self.interrupted().await {
            return Err(error);
        }
        let written = match self.session.close() {
            Some(frame) => self.write_frame(&frame).await,
            None => Ok(()),
        };
        self.finish().await;
        written
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

#[cfg(test)]
mod tests {
    use core::pin::Pin;
    use core::task::Context;
    use std::sync::Mutex;

    use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};
    use monolith_protocol::card::EndpointSet;
    use monolith_protocol::credential::Credentials;
    use monolith_protocol::session::PeerRecord;
    use monolith_session::{LocalParty, TransportSecretKey};
    use tokio::io::ReadBuf;

    use super::*;

    fn party(seed: u8) -> LocalParty {
        let endpoint = IdentitySecretKey::from_seed(&[seed.wrapping_add(100); 32]).public_key();
        LocalParty::issue(
            &IdentitySecretKey::from_seed(&[seed; 32]),
            TransportSecretKey::from_bytes(&[seed ^ 0xA5; 32]).unwrap(),
            EndpointEpoch::FIRST,
            EndpointSet::single(OnionServiceKey::from_bytes(endpoint.as_bytes()).unwrap()),
        )
        .unwrap()
    }

    /// A handshake between Alice and Bob, who hold each other as accepted
    /// contacts. Returns Alice's session, Bob's session and message 3.
    fn sessions() -> (
        AuthenticatedSession,
        AuthenticatedSession,
        [u8; HANDSHAKE_MSG3_LEN],
    ) {
        let (alice, bob) = (party(1), party(2));
        let (initiator, message_1) = HandshakeInitiator::start(&alice, bob.card(), now()).unwrap();
        let responder = HandshakeResponder::new(&bob, now()).unwrap();
        let (waiting, message_2) = responder.read_message_1(&message_1, now()).unwrap();
        let outbound = initiator.read_message_2(&message_2, now()).unwrap();
        let OutboundAdmission::Granted {
            session: at_alice,
            message_3,
            ..
        } = outbound
            .admit(PeerRecord::Accepted(&mut Credentials::new(
                bob.card().clone(),
            )))
            .unwrap()
        else {
            panic!("refused");
        };
        let message_3 = *message_3.as_bytes();
        let inbound = waiting.read_message_3(&message_3, now()).unwrap();
        let (at_bob, _, _) = inbound
            .admit(PeerRecord::Accepted(&mut Credentials::new(
                alice.card().clone(),
            )))
            .unwrap();
        (at_alice, at_bob, message_3)
    }

    /// A stream that takes `room` bytes and then accepts nothing more, and
    /// never has anything to read. What it took is kept.
    struct Stalling {
        room: usize,
        taken: std::sync::Arc<Mutex<Vec<u8>>>,
    }

    impl AsyncWrite for Stalling {
        fn poll_write(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            if self.room == 0 {
                return Poll::Pending;
            }
            let taken = self.room.min(bytes.len());
            self.room = self.room.saturating_sub(taken);
            self.taken
                .lock()
                .unwrap()
                .extend_from_slice(&bytes[..taken]);
            Poll::Ready(Ok(taken))
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncRead for Stalling {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Pending
        }
    }

    fn stalling(room: usize) -> (Stalling, std::sync::Arc<Mutex<Vec<u8>>>) {
        let taken = std::sync::Arc::new(Mutex::new(Vec::new()));
        (
            Stalling {
                room,
                taken: taken.clone(),
            },
            taken,
        )
    }

    /// Runs a test in paused time. A test that waits for something that
    /// never comes fails instead of hanging: the clock moves on to the
    /// limit when nothing else is due.
    fn run<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap()
            .block_on(async {
                tokio::time::timeout(core::time::Duration::from_secs(30 * 24 * 60 * 60), future)
                    .await
                    .expect("the test waited for something that never came")
            })
    }

    #[test]
    fn message_3_is_not_written_once_the_session_is_withdrawn() {
        run(async {
            let (session, _, message_3) = sessions();
            let (stream, taken) = stalling(usize::MAX);
            let mut link = Link::new(stream, session, Withdrawal::new(), None, None);
            link.withdrawal.withdraw();
            assert_eq!(
                link.write_unless_withdrawn(&message_3).await,
                Err(LinkError::Withdrawn)
            );
            assert!(taken.lock().unwrap().is_empty());
            assert_eq!(
                link.session.state(),
                monolith_protocol::SessionState::Closed
            );
        });
    }

    #[test]
    fn a_withdrawal_ends_a_write_of_message_3_that_is_pending() {
        // The stream takes 100 bytes of message 3 and then stalls. The
        // withdrawal ends the write; the 100 bytes it took before cannot
        // be called back, and nothing is written after them.
        run(async {
            let (session, _, message_3) = sessions();
            let (stream, taken) = stalling(100);
            let mut link = Link::new(stream, session, Withdrawal::new(), None, None);
            let withdrawal = link.withdrawal.clone();
            let mut written = None;
            let mut retired = false;
            {
                let write = link.write_unless_withdrawn(&message_3);
                let retire = async {
                    tokio::task::yield_now().await;
                    withdrawal.withdraw();
                };
                let mut write = pin!(write);
                let mut retire = pin!(retire);
                poll_fn(|cx| {
                    if written.is_none() {
                        if let Poll::Ready(result) = write.as_mut().poll(cx) {
                            written = Some(result);
                        }
                    }
                    if !retired && retire.as_mut().poll(cx).is_ready() {
                        retired = true;
                    }
                    if written.is_some() {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                })
                .await;
            }
            assert_eq!(written, Some(Err(LinkError::Withdrawn)));
            assert_eq!(taken.lock().unwrap().as_slice(), &message_3[..100]);
            assert_eq!(
                link.session.state(),
                monolith_protocol::SessionState::Closed
            );
        });
    }

    /// How Bob holds Alice in the sessions below.
    #[derive(Clone, Copy)]
    enum BobHolds {
        Nothing,
        Accepted,
    }

    /// A handshake in which Alice holds Bob as requested and Bob holds
    /// Alice as `bob_holds`. Returns Alice's and Bob's sessions.
    fn unconfirmed(bob_holds: BobHolds) -> (AuthenticatedSession, AuthenticatedSession) {
        let (alice, bob) = (party(1), party(2));
        let (initiator, message_1) = HandshakeInitiator::start(&alice, bob.card(), now()).unwrap();
        let responder = HandshakeResponder::new(&bob, now()).unwrap();
        let (waiting, message_2) = responder.read_message_1(&message_1, now()).unwrap();
        let outbound = initiator.read_message_2(&message_2, now()).unwrap();
        let OutboundAdmission::Granted {
            session: at_alice,
            message_3,
            ..
        } = outbound
            .admit(PeerRecord::Requested(&mut Credentials::new(
                bob.card().clone(),
            )))
            .unwrap()
        else {
            panic!("refused");
        };
        let inbound = waiting.read_message_3(message_3.as_bytes(), now()).unwrap();
        let mut held = Credentials::new(alice.card().clone());
        let record = match bob_holds {
            BobHolds::Nothing => PeerRecord::None,
            BobHolds::Accepted => PeerRecord::Accepted(&mut held),
        };
        let (at_bob, _, _) = inbound.admit(record).unwrap();
        (at_alice, at_bob)
    }

    /// Alice and Bob as accepted contacts, confirmed on both sides.
    fn confirmed() -> (AuthenticatedSession, AuthenticatedSession) {
        let (mut alice, mut bob, _) = sessions();
        let to_bob = alice.send(&Message::ContactAccept, now()).unwrap();
        let to_alice = bob.send(&Message::ContactAccept, now()).unwrap();
        bob.receive(&to_bob, now()).unwrap();
        alice.receive(&to_alice, now()).unwrap();
        (alice, bob)
    }

    fn chat() -> Message {
        Message::ChatMessage {
            id: monolith_protocol::body::MessageId::from_bytes([1; 16]),
            text: monolith_protocol::text::ChatText::new("hello").unwrap(),
        }
    }

    /// Runs `bob` until it is done and `peer` alongside it.
    async fn alongside<A: Future, B: Future<Output = ()>>(bob: A, peer: B) -> A::Output {
        let mut bob = pin!(bob);
        let mut peer = pin!(peer);
        let mut peer_done = false;
        poll_fn(|cx| {
            if !peer_done && peer.as_mut().poll(cx).is_ready() {
                peer_done = true;
            }
            bob.as_mut().poll(cx)
        })
        .await
    }

    fn assert_over<S: AsyncRead + AsyncWrite + Unpin>(link: &Link<S>) {
        assert!(link.is_over());
        assert!(!link.holds_unknown_slot());
    }

    #[test]
    fn a_frame_sent_a_byte_at_a_time_ends_at_the_frame_deadline() {
        // The peer sends one byte of a frame and another every 59 seconds,
        // each within what a timeout per read would allow. The frame has
        // to complete within FRAME_READ_TIMEOUT of its first byte.
        run(async {
            let (mut alice, bob) = confirmed();
            let frame = alice.send(&chat(), now()).unwrap();
            let (mut peer_end, bob_end) = tokio::io::duplex(1 << 16);
            let mut link = Link::new(bob_end, bob, Withdrawal::new(), None, None);
            let started = tokio::time::Instant::now();
            let peer = async {
                for byte in &frame {
                    if peer_end
                        .write_all(core::slice::from_ref(byte))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    tokio::time::sleep(core::time::Duration::from_secs(59)).await;
                }
            };
            let result = alongside(link.receive(), peer).await;
            assert_eq!(result.err(), Some(LinkError::TimedOut));
            assert_eq!(
                started.elapsed(),
                monolith_protocol::limits::FRAME_READ_TIMEOUT
            );
            assert_over(&link);
            // Nothing revives it: later calls fail at once.
            assert_eq!(
                link.receive().await.err(),
                Some(LinkError::Session(SessionError::Closed))
            );
            assert_eq!(
                link.send(&chat()).await.err(),
                Some(LinkError::Session(SessionError::Closed))
            );
        });
    }

    #[test]
    fn a_stranger_that_sends_nothing_is_closed_and_frees_its_slot() {
        run(async {
            let strangers =
                crate::strangers::Strangers::new(monolith_protocol::limits::MAX_UNKNOWN_SESSIONS);
            let (mut alice, bob) = unconfirmed(BobHolds::Nothing);
            let (mut peer_end, bob_end) = tokio::io::duplex(1 << 16);
            let slot = strangers.take();
            let mut link = Link::new(bob_end, bob, Withdrawal::new(), slot, None);
            assert!(link.holds_unknown_slot());
            let started = tokio::time::Instant::now();
            assert_eq!(link.receive().await.err(), Some(LinkError::TimedOut));
            assert_eq!(
                started.elapsed(),
                monolith_protocol::limits::UNKNOWN_FIRST_MESSAGE_TIMEOUT
            );
            assert_over(&link);
            assert_eq!(
                strangers.free(),
                monolith_protocol::limits::MAX_UNKNOWN_SESSIONS
            );
            // The stranger sees an ordinary Close.
            let mut bytes = Vec::new();
            peer_end.read_to_end(&mut bytes).await.unwrap();
            let (_, received) = alice.receive(&bytes, now()).unwrap();
            assert_eq!(received.unwrap().message, Message::Close);
        });
    }

    /// Alice's contact request.
    fn request() -> Message {
        Message::ContactRequest(Box::new(monolith_protocol::body::ContactRequest {
            card: party(1).card().clone(),
            invitation: None,
            display_name: monolith_protocol::text::DisplayName::new("Alice").unwrap(),
            introduction: monolith_protocol::text::IntroductionText::new("hi").unwrap(),
        }))
    }

    #[test]
    fn a_stranger_evicted_as_its_first_message_is_taken_gets_nothing_delivered() {
        // Alice's request is in Bob's buffer and the session has taken it.
        // Right then, before the link claims her slot for a stranger who
        // was heard, a newer stranger takes the slot and evicts her. Her
        // request is not delivered, and the newcomer keeps the slot.
        run(async {
            let strangers = crate::strangers::Strangers::new(1);
            let (mut alice, bob) = unconfirmed(BobHolds::Nothing);
            let frame = alice.send(&request(), now()).unwrap();
            let (mut peer_end, bob_end) = tokio::io::duplex(1 << 16);
            peer_end.write_all(&frame).await.unwrap();
            let mut link = Link::new(bob_end, bob, Withdrawal::new(), strangers.take(), None);
            let newcomer = std::sync::Arc::new(Mutex::new(None));
            link.hook = Some(Box::new({
                let strangers = strangers.clone();
                let newcomer = newcomer.clone();
                move || {
                    *newcomer.lock().unwrap() = strangers.take();
                }
            }));
            assert_eq!(link.receive().await.err(), Some(LinkError::Evicted));
            assert!(newcomer.lock().unwrap().is_some());
            assert_over(&link);
        });
    }

    #[test]
    fn a_stranger_gets_the_close_after_its_first_message_and_frees_its_slot() {
        // Bob holds no record of Alice. Her request is returned with the
        // Close the session decided already written, and the slot is back
        // at once, whether or not the caller closes the link.
        run(async {
            let strangers =
                crate::strangers::Strangers::new(monolith_protocol::limits::MAX_UNKNOWN_SESSIONS);
            let (mut alice, bob) = unconfirmed(BobHolds::Nothing);
            let frame = alice.send(&request(), now()).unwrap();
            let (mut peer_end, bob_end) = tokio::io::duplex(1 << 16);
            peer_end.write_all(&frame).await.unwrap();
            let mut link = Link::new(bob_end, bob, Withdrawal::new(), strangers.take(), None);
            let started = tokio::time::Instant::now();
            let received = link.receive().await.unwrap();
            assert!(received.actions.contains(&Action::SendClose));
            assert_eq!(started.elapsed(), core::time::Duration::ZERO);
            assert_over(&link);
            assert_eq!(
                strangers.free(),
                monolith_protocol::limits::MAX_UNKNOWN_SESSIONS
            );
            let mut bytes = Vec::new();
            peer_end.read_to_end(&mut bytes).await.unwrap();
            let (_, received) = alice.receive(&bytes, now()).unwrap();
            assert_eq!(received.unwrap().message, Message::Close);
            assert_eq!(
                link.receive().await.err(),
                Some(LinkError::Session(SessionError::Closed))
            );
            // Closing it afterwards writes nothing more.
            link.close().await.unwrap();
        });
    }

    #[test]
    fn a_close_from_the_peer_ends_the_link_and_frees_its_slot() {
        run(async {
            let strangers =
                crate::strangers::Strangers::new(monolith_protocol::limits::MAX_UNKNOWN_SESSIONS);
            let (mut alice, bob) = confirmed();
            let close = alice.close().unwrap();
            let (mut peer_end, bob_end) = tokio::io::duplex(1 << 16);
            peer_end.write_all(&close).await.unwrap();
            let mut link = Link::new(bob_end, bob, Withdrawal::new(), strangers.take(), None);
            let received = link.receive().await.unwrap();
            assert_eq!(received.message, Message::Close);
            assert_over(&link);
            assert_eq!(
                strangers.free(),
                monolith_protocol::limits::MAX_UNKNOWN_SESSIONS
            );
            // The stream was shut down.
            let mut bytes = Vec::new();
            peer_end.read_to_end(&mut bytes).await.unwrap();
            assert!(bytes.is_empty());
        });
    }

    #[test]
    fn a_withdrawal_ends_a_send_that_is_pending() {
        // The stream takes 10 bytes of a chat frame and stalls. The key is
        // retired meanwhile: the write ends at once, nothing follows the
        // 10 bytes, not even a Close, and the link is over.
        run(async {
            let (_, bob) = confirmed();
            let (stream, taken) = stalling(10);
            let mut link = Link::new(stream, bob, Withdrawal::new(), None, None);
            let withdrawal = link.withdrawal.clone();
            let started = tokio::time::Instant::now();
            let sent = alongside(link.send(&chat()), async {
                tokio::task::yield_now().await;
                withdrawal.withdraw();
            })
            .await;
            assert_eq!(sent, Err(LinkError::Withdrawn));
            assert_eq!(started.elapsed(), core::time::Duration::ZERO);
            assert_eq!(taken.lock().unwrap().len(), 10);
            assert_over(&link);
            assert_eq!(link.send(&chat()).await.err(), Some(LinkError::Withdrawn));
        });
    }

    #[test]
    fn an_unconfirmed_session_ends_in_time() {
        // Bob holds Alice as accepted. She sends a request and never her
        // ContactAccept: the session stays unconfirmed.
        run(async {
            let (mut alice, bob) = unconfirmed(BobHolds::Accepted);
            let frame = alice.send(&request(), now()).unwrap();
            let (mut peer_end, bob_end) = tokio::io::duplex(1 << 16);
            peer_end.write_all(&frame).await.unwrap();
            let mut link = Link::new(bob_end, bob, Withdrawal::new(), None, None);
            let started = tokio::time::Instant::now();
            let received = link.receive().await.unwrap();
            assert!(matches!(received.message, Message::ContactRequest(_)));
            assert_eq!(link.receive().await.err(), Some(LinkError::TimedOut));
            assert_eq!(
                started.elapsed(),
                monolith_protocol::limits::UNKNOWN_SESSION_TIMEOUT
            );
            assert_over(&link);
        });
    }

    /// A stream whose reads and writes fail.
    struct Broken;

    impl AsyncRead for Broken {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Ready(Err(std::io::ErrorKind::ConnectionReset.into()))
        }
    }

    impl AsyncWrite for Broken {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Err(std::io::ErrorKind::ConnectionReset.into()))
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    /// A stream that takes every write, fails every read if asked to and
    /// otherwise never has anything to read, and whose shutdown never
    /// completes.
    struct StuckShutdown {
        read_fails: bool,
    }

    impl AsyncRead for StuckShutdown {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.read_fails {
                Poll::Ready(Err(std::io::ErrorKind::ConnectionReset.into()))
            } else {
                Poll::Pending
            }
        }
    }

    impl AsyncWrite for StuckShutdown {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Ok(bytes.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Pending
        }
    }

    #[test]
    fn a_withdrawn_link_waits_for_its_stream_once() {
        // The first call ends the link: it writes the Close and gives the
        // shutdown FRAME_WRITE_TIMEOUT. Every later call fails at once.
        run(async {
            let (_, bob) = confirmed();
            let stream = StuckShutdown { read_fails: false };
            let mut link = Link::new(stream, bob, Withdrawal::new(), None, None);
            link.withdrawal.withdraw();
            let started = tokio::time::Instant::now();
            assert_eq!(link.receive().await.err(), Some(LinkError::Withdrawn));
            assert_eq!(started.elapsed(), FRAME_WRITE_TIMEOUT);
            let started = tokio::time::Instant::now();
            assert_eq!(link.receive().await.err(), Some(LinkError::Withdrawn));
            assert_eq!(link.send(&chat()).await.err(), Some(LinkError::Withdrawn));
            link.close().await.unwrap();
            assert_eq!(started.elapsed(), core::time::Duration::ZERO);
        });
    }

    #[test]
    fn a_failed_link_waits_for_its_stream_once() {
        run(async {
            let (_, bob) = confirmed();
            let stream = StuckShutdown { read_fails: true };
            let mut link = Link::new(stream, bob, Withdrawal::new(), None, None);
            let started = tokio::time::Instant::now();
            assert_eq!(link.receive().await.err(), Some(LinkError::Stream));
            assert_eq!(started.elapsed(), FRAME_WRITE_TIMEOUT);
            let started = tokio::time::Instant::now();
            assert_eq!(
                link.receive().await.err(),
                Some(LinkError::Session(SessionError::Closed))
            );
            assert_eq!(
                link.send(&chat()).await.err(),
                Some(LinkError::Session(SessionError::Closed))
            );
            link.close().await.unwrap();
            assert_eq!(started.elapsed(), core::time::Duration::ZERO);
        });
    }

    #[test]
    fn a_read_error_ends_the_link() {
        run(async {
            let (_, bob) = confirmed();
            let strangers =
                crate::strangers::Strangers::new(monolith_protocol::limits::MAX_UNKNOWN_SESSIONS);
            let mut link = Link::new(Broken, bob, Withdrawal::new(), strangers.take(), None);
            assert_eq!(link.receive().await.err(), Some(LinkError::Stream));
            assert_over(&link);
            assert_eq!(
                strangers.free(),
                monolith_protocol::limits::MAX_UNKNOWN_SESSIONS
            );
            assert_eq!(
                link.receive().await.err(),
                Some(LinkError::Session(SessionError::Closed))
            );
            assert_eq!(
                link.send(&chat()).await.err(),
                Some(LinkError::Session(SessionError::Closed))
            );
        });
    }

    #[test]
    fn a_write_error_ends_the_link() {
        run(async {
            let (bob, _) = confirmed();
            let mut link = Link::new(Broken, bob, Withdrawal::new(), None, None);
            assert_eq!(link.send(&chat()).await.err(), Some(LinkError::Stream));
            assert_over(&link);
            assert_eq!(
                link.send(&chat()).await.err(),
                Some(LinkError::Session(SessionError::Closed))
            );
            assert_eq!(
                link.receive().await.err(),
                Some(LinkError::Session(SessionError::Closed))
            );
        });
    }

    #[test]
    fn a_withdrawal_at_the_moment_a_write_times_out_leaves_the_link_terminal() {
        // The stream takes nothing. The key is retired exactly when the
        // write deadline passes. Either outcome may be reported, and the
        // link is over either way: nothing is delivered or sent afterwards.
        for retire_after in [
            FRAME_WRITE_TIMEOUT - core::time::Duration::from_millis(1),
            FRAME_WRITE_TIMEOUT,
            FRAME_WRITE_TIMEOUT + core::time::Duration::from_millis(1),
        ] {
            run(async {
                let (_, bob) = confirmed();
                let (stream, taken) = stalling(0);
                let mut link = Link::new(stream, bob, Withdrawal::new(), None, None);
                let withdrawal = link.withdrawal.clone();
                let sent = alongside(link.send(&chat()), async move {
                    tokio::time::sleep(retire_after).await;
                    withdrawal.withdraw();
                })
                .await;
                assert!(
                    matches!(sent, Err(LinkError::TimedOut | LinkError::Withdrawn)),
                    "{sent:?}"
                );
                assert!(taken.lock().unwrap().is_empty());
                assert_over(&link);
                let started = tokio::time::Instant::now();
                assert!(link.send(&chat()).await.is_err());
                assert!(link.receive().await.is_err());
                link.close().await.unwrap();
                assert_eq!(started.elapsed(), core::time::Duration::ZERO);
            });
        }
    }

    /// A stream that takes one byte per write call and withdraws the link
    /// right after it took the byte number `at`: the withdrawal comes
    /// between two writes, at the moment the stream could take more.
    struct Trickling {
        at: usize,
        withdrawal: Withdrawal,
        taken: std::sync::Arc<Mutex<Vec<u8>>>,
    }

    impl AsyncWrite for Trickling {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            let Some(byte) = bytes.first() else {
                return Poll::Ready(Ok(0));
            };
            let mut taken = self.taken.lock().unwrap();
            taken.push(*byte);
            if taken.len() == self.at {
                self.withdrawal.withdraw();
            }
            Poll::Ready(Ok(1))
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncRead for Trickling {
        fn poll_read(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Poll::Pending
        }
    }

    #[test]
    fn nothing_is_written_after_a_withdrawal_that_comes_between_two_writes() {
        // The stream is always ready and takes a byte at a time; the key
        // is retired right after byte `at`. Not one more byte follows, for
        // message 3 and for a frame of the session.
        for at in [1, 2, 50] {
            run(async {
                let (session, _, message_3) = sessions();
                let withdrawal = Withdrawal::new();
                let taken = std::sync::Arc::new(Mutex::new(Vec::new()));
                let stream = Trickling {
                    at,
                    withdrawal: withdrawal.clone(),
                    taken: taken.clone(),
                };
                let mut link = Link::new(stream, session, withdrawal, None, None);
                assert_eq!(
                    link.write_unless_withdrawn(&message_3).await,
                    Err(LinkError::Withdrawn)
                );
                assert_eq!(taken.lock().unwrap().as_slice(), &message_3[..at]);
                assert_over(&link);

                let (_, bob) = confirmed();
                let withdrawal = Withdrawal::new();
                let taken = std::sync::Arc::new(Mutex::new(Vec::new()));
                let stream = Trickling {
                    at,
                    withdrawal: withdrawal.clone(),
                    taken: taken.clone(),
                };
                let mut link = Link::new(stream, bob, withdrawal, None, None);
                assert_eq!(link.send(&chat()).await, Err(LinkError::Withdrawn));
                assert_eq!(taken.lock().unwrap().len(), at);
                assert_over(&link);
            });
        }
    }

    #[test]
    fn a_send_dropped_while_it_writes_leaves_the_link_terminal() {
        // A send is dropped (a deadline of the caller, an aborted task)
        // after the frame was sealed: before its first byte was taken,
        // after one, after part of the frame. The session counted the frame
        // as sent, and the stream may hold part of it: the link is over,
        // every later call fails at once, and nothing more is written, not
        // even the Close of a withdrawal that comes afterwards.
        for (room, withdraw) in [(0, false), (1, false), (10, false), (10, true)] {
            run(async {
                let (_, bob) = confirmed();
                let (stream, taken) = stalling(room);
                let mut link = Link::new(stream, bob, Withdrawal::new(), None, None);
                let message = chat();
                {
                    let mut send = pin!(link.send(&message));
                    let pending =
                        poll_fn(|cx| Poll::Ready(send.as_mut().poll(cx).is_pending())).await;
                    assert!(pending);
                }
                assert_eq!(taken.lock().unwrap().len(), room);
                if withdraw {
                    link.withdrawal.withdraw();
                }
                let started = tokio::time::Instant::now();
                assert!(link.send(&chat()).await.is_err());
                assert!(link.is_over());
                assert!(link.receive().await.is_err());
                assert!(link.send(&chat()).await.is_err());
                let _ = link.close().await;
                assert_eq!(started.elapsed(), core::time::Duration::ZERO);
                assert_eq!(taken.lock().unwrap().len(), room);
            });
        }
    }

    /// A stream that has `input` to read and takes no write.
    struct Mute {
        input: Vec<u8>,
    }

    impl AsyncRead for Mute {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if self.input.is_empty() {
                return Poll::Pending;
            }
            let n = buffer.remaining().min(self.input.len());
            let taken: Vec<u8> = self.input.drain(..n).collect();
            buffer.put_slice(&taken);
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for Mute {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            _: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Pending
        }

        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[test]
    fn a_message_taken_before_a_receive_was_dropped_is_still_delivered() {
        // A stranger's request was taken from the buffer, and the receive
        // is dropped while it writes the Close the request called for. The
        // request is not lost: the next receive returns it, and the link
        // is over after that.
        run(async {
            let strangers = crate::strangers::Strangers::new(1);
            let (mut alice, bob) = unconfirmed(BobHolds::Nothing);
            let frame = alice.send(&request(), now()).unwrap();
            let mut link = Link::new(
                Mute { input: frame },
                bob,
                Withdrawal::new(),
                strangers.take(),
                None,
            );
            {
                let mut receiving = pin!(link.receive());
                let pending =
                    poll_fn(|cx| Poll::Ready(receiving.as_mut().poll(cx).is_pending())).await;
                assert!(pending);
            }
            let received = link.receive().await.unwrap();
            assert_eq!(received.message, request());
            assert!(received.actions.contains(&Action::ConsiderRequest));
            assert!(link.receive().await.is_err());
            assert!(link.is_over());
        });
    }
}

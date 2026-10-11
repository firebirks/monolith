//! The messaging runtime (`docs/adr/0007-delivery-semantics.md`,
//! `docs/adr/0009-session-manager.md`).
//!
//! [`Runtime`] runs the local identities of one installation: the
//! publication of their Onion Services, every session of every identity,
//! the delivery of their queues and the dialing it needs, and the commands
//! and events of [`crate::api`]. It is the one owner of the sessions of
//! its identities; nothing else sends or receives on them.
//!
//! The task of a session owns its link. It applies what the contact store
//! has to see (Phase 4), takes in chat messages and acknowledges them once
//! they are durable, applies acknowledgements, answers Pings, and, while
//! its session is the preferred one of its contact record, sends the head
//! of the queue of that record and waits for its acknowledgement before it
//! sends the next. One scheduler per conversation with queued messages
//! dials when no session of its record exists, with the reconnect delays,
//! through the Tor backend only.
//!
//! Everything is bound to the local identity instance and the contact
//! record it was started for: a task finds its identity closed or its
//! record gone and ends, and changes nothing of what came after.

use core::fmt;
use core::future::{Future, poll_fn};
use core::pin::pin;
use core::task::Poll;
use core::time::Duration;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};

use monolith_identity::{Fingerprint, IdentityPublicKey};
use monolith_protocol::ProtocolError;
use monolith_protocol::SessionState;
use monolith_protocol::body::{ContactRequest, Message, MessageId};
use monolith_protocol::card::ContactCard;
use monolith_protocol::limits::{
    DUPLICATE_PROBE_TIMEOUT, MAX_UI_EVENTS, MESSAGE_ACK_TIMEOUT, PING_NONCE_LEN,
    RECONNECT_RESET_AFTER, SESSION_LINGER, STARTUP_DIAL_SPREAD,
};
use monolith_protocol::session::Action;
use monolith_protocol::text::{ChatText, DisplayName, IntroductionText};
use monolith_storage::message_records::EntryKind;
use monolith_tor::{KeySource, OnionService, TorBackend};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{broadcast, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::api::{
    CommandError, ContactId, ContactSummary, ConversationSummary, DeferReason, Event, IdentityId,
    IdentitySummary, MessageRef, MessageStatus, MessageView, RefusedReason, SendError,
    ServiceState,
};
use crate::budget::Budgets;
use crate::contacts::StoreError;
use crate::identity::{Installation, InstallationError, LocalIdentity, MessageError};
use crate::link::{Established, Link, LinkError, answer, dial};
use crate::messages::{Receipt, Refusal};
use crate::sessions::{Registration, SessionControl, SessionManager};
use crate::supervisor::{Backoff, Publication, supervise};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The most messages one page of a conversation holds.
pub const MAX_PAGE: usize = 500;

/// A number in `0..=100` from the random source, for the jitter of a delay.
/// A failing source gives the middle: the jitter only spreads attempts.
fn jitter() -> u32 {
    let mut byte = [0_u8; 1];
    match getrandom::fill(&mut byte) {
        Ok(()) => u32::from(u8::from_be_bytes(byte)) % 101,
        Err(_) => 50,
    }
}

/// A random delay in `0..spread`, to spread the first attempts at startup.
fn spread(spread: Duration) -> Duration {
    let mut bytes = [0_u8; 4];
    if getrandom::fill(&mut bytes).is_err() {
        return spread.checked_div(2).unwrap_or_default();
    }
    let fraction = u32::from_be_bytes(bytes);
    spread.mul_f64(f64::from(fraction) / f64::from(u32::MAX))
}

/// `duration` after `instant`. The durations here are fixed and small; a
/// sum the clock cannot hold is the instant itself, which only makes a
/// deadline come at once.
fn after(instant: Instant, duration: Duration) -> Instant {
    instant.checked_add(duration).unwrap_or(instant)
}

/// How the runtime departs from its defaults. Only test tooling changes
/// them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeOptions {
    /// How long a received message waits, durable, before its
    /// acknowledgement is sent. Zero in normal operation; test tooling sets
    /// it to place a crash between the two. It can only delay an
    /// acknowledgement, never send one earlier.
    pub acknowledgement_delay: Duration,
    /// Over how long the first delivery attempts are spread at startup.
    pub startup_spread: Duration,
}

impl Default for RuntimeOptions {
    fn default() -> Self {
        Self {
            acknowledgement_delay: Duration::ZERO,
            startup_spread: STARTUP_DIAL_SPREAD,
        }
    }
}

/// What the runtime holds of one local identity.
struct IdentityRuntime {
    identity: Arc<LocalIdentity>,
    manager: SessionManager,
    /// The conversations whose scheduler runs.
    schedulers: Mutex<HashSet<ContactId>>,
}

impl fmt::Debug for IdentityRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IdentityRuntime").finish_non_exhaustive()
    }
}

struct Inner<B: TorBackend> {
    installation: Installation,
    backend: B,
    budgets: Budgets,
    options: RuntimeOptions,
    events: broadcast::Sender<Event>,
    shutdown: watch::Sender<bool>,
    identities: Mutex<HashMap<IdentityId, Arc<IdentityRuntime>>>,
    tasks: Mutex<JoinSet<()>>,
}

/// The messaging runtime of one installation. See the module
/// documentation.
pub struct Runtime<B: TorBackend + 'static> {
    inner: Arc<Inner<B>>,
}

impl<B: TorBackend + 'static> Clone for Runtime<B> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<B: TorBackend + 'static> fmt::Debug for Runtime<B> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Runtime").finish_non_exhaustive()
    }
}

/// How a wait that the identity, the runtime or a session can cut short
/// ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Waited {
    /// The time passed, or the condition holds.
    Done,
    /// The identity closed or the runtime shuts down.
    Stop,
}

/// How a session ended, for whoever decides the next attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ended {
    /// No acknowledgement came in time: delivery is uncertain.
    AckTimeout,
    /// Any other end.
    Other,
}

/// What woke the task of a session.
enum Next {
    Woken,
    Changed,
    Deadline,
    Received(Result<Box<crate::link::Arrived>, LinkError>),
}

/// The local state of the task of one session: what it sent and waits for.
struct Running {
    serial: u64,
    control: Arc<SessionControl>,
    /// The contact record the session was registered for.
    record: Option<ContactId>,
    /// The message sent and not acknowledged, with the deadline of its
    /// acknowledgement. Local to the task; never evidence of delivery.
    in_flight: Option<(MessageId, Instant)>,
    /// The probe sent, with its deadline.
    probe: Option<([u8; PING_NONCE_LEN], Instant)>,
    /// The last ChatMessage or MessageAck sent or received.
    activity: Instant,
}

fn refused(refusal: Refusal) -> SendError {
    match refusal {
        Refusal::ContactQueueFull | Refusal::IdentityQueueFull | Refusal::QueueBytesFull => {
            SendError::QueueFull
        }
        Refusal::ConversationFull | Refusal::TooManyConversations | Refusal::StoreFull => {
            SendError::StoreFull
        }
        Refusal::IdTaken | Refusal::OtherPeer => SendError::Storage,
    }
}

fn send_error(error: MessageError) -> SendError {
    match error {
        MessageError::Closed => SendError::UnknownIdentity,
        MessageError::NotAContact => SendError::NotAContact,
        MessageError::Refused(refusal) => refused(refusal),
        _ => SendError::Storage,
    }
}

fn store_error(error: StoreError) -> CommandError {
    match error {
        StoreError::NotFound | StoreError::NotThatCard => CommandError::UnknownContact,
        StoreError::Blocked => CommandError::Blocked,
        StoreError::OwnIdentity => CommandError::OwnIdentity,
        StoreError::Full => CommandError::Full,
        StoreError::Protocol(_) => CommandError::InvalidCard,
        _ => CommandError::Storage,
    }
}

fn installation_error(error: InstallationError) -> CommandError {
    match error {
        InstallationError::Full => CommandError::Full,
        InstallationError::Store(error) => store_error(error),
        _ => CommandError::Storage,
    }
}

fn defer_reason(error: LinkError) -> DeferReason {
    match error {
        LinkError::Tor(_) => DeferReason::Tor,
        LinkError::NoEndpoint => DeferReason::NoEndpoint,
        LinkError::Budget => DeferReason::Budget,
        _ => DeferReason::Handshake,
    }
}

fn status(kind: EntryKind) -> MessageStatus {
    match kind {
        EntryKind::Queued => MessageStatus::Queued,
        EntryKind::Delivered => MessageStatus::Delivered,
        EntryKind::Received => MessageStatus::Received,
    }
}

/// Waits until `deadline`, or without end if there is none, unless the
/// runtime shuts down or the identity closes first, or `cut` holds when
/// the sessions change.
async fn wait_until(
    deadline: Option<Instant>,
    shutdown: &mut watch::Receiver<bool>,
    closing: &mut watch::Receiver<bool>,
    sessions: &mut watch::Receiver<u64>,
    cut: impl Fn() -> bool,
) -> Waited {
    loop {
        if *shutdown.borrow_and_update() || *closing.borrow_and_update() {
            return Waited::Stop;
        }
        if cut() {
            return Waited::Done;
        }
        let woke = {
            let mut timer = pin!(async {
                match deadline {
                    Some(deadline) => tokio::time::sleep_until(deadline).await,
                    None => core::future::pending::<()>().await,
                }
            });
            let mut stopping = pin!(shutdown.changed());
            let mut closed = pin!(closing.changed());
            let mut changed = pin!(sessions.changed());
            poll_fn(|cx| {
                if timer.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(None);
                }
                if let Poll::Ready(result) = stopping.as_mut().poll(cx) {
                    return Poll::Ready(Some(result.is_err()));
                }
                if let Poll::Ready(result) = closed.as_mut().poll(cx) {
                    return Poll::Ready(Some(result.is_err()));
                }
                if let Poll::Ready(result) = changed.as_mut().poll(cx) {
                    return Poll::Ready(Some(result.is_err()));
                }
                Poll::Pending
            })
            .await
        };
        match woke {
            None => return Waited::Done,
            // A sender is gone: nobody can stop or wake this any more.
            Some(true) => return Waited::Stop,
            Some(false) => {}
        }
    }
}

impl<B: TorBackend + 'static> Inner<B> {
    fn emit(&self, event: Event) {
        // Nobody listening is not an error: events are notifications.
        let _ = self.events.send(event);
    }

    fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) {
        let mut tasks = lock(&self.tasks);
        while tasks.try_join_next().is_some() {}
        tasks.spawn(future);
    }

    fn stopping(&self) -> bool {
        *self.shutdown.borrow()
    }

    fn identity(&self, identity: &IdentityId) -> Option<Arc<IdentityRuntime>> {
        lock(&self.identities)
            .get(identity)
            .filter(|held| !held.identity.is_closed())
            .cloned()
    }

    /// Starts the runtime of `identity`: its publication, the delivery of
    /// its queues, and what follows its closing.
    fn add_identity(self: &Arc<Self>, identity: Arc<LocalIdentity>, startup: bool) {
        let runtime = Arc::new(IdentityRuntime {
            manager: SessionManager::new(*identity.identity()),
            identity: identity.clone(),
            schedulers: Mutex::new(HashSet::new()),
        });
        lock(&self.identities).insert(identity.id(), runtime.clone());
        let inner = self.clone();
        let publishing = runtime.clone();
        self.spawn(async move { inner.publish(publishing).await });
        for (contact, peer) in identity.conversations_with_queue() {
            if identity.current_record(&contact) == Some(peer) {
                let delay = if startup {
                    spread(self.options.startup_spread)
                } else {
                    Duration::ZERO
                };
                self.ensure_scheduler(&runtime, contact, peer, delay);
            }
        }
        let inner = self.clone();
        let watched = runtime;
        self.spawn(async move { inner.follow_closing(watched).await });
    }

    /// Waits for the identity to close, or the runtime to shut down, and
    /// lets go of it: every session closes, and it takes no command.
    async fn follow_closing(self: Arc<Self>, runtime: Arc<IdentityRuntime>) {
        let mut closing = runtime.identity.closing();
        let mut shutdown = self.shutdown.subscribe();
        loop {
            if *closing.borrow_and_update() {
                break;
            }
            if *shutdown.borrow_and_update() {
                return;
            }
            let gone = {
                let mut closed = pin!(closing.changed());
                let mut stopping = pin!(shutdown.changed());
                poll_fn(|cx| {
                    if let Poll::Ready(result) = closed.as_mut().poll(cx) {
                        return Poll::Ready(result.is_err());
                    }
                    if let Poll::Ready(result) = stopping.as_mut().poll(cx) {
                        return Poll::Ready(result.is_err());
                    }
                    Poll::Pending
                })
                .await
            };
            if gone {
                break;
            }
        }
        let id = runtime.identity.id();
        let removed = {
            let mut identities = lock(&self.identities);
            if identities
                .get(&id)
                .is_some_and(|held| Arc::ptr_eq(held, &runtime))
            {
                identities.remove(&id);
                true
            } else {
                false
            }
        };
        runtime.manager.close_all();
        if removed && runtime.identity.is_closed() {
            self.emit(Event::IdentityClosed { identity: id });
        }
    }

    /// Keeps the Onion Service of the identity published and runs a session
    /// task for every stream that reaches it.
    async fn publish(self: Arc<Self>, runtime: Arc<IdentityRuntime>) {
        let (state, mut states) = watch::channel(Publication::Publishing);
        let id = runtime.identity.id();
        let relaying = self.clone();
        self.spawn(async move {
            while states.changed().await.is_ok() {
                let current = *states.borrow_and_update();
                let state = match current {
                    Publication::Publishing => ServiceState::Publishing,
                    Publication::Available => ServiceState::Available,
                    Publication::Unavailable(_) => ServiceState::Unavailable,
                    Publication::NoKey => ServiceState::NoKey,
                    Publication::Stopped => ServiceState::Stopped,
                };
                relaying.emit(Event::Service {
                    identity: id,
                    state,
                });
            }
        });
        let inner = self.clone();
        let answering = runtime.clone();
        supervise(
            &self.backend,
            &self.budgets,
            &runtime.identity,
            self.shutdown.subscribe(),
            &state,
            move |stream, permit| {
                let inner = inner.clone();
                let runtime = answering.clone();
                async move {
                    let answered = answer(stream, &inner.budgets, &runtime.identity).await;
                    drop(permit);
                    if let Ok(established) = answered {
                        let _ = inner.run_session(&runtime, established).await;
                    }
                }
            },
        )
        .await;
        state.send_replace(Publication::Stopped);
    }

    /// Starts the scheduler of the conversation of `contact` unless it
    /// runs: the first attempt after `delay`.
    fn ensure_scheduler(
        self: &Arc<Self>,
        runtime: &Arc<IdentityRuntime>,
        contact: ContactId,
        peer: IdentityPublicKey,
        delay: Duration,
    ) {
        if self.stopping() || !lock(&runtime.schedulers).insert(contact) {
            return;
        }
        let inner = self.clone();
        let runtime = runtime.clone();
        self.spawn(async move {
            inner.schedule(&runtime, contact, peer, delay).await;
            lock(&runtime.schedulers).remove(&contact);
            // A message queued while the scheduler decided to end finds it
            // gone here, or the scheduler finds the message: looked at
            // again after it is out of the set.
            if inner.needs_scheduler(&runtime, &contact, &peer) {
                inner.ensure_scheduler(&runtime, contact, peer, Duration::ZERO);
            }
        });
    }

    /// Whether the conversation of `contact` has queued messages, a current
    /// record and no session to deliver them.
    fn needs_scheduler(
        &self,
        runtime: &IdentityRuntime,
        contact: &ContactId,
        peer: &IdentityPublicKey,
    ) -> bool {
        !self.stopping()
            && runtime.identity.current_record(contact) == Some(*peer)
            && runtime.identity.next_queued(contact).is_some()
            && !runtime.manager.has_session(contact)
    }

    /// The scheduler of one conversation (`docs/adr/0009-session-manager.md`):
    /// while the conversation has queued messages and its record is the
    /// current one, and no session of the record exists, it dials, with the
    /// reconnect delays between attempts.
    async fn schedule(
        self: &Arc<Self>,
        runtime: &Arc<IdentityRuntime>,
        contact: ContactId,
        peer: IdentityPublicKey,
        first: Duration,
    ) {
        let identity = runtime.identity.clone();
        let mut shutdown = self.shutdown.subscribe();
        let mut closing = identity.closing();
        let mut sessions = runtime.manager.subscribe();
        let mut backoff = Backoff::new();
        let mut delay = first;
        loop {
            if self.stopping() || identity.is_closed() {
                return;
            }
            // The record ended (deleted, blocked, made again) or nothing is
            // left to deliver: the scheduler ends.
            if identity.current_record(&contact) != Some(peer)
                || identity.next_queued(&contact).is_none()
            {
                return;
            }
            if runtime.manager.has_session(&contact) {
                // A session delivers. When it ends with messages left, the
                // next attempt follows a delay.
                let since = Instant::now();
                let manager = &runtime.manager;
                let ended = wait_until(None, &mut shutdown, &mut closing, &mut sessions, || {
                    !manager.has_session(&contact)
                })
                .await;
                if ended == Waited::Stop {
                    return;
                }
                if since.elapsed() >= RECONNECT_RESET_AFTER {
                    backoff.reset();
                }
                delay = backoff.delay(jitter());
                continue;
            }
            if !delay.is_zero() {
                let manager = &runtime.manager;
                let waited = wait_until(
                    Some(after(Instant::now(), delay)),
                    &mut shutdown,
                    &mut closing,
                    &mut sessions,
                    || manager.has_session(&contact),
                )
                .await;
                if waited == Waited::Stop {
                    return;
                }
                delay = Duration::ZERO;
                continue;
            }
            let Some(plan) = identity.dial_plan(&peer) else {
                return;
            };
            let Some(card) = plan.cards.first().cloned() else {
                let retry_in = backoff.delay(jitter());
                self.emit(Event::DeliveryDeferred {
                    identity: identity.id(),
                    contact,
                    reason: DeferReason::NoEndpoint,
                    retry_in,
                });
                delay = retry_in;
                continue;
            };
            let dialed = {
                let manager = &runtime.manager;
                let mut dialing = pin!(dial(&self.backend, &self.budgets, &identity, &card));
                let mut cut = pin!(wait_until(
                    None,
                    &mut shutdown,
                    &mut closing,
                    &mut sessions,
                    || manager.has_session(&contact),
                ));
                // A session of the record that registers meanwhile (an
                // inbound one) ends the dial: it is dropped.
                poll_fn(|cx| {
                    if let Poll::Ready(waited) = cut.as_mut().poll(cx) {
                        return Poll::Ready(Err(waited));
                    }
                    dialing.as_mut().poll(cx).map(Ok)
                })
                .await
            };
            match dialed {
                Err(Waited::Stop) => return,
                Err(Waited::Done) => {}
                Ok(Ok(established)) => {
                    // The session runs here; the scheduler waits for it.
                    let started = Instant::now();
                    let ended = self.run_session(runtime, established).await;
                    if started.elapsed() >= RECONNECT_RESET_AFTER {
                        backoff.reset();
                    }
                    if identity.next_queued(&contact).is_some()
                        && !runtime.manager.has_session(&contact)
                    {
                        delay = backoff.delay(jitter());
                        self.emit(Event::DeliveryDeferred {
                            identity: identity.id(),
                            contact,
                            reason: match ended {
                                Ended::AckTimeout => DeferReason::AckTimeout,
                                Ended::Other => DeferReason::SessionEnded,
                            },
                            retry_in: delay,
                        });
                    }
                }
                Ok(Err(error)) => {
                    delay = backoff.delay(jitter());
                    self.emit(Event::DeliveryDeferred {
                        identity: identity.id(),
                        contact,
                        reason: defer_reason(error),
                        retry_in: delay,
                    });
                }
            }
        }
    }

    /// Runs one session until it ends. See the module documentation.
    async fn run_session<S>(
        self: &Arc<Self>,
        runtime: &Arc<IdentityRuntime>,
        established: Established<S>,
    ) -> Ended
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let Established {
            mut link, first, ..
        } = established;
        let identity = runtime.identity.clone();
        // The first message of `docs/PROTOCOL.md` section 6.4.
        for action in &first {
            if send_first(&mut link, *action).await.is_err() {
                return Ended::Other;
            }
        }
        let mut running = Running {
            serial: runtime.manager.next_serial(),
            control: Arc::new(SessionControl::default()),
            record: None,
            in_flight: None,
            probe: None,
            activity: Instant::now(),
        };
        let mut changes = identity.message_changes();
        let ended = match self
            .drive(runtime, &mut link, &mut running, &mut changes)
            .await
        {
            Ok(ended) => ended,
            Err(_) => Ended::Other,
        };
        if let Some(contact) = running.record {
            runtime.manager.unregister(&contact, running.serial);
            if !runtime.manager.has_session(&contact) {
                self.emit(Event::SessionUnavailable {
                    identity: identity.id(),
                    contact,
                });
                // Messages left, and no scheduler: one starts, after the
                // first delay.
                if let Some(peer) = identity.current_record(&contact) {
                    if self.needs_scheduler(runtime, &contact, &peer) {
                        let delay = Backoff::new().delay(jitter());
                        if !lock(&runtime.schedulers).contains(&contact) {
                            self.emit(Event::DeliveryDeferred {
                                identity: identity.id(),
                                contact,
                                reason: match ended {
                                    Ended::AckTimeout => DeferReason::AckTimeout,
                                    Ended::Other => DeferReason::SessionEnded,
                                },
                                retry_in: delay,
                            });
                        }
                        self.ensure_scheduler(runtime, contact, peer, delay);
                    }
                }
            }
        }
        ended
    }

    /// The loop of a session task. Returns when the session is over.
    async fn drive<S>(
        self: &Arc<Self>,
        runtime: &Arc<IdentityRuntime>,
        link: &mut Link<S>,
        running: &mut Running,
        changes: &mut Option<watch::Receiver<u64>>,
    ) -> Result<Ended, LinkError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let identity = runtime.identity.clone();
        loop {
            if running.control.closing() || self.stopping() || identity.is_closed() {
                return link_close(link).await.map(|()| Ended::Other);
            }
            // A message the queue no longer holds as its head was
            // acknowledged on another session, or deleted: nothing waits
            // for it here any more.
            if let (Some((id, _)), Some(contact)) = (running.in_flight, running.record) {
                if identity
                    .next_queued(&contact)
                    .is_none_or(|(_, head, _)| head != id)
                {
                    running.in_flight = None;
                }
            }
            if let Some(nonce) = running.control.take_probe() {
                link.send(&Message::Ping(nonce)).await?;
                running.probe = Some((nonce, after(Instant::now(), DUPLICATE_PROBE_TIMEOUT)));
            }
            // The preferred session sends the head of the queue, one
            // message at a time.
            if let Some(contact) = running.record {
                if running.in_flight.is_none()
                    && runtime.manager.is_preferred(&contact, running.serial)
                {
                    if let Some((_, id, text)) = identity.next_queued(&contact) {
                        link.send(&Message::ChatMessage { id, text }).await?;
                        let now = Instant::now();
                        running.in_flight = Some((id, after(now, MESSAGE_ACK_TIMEOUT)));
                        running.activity = now;
                    }
                }
            }
            let deadline = self.deadline(runtime, link, running);
            let next = {
                let control = running.control.clone();
                let mut woken = pin!(control.woken());
                let mut changed = pin!(async {
                    if let Some(changes) = changes.as_mut() {
                        if changes.changed().await.is_ok() {
                            return;
                        }
                    }
                    core::future::pending::<()>().await;
                });
                let mut timer = pin!(async {
                    match deadline {
                        Some(deadline) => tokio::time::sleep_until(deadline).await,
                        None => core::future::pending::<()>().await,
                    }
                });
                let mut receive = pin!(link.receive());
                poll_fn(|cx| {
                    if woken.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(Next::Woken);
                    }
                    if changed.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(Next::Changed);
                    }
                    if timer.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(Next::Deadline);
                    }
                    receive
                        .as_mut()
                        .poll(cx)
                        .map(|received| Next::Received(received.map(Box::new)))
                })
                .await
            };
            match next {
                Next::Woken | Next::Changed => {}
                Next::Deadline => {
                    let now = Instant::now();
                    if let Some((nonce, due)) = running.probe {
                        if now >= due {
                            running.probe = None;
                            if let Some(contact) = running.record {
                                runtime
                                    .manager
                                    .probe_ended(&contact, running.serial, nonce, false);
                            }
                        }
                    }
                    if running.in_flight.is_some_and(|(_, due)| now >= due) {
                        // No acknowledgement in time: delivery is uncertain.
                        // The message stays queued for a later session.
                        let _ = link_close(link).await;
                        return Ok(Ended::AckTimeout);
                    }
                    if self.lingered(runtime, link, running, now) {
                        return link_close(link).await.map(|()| Ended::Other);
                    }
                }
                Next::Received(Err(error)) => return Err(error),
                Next::Received(Ok(arrived)) => {
                    self.handle(runtime, link, running, &arrived).await?;
                }
            }
        }
    }

    /// The earliest deadline of the task: an acknowledgement, a probe, the
    /// linger of an idle session.
    fn deadline<S>(
        &self,
        runtime: &IdentityRuntime,
        link: &Link<S>,
        running: &Running,
    ) -> Option<Instant>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let mut deadlines = Vec::with_capacity(3);
        if let Some((_, due)) = running.in_flight {
            deadlines.push(due);
        }
        if let Some((_, due)) = running.probe {
            deadlines.push(due);
        }
        if self.idle(runtime, link, running) {
            deadlines.push(after(running.activity, SESSION_LINGER));
        }
        deadlines.into_iter().min()
    }

    /// A confirmed session with nothing in flight and nothing queued for its
    /// record, which may linger and then close.
    fn idle<S>(&self, runtime: &IdentityRuntime, link: &Link<S>, running: &Running) -> bool
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        link.session().state() == SessionState::AuthenticatedContact
            && running.in_flight.is_none()
            && running.probe.is_none()
            && running
                .record
                .is_none_or(|contact| runtime.identity.next_queued(&contact).is_none())
    }

    fn lingered<S>(
        &self,
        runtime: &IdentityRuntime,
        link: &Link<S>,
        running: &Running,
        now: Instant,
    ) -> bool
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        self.idle(runtime, link, running) && now >= after(running.activity, SESSION_LINGER)
    }

    /// Handles one message that arrived on the session.
    async fn handle<S>(
        self: &Arc<Self>,
        runtime: &Arc<IdentityRuntime>,
        link: &mut Link<S>,
        running: &mut Running,
        arrived: &crate::link::Arrived,
    ) -> Result<(), LinkError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let identity = runtime.identity.clone();
        let received = arrived.received();
        let actions = &received.actions;
        // What the contact store has to see (Phase 4): an acceptance, a
        // request of a stranger, a new card, the confirmation of a session
        // of a new local key.
        let store_sees = actions.iter().any(|action| {
            matches!(
                action,
                Action::MarkAccepted | Action::ConsiderRequest | Action::Confirmed
            )
        }) || (actions.contains(&Action::Deliver)
            && matches!(received.message, Message::EndpointUpdate(_)));
        if store_sees {
            if let Ok(applied) = identity.apply(arrived).await {
                if applied.accepted {
                    if let Some(contact) = identity.record_of(link.session_ref()) {
                        self.emit(Event::ContactAccepted {
                            identity: identity.id(),
                            contact,
                        });
                    }
                }
                if let Some(request) = applied.request {
                    self.emit(Event::RequestReceived {
                        identity: identity.id(),
                        queued: request.is_ok(),
                    });
                }
            }
        }
        if actions.contains(&Action::SendContactAccept) {
            link.send(&Message::ContactAccept).await?;
        }
        if actions.contains(&Action::Confirmed) {
            self.confirmed(runtime, link, running).await?;
        }
        if !actions.contains(&Action::Deliver) {
            return Ok(());
        }
        match &received.message {
            Message::ChatMessage { .. } => self.take_in(runtime, link, running, arrived).await,
            Message::MessageAck(id) => {
                running.activity = Instant::now();
                if let Ok(Some((contact, sequence))) = identity.acknowledged(arrived).await {
                    self.emit(Event::MessageDelivered(MessageRef {
                        identity: identity.id(),
                        contact,
                        sequence,
                    }));
                }
                if running.in_flight.is_some_and(|(sent, _)| sent == *id) {
                    running.in_flight = None;
                }
                Ok(())
            }
            Message::Ping(nonce) => link.send(&Message::Pong(*nonce)).await,
            Message::Pong(nonce) => match running.probe {
                Some((sent, _)) if sent == *nonce => {
                    running.probe = None;
                    if let Some(contact) = running.record {
                        runtime
                            .manager
                            .probe_ended(&contact, running.serial, *nonce, true);
                    }
                    Ok(())
                }
                // A Pong that answers no Ping of this side is a violation:
                // the session ends without a Close.
                _ => Err(LinkError::Session(
                    monolith_session::SessionError::Protocol(ProtocolError::MessageNotPermitted),
                )),
            },
            // Profiles, file transfer and the rest are not part of this
            // version of the runtime. What the store had to see it saw.
            _ => Ok(()),
        }
    }

    /// The session is confirmed: it is registered for the contact record it
    /// stands for, and a successor of a local rotation is announced on it.
    async fn confirmed<S>(
        self: &Arc<Self>,
        runtime: &Arc<IdentityRuntime>,
        link: &mut Link<S>,
        running: &mut Running,
    ) -> Result<(), LinkError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let identity = runtime.identity.clone();
        if let Some(contact) = identity.record_of(link.session_ref()) {
            running.record = Some(contact);
            running.activity = Instant::now();
            let initiator = link.session().initiator();
            let remote = *link.session().peer();
            let card = link.session().peer_card().clone();
            let held = identity.clone();
            let mine = running.serial;
            let registration = runtime.manager.register(
                contact,
                &remote,
                running.serial,
                initiator,
                running.control.clone(),
                // Whether a session's key is still the active credential:
                // known here for this session; one registered earlier was
                // withdrawn by the store if its key was retired, and has
                // left or is leaving.
                |serial| serial != mine || held.is_active_key(&card),
            );
            if registration == Registration::Preferred {
                self.emit(Event::SessionAvailable {
                    identity: identity.id(),
                    contact,
                });
            }
        }
        if let Some(announcement) = identity.announcement_for(link.session_ref()) {
            let sent = link.announce(announcement).await?;
            let _ = identity.mark_announced(&sent).await;
        }
        Ok(())
    }

    /// Takes in a chat message and acknowledges it once it is durable.
    async fn take_in<S>(
        self: &Arc<Self>,
        runtime: &Arc<IdentityRuntime>,
        link: &mut Link<S>,
        running: &mut Running,
        arrived: &crate::link::Arrived,
    ) -> Result<(), LinkError>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let identity = runtime.identity.clone();
        running.activity = Instant::now();
        match identity.take_in(arrived).await {
            Ok(taken) => {
                // Durable: it is reported, and only now acknowledged.
                if let Receipt::Stored(sequence) = taken.receipt {
                    self.emit(Event::MessageReceived(MessageRef {
                        identity: identity.id(),
                        contact: taken.contact,
                        sequence,
                    }));
                }
                if !self.options.acknowledgement_delay.is_zero() {
                    tokio::time::sleep(self.options.acknowledgement_delay).await;
                }
                link.send(&Message::MessageAck(taken.id)).await?;
                running.activity = Instant::now();
                Ok(())
            }
            Err(MessageError::Refused(_)) => {
                if let Some(contact) = identity.record_of(link.session_ref()) {
                    self.emit(Event::MessageRefused {
                        identity: identity.id(),
                        contact,
                        reason: RefusedReason::StoreFull,
                    });
                }
                Ok(())
            }
            Err(MessageError::Storage(_)) => {
                self.emit(Event::StorageFailed);
                Ok(())
            }
            Err(_) => Ok(()),
        }
    }
}

/// Sends the first message an action of the admission asks for.
async fn send_first<S>(link: &mut Link<S>, action: Action) -> Result<(), LinkError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    match action {
        Action::SendContactAccept => link.send(&Message::ContactAccept).await,
        Action::SendContactRequest => {
            // The card and the capability of this session, which it
            // states; not those of the moment, which can differ.
            let card = link.session().local_card().clone();
            let invitation = link.session().invitation().cloned();
            let request = Message::ContactRequest(Box::new(ContactRequest {
                card,
                invitation,
                display_name: DisplayName::new("").map_err(|error| {
                    LinkError::Session(monolith_session::SessionError::Protocol(error))
                })?,
                introduction: IntroductionText::new("").map_err(|error| {
                    LinkError::Session(monolith_session::SessionError::Protocol(error))
                })?,
            }));
            link.send(&request).await
        }
        _ => Ok(()),
    }
}

/// Ends a link with its Close.
async fn link_close<S>(link: &mut Link<S>) -> Result<(), LinkError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    link.end().await
}

impl<B: TorBackend + 'static> Runtime<B> {
    /// Starts the runtime of every local identity of `installation`: their
    /// services are published, and the queues they hold are delivered as
    /// their contacts are reached. Runs on the current tokio runtime.
    pub fn start(
        installation: Installation,
        backend: B,
        budgets: Budgets,
        options: RuntimeOptions,
    ) -> Self {
        let (events, _) = broadcast::channel(MAX_UI_EVENTS);
        let inner = Arc::new(Inner {
            installation,
            backend,
            budgets,
            options,
            events,
            shutdown: watch::Sender::new(false),
            identities: Mutex::new(HashMap::new()),
            tasks: Mutex::new(JoinSet::new()),
        });
        for identity in inner.installation.identities() {
            inner.add_identity(identity, true);
        }
        Self { inner }
    }

    /// Follows the events. A receiver that falls more than `MAX_UI_EVENTS`
    /// behind is told how many it missed, and reads the state again.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    /// The local identities.
    pub fn identities(&self) -> Vec<IdentitySummary> {
        let mut summaries: Vec<IdentitySummary> = lock(&self.inner.identities)
            .values()
            .filter(|held| !held.identity.is_closed())
            .map(|held| IdentitySummary {
                identity: held.identity.id(),
                fingerprint: Fingerprint::of(held.identity.identity()).to_compact_text(),
                label: held.identity.label().map(|label| label.as_str().to_owned()),
            })
            .collect();
        summaries.sort_by(|a, b| a.fingerprint.cmp(&b.fingerprint));
        summaries
    }

    fn held(&self, identity: &IdentityId) -> Result<Arc<IdentityRuntime>, CommandError> {
        if self.inner.stopping() {
            return Err(CommandError::ShuttingDown);
        }
        self.inner
            .identity(identity)
            .ok_or(CommandError::UnknownIdentity)
    }

    /// The requested and accepted contacts of `identity`.
    pub fn contacts(&self, identity: &IdentityId) -> Result<Vec<ContactSummary>, CommandError> {
        let held = self.held(identity)?;
        let mut contacts: Vec<ContactSummary> = held
            .identity
            .contact_records()
            .into_iter()
            .filter_map(|(contact, peer)| {
                let view = held.identity.contact(&peer)?;
                Some(ContactSummary {
                    contact,
                    fingerprint: Fingerprint::of(&peer).to_compact_text(),
                    alias: view.alias.map(|alias| alias.as_str().to_owned()),
                    accepted: view.kind == monolith_protocol::contact::RecordKind::Accepted,
                    verified: view.verified,
                })
            })
            .collect();
        contacts.sort_by(|a, b| a.fingerprint.cmp(&b.fingerprint));
        Ok(contacts)
    }

    /// The conversations of `identity`, ended ones included.
    pub fn conversations(
        &self,
        identity: &IdentityId,
    ) -> Result<Vec<ConversationSummary>, CommandError> {
        let held = self.held(identity)?;
        Ok(held
            .identity
            .conversations()
            .into_iter()
            .map(|view| ConversationSummary {
                contact: view.contact,
                fingerprint: Fingerprint::of(&view.peer).to_compact_text(),
                active: held.identity.current_record(&view.contact) == Some(view.peer),
                messages: view.messages,
                queued: view.queued,
                latest: view.latest,
            })
            .collect())
    }

    /// Up to `limit` messages (at most [`MAX_PAGE`]) of the conversation of
    /// `contact`, newest last, those before the sequence `before` if it is
    /// given.
    pub fn messages(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
        before: Option<u64>,
        limit: usize,
    ) -> Result<Vec<MessageView>, CommandError> {
        let held = self.held(identity)?;
        Ok(held
            .identity
            .conversation(contact, before, limit.min(MAX_PAGE))
            .into_iter()
            .map(|entry| MessageView {
                reference: MessageRef {
                    identity: *identity,
                    contact: *contact,
                    sequence: entry.sequence,
                },
                status: status(entry.kind),
                time: entry.time,
                text: entry.text.as_str().to_owned(),
            })
            .collect())
    }

    /// One message.
    pub fn message(&self, reference: &MessageRef) -> Result<Option<MessageView>, CommandError> {
        let held = self.held(&reference.identity)?;
        Ok(held
            .identity
            .message(&reference.contact, reference.sequence)
            .map(|entry| MessageView {
                reference: *reference,
                status: status(entry.kind),
                time: entry.time,
                text: entry.text.as_str().to_owned(),
            }))
    }

    /// Queues `text` for the contact record `contact` of `identity`
    /// (`docs/adr/0007-delivery-semantics.md`). The text is checked before
    /// anything is stored. Succeeds once the message is durably queued, with
    /// or without a session, Tor or the contact being reachable; it is
    /// delivered when the contact is reached.
    pub async fn send_message(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
        text: &str,
    ) -> Result<MessageRef, SendError> {
        let text = ChatText::new(text).map_err(|error| match error {
            ProtocolError::FieldTooLong => SendError::TooLong,
            _ => SendError::InvalidText,
        })?;
        let held = self
            .held(identity)
            .map_err(|_| SendError::UnknownIdentity)?;
        let queued = held
            .identity
            .queue_message(*contact, text)
            .await
            .map_err(send_error)?;
        let reference = MessageRef {
            identity: *identity,
            contact: *contact,
            sequence: queued.sequence,
        };
        self.inner.emit(Event::MessageQueued(reference));
        if let Some(peer) = held.identity.current_record(contact) {
            if !held.manager.has_session(contact) {
                self.inner
                    .ensure_scheduler(&held, *contact, peer, Duration::ZERO);
            }
        }
        Ok(reference)
    }

    /// Deletes the messages of the conversation of `contact`, queued ones
    /// included, from the message store. Nothing is sent to the contact.
    pub async fn delete_conversation(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
    ) -> Result<bool, CommandError> {
        let held = self.held(identity)?;
        held.identity
            .delete_conversation(contact)
            .await
            .map_err(|_| CommandError::Storage)
    }

    /// The card of `identity` in its text form, without a capability.
    pub fn own_card(&self, identity: &IdentityId) -> Result<String, CommandError> {
        Ok(self.held(identity)?.identity.card().to_text())
    }

    /// Imports a contact card given in its text form. Returns the contact
    /// record of its identity.
    pub async fn add_contact(
        &self,
        identity: &IdentityId,
        card: &str,
    ) -> Result<ContactId, CommandError> {
        let held = self.held(identity)?;
        let card = ContactCard::from_text(card).map_err(|_| CommandError::InvalidCard)?;
        held.identity.import(&card).await.map_err(store_error)?;
        held.identity
            .contact_id(card.identity())
            .ok_or(CommandError::UnknownContact)
    }

    fn peer_of(
        held: &IdentityRuntime,
        contact: &ContactId,
    ) -> Result<IdentityPublicKey, CommandError> {
        held.identity
            .current_record(contact)
            .ok_or(CommandError::UnknownContact)
    }

    /// Blocks the contact of the record `contact`: its sessions end, its
    /// queue stays stored and is never sent; its record ends.
    pub async fn block_contact(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
    ) -> Result<(), CommandError> {
        let held = self.held(identity)?;
        let peer = Self::peer_of(&held, contact)?;
        held.identity.block(&peer).await.map_err(store_error)?;
        held.manager.close(contact);
        Ok(())
    }

    /// Deletes the contact of the record `contact`: its sessions end, its
    /// queue stays stored and is never sent; a later card of the same
    /// identity is a new record with a new conversation.
    pub async fn delete_contact(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
    ) -> Result<(), CommandError> {
        let held = self.held(identity)?;
        let peer = Self::peer_of(&held, contact)?;
        held.identity.delete(&peer).await.map_err(store_error)?;
        held.manager.close(contact);
        Ok(())
    }

    /// Makes a new local identity: Tor makes the key of its Onion Service,
    /// the identity is made durable with it, and its service is published.
    pub async fn create_identity(&self, label: Option<&str>) -> Result<IdentityId, CommandError> {
        if self.inner.stopping() {
            return Err(CommandError::ShuttingDown);
        }
        let label = label
            .map(DisplayName::new)
            .transpose()
            .map_err(|_| CommandError::InvalidText)?;
        let mut service = self
            .inner
            .backend
            .publish_onion(KeySource::Generate)
            .await
            .map_err(|_| CommandError::Tor)?;
        let secret = service.take_generated_secret().ok_or(CommandError::Tor)?;
        let endpoint = *service.service_key();
        // Published again by the supervisor, from the stored key.
        let _ = service.close().await;
        let identity = self
            .inner
            .installation
            .create_identity(endpoint, Some(&secret), label)
            .await
            .map_err(installation_error)?;
        let id = identity.id();
        self.inner.add_identity(identity, false);
        Ok(id)
    }

    /// Deletes a local identity: its keys, its contacts, its messages. Its
    /// service is removed and its sessions end.
    pub async fn delete_identity(&self, identity: &IdentityId) -> Result<(), CommandError> {
        let held = self.held(identity)?;
        held.manager.close_all();
        self.inner
            .installation
            .delete_identity(&held.identity)
            .await
            .map_err(installation_error)
    }

    /// Asks for a session with the contact of the record `contact` now, an
    /// explicit local operation, whether or not anything is queued. One
    /// attempt, through the Tor backend.
    pub fn connect(&self, identity: &IdentityId, contact: &ContactId) -> Result<(), CommandError> {
        let held = self.held(identity)?;
        let peer = Self::peer_of(&held, contact)?;
        let plan = held
            .identity
            .dial_plan(&peer)
            .ok_or(CommandError::UnknownContact)?;
        let card = plan
            .cards
            .first()
            .cloned()
            .ok_or(CommandError::UnknownContact)?;
        let inner = self.inner.clone();
        self.inner.spawn(async move {
            if let Ok(established) =
                dial(&inner.backend, &inner.budgets, &held.identity, &card).await
            {
                let _ = inner.run_session(&held, established).await;
            }
        });
        Ok(())
    }

    /// Closes every session of the contact record `contact`.
    pub fn close_sessions(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
    ) -> Result<(), CommandError> {
        self.held(identity)?.manager.close(contact);
        Ok(())
    }

    /// The number of registered sessions of the contact record `contact`.
    pub fn sessions(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
    ) -> Result<usize, CommandError> {
        Ok(self.held(identity)?.manager.sessions(contact))
    }

    /// The local identity behind `identity`, for tooling built on the core.
    /// A front end has no use for it: it holds the contact store and acts
    /// with the identity's authority.
    pub fn local_identity(&self, identity: &IdentityId) -> Option<Arc<LocalIdentity>> {
        self.inner
            .identity(identity)
            .map(|held| held.identity.clone())
    }

    /// Stops the runtime: every session closes, every service is removed,
    /// and every task ends, within `deadline`; what is left then is
    /// aborted.
    pub async fn shutdown(&self, deadline: Duration) {
        self.inner.shutdown.send_replace(true);
        for held in lock(&self.inner.identities).values() {
            held.manager.close_all();
        }
        let mut tasks = core::mem::take(&mut *lock(&self.inner.tasks));
        let drained = tokio::time::timeout(deadline, async {
            while tasks.join_next().await.is_some() {}
        })
        .await;
        if drained.is_err() {
            tasks.abort_all();
            while tasks.join_next().await.is_some() {}
        }
    }
}

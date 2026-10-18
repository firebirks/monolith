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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use monolith_identity::{Fingerprint, IdentityPublicKey};
use monolith_protocol::ProtocolError;
use monolith_protocol::SessionState;
use monolith_protocol::body::{ContactRequest, Message, MessageId};
use monolith_protocol::card::ContactCard;
use monolith_protocol::limits::{
    DUPLICATE_PROBE_TIMEOUT, MAX_UI_EVENTS, MESSAGE_ACK_TIMEOUT, PING_NONCE_LEN, PONG_TIMEOUT,
    RECONNECT_RESET_AFTER, SESSION_LINGER, STARTUP_DIAL_SPREAD,
};
use monolith_protocol::session::Action;
use monolith_protocol::text::{ChatText, DisplayName, IntroductionText};
use monolith_storage::message_records::EntryKind;
use monolith_tor::{KeySource, OnionService, TorBackend};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{broadcast, oneshot, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::api::{
    AddedContact, BlockedRef, BlockedSummary, CommandError, ContactId, ContactSummary,
    ConversationSummary, DeferReason, Event, IdentityId, IdentitySummary, InvitationRef,
    InvitationSummary, MessageRef, MessageStatus, MessageView, RefusedReason, RequestMode,
    RequestRef, RequestSummary, SendError, ServiceState, TorCheck,
};
use crate::budget::Budgets;
use crate::contacts::StoreError;
use crate::identity::{Installation, InstallationError, LocalIdentity, MessageError};
use crate::link::{Established, Link, LinkError, answer, dial_plan};
use crate::messages::{Receipt, Refusal};
use crate::rates::{ContactRates, Kind};
use crate::requests::{InvitationId, RequestId};
use crate::sessions::{SessionControl, SessionManager};
use crate::supervisor::{Backoff, Publication, jitter, supervise};
use crate::transfers::{Answer, Transfers};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The most messages one page of a conversation holds.
pub const MAX_PAGE: usize = 500;

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
    /// The records whose last session ended for want of an
    /// acknowledgement: their delays do not start again.
    timed_out: Mutex<HashSet<ContactId>>,
    /// The records a session was reported available for: an event is
    /// raised only when that changes.
    available: Mutex<HashSet<ContactId>>,
    /// The state of the Onion Service last reported, for a front end that
    /// asks instead of following the events.
    service: Mutex<ServiceState>,
    /// What the contacts sent and opened, against their rates
    /// (`crate::rates`).
    rates: Mutex<ContactRates>,
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
    /// Set once a failed store was reported.
    storage_failed: AtomicBool,
}

/// The messaging runtime of one installation. See the module
/// documentation.
pub struct Runtime<B: TorBackend + 'static> {
    inner: Arc<Inner<B>>,
    /// Shared by the clones of this handle and by nothing the runtime
    /// runs: when the last handle goes, the runtime stops.
    owner: Arc<Owner<B>>,
}

impl<B: TorBackend + 'static> Clone for Runtime<B> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            owner: self.owner.clone(),
        }
    }
}

/// Stops the runtime when the last handle to it is dropped without a
/// shutdown: its tasks hold the runtime, so without this they would run on
/// for nobody, with every service published and every key in memory.
struct Owner<B: TorBackend + 'static> {
    inner: Arc<Inner<B>>,
}

impl<B: TorBackend + 'static> Drop for Owner<B> {
    fn drop(&mut self) {
        self.inner.shutdown.send_replace(true);
        for held in lock(&self.inner.identities).values() {
            held.manager.close_all();
        }
        lock(&self.inner.tasks).abort_all();
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
    /// The probe of a duplicate session that runs: its nonce, and what is
    /// left of `DUPLICATE_PROBE_TIMEOUT`. Like that of `ping`, the time
    /// counts only while the task reads (`RESOURCE_LIMITS.md` section 6).
    probe: Option<([u8; PING_NONCE_LEN], Duration)>,
    /// The Ping sent and not answered, and what is left of
    /// `PONG_TIMEOUT`. One at a time: a probe asked while one is
    /// outstanding waits for its Pong.
    ping: Option<([u8; PING_NONCE_LEN], Duration)>,
    /// The last ChatMessage sent or taken in, or acknowledgement that
    /// delivered one.
    activity: Instant,
    /// The file offers of the peer that this session rejected
    /// (`crate::transfers`).
    transfers: Transfers,
}

impl Running {
    /// The task read for `elapsed`: that much of the probe and of the
    /// Ping is gone.
    fn read_for(&mut self, elapsed: Duration) {
        for (_, left) in self.probe.iter_mut().chain(self.ping.iter_mut()) {
            *left = left.saturating_sub(elapsed);
        }
    }
}

/// The task of one session as the runtime holds it. Its end is handled
/// when it is dropped, however the task ends: returned, or aborted with
/// the accept loop of a service that went away. A session never stays
/// registered once its task is gone.
struct SessionTask<B: TorBackend + 'static> {
    inner: Arc<Inner<B>>,
    runtime: Arc<IdentityRuntime>,
    running: Running,
    ended: Ended,
}

impl<B: TorBackend + 'static> Drop for SessionTask<B> {
    fn drop(&mut self) {
        if let Some(contact) = self.running.record {
            self.inner
                .session_ended(&self.runtime, contact, self.running.serial, self.ended);
        }
    }
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
        MessageError::Randomness => SendError::Randomness,
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

fn request_error(error: StoreError) -> CommandError {
    match error {
        StoreError::NotFound => CommandError::UnknownRequest,
        error => store_error(error),
    }
}

fn invitation_error(error: StoreError) -> CommandError {
    match error {
        StoreError::NotFound => CommandError::UnknownInvitation,
        error => store_error(error),
    }
}

fn blocked_error(error: StoreError) -> CommandError {
    match error {
        StoreError::NotFound => CommandError::NotBlocked,
        error => store_error(error),
    }
}

/// What a front end is told of a status query of the system Tor.
fn tor_check(status: &monolith_tor::TorStatus) -> TorCheck {
    use monolith_tor::{Bootstrap, ControlStatus, SocksStatus};
    let socks = status.socks == SocksStatus::Reachable;
    match status.control {
        ControlStatus::Reachable {
            version,
            circuit_established,
            bootstrap,
        } => {
            if !socks {
                TorCheck::SocksUnavailable
            } else if circuit_established {
                TorCheck::Ready {
                    version: version.to_string(),
                    outdated: version < monolith_tor::RECOMMENDED,
                }
            } else {
                TorCheck::Connecting {
                    progress: match bootstrap {
                        Bootstrap::InProgress(percent) => Some(percent),
                        Bootstrap::Done => Some(100),
                        Bootstrap::Unknown => None,
                    },
                }
            }
        }
        ControlStatus::UnsupportedVersion(version) => TorCheck::TooOld {
            version: version.to_string(),
        },
        ControlStatus::AuthenticationFailed | ControlStatus::AuthenticationUnavailable => {
            TorCheck::AuthenticationFailed
        }
        ControlStatus::Unavailable | ControlStatus::InvalidResponse => {
            if socks {
                TorCheck::ControlUnavailable
            } else {
                TorCheck::NotRunning
            }
        }
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

    /// Runs `work` as a task of the runtime and waits for it: a command,
    /// once begun, completes and raises its events also if its caller
    /// stops waiting, and a shutdown waits for it like for every task.
    /// `None` if it did not complete (a shutdown that ran out of time, or a
    /// runtime dropped).
    async fn detached<T: Send + 'static>(
        &self,
        work: impl Future<Output = T> + Send + 'static,
    ) -> Option<T> {
        let (result, done) = oneshot::channel();
        self.spawn(async move {
            let _ = result.send(work.await);
        });
        done.await.ok()
    }

    /// Raises `SessionAvailable` or `SessionUnavailable` for `contact` if
    /// whether a session of it is registered changed since the last event.
    /// Decided under one lock, so that the events alternate.
    fn announce(&self, runtime: &IdentityRuntime, contact: ContactId) {
        let mut available = lock(&runtime.available);
        let now = runtime.manager.has_session(&contact);
        let changed = if now {
            available.insert(contact)
        } else {
            available.remove(&contact)
        };
        if changed {
            let identity = runtime.identity.id();
            self.emit(if now {
                Event::SessionAvailable { identity, contact }
            } else {
                Event::SessionUnavailable { identity, contact }
            });
        }
    }

    /// Reports a failed store as soon as it fails, also with no session or
    /// attempt running that would notice it.
    async fn watch_storage(self: Arc<Self>) {
        let (mut vault, mut messages) = self.installation.failure_changes();
        let mut shutdown = self.shutdown.subscribe();
        loop {
            if self.installation.is_failed() || self.installation.messages_failed() {
                self.storage_failure();
                return;
            }
            if *shutdown.borrow_and_update() {
                return;
            }
            let gone = {
                let mut vault = pin!(vault.changed());
                let mut messages = pin!(messages.changed());
                let mut stopping = pin!(shutdown.changed());
                poll_fn(|cx| {
                    if let Poll::Ready(result) = vault.as_mut().poll(cx) {
                        return Poll::Ready(result.is_err());
                    }
                    if let Poll::Ready(result) = messages.as_mut().poll(cx) {
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
                return;
            }
        }
    }

    /// Reports a failed store, once.
    fn storage_failure(&self) {
        if !self.storage_failed.swap(true, Ordering::SeqCst) {
            self.emit(Event::StorageFailed);
        }
    }

    /// Returns true, and reports it, if the vault or the message store of
    /// `identity` failed: nothing more is made durable, so no session and
    /// no attempt goes on.
    fn halted(&self, identity: &LocalIdentity) -> bool {
        let halted = identity.messages_failed() || identity.vault_failed();
        if halted {
            self.storage_failure();
        }
        halted
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
            timed_out: Mutex::new(HashSet::new()),
            available: Mutex::new(HashSet::new()),
            service: Mutex::new(ServiceState::Publishing),
            rates: Mutex::new(ContactRates::default()),
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
        let recording = runtime.clone();
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
                // Recorded before the event, so that a front end that asks
                // when the event arrives finds the state it reports.
                *lock(&recording.service) = state;
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
                        // A contact that opens sessions above its rate has
                        // this one closed once it is admitted.
                        let session = established.link.session();
                        if session.standing().is_contact_record()
                            && !lock(&runtime.rates).session(*session.peer(), Instant::now())
                        {
                            let _ = established.link.close().await;
                            return;
                        }
                        let _ = inner.run_session(&runtime, established).await;
                    }
                }
            },
        )
        .await;
        // The supervisor reported how it ended, `Stopped` or `NoKey`, and
        // that stands.
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
    /// record and no session to deliver them, in stores that have not
    /// failed.
    fn needs_scheduler(
        &self,
        runtime: &IdentityRuntime,
        contact: &ContactId,
        peer: &IdentityPublicKey,
    ) -> bool {
        !self.stopping()
            && !self.halted(&runtime.identity)
            && runtime.identity.is_current(contact, peer)
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
        // A session of the record took over, or the record ended: a delay
        // or a dial in progress is cut short.
        let manager = &runtime.manager;
        let over = || manager.has_session(&contact) || !identity.is_current(&contact, &peer);
        loop {
            if self.stopping() || identity.is_closed() || self.halted(&identity) {
                return;
            }
            // The record ended (deleted, blocked, made again) or nothing is
            // left to deliver: the scheduler ends.
            if !identity.is_current(&contact, &peer) || identity.next_queued(&contact).is_none() {
                return;
            }
            if manager.has_session(&contact) {
                // A session delivers. When it ends with messages left, the
                // next attempt follows a delay.
                let since = Instant::now();
                let ended = wait_until(None, &mut shutdown, &mut closing, &mut sessions, || {
                    !manager.has_session(&contact)
                })
                .await;
                if ended == Waited::Stop {
                    return;
                }
                // Unless it ended for want of an acknowledgement, a session
                // that lasted starts the delays again.
                if since.elapsed() >= RECONNECT_RESET_AFTER
                    && !lock(&runtime.timed_out).contains(&contact)
                {
                    backoff.reset();
                }
                delay = backoff.delay(jitter());
                continue;
            }
            if !delay.is_zero() {
                let waited = wait_until(
                    Some(after(Instant::now(), delay)),
                    &mut shutdown,
                    &mut closing,
                    &mut sessions,
                    over,
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
            if plan.cards.is_empty() {
                let retry_in = backoff.delay(jitter());
                self.emit(Event::DeliveryDeferred {
                    identity: identity.id(),
                    contact,
                    reason: DeferReason::NoEndpoint,
                    retry_in,
                });
                delay = retry_in;
                continue;
            }
            let dialed = {
                // The successor first and the active key if it does not
                // answer; no further card once the attempt is over.
                let mut dialing = pin!(dial_plan(
                    &self.backend,
                    &self.budgets,
                    &identity,
                    &plan.cards,
                    || !over(),
                ));
                let mut cut = pin!(wait_until(
                    None,
                    &mut shutdown,
                    &mut closing,
                    &mut sessions,
                    over,
                ));
                // A session of the record that registers meanwhile (an
                // inbound one), or the end of the record, ends the dial: it
                // is dropped.
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
                    // The session runs here; the scheduler waits for it. A
                    // session that lasted is a reason to start the delays
                    // again, unless it ended for want of an acknowledgement:
                    // a peer that never acknowledges is not dialed faster.
                    let started = Instant::now();
                    let ended = self.run_session(runtime, established).await;
                    if ended == Ended::Other && started.elapsed() >= RECONNECT_RESET_AFTER {
                        backoff.reset();
                    }
                    // Announced only if it comes: not for a record that
                    // ended, an identity that closed, or a store that
                    // failed.
                    if self.needs_scheduler(runtime, &contact, &peer) {
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
                    if self.needs_scheduler(runtime, &contact, &peer) {
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
        let mut task = SessionTask {
            inner: self.clone(),
            runtime: runtime.clone(),
            running: Running {
                serial: runtime.manager.next_serial(),
                control: Arc::new(SessionControl::default()),
                record: None,
                in_flight: None,
                probe: None,
                ping: None,
                activity: Instant::now(),
                transfers: Transfers::default(),
            },
            ended: Ended::Other,
        };
        let mut changes = identity.message_changes();
        task.ended = match self
            .drive(runtime, &mut link, &mut task.running, &mut changes)
            .await
        {
            Ok(ended) => ended,
            Err(_) => Ended::Other,
        };
        // Dropping the task ends its registration.
        task.ended
    }

    /// The session `serial` of the record `contact` ended: it leaves the
    /// manager, and if no session of the record is left and messages are,
    /// a scheduler starts after the first delay. Runs when the task of the
    /// session is dropped, also where no tokio runtime runs any more; then
    /// it starts nothing.
    fn session_ended(
        self: &Arc<Self>,
        runtime: &Arc<IdentityRuntime>,
        contact: ContactId,
        serial: u64,
        ended: Ended,
    ) {
        {
            let mut timed_out = lock(&runtime.timed_out);
            if ended == Ended::AckTimeout {
                timed_out.insert(contact);
            } else {
                timed_out.remove(&contact);
            }
        }
        runtime.manager.unregister(&contact, serial);
        self.announce(runtime, contact);
        if runtime.manager.has_session(&contact) {
            return;
        }
        let identity = &runtime.identity;
        let Some(peer) = identity.current_record(&contact) else {
            return;
        };
        if !self.needs_scheduler(runtime, &contact, &peer)
            || tokio::runtime::Handle::try_current().is_err()
        {
            return;
        }
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

    /// Queues a message and raises its event: the work of
    /// [`Runtime::send_message`] once its text was checked.
    async fn send(
        self: &Arc<Self>,
        held: &Arc<IdentityRuntime>,
        identity: IdentityId,
        contact: ContactId,
        text: ChatText,
    ) -> Result<MessageRef, SendError> {
        let queued = held
            .identity
            .queue_message(contact, text)
            .await
            .map_err(|error| {
                if matches!(error, MessageError::Storage(_)) {
                    self.storage_failure();
                }
                send_error(error)
            })?;
        let reference = MessageRef {
            identity,
            contact,
            sequence: queued.sequence,
        };
        self.emit(Event::MessageQueued(reference));
        if let Some(peer) = held.identity.current_record(&contact) {
            if !held.manager.has_session(&contact) {
                self.ensure_scheduler(held, contact, peer, Duration::ZERO);
            }
        }
        Ok(reference)
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
            // A message store that failed takes nothing in and records no
            // delivery: the session has nothing left to do.
            if running.control.closing()
                || self.stopping()
                || identity.is_closed()
                || self.halted(&identity)
            {
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
                if running.ping.is_none() {
                    link.send(&Message::Ping(nonce)).await?;
                    running.ping = Some((nonce, PONG_TIMEOUT));
                }
                running.probe = Some((nonce, DUPLICATE_PROBE_TIMEOUT));
            }
            // The preferred session sends the head of the queue, one
            // message at a time.
            if let Some(contact) = running.record {
                if running.in_flight.is_none()
                    && runtime.manager.is_preferred(&contact, running.serial)
                {
                    // Only what the message store holds leaves.
                    let next = identity.next_to_send(&contact).await;
                    if let Err(MessageError::Storage(_)) = next {
                        self.storage_failure();
                    }
                    if let Some((_, id, text)) = next.ok().flatten() {
                        link.send(&Message::ChatMessage { id, text }).await?;
                        let now = Instant::now();
                        running.in_flight = Some((id, after(now, MESSAGE_ACK_TIMEOUT)));
                        running.activity = now;
                    }
                }
            }
            let reading = Instant::now();
            let deadline = self.deadline(runtime, link, running, reading);
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
            running.read_for(reading.elapsed());
            match next {
                Next::Woken | Next::Changed => {}
                Next::Deadline => {
                    let now = Instant::now();
                    if let Some((nonce, left)) = running.probe {
                        if left.is_zero() {
                            running.probe = None;
                            if let Some(contact) = running.record {
                                runtime.manager.probe_ended(
                                    &contact,
                                    running.serial,
                                    nonce,
                                    false,
                                    identity.is_active_key(link.session().peer_card()),
                                );
                            }
                        }
                    }
                    if running.ping.is_some_and(|(_, left)| left.is_zero()) {
                        // A Ping not answered in time ends the session
                        // (`PROTOCOL.md` section 8.2).
                        return link_close(link).await.map(|()| Ended::Other);
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
                    if self.pace(runtime, running, &arrived).await == Waited::Stop {
                        return link_close(link).await.map(|()| Ended::Other);
                    }
                    self.handle(runtime, link, running, &arrived).await?;
                }
            }
        }
    }

    /// The earliest deadline of the task, which reads from `now`: an
    /// acknowledgement, a probe, a Pong, the linger of an idle session.
    fn deadline<S>(
        &self,
        runtime: &IdentityRuntime,
        link: &Link<S>,
        running: &Running,
        now: Instant,
    ) -> Option<Instant>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let mut deadlines = Vec::with_capacity(4);
        if let Some((_, due)) = running.in_flight {
            deadlines.push(due);
        }
        for (_, left) in running.probe.iter().chain(running.ping.iter()) {
            deadlines.push(after(now, *left));
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

    /// Waits until what arrived on a confirmed session has its token
    /// (`crate::rates`), reading nothing meanwhile: a contact above its rate
    /// is slowed, and the time is not spent of its probe or of its Ping.
    /// `Waited::Stop` if the session is to end first; the message is then
    /// not handled.
    async fn pace(
        &self,
        runtime: &IdentityRuntime,
        running: &Running,
        arrived: &crate::link::Arrived,
    ) -> Waited {
        let (Some(contact), Some(kind)) = (running.record, Kind::of(&arrived.received().message))
        else {
            return Waited::Done;
        };
        loop {
            let at = match lock(&runtime.rates).message(contact, kind, Instant::now()) {
                Ok(()) => return Waited::Done,
                Err(at) => at,
            };
            {
                let mut woken = pin!(running.control.woken());
                let mut timer = pin!(tokio::time::sleep_until(at));
                poll_fn(|cx| {
                    if woken.as_mut().poll(cx).is_ready() || timer.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(());
                    }
                    Poll::Pending
                })
                .await;
            }
            if running.control.closing() || self.stopping() || runtime.identity.is_closed() {
                return Waited::Stop;
            }
        }
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
                match identity.acknowledged(arrived).await {
                    Ok(Some((contact, sequence))) => {
                        // Only an acknowledgement that delivered a message
                        // is activity.
                        running.activity = Instant::now();
                        self.emit(Event::MessageDelivered(MessageRef {
                            identity: identity.id(),
                            contact,
                            sequence,
                        }));
                    }
                    Err(MessageError::Storage(_)) => self.storage_failure(),
                    Ok(None) | Err(_) => {}
                }
                if running.in_flight.is_some_and(|(sent, _)| sent == *id) {
                    running.in_flight = None;
                }
                Ok(())
            }
            Message::Ping(nonce) => link.send(&Message::Pong(*nonce)).await,
            Message::Pong(nonce) => match running.ping {
                Some((sent, _)) if sent == *nonce => {
                    running.ping = None;
                    // It answers the probe that runs, if one does; one
                    // decided before it came stands.
                    if let (Some((probe, _)), Some(contact)) =
                        (running.probe.take(), running.record)
                    {
                        runtime.manager.probe_ended(
                            &contact,
                            running.serial,
                            probe,
                            true,
                            identity.is_active_key(link.session().peer_card()),
                        );
                    }
                    Ok(())
                }
                // A Pong that answers no outstanding Ping of this side is a
                // violation: the session ends without a Close.
                _ => Err(LinkError::Session(
                    monolith_session::SessionError::Protocol(ProtocolError::MessageNotPermitted),
                )),
            },
            // A profile is a message of this version that is not used:
            // the session admitted it, its texts were checked, and it is
            // dropped (`PROTOCOL.md` 8.7). An endpoint update was for the
            // store, which saw it.
            Message::Profile { .. } | Message::EndpointUpdate(_) => Ok(()),
            // No file is transferred; what section 13.1 asks of a
            // receiver is kept (`crate::transfers`).
            message => match running.transfers.receive(message) {
                Some(Answer::Reject(transfer)) => link.send(&Message::FileReject(transfer)).await,
                Some(Answer::Violation) => Err(LinkError::Session(
                    monolith_session::SessionError::Protocol(ProtocolError::MessageNotPermitted),
                )),
                Some(Answer::Discard) | None => Ok(()),
            },
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
            runtime.manager.register(
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
            self.announce(runtime, contact);
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
                // Only a message taken in is activity: one held already is
                // acknowledged again, and keeps no session open.
                if matches!(taken.receipt, Receipt::Stored(_)) {
                    running.activity = Instant::now();
                }
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
                self.storage_failure();
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
            storage_failed: AtomicBool::new(false),
        });
        for identity in inner.installation.identities() {
            inner.add_identity(identity, true);
        }
        let watching = inner.clone();
        inner.spawn(async move { watching.watch_storage().await });
        Self {
            owner: Arc::new(Owner {
                inner: inner.clone(),
            }),
            inner,
        }
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
            .map(|held| {
                let fingerprint = Fingerprint::of(held.identity.identity());
                IdentitySummary {
                    identity: held.identity.id(),
                    fingerprint: fingerprint.to_compact_text(),
                    full_fingerprint: fingerprint.to_full_text(),
                    label: held.identity.label().map(|label| label.as_str().to_owned()),
                }
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
                let fingerprint = Fingerprint::of(&peer);
                Some(ContactSummary {
                    contact,
                    fingerprint: fingerprint.to_compact_text(),
                    full_fingerprint: fingerprint.to_full_text(),
                    alias: view.alias.map(|alias| alias.as_str().to_owned()),
                    accepted: view.kind == monolith_protocol::contact::RecordKind::Accepted,
                    verified: view.verified,
                    invitation: view
                        .credentials
                        .as_ref()
                        .is_some_and(|credentials| credentials.invitation().is_some()),
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
                active: held.identity.is_current(&view.contact, &view.peer),
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
    /// delivered when the contact is reached. Like every command that
    /// changes something, it completes and raises its event also if the
    /// caller stops waiting for it.
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
        let held = self.held(identity).map_err(|error| match error {
            CommandError::ShuttingDown => SendError::ShuttingDown,
            _ => SendError::UnknownIdentity,
        })?;
        let inner = self.inner.clone();
        let (identity, contact) = (*identity, *contact);
        // A send the shutdown cut short stored nothing, as at once.
        self.inner
            .detached(async move { inner.send(&held, identity, contact, text).await })
            .await
            .unwrap_or(Err(SendError::ShuttingDown))
    }

    /// Deletes the messages of the conversation of `contact`, queued ones
    /// included, from the message store. Nothing is sent to the contact.
    pub async fn delete_conversation(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
    ) -> Result<bool, CommandError> {
        let held = self.held(identity)?;
        let inner = self.inner.clone();
        let contact = *contact;
        self.inner
            .detached(async move {
                held.identity
                    .delete_conversation(&contact)
                    .await
                    .map_err(|error| {
                        if matches!(error, MessageError::Storage(_)) {
                            inner.storage_failure();
                        }
                        CommandError::Storage
                    })
            })
            .await
            .unwrap_or(Err(CommandError::ShuttingDown))
    }

    /// The card of `identity` in its text form, without a capability.
    pub fn own_card(&self, identity: &IdentityId) -> Result<String, CommandError> {
        Ok(self.held(identity)?.identity.card().to_text())
    }

    /// Imports a contact card given in its text form. Returns the contact
    /// record of its identity, and whether the card that record holds
    /// afterwards carries an invitation, which is not the card given when
    /// that changed nothing.
    pub async fn add_contact(
        &self,
        identity: &IdentityId,
        card: &str,
    ) -> Result<AddedContact, CommandError> {
        let held = self.held(identity)?;
        let card = ContactCard::from_text(card).map_err(|_| CommandError::InvalidCard)?;
        self.inner
            .detached(async move {
                held.identity.import(&card).await.map_err(store_error)?;
                let contact = held
                    .identity
                    .contact_id(card.identity())
                    .ok_or(CommandError::UnknownContact)?;
                // What the record holds after the import, as its summary
                // says: the card of an accepted contact changes nothing
                // (`PROTOCOL.md` section 12.2).
                let invitation = held
                    .identity
                    .contact(card.identity())
                    .and_then(|view| view.credentials)
                    .is_some_and(|credentials| credentials.invitation().is_some());
                Ok(AddedContact {
                    contact,
                    invitation,
                })
            })
            .await
            .unwrap_or(Err(CommandError::ShuttingDown))
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
        self.end_record(identity, contact, true).await
    }

    /// Deletes the contact of the record `contact`: its sessions end, its
    /// queue stays stored and is never sent; a later card of the same
    /// identity is a new record with a new conversation.
    pub async fn delete_contact(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
    ) -> Result<(), CommandError> {
        self.end_record(identity, contact, false).await
    }

    /// Blocks or deletes the contact of the record `contact`, and closes
    /// its sessions, which also ends its scheduler.
    async fn end_record(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
        block: bool,
    ) -> Result<(), CommandError> {
        let held = self.held(identity)?;
        Self::peer_of(&held, contact)?;
        let contact = *contact;
        self.inner
            .detached(async move {
                // The contact is named here, where nothing awaited
                // separates it from the change: a record ended and made
                // again before the work runs is not taken for this one.
                let peer = Self::peer_of(&held, &contact)?;
                if block {
                    held.identity.block(&peer).await
                } else {
                    held.identity.delete(&peer).await
                }
                .map_err(store_error)?;
                held.manager.close(&contact);
                lock(&held.rates).ended(&contact);
                Ok(())
            })
            .await
            .unwrap_or(Err(CommandError::ShuttingDown))
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
        let inner = self.inner.clone();
        self.inner
            .detached(async move {
                let mut service = inner
                    .backend
                    .publish_onion(KeySource::Generate)
                    .await
                    .map_err(|_| CommandError::Tor)?;
                let secret = service.take_generated_secret().ok_or(CommandError::Tor)?;
                let endpoint = *service.service_key();
                // Published again by the supervisor, from the stored key.
                let _ = service.close().await;
                let identity = inner
                    .installation
                    .create_identity(endpoint, Some(&secret), label)
                    .await
                    .map_err(installation_error)?;
                let id = identity.id();
                inner.add_identity(identity, false);
                Ok(id)
            })
            .await
            .unwrap_or(Err(CommandError::ShuttingDown))
    }

    /// Deletes a local identity: its keys, its contacts, its messages. Its
    /// service is removed and its sessions end.
    pub async fn delete_identity(&self, identity: &IdentityId) -> Result<(), CommandError> {
        let held = self.held(identity)?;
        let inner = self.inner.clone();
        self.inner
            .detached(async move {
                held.manager.close_all();
                inner
                    .installation
                    .delete_identity(&held.identity)
                    .await
                    .map_err(installation_error)
            })
            .await
            .unwrap_or(Err(CommandError::ShuttingDown))
    }

    /// Sets the local name of the contact record `contact`, or removes it
    /// with `None` or an empty name. The name is never sent. Fails with
    /// [`CommandError::UnknownContact`] if the record is no longer the
    /// current one of its identity.
    pub async fn set_contact_name(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
        name: Option<&str>,
    ) -> Result<(), CommandError> {
        let held = self.held(identity)?;
        let alias = name
            .filter(|name| !name.is_empty())
            .map(DisplayName::new)
            .transpose()
            .map_err(|_| CommandError::InvalidText)?;
        let contact = *contact;
        self.inner
            .detached(async move {
                held.identity
                    .set_record_alias(&contact, alias)
                    .await
                    .map_err(store_error)
            })
            .await
            .unwrap_or(Err(CommandError::ShuttingDown))
    }

    /// Records whether the user compared the fingerprint of the contact
    /// record `contact` out of band.
    pub async fn set_contact_verified(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
        verified: bool,
    ) -> Result<(), CommandError> {
        let held = self.held(identity)?;
        let contact = *contact;
        self.inner
            .detached(async move {
                held.identity
                    .set_record_verified(&contact, verified)
                    .await
                    .map_err(store_error)
            })
            .await
            .unwrap_or(Err(CommandError::ShuttingDown))
    }

    /// The contact requests of `identity` waiting for an answer.
    pub fn requests(&self, identity: &IdentityId) -> Result<Vec<RequestSummary>, CommandError> {
        let held = self.held(identity)?;
        Ok(held
            .identity
            .requests()
            .into_iter()
            .map(|pending| {
                let fingerprint = Fingerprint::of(pending.identity());
                RequestSummary {
                    request: RequestRef::new(*identity, pending.id.parts()),
                    fingerprint: fingerprint.to_compact_text(),
                    full_fingerprint: fingerprint.to_full_text(),
                    display_name: pending.display_name.as_str().to_owned(),
                    introduction: pending.introduction.as_str().to_owned(),
                    invitation: pending
                        .admitted_by
                        .map(|invitation| InvitationRef::new(*identity, invitation.parts())),
                }
            })
            .collect())
    }

    /// Accepts the contact request `request`: its sender becomes an
    /// accepted contact with the card of that request. Returns the record.
    /// A request answered or dropped since, also one its sender replaced
    /// with another, is [`CommandError::UnknownRequest`].
    pub async fn accept_request(&self, request: &RequestRef) -> Result<ContactId, CommandError> {
        let held = self.held(&request.identity)?;
        let id = RequestId::from_parts(request.request());
        self.inner
            .detached(async move {
                let peer = held
                    .identity
                    .requests()
                    .into_iter()
                    .find(|pending| pending.id == id)
                    .map(|pending| *pending.identity())
                    .ok_or(CommandError::UnknownRequest)?;
                held.identity
                    .accept_request(id)
                    .await
                    .map_err(request_error)?;
                held.identity
                    .contact_id(&peer)
                    .ok_or(CommandError::UnknownContact)
            })
            .await
            .unwrap_or(Err(CommandError::ShuttingDown))
    }

    /// Declines the contact request `request`. Nothing is sent.
    pub async fn decline_request(&self, request: &RequestRef) -> Result<(), CommandError> {
        self.answer_request(request, false).await
    }

    /// Blocks the sender of the contact request `request`.
    pub async fn block_request(&self, request: &RequestRef) -> Result<(), CommandError> {
        self.answer_request(request, true).await
    }

    async fn answer_request(&self, request: &RequestRef, block: bool) -> Result<(), CommandError> {
        let held = self.held(&request.identity)?;
        let id = RequestId::from_parts(request.request());
        self.inner
            .detached(async move {
                if block {
                    held.identity.block_request(id).await
                } else {
                    held.identity.decline_request(id).await
                }
                .map_err(request_error)
            })
            .await
            .unwrap_or(Err(CommandError::ShuttingDown))
    }

    /// The invitations of the active set of `identity`.
    pub fn invitations(
        &self,
        identity: &IdentityId,
    ) -> Result<Vec<InvitationSummary>, CommandError> {
        let held = self.held(identity)?;
        Ok(held
            .identity
            .invitations()
            .into_iter()
            .map(|(invitation, label)| InvitationSummary {
                invitation: InvitationRef::new(*identity, invitation.parts()),
                label: label.map(|label| label.as_str().to_owned()),
            })
            .collect())
    }

    /// Makes a new invitation with an optional local label. Returns it with
    /// the card that carries it, in its text form, for the user to hand to
    /// one person. Fails with [`CommandError::Full`] when the active set is
    /// full; nothing is revoked to make room.
    pub async fn create_invitation(
        &self,
        identity: &IdentityId,
        label: Option<&str>,
    ) -> Result<(InvitationRef, String), CommandError> {
        let held = self.held(identity)?;
        let label = label
            .filter(|label| !label.is_empty())
            .map(DisplayName::new)
            .transpose()
            .map_err(|_| CommandError::InvalidText)?;
        let identity = *identity;
        self.inner
            .detached(async move {
                let (invitation, card) = held
                    .identity
                    .create_invitation(label)
                    .await
                    .map_err(invitation_error)?;
                Ok((
                    InvitationRef::new(identity, invitation.parts()),
                    card.to_text(),
                ))
            })
            .await
            .unwrap_or(Err(CommandError::ShuttingDown))
    }

    /// The card of the invitation `invitation` again, in its text form.
    pub fn invitation_card(&self, invitation: &InvitationRef) -> Result<String, CommandError> {
        let held = self.held(&invitation.identity)?;
        held.identity
            .invitation_card(InvitationId::from_parts(invitation.invitation()))
            .map(|card| card.to_text())
            .map_err(invitation_error)
    }

    /// Revokes the invitation `invitation`: requests that carry it are
    /// dropped from now on. With `discard_requests`, the requests it
    /// admitted that still wait are dropped too. Returns how many were.
    pub async fn revoke_invitation(
        &self,
        invitation: &InvitationRef,
        discard_requests: bool,
    ) -> Result<usize, CommandError> {
        let held = self.held(&invitation.identity)?;
        let id = InvitationId::from_parts(invitation.invitation());
        self.inner
            .detached(async move {
                if discard_requests {
                    held.identity.revoke_and_discard(id).await
                } else {
                    held.identity.revoke_invitation(id).await.map(|()| 0)
                }
                .map_err(invitation_error)
            })
            .await
            .unwrap_or(Err(CommandError::ShuttingDown))
    }

    /// The identities `identity` blocked, by their fingerprints.
    pub fn blocked(&self, identity: &IdentityId) -> Result<Vec<BlockedSummary>, CommandError> {
        let held = self.held(identity)?;
        let instance = held.identity.instance();
        let mut blocked: Vec<BlockedSummary> = held
            .identity
            .blocked()
            .iter()
            .map(|peer| {
                let fingerprint = Fingerprint::of(peer);
                BlockedSummary {
                    blocked: BlockedRef::new(*identity, (instance, *fingerprint.as_bytes())),
                    fingerprint: fingerprint.to_compact_text(),
                    full_fingerprint: fingerprint.to_full_text(),
                }
            })
            .collect();
        blocked.sort_by(|a, b| a.fingerprint.cmp(&b.fingerprint));
        Ok(blocked)
    }

    /// Unblocks the identity `blocked` names: it has no record afterwards,
    /// as one never met. It is not a contact again, and a request of its is
    /// considered as the request mode says. Fails with
    /// [`CommandError::NotBlocked`] if the identity is not blocked now, or
    /// the handle is of another local identity or of an instance of it
    /// from before. The handle names the identity blocked, not one block of
    /// it: the store keeps nothing else of a block.
    pub async fn unblock(&self, blocked: &BlockedRef) -> Result<(), CommandError> {
        let held = self.held(&blocked.identity)?;
        let (instance, fingerprint) = blocked.blocked();
        if instance != held.identity.instance() {
            return Err(CommandError::NotBlocked);
        }
        let peer = held
            .identity
            .blocked()
            .into_iter()
            .find(|peer| Fingerprint::of(peer).as_bytes() == &fingerprint)
            .ok_or(CommandError::NotBlocked)?;
        self.inner
            .detached(async move { held.identity.unblock(&peer).await.map_err(blocked_error) })
            .await
            .unwrap_or(Err(CommandError::ShuttingDown))
    }

    /// Which requests from strangers `identity` considers.
    pub fn request_mode(&self, identity: &IdentityId) -> Result<RequestMode, CommandError> {
        Ok(match self.held(identity)?.identity.request_mode() {
            monolith_protocol::contact::RequestMode::Invitation => RequestMode::Invitation,
            monolith_protocol::contact::RequestMode::Open => RequestMode::Open,
            monolith_protocol::contact::RequestMode::Closed => RequestMode::Closed,
        })
    }

    /// Sets which requests from strangers `identity` considers.
    pub async fn set_request_mode(
        &self,
        identity: &IdentityId,
        mode: RequestMode,
    ) -> Result<(), CommandError> {
        let held = self.held(identity)?;
        let mode = match mode {
            RequestMode::Invitation => monolith_protocol::contact::RequestMode::Invitation,
            RequestMode::Open => monolith_protocol::contact::RequestMode::Open,
            RequestMode::Closed => monolith_protocol::contact::RequestMode::Closed,
        };
        self.inner
            .detached(async move {
                held.identity
                    .set_request_mode(mode)
                    .await
                    .map_err(store_error)
            })
            .await
            .unwrap_or(Err(CommandError::ShuttingDown))
    }

    /// The state of the Onion Service of `identity`, as last reported by
    /// [`Event::Service`].
    pub fn service_state(&self, identity: &IdentityId) -> Result<ServiceState, CommandError> {
        Ok(*lock(&self.held(identity)?.service))
    }

    /// Asks the system Tor how it is: both endpoints, the version and
    /// whether it has a circuit. Contacts only the configured endpoints.
    pub async fn check_tor(&self) -> TorCheck {
        tor_check(&self.inner.backend.status().await)
    }

    /// Asks for a session with the contact of the record `contact` now, an
    /// explicit local operation, whether or not anything is queued. One
    /// attempt, through the Tor backend, over the cards of the dial plan
    /// ([`dial_plan`]); none while a session of the record exists.
    pub fn connect(&self, identity: &IdentityId, contact: &ContactId) -> Result<(), CommandError> {
        let held = self.held(identity)?;
        let peer = Self::peer_of(&held, contact)?;
        if held.manager.has_session(contact) {
            return Ok(());
        }
        let plan = held
            .identity
            .dial_plan(&peer)
            .ok_or(CommandError::UnknownContact)?;
        if plan.cards.is_empty() {
            return Err(CommandError::UnknownContact);
        }
        let inner = self.inner.clone();
        let contact = *contact;
        self.inner.spawn(async move {
            let go_on =
                || !held.manager.has_session(&contact) && held.identity.is_current(&contact, &peer);
            if let Ok(established) = dial_plan(
                &inner.backend,
                &inner.budgets,
                &held.identity,
                &plan.cards,
                go_on,
            )
            .await
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

    /// Returns true if the vault was opened without its message store,
    /// though it holds identities: the messages from before are not there
    /// (`Installation::message_store_missing`).
    pub fn message_store_missing(&self) -> bool {
        self.inner.installation.message_store_missing()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persist::CommitError;

    #[test]
    fn a_failed_send_says_what_failed() {
        assert_eq!(send_error(MessageError::Closed), SendError::UnknownIdentity);
        assert_eq!(
            send_error(MessageError::NotAContact),
            SendError::NotAContact
        );
        assert_eq!(
            send_error(MessageError::Refused(Refusal::ContactQueueFull)),
            SendError::QueueFull
        );
        assert_eq!(
            send_error(MessageError::Storage(CommitError::Failed)),
            SendError::Storage
        );
        // A failed random source is not a failed store.
        assert_eq!(send_error(MessageError::Randomness), SendError::Randomness);
    }
}

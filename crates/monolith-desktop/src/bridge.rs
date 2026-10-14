//! The bridge between the window and the runtime
//! (`docs/adr/0011-ui-security-boundary.md`).
//!
//! The runtime runs on a tokio runtime of its own thread. The window sends
//! commands and takes updates through two bounded channels and never
//! waits on either: a command that does not fit waits in a queue of the
//! window until the next frame, and an update waits in the bridge until
//! the window takes it. Queries and events are handled by one task, in
//! order; commands that write run as tasks of their own and report back
//! through that same task.
//!
//! Requests and invitations are named in the core by references that hold
//! no meaning outside it and do not outlive the process. The bridge gives
//! each one it reports a number of its own, never reused, and checks on
//! every use that the number belongs to the identity the window names
//! with it.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use core::future::Future;
use core::hash::Hash;
use core::time::Duration;

use monolith_runtime::api::{
    self, CommandError, IdentityId, InvitationRef, MessageRef, RequestRef,
};
use monolith_runtime::{MAX_PAGE, Messenger, Missed, TorBackend};
use monolith_ui_model::{
    Command, CommandId, ContactView, ConversationView, Failure, IdentitySnapshot, IdentityView,
    InvitationView, Outcome, ProfileOnDisk, RequestView, RuntimeEvent, UiContactId, UiIdentityId,
    UiInvitationId, UiRequestId, Update,
};
use tokio::sync::mpsc;
use tokio::task::{self, JoinSet, LocalSet};
use zeroize::Zeroizing;

use crate::convert;

/// How many commands, and how many updates, wait in a channel at most.
const CHANNEL: usize = 256;

/// Called when an update is ready, to wake the window.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// What the bridge starts with.
pub struct Setup<B> {
    /// The data directory; none if the system names no place for it, and
    /// then only a session that saves nothing can start.
    pub data_dir: Option<PathBuf>,
    /// Makes the Tor backend of a messenger; called once per profile
    /// opened or created.
    pub backend: Box<dyn FnMut() -> B + Send>,
    /// How long the runtime has to close its sessions and remove its
    /// services when it stops.
    pub shutdown_deadline: Duration,
    /// How long the window waits for the bridge when it closes.
    pub close_wait: Duration,
}

/// The window's end of the bridge.
pub struct Bridge {
    commands: Option<mpsc::Sender<(CommandId, Command)>>,
    backlog: VecDeque<(CommandId, Command)>,
    updates: mpsc::Receiver<Update>,
    /// The window was told that the runtime stopped.
    stopped: bool,
    thread: Option<JoinHandle<()>>,
    close_wait: Duration,
}

impl core::fmt::Debug for Bridge {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Bridge")
            .field("backlog", &self.backlog.len())
            .finish_non_exhaustive()
    }
}

impl Bridge {
    /// Starts the runtime thread. It says [`Update::Started`] first.
    pub fn start<B: TorBackend + 'static>(setup: Setup<B>, wake: Wake) -> std::io::Result<Self> {
        let (command_sender, command_receiver) = mpsc::channel(CHANNEL);
        let (update_sender, update_receiver) = mpsc::channel(CHANNEL);
        let close_wait = setup.close_wait;
        let thread = std::thread::Builder::new()
            .name("monolith-runtime".to_owned())
            .spawn(move || {
                let out = Out {
                    updates: update_sender,
                    wake,
                };
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => {
                        LocalSet::new().block_on(&runtime, run(setup, command_receiver, out));
                    }
                    Err(_) => {
                        let _ = out.updates.blocking_send(Update::Stopped);
                        (out.wake)();
                    }
                }
            })?;
        Ok(Self {
            commands: Some(command_sender),
            backlog: VecDeque::new(),
            updates: update_receiver,
            stopped: false,
            thread: Some(thread),
            close_wait,
        })
    }

    /// Hands a command to the bridge, or queues it until there is room.
    pub fn send(&mut self, command: CommandId, what: Command) {
        self.backlog.push_back((command, what));
        self.flush();
    }

    /// Hands the queued commands to the bridge, as many as fit.
    pub fn flush(&mut self) {
        let Some(sender) = &self.commands else {
            self.backlog.clear();
            return;
        };
        while let Some(item) = self.backlog.pop_front() {
            match sender.try_send(item) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(item)) => {
                    self.backlog.push_front(item);
                    break;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    // The bridge stopped; it said so with an update.
                    self.backlog.clear();
                    self.commands = None;
                    break;
                }
            }
        }
    }

    /// Whether commands wait for room.
    pub fn backlogged(&self) -> bool {
        !self.backlog.is_empty()
    }

    /// The next update, if one is ready. A runtime thread that ended
    /// without a word (it panicked) is said to have stopped, once.
    pub fn try_update(&mut self) -> Option<Update> {
        match self.updates.try_recv() {
            Ok(update) => {
                self.stopped |= update == Update::Stopped;
                Some(update)
            }
            Err(mpsc::error::TryRecvError::Empty) => None,
            Err(mpsc::error::TryRecvError::Disconnected) => {
                (!core::mem::replace(&mut self.stopped, true)).then_some(Update::Stopped)
            }
        }
    }
}

impl Drop for Bridge {
    /// Closes the bridge: the runtime stops, as for
    /// [`Command::Shutdown`], and the window waits for it a while.
    fn drop(&mut self) {
        self.commands = None;
        self.backlog.clear();
        self.updates.close();
        let Some(thread) = self.thread.take() else {
            return;
        };
        let deadline = Instant::now().checked_add(self.close_wait);
        while !thread.is_finished() && deadline.is_some_and(|deadline| Instant::now() < deadline) {
            std::thread::sleep(Duration::from_millis(10));
        }
        if thread.is_finished() {
            let _ = thread.join();
        }
    }
}

/// The bridge's end of the updates.
struct Out {
    updates: mpsc::Sender<Update>,
    wake: Wake,
}

impl Out {
    /// Sends an update. False once the window is gone.
    async fn send(&self, update: Update) -> bool {
        let sent = self.updates.send(update).await.is_ok();
        (self.wake)();
        sent
    }

    async fn done(&self, command: CommandId, outcome: Outcome) -> bool {
        self.send(Update::Done { command, outcome }).await
    }
}

async fn run<B: TorBackend + 'static>(
    mut setup: Setup<B>,
    mut commands: mpsc::Receiver<(CommandId, Command)>,
    out: Out,
) {
    let profile = if setup
        .data_dir
        .as_deref()
        .is_some_and(monolith_runtime::profile_exists)
    {
        ProfileOnDisk::Exists
    } else {
        ProfileOnDisk::Absent
    };
    let started = Update::Started {
        profile,
        location: setup
            .data_dir
            .as_ref()
            .map(|dir| dir.display().to_string())
            .unwrap_or_default(),
    };
    if !out.send(started).await {
        return;
    }
    let Some((messenger, command, outcome)) = open(&mut setup, &mut commands, &out).await else {
        return;
    };
    let messenger = Rc::new(messenger);
    // Followed before the window hears of the profile, so that nothing
    // the runtime does from now on goes unseen.
    let events = messenger.events();
    let mut running = Running {
        messenger,
        out,
        requests: Handles::default(),
        invitations: Handles::default(),
    };
    if running.out.done(command, outcome).await {
        running.serve(commands, events).await;
    }
    running.messenger.shutdown(setup.shutdown_deadline).await;
    let _ = running.out.send(Update::Stopped).await;
}

/// Waits for a profile to be opened, created or started without saving.
async fn open<B: TorBackend + 'static>(
    setup: &mut Setup<B>,
    commands: &mut mpsc::Receiver<(CommandId, Command)>,
    out: &Out,
) -> Option<(Messenger<B>, CommandId, Outcome)> {
    while let Some((command, what)) = commands.recv().await {
        let opened = match (what, setup.data_dir.as_deref()) {
            (Command::OpenProfile { passphrase }, Some(dir)) => {
                Messenger::open(dir, &passphrase, (setup.backend)())
                    .map(|messenger| (messenger, false))
            }
            (Command::CreateProfile { passphrase }, Some(dir)) => {
                // The data directory itself is made by the vault, private;
                // its parents are ordinary folders of the user.
                if let Some(parent) = dir.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                Messenger::create(dir, &passphrase, (setup.backend)())
                    .map(|messenger| (messenger, false))
            }
            (Command::OpenProfile { .. } | Command::CreateProfile { .. }, None) => {
                if !out.done(command, Outcome::Failed(Failure::Disk)).await {
                    return None;
                }
                continue;
            }
            (Command::StartWithoutSaving, _) => Ok((Messenger::ephemeral((setup.backend)()), true)),
            (Command::Shutdown, _) => {
                let _ = out.done(command, Outcome::Done).await;
                let _ = out.send(Update::Stopped).await;
                return None;
            }
            _ => {
                if !out
                    .done(command, Outcome::Failed(Failure::Unexpected))
                    .await
                {
                    return None;
                }
                continue;
            }
        };
        match opened {
            Ok((messenger, ephemeral)) => {
                let outcome = Outcome::ProfileOpen {
                    ephemeral,
                    message_store_missing: messenger.message_store_missing(),
                };
                return Some((messenger, command, outcome));
            }
            Err(error) => {
                let failure = Outcome::Failed(convert::open_error(error));
                if !out.done(command, failure).await {
                    return None;
                }
            }
        }
    }
    None
}

/// A reference of the core that belongs to one identity.
trait Owned: Copy + Eq + Hash {
    fn owner(&self) -> IdentityId;
}

impl Owned for RequestRef {
    fn owner(&self) -> IdentityId {
        self.identity
    }
}

impl Owned for InvitationRef {
    fn owner(&self) -> IdentityId {
        self.identity
    }
}

/// The numbers given to references of the core.
#[derive(Debug)]
struct Handles<T> {
    last: u64,
    by_handle: HashMap<u64, T>,
    by_reference: HashMap<T, u64>,
}

impl<T> Default for Handles<T> {
    fn default() -> Self {
        Self {
            last: 0,
            by_handle: HashMap::new(),
            by_reference: HashMap::new(),
        }
    }
}

impl<T: Owned> Handles<T> {
    /// The number of `reference`: the one it was given, or a new one.
    fn handle(&mut self, reference: T) -> u64 {
        if let Some(handle) = self.by_reference.get(&reference) {
            return *handle;
        }
        // A u64 counted up by one per reference seen is never exhausted.
        self.last = self.last.saturating_add(1);
        self.by_handle.insert(self.last, reference);
        self.by_reference.insert(reference, self.last);
        self.last
    }

    /// The reference numbered `handle`, if it belongs to `owner`.
    fn get(&self, owner: UiIdentityId, handle: u64) -> Option<T> {
        self.by_handle
            .get(&handle)
            .copied()
            .filter(|reference| convert::identity(&reference.owner()) == owner)
    }

    /// Forgets the references of `owner` that are not in `listed`: they
    /// were answered, revoked or dropped. Their numbers are not given
    /// again.
    fn keep(&mut self, owner: &IdentityId, listed: &HashSet<T>) {
        self.by_reference
            .retain(|reference, _| reference.owner() != *owner || listed.contains(reference));
        self.by_handle
            .retain(|_, reference| reference.owner() != *owner || listed.contains(reference));
    }
}

/// A command that ran as a task of its own.
enum Finished {
    /// It ended with this.
    Outcome(Outcome),
    /// An invitation was made; it needs a number.
    Invitation(Result<(InvitationRef, Zeroizing<String>), Failure>),
}

/// The commands that run as tasks of their own. The command a task
/// answers is kept outside it, so that a task that panicked is answered
/// all the same.
#[derive(Default)]
struct Tasks {
    set: JoinSet<Finished>,
    commands: HashMap<task::Id, CommandId>,
}

impl Tasks {
    /// Runs `task` for `command`.
    fn spawn(&mut self, command: CommandId, task: impl Future<Output = Finished> + 'static) {
        let handle = self.set.spawn_local(task);
        self.commands.insert(handle.id(), command);
    }

    fn is_empty(&self) -> bool {
        self.set.is_empty()
    }

    /// The next command whose task ended, with what it ended with; none
    /// if it panicked.
    async fn next(&mut self) -> Option<(CommandId, Option<Finished>)> {
        loop {
            let (id, finished) = match self.set.join_next_with_id().await? {
                Ok((id, finished)) => (id, Some(finished)),
                Err(error) => (error.id(), None),
            };
            // Every task was spawned with its command.
            if let Some(command) = self.commands.remove(&id) {
                return Some((command, finished));
            }
        }
    }

    async fn shutdown(&mut self) {
        self.set.shutdown().await;
        self.commands.clear();
    }
}

struct Running<B: TorBackend + 'static> {
    messenger: Rc<Messenger<B>>,
    out: Out,
    requests: Handles<RequestRef>,
    invitations: Handles<InvitationRef>,
}

impl<B: TorBackend + 'static> Running<B> {
    /// Answers commands and forwards events until the window closes, asks
    /// to stop, or the runtime stops.
    async fn serve(
        &mut self,
        mut commands: mpsc::Receiver<(CommandId, Command)>,
        mut events: monolith_runtime::Events,
    ) {
        let mut tasks = Tasks::default();
        loop {
            let going = tokio::select! {
                received = commands.recv() => match received {
                    None => false,
                    Some((command, Command::Shutdown)) => {
                        let _ = self.out.done(command, Outcome::Done).await;
                        false
                    }
                    Some((command, what)) => self.command(command, what, &mut tasks).await,
                },
                event = events.next() => match event {
                    Ok(event) => match convert::event(event) {
                        Some(event) => self.out.send(Update::Event(event)).await,
                        None => true,
                    },
                    Err(Missed::Lagged(_)) => {
                        self.out.send(Update::Event(RuntimeEvent::Missed)).await
                    }
                    Err(Missed::Closed) => false,
                },
                Some((command, finished)) = tasks.next(), if !tasks.is_empty() => {
                    let outcome = match finished {
                        Some(finished) => self.finished(finished),
                        // A task does not panic; if one did, its command
                        // is answered all the same, and the window does
                        // not show it as running for ever.
                        None => Outcome::Failed(Failure::Unexpected),
                    };
                    self.out.done(command, outcome).await
                }
            };
            if !going {
                break;
            }
        }
        tasks.shutdown().await;
    }

    fn finished(&mut self, finished: Finished) -> Outcome {
        match finished {
            Finished::Outcome(outcome) => outcome,
            Finished::Invitation(Ok((invitation, card))) => Outcome::InvitationCreated {
                invitation: UiInvitationId::new(
                    convert::identity(&invitation.identity),
                    self.invitations.handle(invitation),
                ),
                card,
            },
            Finished::Invitation(Err(failure)) => Outcome::Failed(failure),
        }
    }

    /// Runs a command: a query here and now, anything else as a task.
    /// False once the window is gone.
    async fn command(&mut self, command: CommandId, what: Command, tasks: &mut Tasks) -> bool {
        let outcome = match what {
            Command::LoadIdentities => Outcome::Identities(
                self.messenger
                    .identities()
                    .into_iter()
                    .map(|summary| IdentityView {
                        id: convert::identity(&summary.identity),
                        name: summary.label,
                        fingerprint: summary.fingerprint,
                        full_fingerprint: summary.full_fingerprint,
                    })
                    .collect(),
            ),
            Command::LoadIdentity(identity) => match self.snapshot(identity) {
                Ok(snapshot) => Outcome::Identity(Box::new(snapshot)),
                Err(failure) => Outcome::Failed(failure),
            },
            Command::LoadMessages { contact, limit } => self.messages(contact, None, limit),
            Command::LoadEarlier {
                contact,
                before,
                limit,
            } => self.messages(contact, Some(before), limit),
            Command::OwnCard(identity) => {
                match convert::identity_id(identity).and_then(|identity| {
                    self.messenger
                        .own_card(&identity)
                        .map_err(convert::command_error)
                }) {
                    Ok(card) => Outcome::Card(Zeroizing::new(card)),
                    Err(failure) => Outcome::Failed(failure),
                }
            }
            Command::InvitationCard(invitation) => {
                match self.invitation(invitation).and_then(|reference| {
                    self.messenger
                        .invitation_card(&reference)
                        .map_err(convert::command_error)
                }) {
                    Ok(card) => Outcome::Card(Zeroizing::new(card)),
                    Err(failure) => Outcome::Failed(failure),
                }
            }
            Command::OpenProfile { .. }
            | Command::CreateProfile { .. }
            | Command::StartWithoutSaving
            | Command::Shutdown => Outcome::Failed(Failure::Unexpected),
            what => match self.task(what) {
                Ok(task) => {
                    tasks.spawn(command, task);
                    return true;
                }
                Err(failure) => Outcome::Failed(failure),
            },
        };
        self.out.done(command, outcome).await
    }

    fn invitation(&self, invitation: UiInvitationId) -> Result<InvitationRef, Failure> {
        self.invitations
            .get(invitation.identity(), invitation.handle())
            .ok_or(Failure::NoSuchInvitation)
    }

    fn request(&self, request: UiRequestId) -> Result<RequestRef, Failure> {
        self.requests
            .get(request.identity(), request.handle())
            .ok_or(Failure::NoSuchRequest)
    }

    /// The task of a command that writes or waits. The handles it names
    /// are looked up now, so that an unknown one fails at once.
    fn task(
        &self,
        what: Command,
    ) -> Result<core::pin::Pin<Box<dyn core::future::Future<Output = Finished>>>, Failure> {
        let messenger = Rc::clone(&self.messenger);
        let done = |result: Result<(), CommandError>| {
            Finished::Outcome(match result {
                Ok(()) => Outcome::Done,
                Err(error) => Outcome::Failed(convert::command_error(error)),
            })
        };
        let failed =
            |error: CommandError| Finished::Outcome(Outcome::Failed(convert::command_error(error)));
        Ok(match what {
            Command::CheckTor => Box::pin(async move {
                Finished::Outcome(Outcome::Tor(convert::tor_check(
                    messenger.check_tor().await,
                )))
            }),
            Command::CreateIdentity { name } => Box::pin(async move {
                match messenger.create_identity(name.as_deref()).await {
                    Ok(identity) => {
                        Finished::Outcome(Outcome::IdentityCreated(convert::identity(&identity)))
                    }
                    Err(error) => failed(error),
                }
            }),
            Command::DeleteIdentity(identity) => {
                let identity = convert::identity_id(identity)?;
                Box::pin(async move { done(messenger.delete_identity(&identity).await) })
            }
            Command::AddContact {
                identity,
                card,
                name,
            } => {
                // The card can carry an invitation of the other side; it
                // is wiped when the task drops it.
                let identity = convert::identity_id(identity)?;
                Box::pin(async move {
                    match messenger.add_contact(&identity, &card).await {
                        Ok(added) => {
                            // The contact exists; a name that cannot be
                            // kept leaves it unnamed, to be named again.
                            if let Some(name) = name.as_deref() {
                                let _ = messenger
                                    .set_contact_name(&identity, &added.contact, Some(name))
                                    .await;
                            }
                            Finished::Outcome(Outcome::ContactAdded {
                                contact: convert::contact(&identity, &added.contact),
                                invitation: added.invitation,
                            })
                        }
                        Err(error) => failed(error),
                    }
                })
            }
            Command::SetContactName { contact, name } => {
                let (identity, record) = convert::contact_ids(contact)?;
                Box::pin(async move {
                    done(
                        messenger
                            .set_contact_name(&identity, &record, name.as_deref())
                            .await,
                    )
                })
            }
            Command::SetVerified { contact, verified } => {
                let (identity, record) = convert::contact_ids(contact)?;
                Box::pin(async move {
                    done(
                        messenger
                            .set_contact_verified(&identity, &record, verified)
                            .await,
                    )
                })
            }
            Command::BlockContact(contact) => {
                let (identity, record) = convert::contact_ids(contact)?;
                Box::pin(async move { done(messenger.block_contact(&identity, &record).await) })
            }
            Command::RemoveContact(contact) => {
                let (identity, record) = convert::contact_ids(contact)?;
                Box::pin(async move { done(messenger.delete_contact(&identity, &record).await) })
            }
            Command::DeleteConversation(contact) => {
                let (identity, record) = convert::contact_ids(contact)?;
                Box::pin(async move {
                    done(
                        messenger
                            .delete_conversation(&identity, &record)
                            .await
                            .map(|_| ()),
                    )
                })
            }
            Command::Send { contact, text } => {
                let (identity, record) = convert::contact_ids(contact)?;
                Box::pin(async move {
                    Finished::Outcome(
                        match messenger.send_message(&identity, &record, &text).await {
                            Ok(reference) => Outcome::MessageQueued(convert::message(&reference)),
                            Err(error) => Outcome::Failed(convert::send_error(error)),
                        },
                    )
                })
            }
            Command::AcceptRequest { request, name } => {
                let request = self.request(request)?;
                Box::pin(async move {
                    match messenger.accept_request(&request).await {
                        Ok(record) => {
                            if let Some(name) = name.as_deref() {
                                let _ = messenger
                                    .set_contact_name(&request.identity, &record, Some(name))
                                    .await;
                            }
                            Finished::Outcome(Outcome::RequestAccepted(convert::contact(
                                &request.identity,
                                &record,
                            )))
                        }
                        Err(error) => failed(error),
                    }
                })
            }
            Command::DeclineRequest(request) => {
                let request = self.request(request)?;
                Box::pin(async move { done(messenger.decline_request(&request).await) })
            }
            Command::BlockRequest(request) => {
                let request = self.request(request)?;
                Box::pin(async move { done(messenger.block_request(&request).await) })
            }
            Command::CreateInvitation { identity, label } => {
                let identity = convert::identity_id(identity)?;
                Box::pin(async move {
                    Finished::Invitation(
                        messenger
                            .create_invitation(&identity, label.as_deref())
                            .await
                            .map(|(invitation, card)| (invitation, Zeroizing::new(card)))
                            .map_err(convert::command_error),
                    )
                })
            }
            Command::RevokeInvitation {
                invitation,
                discard_requests,
            } => {
                let invitation = self.invitation(invitation)?;
                Box::pin(async move {
                    Finished::Outcome(
                        match messenger
                            .revoke_invitation(&invitation, discard_requests)
                            .await
                        {
                            Ok(discarded) => Outcome::Revoked { discarded },
                            Err(error) => Outcome::Failed(convert::command_error(error)),
                        },
                    )
                })
            }
            Command::SetRequestMode { identity, mode } => {
                let identity = convert::identity_id(identity)?;
                let mode = convert::runtime_request_mode(mode);
                Box::pin(async move { done(messenger.set_request_mode(&identity, mode).await) })
            }
            // Queries and the profile commands are answered by `command`.
            _ => return Err(Failure::Unexpected),
        })
    }

    /// Everything about one identity, read at one time: no other command
    /// or event is handled between the reads.
    fn snapshot(&mut self, handle: UiIdentityId) -> Result<IdentitySnapshot, Failure> {
        let identity = convert::identity_id(handle)?;
        let messenger = Rc::clone(&self.messenger);
        let error = convert::command_error;
        let contacts = messenger
            .contacts(&identity)
            .map_err(error)?
            .into_iter()
            .map(|summary| ContactView {
                id: convert::contact(&identity, &summary.contact),
                name: summary.alias,
                fingerprint: summary.fingerprint,
                full_fingerprint: summary.full_fingerprint,
                accepted: summary.accepted,
                verified: summary.verified,
                invitation: summary.invitation,
            })
            .collect();
        let conversations = messenger
            .conversations(&identity)
            .map_err(error)?
            .into_iter()
            .map(|summary| ConversationView {
                contact: convert::contact(&identity, &summary.contact),
                fingerprint: summary.fingerprint,
                active: summary.active,
                messages: summary.messages,
                queued: summary.queued,
                latest: summary.latest.and_then(|sequence| {
                    messenger
                        .message(&MessageRef {
                            identity,
                            contact: summary.contact,
                            sequence,
                        })
                        .ok()
                        .flatten()
                        .map(convert::message_view)
                }),
            })
            .collect();
        let pending = messenger.requests(&identity).map_err(error)?;
        let active = messenger.invitations(&identity).map_err(error)?;
        let service = convert::service(messenger.service_state(&identity).map_err(error)?);
        let request_mode = convert::request_mode(messenger.request_mode(&identity).map_err(error)?);

        // A request is known while it waits; an invitation while it is
        // active or a request it admitted waits. The others are forgotten.
        let listed_requests: HashSet<RequestRef> =
            pending.iter().map(|summary| summary.request).collect();
        let listed_invitations: HashSet<InvitationRef> = active
            .iter()
            .map(|summary| summary.invitation)
            .chain(pending.iter().filter_map(|summary| summary.invitation))
            .collect();
        self.requests.keep(&identity, &listed_requests);
        self.invitations.keep(&identity, &listed_invitations);
        let requests = pending
            .into_iter()
            .map(|summary| RequestView {
                id: UiRequestId::new(handle, self.requests.handle(summary.request)),
                fingerprint: summary.fingerprint,
                full_fingerprint: summary.full_fingerprint,
                display_name: summary.display_name,
                introduction: summary.introduction,
                invitation: summary.invitation.map(|invitation| {
                    UiInvitationId::new(handle, self.invitations.handle(invitation))
                }),
            })
            .collect();
        let invitations = active
            .into_iter()
            .map(|summary| InvitationView {
                id: UiInvitationId::new(handle, self.invitations.handle(summary.invitation)),
                label: summary.label,
            })
            .collect();
        Ok(IdentitySnapshot {
            identity: handle,
            contacts,
            conversations,
            requests,
            invitations,
            service,
            request_mode,
        })
    }

    /// The `limit` messages before `before` (or the newest), read in pages
    /// of at most [`MAX_PAGE`], newest last.
    fn messages(&self, handle: UiContactId, before: Option<u64>, limit: usize) -> Outcome {
        let (identity, record) = match convert::contact_ids(handle) {
            Ok(ids) => ids,
            Err(failure) => return Outcome::Failed(failure),
        };
        let limit = limit.max(1);
        let mut window: Vec<api::MessageView> = Vec::new();
        let mut before = before;
        let complete = loop {
            // One more than wanted, to know whether there are older ones.
            let ask = limit
                .saturating_sub(window.len())
                .saturating_add(1)
                .min(MAX_PAGE);
            let mut page = match self.messenger.messages(&identity, &record, before, ask) {
                Ok(page) => page,
                Err(error) => return Outcome::Failed(convert::command_error(error)),
            };
            let got = page.len();
            before = page.first().map(|view| view.reference.sequence);
            page.append(&mut window);
            window = page;
            if window.len() > limit {
                window.drain(..window.len().saturating_sub(limit));
                break false;
            }
            if got < ask || before.is_none() {
                break true;
            }
        };
        Outcome::Messages {
            contact: handle,
            messages: window.into_iter().map(convert::message_view).collect(),
            complete,
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    struct Fake(IdentityId, u8);

    impl Owned for Fake {
        fn owner(&self) -> IdentityId {
            self.0
        }
    }

    fn id(byte: u8) -> IdentityId {
        IdentityId::from_bytes([byte; 16]).unwrap()
    }

    #[test]
    fn a_task_that_panicked_is_answered_for_its_command() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let ended = LocalSet::new().block_on(&runtime, async {
            let mut tasks = Tasks::default();
            tasks.spawn(CommandId::new(7), async {
                if true {
                    panic!("a task that panics");
                }
                Finished::Outcome(Outcome::Done)
            });
            tasks.spawn(CommandId::new(8), async {
                Finished::Outcome(Outcome::Done)
            });
            let mut ended = Vec::new();
            while let Some((command, finished)) = tasks.next().await {
                ended.push((
                    command,
                    matches!(finished, Some(Finished::Outcome(Outcome::Done))),
                ));
            }
            ended.sort_by_key(|(command, _)| *command);
            ended
        });
        assert_eq!(
            ended,
            [(CommandId::new(7), false), (CommandId::new(8), true)]
        );
    }

    #[test]
    fn a_number_is_given_once_and_kept_for_its_reference() {
        let mut handles = Handles::default();
        let first = handles.handle(Fake(id(1), 1));
        let second = handles.handle(Fake(id(1), 2));
        assert_ne!(first, second);
        assert_eq!(handles.handle(Fake(id(1), 1)), first);
    }

    #[test]
    fn a_number_names_nothing_for_another_identity() {
        let mut handles = Handles::default();
        let number = handles.handle(Fake(id(1), 1));
        assert_eq!(
            handles.get(convert::identity(&id(1)), number),
            Some(Fake(id(1), 1))
        );
        assert_eq!(handles.get(convert::identity(&id(2)), number), None);
    }

    #[test]
    fn a_forgotten_reference_is_never_given_its_number_again() {
        let mut handles = Handles::default();
        let gone = handles.handle(Fake(id(1), 1));
        let other = handles.handle(Fake(id(2), 1));
        handles.keep(&id(1), &HashSet::new());
        assert_eq!(handles.get(convert::identity(&id(1)), gone), None);
        // Another identity's references are not touched.
        assert_eq!(
            handles.get(convert::identity(&id(2)), other),
            Some(Fake(id(2), 1))
        );
        let again = handles.handle(Fake(id(1), 1));
        assert_ne!(again, gone);
        assert_ne!(again, other);
        // Nor anyone else's.
        assert_eq!(
            handles.get(convert::identity(&id(2)), other),
            Some(Fake(id(2), 1))
        );
    }
}

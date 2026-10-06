//! `dev-node`: a development node over a persistent vault, for the private
//! Tor network test.
//!
//! The node opens the vault of a data directory, or creates it, publishes
//! the Onion Service of every local identity through its supervisor,
//! answers every stream with that identity's contact store, and keeps the
//! sessions open. It reads commands from standard input, one per line,
//! and writes what happens to standard output, one event per line, so that
//! a script can drive two nodes and read what they did. Every command
//! names the local identity it acts for by its number, from 0, in the
//! order of the vault.
//!
//! It is a test driver built on the core, not the product's interface:
//! the decisions are all the store's, the link's and the supervisor's.
//! Peer text is printed escaped. No command prints a key.
//!
//! Commands:
//!
//! ```text
//! new-identity [label]          publish a new service, make an identity
//! card <n>                      the card the identity answers with
//! invite <n> [label]            a card with a new capability
//! revoke <n> <id>               revoke a capability
//! revoke-discard <n> <id>       revoke it and discard its requests
//! mode <n> invitation|open|closed
//! add <n> <card>                import a card
//! confirm <n> <card>            confirm a pending successor
//! confirm-dial <n> <card>       confirm the active card for dialing
//! block|unblock|delete <n> <peer>
//! accept|decline <n> <peer>     answer the request of <peer> that `requests` listed
//! show <n> <peer>               what is held about a peer
//! requests <n>                  the pending requests
//! send <n> <peer> <text>        send a chat message on the newest session,
//!                               dialing if none is open
//! dial <n> <peer> [text]        open a new session, and send the text
//! close <n> <peer>              close the newest session with a peer
//! rotate <n> begin|switch|finish|force-switch|force-finish
//! quit                          remove the services and stop
//! ```
//!
//! `<peer>` is the start of the hexadecimal identity key of a known peer,
//! as the events print it.

use std::collections::HashMap;
use std::io::BufRead;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use monolith_core::budget::Budgets;
use monolith_core::contacts::{ImportOutcome, StoreError};
use monolith_core::identity::{Installation, LocalIdentity, RotationState};
use monolith_core::link::{Established, LinkError, answer, dial};
use monolith_core::requests::RequestId;
use monolith_core::supervisor::{Publication, supervise};
use monolith_identity::IdentityPublicKey;
use monolith_protocol::body::{ContactRequest, Message, MessageId};
use monolith_protocol::card::ContactCard;
use monolith_protocol::contact::RequestMode;
use monolith_protocol::session::Action;
use monolith_protocol::text::{ChatText, DisplayName, IntroductionText};
use monolith_storage::dir::DiskDir;
use monolith_storage::vault::{KdfParams, Passphrase};
use monolith_tor::{
    KeySource, OnionService, SystemTorBackend, SystemTorConfig, TorBackend, TorError, TorStream,
};
use tokio::sync::{mpsc, watch};
use zeroize::Zeroizing;

/// How often, and how far apart, a dial is tried while Tor cannot reach
/// the peer's service yet: its descriptor may still be on its way.
const DIAL_ATTEMPTS: u32 = 30;
const DIAL_PAUSE: Duration = Duration::from_secs(10);

/// Commands waiting for a session.
const SESSION_QUEUE: usize = 16;

/// Lines waiting for the command loop.
const COMMAND_QUEUE: usize = 64;

/// What a session task is told to do.
enum Outgoing {
    Chat(String),
    Close,
}

/// The state shared by the tasks of the node.
struct Node {
    installation: Installation,
    backend: SystemTorBackend,
    budgets: Budgets,
    /// The open sessions of each local identity with each peer, newest
    /// last; a contact may have two while one of the keys changes.
    sessions: Mutex<HashMap<(IdentityPublicKey, IdentityPublicKey), Vec<OpenSession>>>,
    /// The requests each local identity listed last: an answer is for one
    /// of them, the request that was shown, and not for another of the
    /// same sender that came later.
    listed: Mutex<HashMap<IdentityPublicKey, Vec<RequestId>>>,
    serial: AtomicU64,
    shutdown: watch::Sender<bool>,
    supervisors: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    echo: bool,
}

struct OpenSession {
    serial: u64,
    sender: mpsc::Sender<Outgoing>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The start of the hexadecimal identity key.
fn short(identity: &IdentityPublicKey) -> String {
    identity
        .as_bytes()
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn say(line: &str) {
    println!("{line}");
}

impl Node {
    fn identity(&self, number: &str) -> Result<(usize, Arc<LocalIdentity>), String> {
        let index: usize = number.parse().map_err(|_| "no such identity".to_owned())?;
        let identities = self.installation.identities();
        identities
            .get(index)
            .cloned()
            .map(|identity| (index, identity))
            .ok_or_else(|| "no such identity".to_owned())
    }

    /// The peer whose short form starts with `prefix`, among the
    /// identities `local` holds a record of and the senders of its pending
    /// requests. A prefix two of them share names neither.
    fn peer(&self, local: &LocalIdentity, prefix: &str) -> Result<IdentityPublicKey, String> {
        let mut candidates = local.known();
        for request in local.requests() {
            if !candidates.contains(request.identity()) {
                candidates.push(*request.identity());
            }
        }
        let mut found = candidates
            .into_iter()
            .filter(|known| short(known).starts_with(prefix));
        let peer = found.next().ok_or_else(|| "no such peer".to_owned())?;
        if found.next().is_some() {
            return Err("ambiguous peer".to_owned());
        }
        Ok(peer)
    }

    /// Starts the supervisor of the identity number `index`.
    fn supervise(self: &Arc<Self>, index: usize, local: Arc<LocalIdentity>) {
        let node = self.clone();
        let shutdown = self.shutdown.subscribe();
        let task = tokio::spawn(async move {
            let (state, mut states) = watch::channel(Publication::Publishing);
            let watcher = tokio::spawn(async move {
                loop {
                    if states.changed().await.is_err() {
                        return;
                    }
                    let current = *states.borrow_and_update();
                    match current {
                        Publication::Available => say(&format!("published {index}")),
                        Publication::Unavailable(error) => {
                            say(&format!("unavailable {index} {error}"));
                        }
                        Publication::NoKey => say(&format!("no-key {index}")),
                        Publication::Publishing | Publication::Stopped => {}
                    }
                }
            });
            let answering = node.clone();
            let identity = local.clone();
            supervise(
                &node.backend,
                &node.budgets,
                &local,
                shutdown,
                &state,
                move |stream, permit| {
                    let node = answering.clone();
                    let local = identity.clone();
                    async move {
                        let answered = answer(stream, &node.budgets, &local).await;
                        drop(permit);
                        match answered {
                            Ok(established) => {
                                let peer = *established.link.session().peer();
                                say(&format!(
                                    "inbound {index} {} {:?}",
                                    short(&peer),
                                    established.admission.standing
                                ));
                                node.run(index, local, established, Vec::new()).await;
                            }
                            Err(error) => say(&format!("inbound-failed {index} {error}")),
                        }
                    }
                },
            )
            .await;
            drop(state);
            let _ = watcher.await;
        });
        lock(&self.supervisors).push(task);
    }

    /// The sender of the newest open session of `local` with `peer`.
    fn newest_session(
        &self,
        local: &LocalIdentity,
        peer: &IdentityPublicKey,
    ) -> Option<mpsc::Sender<Outgoing>> {
        lock(&self.sessions)
            .get(&(*local.identity(), *peer))
            .and_then(|open| open.last())
            .map(|session| session.sender.clone())
    }

    /// Runs one session until it ends, applying what arrives to the store
    /// of `local` and sending `queued` once it is confirmed.
    async fn run(
        self: &Arc<Self>,
        index: usize,
        local: Arc<LocalIdentity>,
        established: Established<TorStream>,
        mut queued: Vec<String>,
    ) {
        let peer = *established.link.session().peer();
        let (sender, outgoing) = mpsc::channel(SESSION_QUEUE);
        let key = (*local.identity(), peer);
        let serial = self.serial.fetch_add(1, Ordering::Relaxed);
        // Only a session that stands for the contact is the peer's for
        // what the user sends or closes; one of a stale or pending key, or
        // of a stranger, is not.
        if established.link.session().standing().is_contact_record() {
            lock(&self.sessions)
                .entry(key)
                .or_default()
                .push(OpenSession { serial, sender });
        }
        let ended = self
            .session(index, &local, established, outgoing, &mut queued)
            .await;
        {
            let mut sessions = lock(&self.sessions);
            if let Some(open) = sessions.get_mut(&key) {
                open.retain(|session| session.serial != serial);
                if open.is_empty() {
                    sessions.remove(&key);
                }
            }
        }
        say(&format!("ended {index} {} {ended}", short(&peer)));
    }

    async fn session(
        &self,
        index: usize,
        local: &LocalIdentity,
        established: Established<TorStream>,
        mut outgoing: mpsc::Receiver<Outgoing>,
        queued: &mut Vec<String>,
    ) -> String {
        let Established {
            link: mut owned,
            first,
            ..
        } = established;
        let peer = *owned.session().peer();
        let link = &mut owned;
        for action in &first {
            let sent = match action {
                Action::SendContactAccept => link.send(&Message::ContactAccept).await,
                Action::SendContactRequest => {
                    // The card and the capability of this session, which
                    // it sends; not those of the moment, which can differ.
                    let card = link.session().local_card().clone();
                    let invitation = link.session().invitation().cloned();
                    let Ok(request) = request(card, invitation) else {
                        return "invalid request".to_owned();
                    };
                    link.send(&request).await
                }
                _ => Ok(()),
            };
            if let Err(error) = sent {
                return error.to_string();
            }
        }
        let mut confirmed = false;
        loop {
            let next = {
                let mut command = core::pin::pin!(outgoing.recv());
                let mut receive = core::pin::pin!(link.receive());
                core::future::poll_fn(|cx| {
                    if let core::task::Poll::Ready(command) = command.as_mut().poll(cx) {
                        return core::task::Poll::Ready(Err(command));
                    }
                    receive.as_mut().poll(cx).map(Ok)
                })
                .await
            };
            let received = match next {
                Err(Some(Outgoing::Chat(text))) => {
                    if confirmed {
                        if let Err(error) = send_chat(link, &text).await {
                            return error.to_string();
                        }
                        say(&format!("sent {index} {}", short(&peer)));
                    } else {
                        queued.push(text);
                    }
                    continue;
                }
                Err(Some(Outgoing::Close) | None) => {
                    let _ = owned.close().await;
                    return "closed".to_owned();
                }
                Ok(Err(error)) => return reason(error),
                Ok(Ok(received)) => received,
            };
            match local.apply(link.session_ref(), &received).await {
                Ok(applied) => {
                    if applied.accepted {
                        say(&format!("accepted {index} {}", short(&peer)));
                    }
                    if let Some(change) = applied.announced {
                        say(&format!(
                            "endpoint-update {index} {} {change:?}",
                            short(&peer)
                        ));
                    }
                    if let Some(decided) = applied.request {
                        let outcome = match decided {
                            Ok(()) => "queued".to_owned(),
                            Err(dropped) => format!("dropped:{dropped:?}"),
                        };
                        say(&format!("request {index} {} {outcome}", short(&peer)));
                    }
                    if applied.promoted_successor {
                        say(&format!("promoted-successor {index} {}", short(&peer)));
                    }
                }
                Err(error) => say(&format!("apply-failed {index} {} {error}", short(&peer))),
            }
            if received.actions.contains(&Action::SendContactAccept) {
                if let Err(error) = link.send(&Message::ContactAccept).await {
                    return error.to_string();
                }
            }
            if received.actions.contains(&Action::Confirmed) {
                confirmed = true;
                let key = if Some(link.session().local_card()) == local.successor_card().as_ref() {
                    "new"
                } else {
                    "old"
                };
                say(&format!("confirmed {index} {} key={key}", short(&peer)));
                if let Some(announcement) = local.announcement_for(link.session_ref()) {
                    let update = Message::EndpointUpdate(Box::new(announcement.card().clone()));
                    if let Err(error) = link.send(&update).await {
                        return error.to_string();
                    }
                    if local
                        .mark_announced(&announcement)
                        .await
                        .is_ok_and(|recorded| recorded)
                    {
                        say(&format!("announced {index} {}", short(&peer)));
                    }
                }
                for text in core::mem::take(queued) {
                    if let Err(error) = send_chat(link, &text).await {
                        return error.to_string();
                    }
                    say(&format!("sent {index} {}", short(&peer)));
                }
            }
            if let Message::ChatMessage { text, .. } = &received.message {
                say(&format!(
                    "message {index} {} {:?}",
                    short(&peer),
                    text.as_str()
                ));
                if self.echo && text.as_str() == "hello" {
                    if let Err(error) = send_chat(link, "hello back").await {
                        return error.to_string();
                    }
                }
            }
        }
    }

    /// Dials `peer` for `local` by its dial plan, and runs the session.
    async fn open(
        self: &Arc<Self>,
        index: usize,
        local: Arc<LocalIdentity>,
        peer: IdentityPublicKey,
        queued: Vec<String>,
    ) {
        let Some(plan) = local.dial_plan(&peer) else {
            say(&format!(
                "dial-failed {index} {} not a contact",
                short(&peer)
            ));
            return;
        };
        if plan.cards.is_empty() {
            say(&format!(
                "dial-failed {index} {} confirm where to connect first",
                short(&peer)
            ));
            return;
        }
        let mut last = String::new();
        for attempt in 1..=DIAL_ATTEMPTS {
            for card in &plan.cards {
                match dial(&self.backend, &self.budgets, &local, card).await {
                    Ok(established) => {
                        say(&format!(
                            "outbound {index} {} {:?}",
                            short(&peer),
                            established.admission.standing
                        ));
                        self.run(index, local, established, queued).await;
                        return;
                    }
                    // A service Tor cannot reach yet, or a dial that timed
                    // out on the way, in Tor or in the handshake: tried
                    // again, through Tor only.
                    Err(
                        error @ (LinkError::Tor(TorError::OnionUnreachable | TorError::TimedOut)
                        | LinkError::TimedOut),
                    ) => {
                        last = error.to_string();
                    }
                    Err(error) => {
                        say(&format!("dial-failed {index} {} {error}", short(&peer)));
                        return;
                    }
                }
            }
            say(&format!(
                "dial-retry {index} {} {attempt} {last}",
                short(&peer)
            ));
            tokio::time::sleep(DIAL_PAUSE).await;
        }
        say(&format!("dial-failed {index} {} {last}", short(&peer)));
    }
}

fn request(
    card: ContactCard,
    invitation: Option<monolith_protocol::card::InvitationCapability>,
) -> Result<Message, ()> {
    Ok(Message::ContactRequest(Box::new(ContactRequest {
        card,
        invitation,
        display_name: DisplayName::new("").map_err(|_| ())?,
        introduction: IntroductionText::new("").map_err(|_| ())?,
    })))
}

async fn send_chat(
    link: &mut monolith_core::link::Link<TorStream>,
    text: &str,
) -> Result<(), LinkError> {
    let mut id = [0_u8; 16];
    getrandom::fill(&mut id).map_err(|_| LinkError::Stream)?;
    let text = ChatText::new(text).map_err(|_| LinkError::Stream)?;
    link.send(&Message::ChatMessage {
        id: MessageId::from_bytes(id),
        text,
    })
    .await
}

fn reason(error: LinkError) -> String {
    match error {
        LinkError::Withdrawn => "withdrawn".to_owned(),
        LinkError::Evicted => "evicted".to_owned(),
        LinkError::TimedOut => "timed-out".to_owned(),
        LinkError::Stream => "stream".to_owned(),
        other => other.to_string(),
    }
}

/// Reads the lines of standard input on a thread of their own and hands
/// them to the command loop. Ends at the end of the input.
fn read_commands() -> mpsc::Receiver<String> {
    let (lines, commands) = mpsc::channel(COMMAND_QUEUE);
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if lines.blocking_send(line).is_err() {
                break;
            }
        }
    });
    commands
}

/// The node: opens or creates the vault in `data_dir`, publishes every
/// identity, and runs commands until `quit` or the end of the input.
pub(crate) async fn run(
    config: SystemTorConfig,
    data_dir: PathBuf,
    passphrase_file: PathBuf,
    floor: bool,
    echo: bool,
) -> ExitCode {
    let passphrase = match std::fs::read_to_string(&passphrase_file).map(Zeroizing::new) {
        Ok(text) => match Passphrase::new(text.trim_end_matches(['\r', '\n'])) {
            Ok(passphrase) => passphrase,
            Err(error) => return fail(&error.to_string()),
        },
        Err(_) => return fail("cannot read the passphrase file"),
    };
    let created = !data_dir.join(monolith_storage::dir::VAULT).exists();
    let dir = match DiskDir::open(&data_dir, created) {
        Ok(dir) => dir,
        Err(error) => return fail(&error.to_string()),
    };
    let installation = if created {
        let params = if floor {
            KdfParams::FLOOR
        } else {
            KdfParams::DEFAULT
        };
        Installation::create(Box::new(dir), &passphrase, params).inspect(|_| say("created"))
    } else {
        Installation::open(Box::new(dir), &passphrase).map(|(installation, recovery)| {
            say(&format!("opened {recovery:?}"));
            installation
        })
    };
    let installation = match installation {
        Ok(installation) => installation,
        Err(error) => return fail(&error.to_string()),
    };
    drop(passphrase);
    let (shutdown, _) = watch::channel(false);
    let node = Arc::new(Node {
        installation,
        backend: SystemTorBackend::new(config),
        budgets: Budgets::new(),
        sessions: Mutex::new(HashMap::new()),
        listed: Mutex::new(HashMap::new()),
        serial: AtomicU64::new(0),
        shutdown,
        supervisors: Mutex::new(Vec::new()),
        echo,
    });
    for (index, local) in node.installation.identities().into_iter().enumerate() {
        say(&format!("identity {index} {}", short(local.identity())));
        node.supervise(index, local);
    }
    say("ready");
    let mut commands = read_commands();
    while let Some(line) = commands.recv().await {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.first() == Some(&"quit") {
            break;
        }
        if let Err(error) = node.command(&words).await {
            say(&format!("error {error}"));
        }
    }
    node.shutdown.send_replace(true);
    let supervisors: Vec<_> = core::mem::take(&mut *lock(&node.supervisors));
    for supervisor in supervisors {
        let _ = supervisor.await;
    }
    let sessions: Vec<_> = lock(&node.sessions)
        .values()
        .flatten()
        .map(|session| session.sender.clone())
        .collect();
    for session in sessions {
        let _ = session.send(Outgoing::Close).await;
    }
    say("bye");
    ExitCode::SUCCESS
}

fn fail(what: &str) -> ExitCode {
    eprintln!("monolith dev-node: {what}");
    ExitCode::FAILURE
}

fn outcome(outcome: ImportOutcome) -> String {
    match outcome {
        ImportOutcome::Created => "created".to_owned(),
        ImportOutcome::Evaluated { change, .. } => format!("{change:?}"),
    }
}

impl Node {
    async fn command(self: &Arc<Self>, words: &[&str]) -> Result<(), String> {
        let text = |error: &dyn core::fmt::Display| error.to_string();
        match words {
            ["new-identity", label @ ..] => {
                let label = if label.is_empty() {
                    None
                } else {
                    Some(DisplayName::new(&label.join(" ")).map_err(|error| text(&error))?)
                };
                let mut service = self
                    .backend
                    .publish_onion(KeySource::Generate)
                    .await
                    .map_err(|error| text(&error))?;
                let secret = service
                    .take_generated_secret()
                    .ok_or_else(|| "Tor returned no key".to_owned())?;
                let endpoint = *service.service_key();
                // The supervisor publishes it again from the stored key.
                let _ = service.close().await;
                let local = self
                    .installation
                    .create_identity(endpoint, Some(&secret), label)
                    .await
                    .map_err(|error| text(&error))?;
                let index = self.installation.identities().len().saturating_sub(1);
                say(&format!("identity {index} {}", short(local.identity())));
                self.supervise(index, local);
                Ok(())
            }
            ["card", n] => {
                let (index, local) = self.identity(n)?;
                say(&format!("card {index} {}", local.card().to_text()));
                Ok(())
            }
            ["invite", n, label @ ..] => {
                let (index, local) = self.identity(n)?;
                let label = if label.is_empty() {
                    None
                } else {
                    Some(DisplayName::new(&label.join(" ")).map_err(|error| text(&error))?)
                };
                let (id, card) = local
                    .create_invitation(label)
                    .await
                    .map_err(|error| text(&error))?;
                say(&format!("invitation {index} {id:?} {}", card.to_text()));
                Ok(())
            }
            ["revoke" | "revoke-discard", n, id] => {
                let (index, local) = self.identity(n)?;
                let id = local
                    .invitations()
                    .into_iter()
                    .map(|(id, _)| id)
                    .find(|held| format!("{held:?}") == *id)
                    .ok_or_else(|| "no such invitation".to_owned())?;
                if words.first() == Some(&"revoke") {
                    local
                        .revoke_invitation(id)
                        .await
                        .map_err(|error| text(&error))?;
                    say(&format!("revoked {index}"));
                } else {
                    let discarded = local
                        .revoke_and_discard(id)
                        .await
                        .map_err(|error| text(&error))?;
                    say(&format!("revoked {index} discarded={discarded}"));
                }
                Ok(())
            }
            ["mode", n, mode] => {
                let (index, local) = self.identity(n)?;
                let mode = match *mode {
                    "invitation" => RequestMode::Invitation,
                    "open" => RequestMode::Open,
                    "closed" => RequestMode::Closed,
                    _ => return Err("mode is invitation, open or closed".to_owned()),
                };
                local
                    .set_request_mode(mode)
                    .await
                    .map_err(|error| text(&error))?;
                say(&format!("mode {index} {mode:?}"));
                Ok(())
            }
            ["add" | "confirm" | "confirm-dial", n, card] => {
                let (index, local) = self.identity(n)?;
                let card = ContactCard::from_text(card).map_err(|error| text(&error))?;
                let peer = short(card.identity());
                match words.first() {
                    Some(&"add") => {
                        let result = local.import(&card).await.map_err(|error| text(&error))?;
                        say(&format!("added {index} {peer} {}", outcome(result)));
                    }
                    Some(&"confirm") => {
                        local
                            .confirm_pending(&card)
                            .await
                            .map_err(|error| text(&error))?;
                        say(&format!("confirmed-key {index} {peer}"));
                    }
                    _ => {
                        local
                            .confirm_dial(&card)
                            .await
                            .map_err(|error| text(&error))?;
                        say(&format!("confirmed-dial {index} {peer}"));
                    }
                }
                Ok(())
            }
            [
                verb @ ("block" | "unblock" | "delete" | "accept" | "decline"),
                n,
                peer,
            ] => {
                let (index, local) = self.identity(n)?;
                let peer = self.peer(&local, peer)?;
                let done = match *verb {
                    "block" => local.block(&peer).await,
                    "unblock" => local.unblock(&peer).await,
                    "delete" => local.delete(&peer).await,
                    "accept" | "decline" => {
                        // The request of that sender that `requests` showed
                        // last, if it still waits: an answer is for what
                        // the user saw.
                        let shown = lock(&self.listed)
                            .get(local.identity())
                            .cloned()
                            .unwrap_or_default();
                        let request = local
                            .requests()
                            .into_iter()
                            .find(|request| {
                                request.identity() == &peer && shown.contains(&request.id)
                            })
                            .map(|request| request.id);
                        match (request, *verb) {
                            (None, _) => Err(StoreError::NotFound),
                            (Some(request), "accept") => local.accept_request(request).await,
                            (Some(request), _) => local.decline_request(request).await,
                        }
                    }
                    _ => Err(StoreError::NotFound),
                };
                done.map_err(|error| text(&error))?;
                let past = match *verb {
                    "block" => "blocked",
                    "unblock" => "unblocked",
                    "delete" => "deleted",
                    "accept" => "accepted",
                    _ => "declined",
                };
                say(&format!("{past} {index} {}", short(&peer)));
                Ok(())
            }
            ["show", n, peer] => {
                let (index, local) = self.identity(n)?;
                let peer = self.peer(&local, peer)?;
                let view = local.contact(&peer).ok_or_else(|| "no record".to_owned())?;
                let (active, authorized, pending, retired) = match &view.credentials {
                    Some(held) => (
                        held.active().epoch().get(),
                        held.authorized_successor()
                            .map_or(0, |card| card.epoch().get()),
                        held.pending_successor()
                            .map_or(0, |card| card.epoch().get()),
                        held.retired().is_some(),
                    ),
                    None => (0, 0, 0, false),
                };
                say(&format!(
                    "contact {index} {} kind={:?} active={active} authorized={authorized} \
                     pending={pending} retired={retired} verified={} sessions={}",
                    short(&peer),
                    view.kind,
                    view.verified,
                    view.sessions
                ));
                Ok(())
            }
            ["requests", n] => {
                let (index, local) = self.identity(n)?;
                let requests = local.requests();
                lock(&self.listed).insert(
                    *local.identity(),
                    requests.iter().map(|request| request.id).collect(),
                );
                for request in &requests {
                    say(&format!(
                        "request-pending {index} {}",
                        short(request.identity())
                    ));
                }
                say(&format!("requests {index} {}", requests.len()));
                Ok(())
            }
            ["send", n, peer, words @ ..] => {
                let (index, local) = self.identity(n)?;
                let peer = self.peer(&local, peer)?;
                let message = words.join(" ");
                let session = self.newest_session(&local, &peer);
                if let Some(session) = session {
                    if session.send(Outgoing::Chat(message.clone())).await.is_ok() {
                        return Ok(());
                    }
                }
                let node = self.clone();
                tokio::spawn(async move { node.open(index, local, peer, vec![message]).await });
                Ok(())
            }
            ["dial", n, peer, words @ ..] => {
                // A new session, whether or not one is open: what dials a
                // contact with a new local key while the old session lasts.
                let (index, local) = self.identity(n)?;
                let peer = self.peer(&local, peer)?;
                let queued = if words.is_empty() {
                    Vec::new()
                } else {
                    vec![words.join(" ")]
                };
                let node = self.clone();
                tokio::spawn(async move { node.open(index, local, peer, queued).await });
                Ok(())
            }
            ["close", n, peer] => {
                let (_, local) = self.identity(n)?;
                let peer = self.peer(&local, peer)?;
                let session = self.newest_session(&local, &peer);
                match session {
                    Some(session) => {
                        let _ = session.send(Outgoing::Close).await;
                        Ok(())
                    }
                    None => Err("no session".to_owned()),
                }
            }
            ["rotate", n, step] => {
                let (index, local) = self.identity(n)?;
                let done = match *step {
                    "begin" => local.begin_rotation().await.map(|_| true),
                    "switch" | "force-switch" | "finish" | "force-finish" => {
                        // The rotation in progress as the command runs.
                        match local.rotation_id() {
                            None => Err(StoreError::NotFound),
                            Some(rotation) => match *step {
                                "switch" => local.switch_rotation(rotation, false).await,
                                "force-switch" => local.switch_rotation(rotation, true).await,
                                "finish" => local.finish_rotation(rotation, false).await,
                                _ => local.finish_rotation(rotation, true).await,
                            },
                        }
                    }
                    _ => return Err("rotate begin, switch or finish".to_owned()),
                }
                .map_err(|error| text(&error))?;
                let state = match local.rotation() {
                    RotationState::None => "none",
                    RotationState::Announcing => "announcing",
                    RotationState::Switched => "switched",
                };
                say(&format!(
                    "rotation {index} {step} done={done} state={state}"
                ));
                Ok(())
            }
            [] => Ok(()),
            _ => Err("unknown command".to_owned()),
        }
    }
}

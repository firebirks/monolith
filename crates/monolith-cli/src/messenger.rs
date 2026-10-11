//! `dev-runtime`: a development node built on the messaging runtime, for
//! the private Tor network test of Phase 5.
//!
//! The node opens the vault of a data directory, or creates it, and starts
//! the runtime on it: the runtime publishes every identity, owns every
//! session, delivers the queues and takes messages in. The node reads
//! commands from standard input, one per line, and writes the events of
//! the runtime to standard output, one per line. Every command names the
//! local identity it acts for by its number, from 0, in the order of the
//! vault and then of creation.
//!
//! It is a test driver, not the product's interface: every decision is the
//! runtime's. Peer text is printed escaped. No command prints a key.
//!
//! Commands:
//!
//! ```text
//! new-identity [label]          publish a new service, make an identity
//! card <n>                      the card of the identity
//! add <n> <card>                import a card
//! send <n> <peer> <text>        queue a message
//! messages <n> <peer>           the messages of a conversation
//! conversations <n>             the conversations of the identity
//! block|delete <n> <peer>       block or delete a contact
//! sessions <n> <peer>           the sessions of a contact
//! quit                          stop
//! ```
//!
//! `<peer>` is the start of the hexadecimal identity key of a peer the
//! identity holds a conversation or a contact record of.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use monolith_core::api::{ContactId, Event, IdentityId, ServiceState};
use monolith_core::budget::Budgets;
use monolith_core::identity::Installation;
use monolith_core::runtime::{Runtime, RuntimeOptions};
use monolith_identity::IdentityPublicKey;
use monolith_storage::dir::DiskDir;
use monolith_storage::vault::{KdfParams, Passphrase};
use monolith_tor::{SystemTorBackend, SystemTorConfig};
use tokio::sync::{broadcast, mpsc};
use zeroize::Zeroizing;

/// Lines waiting for the command loop.
const COMMAND_QUEUE: usize = 64;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn say(line: &str) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// The first eight bytes of an identity key, in hexadecimal.
fn short(identity: &IdentityPublicKey) -> String {
    identity
        .as_bytes()
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct Node {
    runtime: Runtime<SystemTorBackend>,
    /// The identities, by their number.
    identities: Mutex<Vec<IdentityId>>,
}

impl Node {
    fn number(&self, identity: &IdentityId) -> Option<usize> {
        lock(&self.identities)
            .iter()
            .position(|held| held == identity)
    }

    fn identity(&self, number: &str) -> Result<(usize, IdentityId), String> {
        let index: usize = number.parse().map_err(|_| "not a number".to_owned())?;
        lock(&self.identities)
            .get(index)
            .copied()
            .map(|identity| (index, identity))
            .ok_or_else(|| "no such identity".to_owned())
    }

    /// The remote identity of the record `contact` of `identity`, current
    /// or ended.
    fn peer_of(&self, identity: &IdentityId, contact: &ContactId) -> Option<IdentityPublicKey> {
        let local = self.runtime.local_identity(identity)?;
        local
            .contact_records()
            .into_iter()
            .find(|(record, _)| record == contact)
            .map(|(_, peer)| peer)
            .or_else(|| {
                local
                    .conversations()
                    .into_iter()
                    .find(|view| view.contact == *contact)
                    .map(|view| view.peer)
            })
    }

    fn peer_name(&self, identity: &IdentityId, contact: &ContactId) -> String {
        self.peer_of(identity, contact)
            .map_or_else(|| "?".to_owned(), |peer| short(&peer))
    }

    /// The record of the peer whose key starts with `prefix`: its current
    /// contact record, or the newest conversation of an ended one.
    fn record(&self, identity: &IdentityId, prefix: &str) -> Result<ContactId, String> {
        let local = self
            .runtime
            .local_identity(identity)
            .ok_or_else(|| "no such identity".to_owned())?;
        let mut found: Vec<(ContactId, IdentityPublicKey)> = local
            .contact_records()
            .into_iter()
            .filter(|(_, peer)| short(peer).starts_with(prefix))
            .collect();
        if found.is_empty() {
            found = local
                .conversations()
                .into_iter()
                .filter(|view| short(&view.peer).starts_with(prefix))
                .map(|view| (view.contact, view.peer))
                .collect();
        }
        match found.as_slice() {
            [(record, _)] => Ok(*record),
            [] => Err("no such peer".to_owned()),
            _ => Err("ambiguous peer".to_owned()),
        }
    }

    fn report(&self, event: Event) {
        let line = match event {
            Event::Service { identity, state } => {
                let Some(n) = self.number(&identity) else {
                    return;
                };
                match state {
                    ServiceState::Available => format!("published {n}"),
                    ServiceState::Unavailable => format!("unavailable {n}"),
                    ServiceState::NoKey => format!("no-key {n}"),
                    ServiceState::Publishing | ServiceState::Stopped => return,
                }
            }
            Event::MessageQueued(reference) => format!(
                "queued {} {} {}",
                self.number(&reference.identity).unwrap_or(usize::MAX),
                self.peer_name(&reference.identity, &reference.contact),
                reference.sequence
            ),
            Event::MessageReceived(reference) => {
                let text = self
                    .runtime
                    .message(&reference)
                    .ok()
                    .flatten()
                    .map(|message| message.text)
                    .unwrap_or_default();
                format!(
                    "received {} {} {} {text:?}",
                    self.number(&reference.identity).unwrap_or(usize::MAX),
                    self.peer_name(&reference.identity, &reference.contact),
                    reference.sequence
                )
            }
            Event::MessageDelivered(reference) => format!(
                "delivered {} {} {}",
                self.number(&reference.identity).unwrap_or(usize::MAX),
                self.peer_name(&reference.identity, &reference.contact),
                reference.sequence
            ),
            Event::MessageRefused {
                identity,
                contact,
                reason,
            } => format!(
                "refused {} {} {reason:?}",
                self.number(&identity).unwrap_or(usize::MAX),
                self.peer_name(&identity, &contact)
            ),
            Event::DeliveryDeferred {
                identity,
                contact,
                reason,
                retry_in,
            } => format!(
                "deferred {} {} {reason:?} {}",
                self.number(&identity).unwrap_or(usize::MAX),
                self.peer_name(&identity, &contact),
                retry_in.as_secs()
            ),
            Event::SessionAvailable { identity, contact } => format!(
                "session {} {}",
                self.number(&identity).unwrap_or(usize::MAX),
                self.peer_name(&identity, &contact)
            ),
            Event::SessionUnavailable { identity, contact } => format!(
                "no-session {} {}",
                self.number(&identity).unwrap_or(usize::MAX),
                self.peer_name(&identity, &contact)
            ),
            Event::ContactAccepted { identity, contact } => format!(
                "accepted-by {} {}",
                self.number(&identity).unwrap_or(usize::MAX),
                self.peer_name(&identity, &contact)
            ),
            Event::RequestReceived { identity, queued } => format!(
                "request {} {}",
                self.number(&identity).unwrap_or(usize::MAX),
                if queued { "queued" } else { "dropped" }
            ),
            Event::IdentityClosed { identity } => {
                format!("closed {}", self.number(&identity).unwrap_or(usize::MAX))
            }
            Event::StorageFailed => "storage-failed".to_owned(),
            _ => return,
        };
        say(&line);
    }

    async fn command(&self, words: &[&str], line: &str) -> Result<(), String> {
        match words {
            ["new-identity", label @ ..] => {
                let label = (!label.is_empty()).then(|| label.join(" "));
                let identity = self
                    .runtime
                    .create_identity(label.as_deref())
                    .await
                    .map_err(|error| error.to_string())?;
                let index = {
                    let mut identities = lock(&self.identities);
                    identities.push(identity);
                    identities.len().saturating_sub(1)
                };
                let key = self
                    .runtime
                    .local_identity(&identity)
                    .map(|local| short(local.identity()))
                    .unwrap_or_default();
                say(&format!("identity {index} {key}"));
                Ok(())
            }
            ["card", n] => {
                let (index, identity) = self.identity(n)?;
                let card = self
                    .runtime
                    .own_card(&identity)
                    .map_err(|error| error.to_string())?;
                say(&format!("card {index} {card}"));
                Ok(())
            }
            ["add", n, card] => {
                let (index, identity) = self.identity(n)?;
                let record = self
                    .runtime
                    .add_contact(&identity, card)
                    .await
                    .map_err(|error| error.to_string())?;
                say(&format!(
                    "added {index} {}",
                    self.peer_name(&identity, &record)
                ));
                Ok(())
            }
            ["send", n, peer, _, ..] => {
                let (_, identity) = self.identity(n)?;
                let record = self.record(&identity, peer)?;
                // The text is the rest of the line after the peer.
                let text = line
                    .splitn(4, char::is_whitespace)
                    .nth(3)
                    .unwrap_or_default();
                self.runtime
                    .send_message(&identity, &record, text)
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(())
            }
            ["messages", n, peer] => {
                let (index, identity) = self.identity(n)?;
                let record = self.record(&identity, peer)?;
                let messages = self
                    .runtime
                    .messages(&identity, &record, None, 1000)
                    .map_err(|error| error.to_string())?;
                let name = self.peer_name(&identity, &record);
                say(&format!("messages {index} {name} {}", messages.len()));
                for message in messages {
                    say(&format!(
                        "message {index} {name} {} {:?} {:?}",
                        message.reference.sequence, message.status, message.text
                    ));
                }
                Ok(())
            }
            ["conversations", n] => {
                let (index, identity) = self.identity(n)?;
                for conversation in self
                    .runtime
                    .conversations(&identity)
                    .map_err(|error| error.to_string())?
                {
                    say(&format!(
                        "conversation {index} {} active={} messages={} queued={}",
                        self.peer_name(&identity, &conversation.contact),
                        conversation.active,
                        conversation.messages,
                        conversation.queued
                    ));
                }
                Ok(())
            }
            [verb @ ("block" | "delete"), n, peer] => {
                let (index, identity) = self.identity(n)?;
                let record = self.record(&identity, peer)?;
                let name = self.peer_name(&identity, &record);
                if *verb == "block" {
                    self.runtime.block_contact(&identity, &record).await
                } else {
                    self.runtime.delete_contact(&identity, &record).await
                }
                .map_err(|error| error.to_string())?;
                let done = if *verb == "block" {
                    "blocked"
                } else {
                    "deleted"
                };
                say(&format!("{done} {index} {name}"));
                Ok(())
            }
            ["sessions", n, peer] => {
                let (index, identity) = self.identity(n)?;
                let record = self.record(&identity, peer)?;
                let count = self
                    .runtime
                    .sessions(&identity, &record)
                    .map_err(|error| error.to_string())?;
                say(&format!(
                    "sessions {index} {} {count}",
                    self.peer_name(&identity, &record)
                ));
                Ok(())
            }
            _ => Err("unknown command".to_owned()),
        }
    }
}

fn read_commands() -> mpsc::Receiver<String> {
    let (sender, commands) = mpsc::channel(COMMAND_QUEUE);
    std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else {
                break;
            };
            if sender.blocking_send(line).is_err() {
                break;
            }
        }
    });
    commands
}

fn fail(what: &str) -> ExitCode {
    eprintln!("monolith dev-runtime: {what}");
    ExitCode::FAILURE
}

/// The node: opens or creates the vault in `data_dir`, starts the runtime,
/// and runs commands until `quit` or the end of the input. With
/// `acknowledgement_delay`, a received message waits that long, durable,
/// before it is acknowledged: test tooling uses it to stop the node in
/// between.
pub(crate) async fn run(
    config: SystemTorConfig,
    data_dir: PathBuf,
    passphrase_file: PathBuf,
    floor: bool,
    acknowledgement_delay: Duration,
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
    let identities: Vec<IdentityId> = installation
        .identities()
        .iter()
        .map(|identity| identity.id())
        .collect();
    let runtime = Runtime::start(
        installation,
        SystemTorBackend::new(config),
        Budgets::new(),
        RuntimeOptions {
            acknowledgement_delay,
            ..RuntimeOptions::default()
        },
    );
    let mut events = runtime.subscribe();
    let node = Arc::new(Node {
        runtime,
        identities: Mutex::new(identities),
    });
    for (index, identity) in lock(&node.identities).iter().enumerate() {
        if let Some(local) = node.runtime.local_identity(identity) {
            say(&format!("identity {index} {}", short(local.identity())));
        }
    }
    let reporting = node.clone();
    let reporter = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => reporting.report(event),
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    say(&format!("lagged {missed}"));
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    say("ready");
    let mut commands = read_commands();
    while let Some(line) = commands.recv().await {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.first() == Some(&"quit") {
            break;
        }
        if let Err(error) = node.command(&words, &line).await {
            say(&format!("error {error}"));
        }
    }
    node.runtime.shutdown(Duration::from_secs(10)).await;
    reporter.abort();
    say("bye");
    ExitCode::SUCCESS
}

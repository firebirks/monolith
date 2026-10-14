//! A smoke test of the desktop client over Tor: two clients, A and B, each
//! with a profile on disk, driven through their window code (the screens,
//! the UI model, the bridge, the runtime and the file thread). B makes an
//! invitation and saves its card; A adds it from the file and writes; B
//! sees A's request and accepts it; B answers; each message is shown
//! Delivered to its sender and Received by the other; both close. The
//! protocol itself is proved by the runtime tests of Phase 5; this shows
//! that the window drives that runtime.
//!
//! With Tor, one process per client, as the private Tor network test runs
//! it (`tests/tor-network/two-node.sh`):
//!
//!     tor_smoke a|b SHARED DATA --socks ENDPOINT --control ENDPOINT --cookie-file PATH
//!
//! SHARED is a folder both see, empty at the start, through which B hands
//! its invitation card to A and each says when it is done; DATA is the
//! client's data directory. The Tor configuration is the one the desktop
//! makes of these settings, so a dial waits until Tor lists the SOCKS
//! endpoint. Without arguments, both clients run in this process on the
//! in-memory Tor, in a temporary folder:
//! `cargo run --release -p monolith-desktop --example tor_smoke`.
//!
//! Each wait is bounded; one that ends without its result fails with what
//! it waited for, and the exit status is not 0. A client that passes says
//! so in its last line.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable,
    clippy::arithmetic_side_effects
)]

mod common;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread::JoinHandle;
use std::time::Instant;

use core::time::Duration;

use common::Client;
use monolith_desktop::config;
use monolith_runtime::{SocksProof, SystemTorBackend, TorBackend};
use monolith_tor::MockNetwork;
use monolith_ui::TorSettings;
use monolith_ui_model::{Status, TorIndicator};

/// How long a wait lasts at most over Tor: a new service has to be
/// published and its descriptor found, and the runtime waits longer
/// between delivery attempts after each one that fails.
const TOR_PATIENCE: Duration = Duration::from_secs(900);

/// How long a wait lasts at most on the in-memory Tor.
const MEMORY_PATIENCE: Duration = Duration::from_secs(240);

/// How long the Tor check has to say Tor works: Tor is running and has a
/// circuit when the clients start.
const TOR_CHECK_PATIENCE: Duration = Duration::from_secs(120);

/// The file through which B hands its invitation card to A.
const INVITATION: &str = "b-invitation.txt";

const FROM_A: &str = "Hello B, this is A.";
const FROM_B: &str = "Hello A, this is B.";

const USAGE: &str = "usage: tor_smoke [a|b SHARED DATA --socks ENDPOINT --control ENDPOINT \
                     --cookie-file PATH]";

#[derive(Clone, Copy)]
enum Role {
    A,
    B,
}

impl Role {
    fn name(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
        }
    }

    fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
}

/// One client over Tor, as the arguments say.
struct Node {
    role: Role,
    shared: PathBuf,
    data: PathBuf,
    tor: TorSettings,
}

fn parse(args: Vec<OsString>) -> Option<Node> {
    let mut args = args.into_iter();
    let role = match args.next()?.to_str()? {
        "a" => Role::A,
        "b" => Role::B,
        _ => return None,
    };
    let shared = PathBuf::from(args.next()?);
    let data = PathBuf::from(args.next()?);
    let (mut socks, mut control, mut cookie_file) = (None, None, None);
    while let Some(flag) = args.next() {
        let value = args.next()?.into_string().ok()?;
        match flag.to_str()? {
            "--socks" => socks = Some(value),
            "--control" => control = Some(value),
            "--cookie-file" => cookie_file = Some(value),
            _ => return None,
        }
    }
    Some(Node {
        role,
        shared,
        data,
        tor: TorSettings {
            socks: socks?,
            control: control?,
            cookie_file: cookie_file?,
        },
    })
}

/// Runs client `role` to the end; a wait that fails panics.
fn run<B: TorBackend + 'static>(
    role: Role,
    shared: &Path,
    data: &Path,
    backend: impl FnMut() -> B + Send + 'static,
    tor: TorSettings,
    patience: Duration,
) {
    let begun = Instant::now();
    let name = role.name();
    let saves = shared.join(format!("{name}-saved"));
    std::fs::create_dir_all(&saves).expect("a folder for saved cards");
    let mut client = Client::start(name, data, backend, tor, patience).saving_to(&saves);
    client.open_profile();
    let version = client.tor_works(TOR_CHECK_PATIENCE);
    println!("{name}: the Tor check says Tor works ({version})");
    client.create_identity(&name.to_uppercase());
    client.until("Tor connected is shown", |model| {
        model.tor_indicator() == TorIndicator::Connected
    });
    match role {
        Role::A => a(&mut client, shared),
        Role::B => b(&mut client, shared),
    }
    std::fs::write(shared.join(format!("{name}-done")), b"").expect("say this client is done");
    let other = shared.join(format!("{}-done", role.other().name()));
    client.until_some("the other client is done", |_| other.exists().then_some(()));
    client.close();
    println!("{name}: passed in {:.0} s", begun.elapsed().as_secs_f64());
}

/// A adds B's invitation card from its file, writes, and waits for its
/// message to be delivered and for B's answer.
fn a(client: &mut Client, shared: &Path) {
    let file = shared.join(INVITATION);
    client.until_some("B's invitation card is there", |_| {
        file.exists().then_some(())
    });
    let b = client.add_contact_from_file(&file, "B");
    client.send(b, FROM_A);
    client.wait_status(b, FROM_A, Status::Delivered);
    client.wait_status(b, FROM_B, Status::Received);
}

/// B makes an invitation, saves its card for A, accepts A's request,
/// answers, and waits for A's message and for its answer to be delivered.
fn b(client: &mut Client, shared: &Path) {
    let card = client.create_invitation("For A");
    let saved = client.save_invitation(&card);
    client.model().close();
    // Handed over whole: A never reads a card half written.
    std::fs::rename(&saved, shared.join(INVITATION)).expect("hand the card over");
    println!("b: the invitation card is saved and handed over");
    let request = client.wait_request();
    let a = client.accept_request(request, "A");
    client.send(a, FROM_B);
    client.wait_status(a, FROM_A, Status::Received);
    client.wait_status(a, FROM_B, Status::Delivered);
}

/// One client over the Tor the arguments name, with the configuration the
/// desktop makes of those settings.
fn over_tor(node: Node) -> ExitCode {
    let name = node.role.name();
    let Some(tor) = config::tor_config(&node.tor) else {
        eprintln!("{name}: the desktop refuses these Tor settings");
        return ExitCode::from(64);
    };
    // The proof the desktop needs outside Tails and Whonix, which this test
    // is here to run against a real Tor.
    if tor.socks_proof != SocksProof::ControlListeners {
        eprintln!("{name}: the desktop would dial without proving the SOCKS endpoint here");
        return ExitCode::FAILURE;
    }
    println!(
        "{name}: SOCKS {}, control {}; a dial waits until Tor lists the SOCKS endpoint",
        node.tor.socks, node.tor.control
    );
    run(
        node.role,
        &node.shared,
        &node.data,
        move || SystemTorBackend::new(tor.clone()),
        node.tor,
        TOR_PATIENCE,
    );
    ExitCode::SUCCESS
}

/// Both clients in this process, each on a thread of its own, on the
/// in-memory Tor.
fn in_memory() -> ExitCode {
    let dir = std::env::temp_dir().join(format!("monolith-tor-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let shared = dir.join("shared");
    std::fs::create_dir_all(&shared).expect("a shared folder");
    let network = MockNetwork::new();
    let mut clients: Vec<JoinHandle<()>> = [Role::A, Role::B]
        .into_iter()
        .map(|role| {
            let network = network.clone();
            let shared = shared.clone();
            let data = dir.join(role.name());
            std::thread::Builder::new()
                .name(role.name().to_owned())
                .spawn(move || {
                    run(
                        role,
                        &shared,
                        &data,
                        move || network.backend(),
                        common::in_memory_tor(),
                        MEMORY_PATIENCE,
                    );
                })
                .expect("a thread for the client")
        })
        .collect();
    // A client that failed ends the test at once: the other would wait for
    // it until its patience runs out. Its folder is kept.
    while !clients.is_empty() {
        match clients.iter().position(JoinHandle::is_finished) {
            Some(index) => {
                if clients.swap_remove(index).join().is_err() {
                    return ExitCode::FAILURE;
                }
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.is_empty() {
        return in_memory();
    }
    match parse(args) {
        Some(node) => over_tor(node),
        None => {
            eprintln!("{USAGE}");
            ExitCode::from(64)
        }
    }
}

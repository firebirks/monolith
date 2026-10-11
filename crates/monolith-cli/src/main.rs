//! Command-line front end.
//!
//! The command set is listed in `docs/ARCHITECTURE.md`. Phase 3 implements
//! `tor status`, `doctor` and the two-node test `dev-chat`; the other
//! commands report that they are not implemented yet.

// The CLI is the one place that writes to the terminal directly.
#![allow(clippy::print_stdout, clippy::print_stderr)]

// `fuzzing` gives the session layer handshakes with fixed ephemeral keys,
// for the fuzz targets. A product must never be built that way.
#[cfg(fuzzing)]
compile_error!("monolith must not be built with --cfg fuzzing");

mod dev;
mod node;

use std::process::ExitCode;

use monolith_protocol::limits::PROTOCOL_VERSION;
use monolith_tor::{
    Bootstrap, ControlAuth, ControlStatus, Endpoint, FEATURE_BASELINE, RECOMMENDED, Readiness,
    SocksStatus, SystemTorBackend, SystemTorConfig, TorBackend, TorStatus,
};

const USAGE: &str = "\
Usage: monolith [options] <command>

Commands:
  status                 Show Tor, service and storage state
  identity show          Show the local identity
  identity fingerprint   Show the local identity fingerprint
  contact-card show      Show the local contact card
  contact list           List contacts
  contact add <card>     Add a contact from a contact card
  tor status             Show Tor and service state
  doctor                 Check the Tor and storage setup
  version                Show version information
  help                   Show this text

Development, two-node test (identities in memory only):
  dev-chat serve <own card file> <peer card file>
  dev-chat dial <own card file> <peer card file> <message>

Development node over a persistent vault, driven by commands on standard
input (see the source of node.rs for the commands):
  dev-node <data dir> <passphrase file> [--kdf-floor] [--echo]

Options (Tor endpoints, never anything but loopback or a local socket):
  --socks <endpoint>          default 127.0.0.1:9050
  --control <endpoint>        default unix:/run/tor/control
  --control-auth <mode>       safecookie (default) or trusted-filter
  --cookie-file <path>        default /run/tor/control.authcookie
An endpoint is 127.0.0.1:<port>, [::1]:<port> or unix:<absolute path>.
The cookie file must be the one Tor writes; the control endpoint has to
name that same file. trusted-filter is only for a filtering control proxy
whose access control the platform provides.

No command ever prints a private key.";

/// Default endpoints of a Debian system Tor (`TOR_INTEGRATION.md` section 7).
const DEFAULT_SOCKS: &str = "127.0.0.1:9050";
const DEFAULT_CONTROL: &str = "unix:/run/tor/control";
const DEFAULT_COOKIE: &str = "/run/tor/control.authcookie";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (config, words) = match parse_options(&args) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("monolith: {message}\n\n{USAGE}");
            return ExitCode::from(64);
        }
    };
    let words: Vec<&str> = words.iter().map(String::as_str).collect();

    match words.as_slice() {
        [] | ["help" | "--help" | "-h"] => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        ["version" | "--version" | "-V"] => {
            println!(
                "monolith {} (protocol {PROTOCOL_VERSION})",
                env!("CARGO_PKG_VERSION")
            );
            ExitCode::SUCCESS
        }
        ["tor", "status"] => block_on(tor_status(config)),
        ["doctor"] => block_on(doctor(config)),
        ["dev-chat", "serve", own, peer] => block_on(dev::serve(config, own, peer)),
        ["dev-chat", "dial", own, peer, message] => block_on(dev::dial(config, own, peer, message)),
        ["dev-node", data_dir, passphrase_file, flags @ ..]
            if flags
                .iter()
                .all(|flag| matches!(*flag, "--kdf-floor" | "--echo")) =>
        {
            block_on(node::run(
                config,
                std::path::PathBuf::from(data_dir),
                std::path::PathBuf::from(passphrase_file),
                flags.contains(&"--kdf-floor"),
                flags.contains(&"--echo"),
            ))
        }
        ["status"]
        | ["identity", "show" | "fingerprint"]
        | ["contact-card", "show"]
        | ["contact", "list"]
        | ["contact", "add", _] => {
            eprintln!("monolith: this command is not implemented yet");
            ExitCode::from(2)
        }
        _ => {
            eprintln!("monolith: unknown command\n\n{USAGE}");
            ExitCode::from(64)
        }
    }
}

/// Splits the options from the command words.
fn parse_options(args: &[String]) -> Result<(SystemTorConfig, Vec<String>), String> {
    let mut socks = DEFAULT_SOCKS.to_owned();
    let mut control = DEFAULT_CONTROL.to_owned();
    let mut trusted_filter = false;
    let mut cookie = DEFAULT_COOKIE.to_owned();
    let mut words = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let mut value = || {
            iter.next()
                .cloned()
                .ok_or_else(|| format!("{arg} needs a value"))
        };
        match arg.as_str() {
            "--socks" => socks = value()?,
            "--control" => control = value()?,
            "--cookie-file" => cookie = value()?,
            "--control-auth" => {
                trusted_filter = match value()?.as_str() {
                    "safecookie" => false,
                    "trusted-filter" => true,
                    _ => return Err("--control-auth is safecookie or trusted-filter".to_owned()),
                };
            }
            _ => words.push(arg.clone()),
        }
    }
    let socks = Endpoint::parse(&socks).map_err(|_| format!("invalid SOCKS endpoint {socks}"))?;
    let control =
        Endpoint::parse(&control).map_err(|_| format!("invalid control endpoint {control}"))?;
    let cookie_file = std::path::PathBuf::from(cookie);
    if !cookie_file.is_absolute() {
        return Err("--cookie-file must be an absolute path".to_owned());
    }
    let auth = if trusted_filter {
        eprintln!(
            "monolith: warning: no control authentication; whatever listens on the control \
             endpoint is taken for Tor"
        );
        ControlAuth::TrustedFilter
    } else {
        ControlAuth::SafeCookie { cookie_file }
    };
    Ok((
        SystemTorConfig {
            socks,
            control,
            auth,
        },
        words,
    ))
}

fn block_on<F: core::future::Future<Output = ExitCode>>(future: F) -> ExitCode {
    match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(future),
        Err(_) => {
            eprintln!("monolith: cannot start the runtime");
            ExitCode::FAILURE
        }
    }
}

fn describe(status: &TorStatus) -> Vec<String> {
    let socks = match status.socks {
        SocksStatus::Reachable => "reachable",
        SocksStatus::Unavailable => "unreachable",
        SocksStatus::InvalidResponse => "answers, but not as Tor's SOCKS5 endpoint",
    };
    let mut lines = vec![format!("Tor SOCKS: {socks}")];
    match status.control {
        ControlStatus::Unavailable => lines.push("Tor control: unreachable".to_owned()),
        ControlStatus::AuthenticationUnavailable => lines.push(
            "Tor control: reachable, but the configured authentication is not offered".to_owned(),
        ),
        ControlStatus::AuthenticationFailed => {
            lines.push("Tor control: authentication failed".to_owned());
        }
        ControlStatus::InvalidResponse => {
            lines.push("Tor control: answers, but not as Tor".to_owned());
        }
        ControlStatus::UnsupportedVersion(version) => lines.push(format!(
            "Tor control: Tor {version} is older than the supported baseline {FEATURE_BASELINE}"
        )),
        ControlStatus::Reachable {
            version,
            circuit_established,
            bootstrap,
        } => {
            lines.push("Tor control: reachable, authenticated".to_owned());
            lines.push(format!("Tor version: {version}"));
            lines.push(format!(
                "Tor circuit: {}",
                if circuit_established {
                    "established"
                } else {
                    "none yet"
                }
            ));
            lines.push(format!(
                "Tor bootstrap: {}",
                match bootstrap {
                    Bootstrap::Unknown => "unknown".to_owned(),
                    Bootstrap::InProgress(percent) => format!("{percent}%"),
                    Bootstrap::Done => "done".to_owned(),
                }
            ));
        }
    }
    lines.push(format!(
        "Summary: {}",
        match status.readiness() {
            Readiness::Ready => "ready",
            Readiness::NotReady(_) => "Tor is not ready yet",
            Readiness::SocksUnavailable => "cannot reach peers: no SOCKS endpoint",
            Readiness::ControlUnavailable => "cannot publish: no control endpoint",
            Readiness::Unavailable => "offline: Tor is unavailable",
        }
    ));
    lines
}

async fn tor_status(config: SystemTorConfig) -> ExitCode {
    let status = SystemTorBackend::new(config).status().await;
    for line in describe(&status) {
        println!("{line}");
    }
    if status.readiness() == Readiness::Ready {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

async fn doctor(config: SystemTorConfig) -> ExitCode {
    let auth = config.auth.clone();
    let status = SystemTorBackend::new(config).status().await;
    for line in describe(&status) {
        println!("{line}");
    }
    println!(
        "Tor authentication: {}",
        match auth {
            ControlAuth::SafeCookie { .. } => "SAFECOOKIE",
            ControlAuth::TrustedFilter => "none, trusted filter (explicitly configured)",
        }
    );
    if let ControlStatus::Reachable { version, .. } = status.control {
        if version < RECOMMENDED {
            println!(
                "Tor release: {version} is older than {RECOMMENDED}; run a supported, \
                 security-updated Tor"
            );
        }
        // Publication needs only what the status query proved: an
        // authenticated control connection to a Tor at or above the
        // baseline. Nothing is published to find out.
        println!("Onion publication: supported");
    } else {
        println!("Onion publication: not available");
    }
    println!(
        "Proof of work: requested on publication; Monolith cannot verify that this Tor \
         was built with it"
    );
    println!("Swap: {}", swap_state());
    ExitCode::SUCCESS
}

/// Whether swap is in use, from `/proc/swaps`.
fn swap_state() -> &'static str {
    match std::fs::read_to_string("/proc/swaps") {
        Ok(text) if text.lines().count() > 1 => {
            "in use; secrets can reach the swap device unless it is encrypted"
        }
        Ok(_) => "none",
        Err(_) => "unknown",
    }
}

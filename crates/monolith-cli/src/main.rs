//! Command-line front end.
//!
//! The command set is listed in `docs/ARCHITECTURE.md`. In Phase 0 the binary
//! only knows its own name and version; every command reports that it is not
//! implemented yet.

// The CLI is the one place that writes to the terminal directly.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

use monolith_protocol::limits::PROTOCOL_MAJOR;

const USAGE: &str = "\
Usage: monolith <command>

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

No command ever prints a private key.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();

    match words.as_slice() {
        [] | ["help" | "--help" | "-h"] => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        ["version" | "--version" | "-V"] => {
            println!(
                "monolith {} (protocol {PROTOCOL_MAJOR})",
                env!("CARGO_PKG_VERSION")
            );
            ExitCode::SUCCESS
        }
        ["status"]
        | ["identity", "show" | "fingerprint"]
        | ["contact-card", "show"]
        | ["contact", "list"]
        | ["contact", "add", _]
        | ["tor", "status"]
        | ["doctor"] => {
            eprintln!("monolith: this command is not implemented yet");
            ExitCode::from(2)
        }
        _ => {
            eprintln!("monolith: unknown command\n\n{USAGE}");
            ExitCode::from(64)
        }
    }
}

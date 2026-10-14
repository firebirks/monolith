//! The Monolith desktop client. See the library of this crate.

// No console window on Windows.
#![windows_subsystem = "windows"]
// Wrong arguments are told to whoever started the client from a terminal.
#![allow(clippy::print_stderr)]

use std::process::ExitCode;

use monolith_desktop::Options;

fn main() -> ExitCode {
    match Options::parse(std::env::args_os().skip(1)) {
        Ok(options) => monolith_desktop::run(options),
        Err(usage) => {
            eprintln!("{usage}");
            ExitCode::from(64)
        }
    }
}

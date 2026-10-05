//! Properties of the source that types alone do not hold, checked by
//! reading the code of every crate outside its tests.
//!
//! - S51: message 3 leaves the process only through `link::dial`, after
//!   an admission by the contact store. The session crate makes message 3
//!   only in `OutboundPeer::admit`; the only production code that names an
//!   outbound peer is the session crate, the contact store and the
//!   identity context that passes it on. `link::dial` sees only the
//!   admission the store returns, and writes the message 3 of a granted
//!   one, once.
//! - S46: no crate holds state in a static item or a thread-local.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::fs;
use std::path::{Path, PathBuf};

/// Every source file of the crates, with the production part of its text:
/// what comes before its first `#[cfg(test)]`, which in this workspace
/// opens the test module at the end of a file or declares a test-only
/// module.
fn production_sources() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    let mut pending: Vec<PathBuf> = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path().join("src"))
        .filter(|path| path.is_dir())
        .collect();
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                // A directory named `tests` inside a crate holds tests.
                if path.file_name().is_some_and(|name| name != "tests") {
                    pending.push(path);
                }
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let name = path.strip_prefix(&root).unwrap().display().to_string();
                if name.ends_with("testing.rs") {
                    continue;
                }
                let text = fs::read_to_string(&path).unwrap();
                let production = text
                    .split("#[cfg(test)]")
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                files.push((name, production));
            }
        }
    }
    files.sort();
    files
}

fn files_naming(word: &str) -> Vec<String> {
    production_sources()
        .into_iter()
        .filter(|(_, text)| text.contains(word))
        .map(|(name, _)| name)
        .collect()
}

#[test]
fn only_the_contact_store_admits_a_responder_and_only_dial_writes_message_3() {
    assert_eq!(
        files_naming("OutboundPeer"),
        vec![
            "monolith-core/src/contacts.rs",
            "monolith-core/src/identity.rs",
            "monolith-session/src/handshake.rs",
            "monolith-session/src/lib.rs",
        ]
    );
    // The re-export in the session crate's `lib.rs` follows its first
    // `#[cfg(test)]` and is not read here; it names the type and nothing
    // else.
    assert_eq!(
        files_naming("Message3"),
        vec!["monolith-session/src/handshake.rs"]
    );
    // `link::dial` reaches message 3 through the granted admission only.
    let link = production_sources()
        .into_iter()
        .find(|(name, _)| name == "monolith-core/src/link.rs")
        .unwrap()
        .1;
    assert_eq!(link.matches("message_3.as_bytes()").count(), 1);
    assert_eq!(link.matches("identity.admit_outbound(").count(), 1);
    // Outside the session crate and the store, nobody admits an outbound
    // peer.
    for (name, text) in production_sources() {
        if name.starts_with("monolith-session/") || name == "monolith-core/src/contacts.rs" {
            continue;
        }
        assert!(!text.contains("outbound.admit("), "{name}");
        assert!(!text.contains("peer.admit("), "{name}");
    }
}

#[test]
fn no_crate_holds_state_in_a_static_or_a_thread_local() {
    for (name, text) in production_sources() {
        for line in text.lines() {
            let line = line.trim_start();
            assert!(
                !line.starts_with("static ")
                    && !line.starts_with("pub static ")
                    && !line.starts_with("pub(crate) static "),
                "{name}: {line}"
            );
            assert!(!line.contains("thread_local!"), "{name}: {line}");
        }
    }
}

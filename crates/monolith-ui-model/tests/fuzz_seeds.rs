//! Seeds of the fuzz targets `ui_present` and `ui_model_sequence`: valid
//! inputs built from fixed values, committed under `fuzz/seeds/ui_present/`
//! and `fuzz/seeds/ui_model_sequence/`. The layouts are those described at
//! the top of the two target files. The test fails if a committed seed is
//! not what this file says. To write them again:
//!
//!     MONOLITH_WRITE_FUZZ_SEEDS=1 cargo test -p monolith-ui-model --test fuzz_seeds

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

// The actions of `ui_model_sequence`.
const SELECT_IDENTITY: u8 = 0;
const SELECT_CONVERSATION: u8 = 1;
const SHOW_REQUESTS: u8 = 2;
const OPEN_DIALOG: u8 = 3;
const CONFIRM: u8 = 4;
const CLOSE: u8 = 5;
const DRAFT: u8 = 6;
const SEND: u8 = 7;
const TAKE_COMMANDS: u8 = 8;
const ANSWER: u8 = 9;
const STALE_ANSWER: u8 = 10;
const EVENT: u8 = 11;
const STOPPED: u8 = 12;
const STARTED: u8 = 13;
const TAKE_NOTICES: u8 = 14;
const CREATE_PROFILE: u8 = 19;
const DECLINE: u8 = 21;

// The kinds of outcome.
const PROFILE_OPEN: u8 = 2;
const IDENTITIES: u8 = 3;
const IDENTITY: u8 = 4;
const MESSAGES: u8 = 5;
const CONTACT_ADDED: u8 = 9;
const MESSAGE_QUEUED: u8 = 10;
const FAILED: u8 = 14;

type Step = (u8, u8, u8, u8, u8);

fn bytes(steps: &[Step]) -> Vec<u8> {
    steps
        .iter()
        .flat_map(|(action, a, b, c, d)| [*action, *a, *b, *c, *d])
        .collect()
}

/// A profile created, both identities listed, each read with three
/// contacts, a conversation and a request, and the first conversation shown
/// with two messages. The command numbers follow the order the model
/// issues them: 0 the profile, 1 the identities, 2 the Tor check, 3 and 4
/// the reads of the two identities, 5 the messages.
fn ready() -> Vec<Step> {
    vec![
        (STARTED, 0, 0, 0, 0),
        (CREATE_PROFILE, 2, 0, 0, 0),
        (TAKE_COMMANDS, 0, 0, 0, 0),
        (ANSWER, 0, PROFILE_OPEN, 0, 0),
        (TAKE_COMMANDS, 0, 0, 0, 0),
        (ANSWER, 1, IDENTITIES, 3, 0),
        (TAKE_COMMANDS, 0, 0, 0, 0),
        (ANSWER, 3, IDENTITY, 0, 0x47),
        (ANSWER, 4, IDENTITY, 1, 0x47),
        (SELECT_CONVERSATION, 0, 0, 0, 0),
        (TAKE_COMMANDS, 0, 0, 0, 0),
        (ANSWER, 5, MESSAGES, 0, 0x03),
    ]
}

fn sequence_seeds() -> Vec<(String, Vec<u8>)> {
    let mut send = ready();
    send.extend([
        (DRAFT, 0, 0, 1, 0),
        (SEND, 0, 0, 0, 0),
        (TAKE_COMMANDS, 0, 0, 0, 0),
        (ANSWER, 6, MESSAGE_QUEUED, 0, 0),
        (TAKE_COMMANDS, 0, 0, 0, 0),
        (TAKE_NOTICES, 0, 0, 0, 0),
    ]);

    let mut dialogs = ready();
    dialogs.extend([
        // Add a contact of the first identity, answer with a contact of
        // the second, then close and select the other identity.
        (OPEN_DIALOG, 0, 0, 1, 0),
        (CONFIRM, 0, 0, 0, 0),
        (TAKE_COMMANDS, 0, 0, 0, 0),
        (ANSWER, 6, CONTACT_ADDED, 1, 1),
        (CLOSE, 0, 0, 0, 0),
        // A dialog of the second identity, left by selecting the first.
        (SELECT_IDENTITY, 1, 0, 0, 0),
        (OPEN_DIALOG, 1, 0, 11, 0),
        (SELECT_IDENTITY, 0, 0, 0, 0),
        (OPEN_DIALOG, 0, 1, 7, 0),
        (CONFIRM, 0, 0, 0, 0),
        (TAKE_COMMANDS, 0, 0, 0, 0),
        (ANSWER, 7, FAILED, 0, 0),
        (CLOSE, 0, 0, 0, 0),
        (TAKE_NOTICES, 0, 0, 0, 0),
    ]);

    let mut stale = ready();
    stale.extend([
        // Answers to numbers never issued, and to one answered already.
        (STALE_ANSWER, 0, IDENTITIES, 2, 0),
        (STALE_ANSWER, 5, IDENTITY, 0, 0x47),
        (STALE_ANSWER, 1, MESSAGES, 1, 0x0F),
        (SHOW_REQUESTS, 0, 0, 0, 0),
        (DECLINE, 0, 1, 0, 0),
        (TAKE_COMMANDS, 0, 0, 0, 0),
        (STALE_ANSWER, 2, PROFILE_OPEN, 1, 0),
    ]);

    // One event of each kind, then the runtime stops.
    let mut events = ready();
    for kind in 0..11_u8 {
        events.push((EVENT, 0, kind, 0, 1));
        events.push((EVENT, 1, kind, 2, 3));
        events.push((TAKE_COMMANDS, 0, 0, 0, 0));
    }
    events.extend([(STOPPED, 0, 0, 0, 0), (TAKE_NOTICES, 0, 0, 0, 0)]);

    vec![
        ("ui_model_sequence/send.bin".to_owned(), bytes(&send)),
        ("ui_model_sequence/dialogs.bin".to_owned(), bytes(&dialogs)),
        (
            "ui_model_sequence/stale_answers.bin".to_owned(),
            bytes(&stale),
        ),
        ("ui_model_sequence/events.bin".to_owned(), bytes(&events)),
    ]
}

/// A selector byte, then the text.
fn text(selector: u8, text: &str) -> Vec<u8> {
    let mut seed = vec![selector];
    seed.extend_from_slice(text.as_bytes());
    seed
}

fn present_seeds() -> Vec<(String, Vec<u8>)> {
    let long_cluster: String = core::iter::once('e')
        .chain(core::iter::repeat_n('\u{0301}', 20))
        .collect();
    vec![
        (
            "ui_present/plain.bin".to_owned(),
            text(0x0A, "Hello world, see you at 14:05"),
        ),
        (
            "ui_present/marks.bin".to_owned(),
            text(0x01, "abc\u{202E}def\u{200B}ghi\u{2066}jkl\u{FEFF}"),
        ),
        (
            "ui_present/lines.bin".to_owned(),
            text(0x0C, &"line\n".repeat(230)),
        ),
        (
            "ui_present/cluster.bin".to_owned(),
            text(0x00, &format!("a{long_cluster}b and more text after it")),
        ),
        (
            "ui_present/white_space.bin".to_owned(),
            text(0x10, "  \n\t\u{3000} \u{00A0}"),
        ),
        (
            "ui_present/names.bin".to_owned(),
            text(0x02, "Bob\u{202E}ABCD EFGH IJKL MNOP"),
        ),
        (
            "ui_present/trailing_space.bin".to_owned(),
            text(0x06, "abcde \n  "),
        ),
    ]
}

fn seed_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds")
}

#[test]
fn committed_seeds_are_current() {
    let root = seed_root();
    let write = std::env::var_os("MONOLITH_WRITE_FUZZ_SEEDS").is_some();
    for (directory, expected) in [
        ("ui_present", present_seeds()),
        ("ui_model_sequence", sequence_seeds()),
    ] {
        for (path, content) in &expected {
            let file = root.join(path);
            if write {
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(&file, content).unwrap();
            }
            let on_disk = fs::read(&file)
                .unwrap_or_else(|error| panic!("seed {path} cannot be read: {error}"));
            assert!(on_disk == *content, "seed {path} is out of date");
        }
        let on_disk = fs::read_dir(root.join(directory)).unwrap().count();
        assert_eq!(
            on_disk,
            expected.len(),
            "stray or missing files in fuzz/seeds/{directory}"
        );
    }
}

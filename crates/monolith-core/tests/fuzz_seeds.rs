//! Seeds of the fuzz targets `contact_store`, `message_sequence` and
//! `transfer_sequence`: operation lists, committed under
//! `fuzz/seeds/<target>/`. The layout is that of each target: three bytes
//! per operation for `contact_store` (the operation modulo 7, the remote
//! identity modulo 3, and the card) and for `message_sequence` (the
//! operation modulo 7, the identity and record, and the identifier and
//! length or the deletion mode); four for `transfer_sequence` (the
//! operation modulo 17, the transfer, two arguments). The test fails if a
//! committed seed is not what this file says. To write them again:
//!
//!     MONOLITH_WRITE_FUZZ_SEEDS=1 cargo test -p monolith-core --test fuzz_seeds

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

const IMPORT: u8 = 0;
const CONFIRM: u8 = 1;
const DIAL: u8 = 2;
const BLOCK: u8 = 3;
const UNBLOCK: u8 = 4;
const DELETE: u8 = 5;
const VERIFY: u8 = 6;

/// The card byte of key `key` at epoch `epoch` (1 to 5) at endpoint `at`.
const fn card(key: u8, epoch: u8, at: u8) -> u8 {
    key | ((epoch - 1) << 2) | (at << 5)
}

fn seeds() -> Vec<(String, Vec<u8>)> {
    let list = |operations: &[(u8, u8, u8)]| -> Vec<u8> {
        operations
            .iter()
            .flat_map(|(operation, remote, card)| [*operation, *remote, *card])
            .collect()
    };
    vec![
        (
            "contact_store/import_and_move.bin".to_owned(),
            list(&[
                (IMPORT, 0, card(0, 1, 0)),
                (IMPORT, 0, card(0, 2, 1)),
                (DIAL, 0, card(0, 2, 1)),
                (VERIFY, 0, 0),
            ]),
        ),
        (
            "contact_store/requested_new_key.bin".to_owned(),
            list(&[
                (IMPORT, 1, card(0, 1, 0)),
                (IMPORT, 1, card(1, 3, 0)),
                (IMPORT, 1, card(0, 5, 0)),
                (CONFIRM, 1, card(1, 3, 0)),
            ]),
        ),
        (
            "contact_store/block_unblock_delete.bin".to_owned(),
            list(&[
                (IMPORT, 2, card(2, 1, 0)),
                (BLOCK, 2, 0),
                (IMPORT, 2, card(2, 2, 0)),
                (UNBLOCK, 2, 0),
                (IMPORT, 2, card(3, 4, 1)),
                (DELETE, 2, 0),
                (IMPORT, 2, card(2, 1, 0)),
            ]),
        ),
    ]
}

const QUEUE: u8 = 0;
const RECEIVE: u8 = 1;
const ACKNOWLEDGE: u8 = 2;
const DELETE_CONVERSATION: u8 = 3;
const REMOVE_IDENTITY: u8 = 4;
const RESTART: u8 = 5;
const OTHER_PEER: u8 = 6;

fn message_seeds() -> Vec<(String, Vec<u8>)> {
    let list = |operations: &[(u8, u8, u8)]| -> Vec<u8> {
        operations
            .iter()
            .flat_map(|(operation, target, argument)| [*operation, *target, *argument])
            .collect()
    };
    vec![
        (
            "message_sequence/deliver_and_restart.bin".to_owned(),
            list(&[
                (QUEUE, 0, 1),
                (QUEUE, 0, 2),
                (ACKNOWLEDGE, 0, 1),
                (RESTART, 0, 0),
                (ACKNOWLEDGE, 0, 1),
                (ACKNOWLEDGE, 0, 2),
            ]),
        ),
        (
            "message_sequence/duplicates_and_deletion.bin".to_owned(),
            list(&[
                (RECEIVE, 1, 3),
                (RECEIVE, 1, 3),
                (DELETE_CONVERSATION, 1, 1),
                (RECEIVE, 1, 3),
                (RESTART, 0, 0),
                (RECEIVE, 1, 3),
                (DELETE_CONVERSATION, 1, 0),
                (RECEIVE, 1, 3),
            ]),
        ),
        (
            "message_sequence/isolation.bin".to_owned(),
            list(&[
                (QUEUE, 2, 4),
                (ACKNOWLEDGE, 0, 4),
                (ACKNOWLEDGE, 3, 4),
                (OTHER_PEER, 2, 5),
                (REMOVE_IDENTITY, 1, 0),
                (RESTART, 0, 0),
                (QUEUE, 1, 4),
            ]),
        ),
    ]
}

const OFFER: u8 = 0;
const OFFERED: u8 = 1;
const ACCEPT: u8 = 2;
const REJECT: u8 = 3;
const CANCEL: u8 = 4;
const REMOVE: u8 = 5;
const PEER_ACCEPTED: u8 = 6;
const PEER_COMPLETED: u8 = 7;
const PEER_ABORTED: u8 = 9;
const STAGED: u8 = 10;
const COMPLETE: u8 = 11;
const TICK: u8 = 13;
const CONTACT_ENDED: u8 = 14;
const RESTART_STORE: u8 = 15;
const REMOVE_IDENTITY_OF: u8 = 16;

/// The transfer byte of `transfer` (0 to 7) of contact record `contact`
/// of `identity`, offered by the peer if `incoming`.
const fn named(identity: u8, contact: u8, incoming: bool, transfer: u8) -> u8 {
    identity | (contact << 1) | ((incoming as u8) << 2) | (transfer << 3)
}

fn transfer_seeds() -> Vec<(String, Vec<u8>)> {
    let list = |operations: &[(u8, u8, u8, u8)]| -> Vec<u8> {
        operations
            .iter()
            .flat_map(|(operation, transfer, a, b)| [*operation, *transfer, *a, *b])
            .collect()
    };
    let out = named(0, 0, false, 1);
    let inc = named(0, 0, true, 2);
    vec![
        (
            "transfer_sequence/send_and_restart.bin".to_owned(),
            list(&[
                (OFFER, out, 10, 0),
                (PEER_ACCEPTED, out, 0, 0),
                (RESTART_STORE, 0, 0, 0),
                (PEER_COMPLETED, out, 0, 1),
                (PEER_COMPLETED, out, 0, 0),
                (REMOVE, out, 0, 0),
            ]),
        ),
        (
            "transfer_sequence/receive_and_answer.bin".to_owned(),
            list(&[
                (OFFERED, inc, 10, 0),
                (ACCEPT, inc, 0, 0),
                (STAGED, inc, 5, 0),
                (RESTART_STORE, 0, 0, 0),
                (COMPLETE, inc, 0, 0),
                (OFFERED, inc, 10, 0),
                (REMOVE, inc, 0, 0),
                (OFFERED, inc, 10, 0),
            ]),
        ),
        (
            "transfer_sequence/decisions_and_time.bin".to_owned(),
            list(&[
                (OFFERED, named(0, 1, true, 3), 5, 0),
                (REJECT, named(0, 1, true, 3), 0, 0),
                (OFFERED, named(0, 1, true, 4), 5, 0),
                (OFFERED, named(0, 1, true, 5), 5, 0xFF),
                (OFFER, named(0, 1, false, 6), 5, 0),
                (CANCEL, named(0, 1, false, 6), 0, 0),
                (PEER_ABORTED, named(0, 1, true, 4), 0, 0),
                (TICK, 0, 2, 1),
                (TICK, 0, 20, 1),
            ]),
        ),
        (
            "transfer_sequence/contacts_and_identities.bin".to_owned(),
            list(&[
                (OFFER, named(1, 0, false, 1), 3, 0),
                (OFFERED, named(1, 1, true, 1), 3, 0),
                (ACCEPT, named(1, 1, true, 1), 0, 0),
                (CONTACT_ENDED, named(1, 1, false, 0), 0, 0),
                (CONTACT_ENDED, named(1, 0, false, 0), 0, 1),
                (OFFER, named(1, 0, false, 1), 3, 0),
                (REMOVE_IDENTITY_OF, named(1, 0, false, 0), 0, 0),
                (RESTART_STORE, 0, 0, 0),
            ]),
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
        ("contact_store", seeds()),
        ("message_sequence", message_seeds()),
        ("transfer_sequence", transfer_seeds()),
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

//! Seeds of the fuzz target `contact_store`: operation lists, committed
//! under `fuzz/seeds/contact_store/`. The layout is that of the target:
//! three bytes per operation, the operation modulo 7, the remote identity
//! modulo 3, and the card. The test fails if a committed seed is not what
//! this file says. To write them again:
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

fn seed_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds")
}

#[test]
fn committed_seeds_are_current() {
    let root = seed_root();
    let write = std::env::var_os("MONOLITH_WRITE_FUZZ_SEEDS").is_some();
    let expected = seeds();
    for (path, content) in &expected {
        let file = root.join(path);
        if write {
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, content).unwrap();
        }
        let on_disk =
            fs::read(&file).unwrap_or_else(|error| panic!("seed {path} cannot be read: {error}"));
        assert!(on_disk == *content, "seed {path} is out of date");
    }
    let on_disk = fs::read_dir(root.join("contact_store")).unwrap().count();
    assert_eq!(
        on_disk,
        expected.len(),
        "stray or missing files in fuzz/seeds"
    );
}

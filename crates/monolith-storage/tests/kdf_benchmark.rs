//! The measurement of `docs/STORAGE.md` section 3.3: how long unlocking a
//! vault takes at each candidate KDF setting, and the peak resident memory
//! of the process. Ignored unless asked for; run it built with
//! optimization, on the machine to be measured:
//!
//!     cargo test --release -p monolith-storage --test kdf_benchmark -- --ignored --nocapture
//!
//! Each setting is unlocked ten times; the median and the worst time are
//! printed, with the peak resident memory read from `/proc/self/status`.

// Test code builds its own inputs, and the measurement is printed.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::print_stdout,
    clippy::cast_possible_truncation
)]

use std::time::{Duration, Instant};

use monolith_storage::dir::MemoryDir;
use monolith_storage::vault::{KdfParams, Passphrase, Vault};

fn peak_resident_kib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find(|line| line.starts_with("VmHWM:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

#[test]
#[ignore = "a measurement; run it with --release on the machine to measure"]
fn unlock_times_at_the_candidate_settings() {
    let passphrase = Passphrase::new("benchmark passphrase").unwrap();
    let settings = [
        ("floor, RFC 9106 second setting", KdfParams::FLOOR),
        ("proposed default", KdfParams::DEFAULT),
        (
            "default memory, 2 lanes",
            KdfParams {
                parallelism: 2,
                ..KdfParams::DEFAULT
            },
        ),
    ];
    println!(
        "cpus: {}",
        std::thread::available_parallelism().map_or(0, |count| count.get())
    );
    for (name, params) in settings {
        let dir = MemoryDir::new();
        Vault::create(dir.clone(), &passphrase, params, b"benchmark").unwrap();
        let mut times: Vec<Duration> = (0..10)
            .map(|_| {
                let started = Instant::now();
                Vault::open(dir.clone(), &passphrase).unwrap();
                started.elapsed()
            })
            .collect();
        times.sort();
        println!(
            "{name}: {} MiB, {} iterations, {} lanes: median {} ms, worst {} ms, peak resident {} MiB",
            params.memory_kib / 1024,
            params.iterations,
            params.parallelism,
            times[times.len() / 2].as_millis(),
            times[times.len() - 1].as_millis(),
            peak_resident_kib().unwrap_or(0) / 1024
        );
    }
}

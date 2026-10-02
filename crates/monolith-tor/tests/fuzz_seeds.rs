//! Seed inputs for the Tor fuzz targets in `fuzz/`: replies of every shape
//! the control client reads, and SOCKS replies. They are built here and
//! committed under `fuzz/seeds/<target>/`; the test fails if a committed
//! seed is not what this file produces. To write them again:
//!
//!     MONOLITH_WRITE_FUZZ_SEEDS=1 cargo test -p monolith-tor --test fuzz_seeds

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use monolith_identity::{IdentitySecretKey, OnionServiceKey, ServiceId};

const TARGETS: [&str; 2] = ["tor_control_reply", "socks_reply"];

fn service_id() -> String {
    let key = IdentitySecretKey::from_seed(&[11; 32]).public_key();
    ServiceId::from_key(&OnionServiceKey::from_bytes(key.as_bytes()).unwrap())
        .as_str()
        .to_owned()
}

fn seeds() -> Vec<(String, Vec<u8>)> {
    let id = service_id();
    let key = format!("{}A==", "B".repeat(85));
    let mut seeds = Vec::new();
    let mut control = |name: &str, piece: u8, reply: String| {
        let mut input = vec![piece];
        input.extend_from_slice(reply.as_bytes());
        seeds.push((format!("tor_control_reply/{name}.bin"), input));
    };
    control(
        "protocolinfo",
        200,
        "250-PROTOCOLINFO 1\r\n250-AUTH METHODS=COOKIE,SAFECOOKIE COOKIEFILE=\"/run/tor/control.authcookie\"\r\n250-VERSION Tor=\"0.4.9.13\"\r\n250 OK\r\n".to_owned(),
    );
    control(
        "protocolinfo_escaped_path",
        3,
        "250-PROTOCOLINFO 1\r\n250-AUTH METHODS=SAFECOOKIE COOKIEFILE=\"/tmp/a\\\\b\\\"c\\303\\251\"\r\n250-VERSION Tor=\"0.5.0.1-alpha\" X=1\r\n250-OTHER line\r\n250 OK\r\n".to_owned(),
    );
    control(
        "authchallenge",
        63,
        format!(
            "250 AUTHCHALLENGE SERVERHASH={} SERVERNONCE={}\r\n",
            "AB".repeat(32),
            "CD".repeat(32)
        ),
    );
    control("ok", 1, "250 OK\r\n".to_owned());
    control(
        "circuit",
        10,
        "250-status/circuit-established=1\r\n250 OK\r\n".to_owned(),
    );
    control(
        "bootstrap",
        17,
        "250-status/bootstrap-phase=NOTICE BOOTSTRAP PROGRESS=85 TAG=ap_conn_done SUMMARY=\"Connected\"\r\n250 OK\r\n".to_owned(),
    );
    control(
        "bootstrap_warning",
        40,
        "250-status/bootstrap-phase=WARN BOOTSTRAP PROGRESS=10 TAG=conn_done SUMMARY=\"x\" REASON=TIMEOUT HOSTID=\"AB\" HOSTADDR=\"192.0.2.1:443\"\r\n250 OK\r\n".to_owned(),
    );
    control(
        "add_onion_new",
        64,
        format!("250-ServiceID={id}\r\n250-PrivateKey=ED25519-V3:{key}\r\n250 OK\r\n"),
    );
    control(
        "add_onion_existing",
        5,
        format!("250-ServiceID={id}\r\n250 OK\r\n"),
    );
    control(
        "errors",
        9,
        "512 Tor is in non-anonymous hidden service mode\r\n510 Command filtered\r\n552 Unknown Onion Service id\r\n515 Authentication failed: x\r\n".to_owned(),
    );

    let mut socks = |name: &str, reply: Vec<u8>| {
        seeds.push((format!("socks_reply/{name}.bin"), reply));
    };
    socks("ipv4_success", vec![5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    let mut ipv6 = vec![5, 0, 0, 4];
    ipv6.extend_from_slice(&[0; 18]);
    socks("ipv6_success", ipv6);
    let mut domain = vec![5, 0, 0, 3, 5];
    domain.extend_from_slice(b"a.b.c");
    domain.extend_from_slice(&[0x71, 0xF2, b'p', b'e', b'e', b'r']);
    socks("domain_success_then_data", domain);
    socks("onion_failure", vec![5, 0xF2, 0, 1, 0, 0, 0, 0, 0, 0]);
    socks("truncated", vec![5, 0, 0, 1, 0, 0]);
    seeds
}

fn seed_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/seeds")
}

#[test]
fn committed_tor_seeds_are_current() {
    let root = seed_root();
    let write = std::env::var_os("MONOLITH_WRITE_FUZZ_SEEDS").is_some();
    let expected = seeds();
    for (path, content) in &expected {
        let file = root.join(path);
        if write {
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(&file, content).unwrap();
        }
        let on_disk = fs::read(&file).unwrap_or_else(|error| panic!("seed {path}: {error}"));
        assert!(
            on_disk == *content,
            "seed {path} is out of date; see the top of this file"
        );
    }
    let named: BTreeSet<&str> = expected
        .iter()
        .map(|(path, _)| path.split('/').next().unwrap())
        .collect();
    assert_eq!(named, BTreeSet::from(TARGETS));
    let on_disk: usize = TARGETS
        .iter()
        .map(|target| fs::read_dir(root.join(target)).unwrap().count())
        .sum();
    assert_eq!(
        on_disk,
        expected.len(),
        "stray or missing files in fuzz/seeds"
    );
}

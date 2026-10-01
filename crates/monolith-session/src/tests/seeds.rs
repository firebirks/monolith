//! Seed inputs for the session fuzz targets in `fuzz/`.
//!
//! A fuzzer cannot produce a handshake message or an encrypted frame by
//! chance: it would have to compute X25519. The seeds give each target the
//! genuine messages of the handshake between the two fixed parties, damaged
//! variants of them, and operation lists for the frame target. They are
//! built here and committed under `fuzz/seeds/<target>/`.
//!
//! The test fails if a committed seed is not what the current code
//! produces. To write the seeds again:
//!
//!     MONOLITH_WRITE_FUZZ_SEEDS=1 cargo test -p monolith-session seeds

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use monolith_protocol::body::Message;
use monolith_protocol::session::{PeerRecord, Standing};

use crate::testing::{ALICE, BOB, card, chat, handshake, party, request_with, start};

/// The targets this file owns seeds for.
const TARGETS: [&str; 3] = [
    "handshake_responder",
    "handshake_initiator",
    "session_frames",
];

/// Every seed: path below `fuzz/seeds`, and content.
fn seeds() -> Vec<(String, Vec<u8>)> {
    let mut seeds: Vec<(String, Vec<u8>)> = Vec::new();
    let mut add = |path: &str, content: Vec<u8>| seeds.push((path.to_owned(), content));
    let alice_card = card(ALICE);

    // The handshake between the two fixed parties, which the targets
    // reproduce, and frames from sessions that grew out of it.
    let (_, _, transcript) = handshake(&party(ALICE), &party(BOB));

    let request_from_alice = {
        let (outbound, _, _) = handshake(&party(ALICE), &party(BOB));
        let (mut alice, _) = outbound.admit(Standing::Requested).unwrap();
        alice
            .send(&request_with(alice_card.clone(), None), start())
            .unwrap()
    };
    let (accept_from_alice, accept_from_bob, chat_from_bob) = {
        let (outbound, inbound, _) = handshake(&party(ALICE), &party(BOB));
        let (mut alice, _) = outbound.admit(Standing::Accepted).unwrap();
        let (mut bob, _, _) = inbound.admit(PeerRecord::Accepted(&alice_card)).unwrap();
        let from_alice = alice.send(&Message::ContactAccept, start()).unwrap();
        let from_bob = bob.send(&Message::ContactAccept, start()).unwrap();
        bob.receive(&from_alice, start()).unwrap();
        let chat = bob.send(&chat("hello"), start()).unwrap();
        (from_alice, from_bob, chat)
    };

    // handshake_responder. Odd mode: piece size, mode, damage position
    // (2), damage value, tail. The stream is message 1, message 3, tail.
    let responder = |piece: u8, position: u16, damage: u8, tail: &[u8]| {
        let mut input = vec![piece, 1];
        input.extend_from_slice(&position.to_be_bytes());
        input.push(damage);
        input.extend_from_slice(tail);
        input
    };
    add("handshake_responder/genuine.bin", responder(47, 0, 0, &[]));
    add(
        "handshake_responder/genuine_then_request.bin",
        responder(200, 0, 0, &request_from_alice),
    );
    add(
        "handshake_responder/genuine_then_accept.bin",
        responder(5, 0, 0, &accept_from_alice),
    );
    add(
        "handshake_responder/damaged_ephemeral.bin",
        responder(47, 3, 0x40, &[]),
    );
    add(
        "handshake_responder/damaged_first_tag.bin",
        responder(47, 40, 0x01, &[]),
    );
    add(
        "handshake_responder/damaged_transport_key.bin",
        responder(47, 60, 0x80, &[]),
    );
    add(
        "handshake_responder/damaged_card.bin",
        responder(47, 200, 0x01, &[]),
    );
    // Even mode: piece size, mode, the stream as it is.
    let mut raw = vec![255, 0];
    raw.extend_from_slice(&transcript.message_1);
    raw.extend_from_slice(&transcript.message_3);
    add("handshake_responder/raw_genuine.bin", raw);
    let mut raw = vec![9, 0];
    raw.extend_from_slice(&transcript.message_1);
    raw.extend_from_slice(&[0; 235]);
    add("handshake_responder/raw_first_message_then_zeros.bin", raw);
    let mut raw = vec![0, 0];
    raw.extend_from_slice(&[0; 48]);
    add("handshake_responder/raw_zeros.bin", raw);

    // handshake_initiator. The same layout. The stream is message 2, tail.
    let initiator = responder;
    add("handshake_initiator/genuine.bin", initiator(47, 0, 0, &[]));
    add(
        "handshake_initiator/genuine_then_accept.bin",
        initiator(200, 0, 0, &accept_from_bob),
    );
    let mut accept_then_chat = accept_from_bob.clone();
    accept_then_chat.extend_from_slice(&chat_from_bob);
    add(
        "handshake_initiator/genuine_then_accept_and_chat.bin",
        initiator(7, 0, 0, &accept_then_chat),
    );
    add(
        "handshake_initiator/damaged_ephemeral.bin",
        initiator(47, 5, 0x10, &[]),
    );
    add(
        "handshake_initiator/damaged_tag.bin",
        initiator(47, 47, 0x01, &[]),
    );
    let mut raw = vec![255, 0];
    raw.extend_from_slice(&transcript.message_2);
    add("handshake_initiator/raw_genuine.bin", raw);
    let mut raw = vec![3, 0];
    raw.extend_from_slice(&transcript.message_1);
    add("handshake_initiator/raw_reflected_first_message.bin", raw);

    // session_frames: piece selector, then operations. See the target for
    // the codes.
    for (name, input) in [
        // Alice sends, Bob receives, Bob sends a longer one, Alice
        // receives, two more from Alice in order.
        ("exchange", vec![3, 0, 5, 2, 1, 200, 3, 0, 90, 0, 1, 2, 2]),
        // A frame to Bob is changed; the next one is not delivered either.
        ("tampered", vec![0, 0, 5, 4, 0, 100, 7, 0, 3, 2]),
        // The length prefix of a frame is changed.
        ("tampered_length", vec![0, 0, 5, 4, 0, 0, 12, 0, 3, 2]),
        // The first of two frames is dropped.
        ("dropped", vec![1, 0, 1, 0, 2, 6, 2]),
        // A frame arrives twice.
        ("repeated", vec![1, 0, 1, 8, 0, 2, 2]),
        // Four bytes that look like the start of a frame, then a real one.
        ("injected", vec![1, 10, 4, 0x04, 0x10, 0xAA, 0xBB, 0, 1, 2]),
        // Alice closes; frames that cross the Close are dropped.
        ("closed", vec![2, 0, 1, 12, 2, 2, 1, 5, 3, 13]),
        // Both close at the same time.
        ("crossing_close", vec![2, 12, 13, 2, 3]),
    ] {
        add(&format!("session_frames/{name}.bin"), input);
    }

    seeds
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
        assert!(
            on_disk == *content,
            "seed {path} is out of date; see the top of this file"
        );
    }

    // Nothing else lives in the seed directories of these targets.
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

#[test]
fn handshake_seeds_are_what_their_names_say() {
    let all = seeds();
    let (_, _, transcript) = handshake(&party(ALICE), &party(BOB));
    let mut genuine_for_responder = transcript.message_1.to_vec();
    genuine_for_responder.extend_from_slice(&transcript.message_3);

    for (path, content) in &all {
        let Some((target, name)) = path.split_once('/') else {
            panic!("{path}");
        };
        let genuine: &[u8] = match target {
            "handshake_responder" => &genuine_for_responder,
            "handshake_initiator" => &transcript.message_2,
            _ => continue,
        };
        // The stream as the target builds it.
        let stream = if content[1] % 2 == 0 {
            content[2..].to_vec()
        } else {
            let position = usize::from(u16::from_be_bytes([content[2], content[3]]));
            let mut stream = genuine.to_vec();
            stream[position % genuine.len()] ^= content[4];
            stream.extend_from_slice(&content[5..]);
            stream
        };
        // A seed leads to a session exactly when its name says "genuine".
        assert_eq!(
            stream.starts_with(genuine),
            name.contains("genuine"),
            "{path}"
        );
    }
}

#[test]
fn frame_seeds_are_complete_operation_lists() {
    // Every operation in a seed has all of its arguments, so no seed ends
    // in the middle of one.
    for (path, content) in seeds() {
        if !path.starts_with("session_frames/") {
            continue;
        }
        let mut rest = &content[1..];
        let mut operations = 0;
        while let Some((operation, tail)) = rest.split_first() {
            let arguments = match (operation % 14) / 2 {
                0 => 1,
                2 => 3,
                5 => 1 + usize::from(tail[0]),
                _ => 0,
            };
            assert!(tail.len() >= arguments, "{path} is cut short");
            rest = &tail[arguments..];
            operations += 1;
        }
        assert!(operations >= 3, "{path}");
    }
}

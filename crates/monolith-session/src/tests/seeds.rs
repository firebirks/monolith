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
use monolith_protocol::credential::Credentials;
use monolith_protocol::session::{PeerRecord, Standing};

use crate::testing::{
    ALICE, BOB, EPHEMERAL_R, admit_outbound, card, chat, handshake, party, request_with, start,
};
use crate::{HandshakeInitiator, HandshakeResponder};

/// An initiator other than Alice, with an ephemeral key of its own.
const CAROL: u8 = 0x33;

/// An ephemeral key of Bob other than the one of the transcript.
const OTHER_EPHEMERAL: [u8; 32] = [0x44; 32];

/// The first and third message of a handshake that Carol makes with Bob,
/// who answers with the ephemeral key of the transcript: valid, and not
/// the stored transcript.
fn carol_to_bob() -> ([u8; 48], [u8; 235]) {
    let (carol, message_1) =
        HandshakeInitiator::start_with_ephemeral(&party(CAROL), &card(BOB), start(), [0x33; 32])
            .unwrap();
    let bob = HandshakeResponder::new_with_ephemeral(&party(BOB), start(), EPHEMERAL_R).unwrap();
    let (_, message_2) = bob.read_message_1(&message_1, start()).unwrap();
    let (_, message_3) = carol.read_message_2(&message_2, start()).unwrap();
    (message_1, message_3)
}

/// Bob's second message to Alice's first, made with another ephemeral key:
/// valid, and not the stored transcript.
fn bob_answers_otherwise(message_1: &[u8; 48]) -> [u8; 48] {
    let bob =
        HandshakeResponder::new_with_ephemeral(&party(BOB), start(), OTHER_EPHEMERAL).unwrap();
    bob.read_message_1(message_1, start()).unwrap().1
}

/// The targets this file owns seeds for.
const TARGETS: [&str; 3] = [
    "handshake_responder",
    "handshake_initiator",
    "session_frames",
];

/// The records the responder target can hold of the initiator, as the
/// two high bits of the first input byte.
const NO_RECORD: u8 = 0;
const BLOCKED: u8 = 1;
const REQUESTED: u8 = 2;
const ACCEPTED: u8 = 3;

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
        let (mut alice, _) = admit_outbound(outbound, Standing::Requested);
        alice
            .send(&request_with(alice_card.clone(), None), start())
            .unwrap()
    };
    let (accept_from_alice, chat_from_alice, close_from_alice, accept_from_bob, chat_from_bob) = {
        let (outbound, inbound, _) = handshake(&party(ALICE), &party(BOB));
        let (mut alice, _) = admit_outbound(outbound, Standing::Accepted);
        let (mut bob, _, _) = inbound
            .admit(PeerRecord::Accepted(&mut Credentials::new(
                alice_card.clone(),
            )))
            .unwrap();
        let from_alice = alice.send(&Message::ContactAccept, start()).unwrap();
        let from_bob = bob.send(&Message::ContactAccept, start()).unwrap();
        bob.receive(&from_alice, start()).unwrap();
        alice.receive(&from_bob, start()).unwrap();
        let chat_from_bob = bob.send(&chat("hello"), start()).unwrap();
        let chat_from_alice = alice.send(&chat("hello"), start()).unwrap();
        let close_from_alice = alice.close().unwrap();
        (
            from_alice,
            chat_from_alice,
            close_from_alice,
            from_bob,
            chat_from_bob,
        )
    };
    // Both sides imported the other's card and ask. What Alice sends: her
    // request, her acceptance once Bob's request arrived, and a chat
    // message once Bob's acceptance confirmed the session.
    let both_ask_from_alice = {
        let (outbound, inbound, _) = handshake(&party(ALICE), &party(BOB));
        let (mut alice, _) = admit_outbound(outbound, Standing::Requested);
        let (mut bob, _, _) = inbound
            .admit(PeerRecord::Requested(&mut Credentials::new(
                alice_card.clone(),
            )))
            .unwrap();
        let request_from_alice = alice
            .send(&request_with(alice_card.clone(), None), start())
            .unwrap();
        let request_from_bob = bob.send(&request_with(card(BOB), None), start()).unwrap();
        alice.receive(&request_from_bob, start()).unwrap();
        bob.receive(&request_from_alice, start()).unwrap();
        let accept_from_alice = alice.send(&Message::ContactAccept, start()).unwrap();
        let accept_from_bob = bob.send(&Message::ContactAccept, start()).unwrap();
        alice.receive(&accept_from_bob, start()).unwrap();
        let chat_from_alice = alice.send(&chat("hello"), start()).unwrap();
        [request_from_alice, accept_from_alice, chat_from_alice].concat()
    };

    // handshake_responder. First byte: the record the responder holds in
    // the two high bits, the piece size less one in the low six. Odd
    // mode: first byte, mode, damage position (2), damage value, tail.
    // The stream is message 1, message 3, tail.
    let first = |record: u8, piece: u8| {
        assert!(record < 4 && piece < 64);
        (record << 6) | piece
    };
    let responder = |record: u8, piece: u8, position: u16, damage: u8, tail: &[u8]| {
        let mut input = vec![first(record, piece), 1];
        input.extend_from_slice(&position.to_be_bytes());
        input.push(damage);
        input.extend_from_slice(tail);
        input
    };
    add(
        "handshake_responder/genuine.bin",
        responder(NO_RECORD, 47, 0, 0, &[]),
    );
    add(
        "handshake_responder/genuine_then_request.bin",
        responder(NO_RECORD, 63, 0, 0, &request_from_alice),
    );
    add(
        "handshake_responder/genuine_then_accept.bin",
        responder(NO_RECORD, 5, 0, 0, &accept_from_alice),
    );
    add(
        "handshake_responder/genuine_blocked_then_request.bin",
        responder(BLOCKED, 30, 0, 0, &request_from_alice),
    );
    add(
        "handshake_responder/genuine_requested_then_accept.bin",
        responder(REQUESTED, 11, 0, 0, &accept_from_alice),
    );
    add(
        "handshake_responder/genuine_requested_then_request_accept_chat.bin",
        responder(REQUESTED, 63, 0, 0, &both_ask_from_alice),
    );
    add(
        "handshake_responder/genuine_accepted_then_request.bin",
        responder(ACCEPTED, 2, 0, 0, &request_from_alice),
    );
    add(
        "handshake_responder/genuine_accepted_then_accept_chat_close.bin",
        responder(
            ACCEPTED,
            40,
            0,
            0,
            &[
                accept_from_alice.as_slice(),
                chat_from_alice.as_slice(),
                close_from_alice.as_slice(),
            ]
            .concat(),
        ),
    );
    add(
        "handshake_responder/damaged_ephemeral.bin",
        responder(NO_RECORD, 47, 3, 0x40, &[]),
    );
    add(
        "handshake_responder/damaged_first_tag.bin",
        responder(NO_RECORD, 47, 40, 0x01, &[]),
    );
    add(
        "handshake_responder/damaged_transport_key.bin",
        responder(NO_RECORD, 47, 60, 0x80, &[]),
    );
    add(
        "handshake_responder/damaged_card.bin",
        responder(NO_RECORD, 47, 200, 0x01, &[]),
    );
    add(
        "handshake_responder/damaged_card_of_a_contact.bin",
        responder(ACCEPTED, 47, 200, 0x01, &accept_from_alice),
    );
    // Even mode: first byte, mode, the stream as it is.
    let mut raw = vec![first(NO_RECORD, 63), 0];
    raw.extend_from_slice(&transcript.message_1);
    raw.extend_from_slice(&transcript.message_3);
    add("handshake_responder/raw_genuine.bin", raw);
    let mut raw = vec![first(NO_RECORD, 9), 0];
    raw.extend_from_slice(&transcript.message_1);
    raw.extend_from_slice(&[0; 235]);
    add("handshake_responder/raw_first_message_then_zeros.bin", raw);
    let mut raw = vec![first(NO_RECORD, 0), 0];
    raw.extend_from_slice(&[0; 48]);
    add("handshake_responder/raw_zeros.bin", raw);
    // Another initiator: valid messages that are not the transcript.
    let (message_1, message_3) = carol_to_bob();
    let mut raw = vec![first(ACCEPTED, 63), 0];
    raw.extend_from_slice(&message_1);
    raw.extend_from_slice(&message_3);
    add("handshake_responder/raw_valid_other_initiator.bin", raw);

    // handshake_initiator. Odd mode: piece size less one, mode, damage
    // position (2), damage value, tail. The stream is message 2, tail.
    let initiator = |piece: u8, position: u16, damage: u8, tail: &[u8]| {
        let mut input = vec![piece, 1];
        input.extend_from_slice(&position.to_be_bytes());
        input.push(damage);
        input.extend_from_slice(tail);
        input
    };
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
    // Bob with another ephemeral key: a valid second message that is not
    // the transcript.
    let mut raw = vec![255, 0];
    raw.extend_from_slice(&bob_answers_otherwise(&transcript.message_1));
    add("handshake_initiator/raw_valid_other_ephemeral.bin", raw);

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
        // At 24 hours Alice cannot send any more. Her Close still goes
        // out and arrives.
        ("expired_send", vec![1, 14, 24, 0, 5, 12, 2]),
        // A message sent in time and a Close sent at the age limit arrive
        // one second before the grace ends.
        (
            "close_within_grace",
            vec![1, 0, 1, 14, 24, 12, 15, 119, 2, 2],
        ),
        // A frame arrives when the grace has ended: a violation, without
        // any interference. Bob's session is over.
        (
            "expired_receive",
            vec![1, 0, 1, 14, 24, 15, 120, 2, 1, 1, 3, 13],
        ),
        // Alice blocks Bob; Bob sees a Close like any other.
        ("blocked", vec![2, 0, 1, 16, 2, 2, 1, 5, 3, 17]),
        // Alice removes the contact while a frame from Bob is on its way.
        ("removed", vec![2, 1, 3, 18, 2, 3, 19]),
        // Alice's stream is gone: what was on its way to her is dropped,
        // what she sent before still arrives.
        ("stream_closed", vec![2, 0, 1, 1, 1, 20, 3, 2, 0, 1, 21]),
        // Bob withdraws the session while a message from Alice is on its
        // way; it is not delivered, and Alice sees an ordinary Close.
        ("withdrawn", vec![2, 0, 1, 23, 2, 3, 0, 1, 2]),
        // The first five bytes of a frame reach Bob, then a minute passes:
        // the frame did not complete in time and his session ends
        // silently; the rest is not taken.
        ("partial_frame_expired", vec![1, 0, 1, 26, 4, 15, 61, 25, 2]),
        // Bob hears nothing complete for the idle limit.
        ("idle_expired", vec![1, 0, 1, 2, 25, 3]),
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
fn seeds_named_valid_complete_a_handshake_that_is_not_the_transcript() {
    // The fuzz targets accept these, and must not assume that only the
    // stored transcript is valid.
    let (_, _, transcript) = handshake(&party(ALICE), &party(BOB));
    let (message_1, message_3) = carol_to_bob();
    assert_ne!(message_1, transcript.message_1);
    let bob = HandshakeResponder::new_with_ephemeral(&party(BOB), start(), EPHEMERAL_R).unwrap();
    let (waiting, _) = bob.read_message_1(&message_1, start()).unwrap();
    let inbound = waiting.read_message_3(&message_3, start()).unwrap();
    assert_eq!(inbound.card(), &card(CAROL));

    let message_2 = bob_answers_otherwise(&transcript.message_1);
    assert_ne!(message_2, transcript.message_2);
    let (alice, _) = HandshakeInitiator::start_with_ephemeral(
        &party(ALICE),
        &card(BOB),
        start(),
        crate::testing::EPHEMERAL_I,
    )
    .unwrap();
    let (outbound, message_3) = alice.read_message_2(&message_2, start()).unwrap();
    assert_eq!(outbound.card(), &card(BOB));
    assert_ne!(message_3, transcript.message_3);
}

#[test]
fn responder_seeds_cover_every_record() {
    // Among the seeds whose handshake completes, each of the four records
    // appears.
    let records: BTreeSet<u8> = seeds()
        .iter()
        .filter(|(path, _)| path.starts_with("handshake_responder/genuine"))
        .map(|(_, content)| content[0] >> 6)
        .collect();
    assert_eq!(
        records,
        BTreeSet::from([NO_RECORD, BLOCKED, REQUESTED, ACCEPTED])
    );
}

#[test]
fn frame_seeds_are_complete_operation_lists() {
    // Every operation in a seed has all of its arguments, so no seed ends
    // in the middle of one, and every operation of the target appears in
    // some seed.
    let mut seen = BTreeSet::new();
    for (path, content) in seeds() {
        if !path.starts_with("session_frames/") {
            continue;
        }
        let mut rest = &content[1..];
        let mut operations = 0;
        while let Some((operation, tail)) = rest.split_first() {
            let kind = (operation % 28) / 2;
            let arguments = match kind {
                0 | 7 | 13 => 1,
                2 => 3,
                5 => 1 + usize::from(tail[0]),
                _ => 0,
            };
            assert!(tail.len() >= arguments, "{path} is cut short");
            rest = &tail[arguments..];
            operations += 1;
            seen.insert(kind);
        }
        assert!(operations >= 3, "{path}");
    }
    assert_eq!(seen, (0..14).collect::<BTreeSet<u8>>());
}

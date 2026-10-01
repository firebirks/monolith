//! Seed inputs for the fuzz targets in `fuzz/`.
//!
//! A fuzzer that starts from nothing spends its time in the paths that
//! reject: it cannot produce a signed card or a padded frame by chance. The
//! seeds are valid inputs for each target, built here from fixed values, and
//! committed under `fuzz/seeds/<target>/`.
//!
//! The test fails if a committed seed is not what the current code
//! produces, which happens when a format or the input layout of a target
//! changes. To write the seeds again:
//!
//!     MONOLITH_WRITE_FUZZ_SEEDS=1 cargo test -p monolith-protocol --test fuzz_seeds

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

use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey, base32};
use monolith_protocol::body::{
    AuthProof, ContactRequest, FileChunk, Message, MessageId, TransferId,
};
use monolith_protocol::card::{ContactCard, EndpointSet, InvitationCapability};
use monolith_protocol::frame::{FrameParams, decode_plaintext, encode_outer, encode_plaintext};
use monolith_protocol::text::{ChatText, DisplayName, Filename, IntroductionText, ProfileText};
use monolith_protocol::{MessageType, SessionState};

const PARAMS: FrameParams = FrameParams::PROVISIONAL;

/// Seed of the identity in the seeds: RFC 8032 test vector 1, as in the
/// known-answer test of the contact card.
const IDENTITY_SEED: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];

/// Session states in the order the `frame_plaintext` target selects them.
const STATES: [SessionState; 7] = [
    SessionState::Connecting,
    SessionState::CryptoHandshake,
    SessionState::IdentityAuth,
    SessionState::AuthenticatedUnknown,
    SessionState::AuthenticatedContact,
    SessionState::Closing,
    SessionState::Closed,
];

fn secret() -> IdentitySecretKey {
    IdentitySecretKey::from_seed(&IDENTITY_SEED)
}

fn card(epoch: u64, invitation: bool) -> ContactCard {
    let endpoint = OnionServiceKey::from_bytes(
        IdentitySecretKey::from_seed(&[2; 32])
            .public_key()
            .as_bytes(),
    )
    .unwrap();
    ContactCard::sign(
        &secret(),
        EndpointEpoch::new(epoch).unwrap(),
        EndpointSet::single(endpoint),
        invitation.then(|| InvitationCapability::from_bytes([0xC4; 16])),
    )
    .unwrap()
}

/// One message of every type, in the order of `MessageType::ALL`.
fn messages() -> Vec<Message> {
    let transfer = TransferId::from_bytes([0x33; 16]);
    let id = MessageId::from_bytes([0x44; 16]);
    let all = vec![
        Message::AuthProof(Box::new(AuthProof {
            identity: secret().public_key(),
            features: 0,
            signature: secret().sign(b"stands in for a real proof"),
        })),
        Message::Close,
        Message::Ping([1; 8]),
        Message::Pong([1; 8]),
        Message::ContactRequest(Box::new(ContactRequest {
            card: card(1, false),
            invitation: Some(InvitationCapability::from_bytes([5; 16])),
            display_name: DisplayName::new("Alice").unwrap(),
            introduction: IntroductionText::new("We met at the conference.").unwrap(),
        })),
        Message::ContactAccept,
        Message::ChatMessage {
            id,
            text: ChatText::new("hello\nworld").unwrap(),
        },
        Message::MessageAck(id),
        Message::Profile {
            display_name: DisplayName::new("Alice").unwrap(),
            profile_text: ProfileText::new("away until Monday").unwrap(),
        },
        Message::EndpointUpdate(Box::new(card(2, false))),
        Message::FileOffer {
            transfer,
            size: 12_345,
            filename: Filename::new("notes.txt").unwrap(),
        },
        Message::FileAccept(transfer),
        Message::FileReject(transfer),
        Message::FileChunk(FileChunk::new(transfer, vec![0xAB; 100]).unwrap()),
        Message::FileComplete {
            transfer,
            digest: [0x55; 32],
        },
        Message::FileAbort(transfer),
    ];
    let types: Vec<MessageType> = all.iter().map(Message::message_type).collect();
    assert_eq!(types, MessageType::ALL);
    all
}

/// `ChatMessage` becomes `chat_message`.
fn snake_case(message_type: MessageType) -> String {
    let mut name = String::new();
    for character in format!("{message_type:?}").chars() {
        if character.is_ascii_uppercase() && !name.is_empty() {
            name.push('_');
        }
        name.push(character.to_ascii_lowercase());
    }
    name
}

/// The state in which a message of this type is legal, as an index into
/// [`STATES`].
fn legal_state(message_type: MessageType) -> u8 {
    let index = STATES
        .iter()
        .position(|state| message_type.may_be_received_in(*state))
        .unwrap();
    u8::try_from(index).unwrap()
}

/// Every seed: path below `fuzz/seeds`, and content.
fn seeds() -> Vec<(String, Vec<u8>)> {
    let mut seeds: Vec<(String, Vec<u8>)> = Vec::new();
    let mut add = |path: &str, content: Vec<u8>| seeds.push((path.to_owned(), content));

    // base32 and the two card targets take their input as it is.
    let known = card(1, false);
    let with_invitation = card(2, true);
    add("contact_card/known.bin", known.encode());
    add("contact_card/invitation.bin", with_invitation.encode());

    let text = known.to_text();
    let wrapped: String = with_invitation
        .to_text()
        .to_ascii_lowercase()
        .as_bytes()
        .chunks(40)
        .map(|line| format!("  {}\n", core::str::from_utf8(line).unwrap()))
        .collect();
    add("contact_card_text/known.txt", text.into_bytes());
    add("contact_card_text/wrapped_lower.txt", wrapped.into_bytes());

    add(
        "base32/card.txt",
        base32::encode(&known.encode()).into_bytes(),
    );
    add("base32/short_lower.txt", b"mfrgg".to_vec());

    // message_body: type selector, body.
    // frame_plaintext, odd mode: state, mode, type selector, damage
    // position (2), damage value, body.
    for (index, message) in messages().iter().enumerate() {
        let selector = u8::try_from(index).unwrap();
        let name = snake_case(message.message_type());
        let body = message.encode_body().unwrap();

        let mut input = vec![selector];
        input.extend_from_slice(&body);
        add(&format!("message_body/{index:02}_{name}.bin"), input);

        let mut input = vec![legal_state(message.message_type()), 1, selector, 0, 0, 0];
        input.extend_from_slice(&body);
        add(&format!("frame_plaintext/{index:02}_{name}.bin"), input);
    }

    // frame_plaintext, even mode: state, mode, plaintext.
    let ping = encode_plaintext(&PARAMS, MessageType::Ping, &[1; 8]).unwrap();
    let mut input = vec![4, 0];
    input.extend_from_slice(&ping);
    add("frame_plaintext/raw_ping.bin", input);

    // frame_stream, even mode: state, piece size, mode, stream.
    let mut input = vec![0, 62, 0];
    for blocks in [1, 2] {
        let payload = vec![0x5A; blocks * PARAMS.padding_block() + PARAMS.overhead()];
        input.extend_from_slice(&encode_outer(&PARAMS, &payload).unwrap());
    }
    add("frame_stream/raw_two_frames.bin", input);

    // frame_stream, odd mode: state, piece size, mode, damage position (2),
    // damage value, records of 48 bytes. The first record is a chat message
    // whose body is exactly 47 bytes, the second a Ping.
    let chat = Message::ChatMessage {
        id: MessageId::from_bytes([0x44; 16]),
        text: ChatText::new(&"a".repeat(29)).unwrap(),
    }
    .encode_body()
    .unwrap();
    assert_eq!(chat.len(), 47);
    let mut input = vec![0, 6, 1, 0, 0, 0, 6];
    input.extend_from_slice(&chat);
    input.push(2);
    input.extend_from_slice(&[1; 8]);
    add("frame_stream/records_chat_ping.bin", input);
    add("frame_stream/records_accept.bin", vec![1, 0, 1, 0, 0, 0, 5]);

    // text_fields takes its input as it is.
    for (name, text) in [
        ("chat", "hello\nworld\tindented"),
        ("display_name", "Jos\u{e9} Mar\u{ed}a"),
        ("filename", "report.tar.gz"),
        ("device_name", "con .tar.gz"),
        ("device_port", "LPT\u{b9}.txt"),
        ("reserved_characters", "a<b>c:d|e?f*g\"h"),
        ("bidi_override", "photo\u{202e}gpj.exe"),
        ("hangul_filler", "a\u{3164}b"),
        ("not_nfc", "Jose\u{301}"),
        ("dots", ". ."),
    ] {
        add(&format!("text_fields/{name}.txt"), text.as_bytes().to_vec());
    }

    // session_sequence: progress, standing, events. Events 0 to 15 are the
    // message types, 16 to 19 are close, block, remove and stream closed;
    // the high bit asks for a card of another identity.
    for (name, input) in [
        ("contact_confirmed", vec![3, 4, 5, 6, 9, 2, 1]),
        ("stranger_request", vec![3, 0, 4, 6]),
        ("stranger_accept", vec![3, 0, 5]),
        ("requested_accepts", vec![3, 3, 5, 6, 8]),
        ("crossing_requests", vec![3, 3, 4, 5, 6]),
        ("foreign_request", vec![3, 3, 0x84]),
        ("foreign_update", vec![3, 4, 5, 0x89]),
        ("blocked_while_open", vec![3, 4, 5, 6, 17, 6]),
        ("removed_while_open", vec![3, 4, 5, 18, 5]),
        ("local_close", vec![3, 4, 5, 16, 6, 19]),
        ("before_the_proof", vec![2, 4, 6]),
        ("proof_then_chat", vec![2, 4, 0, 6]),
        ("before_the_handshake", vec![1, 0, 0]),
    ] {
        add(&format!("session_sequence/{name}.bin"), input);
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

    // Nothing else lives in the seed directories.
    let mut on_disk = 0;
    for target in fs::read_dir(&root).unwrap() {
        on_disk += fs::read_dir(target.unwrap().path()).unwrap().count();
    }
    assert_eq!(
        on_disk,
        expected.len(),
        "stray or missing files in fuzz/seeds"
    );
}

#[test]
fn seeds_are_accepted_by_what_they_are_meant_for() {
    // A seed that the decoder rejects would not get the fuzzer anywhere.
    let all = seeds();
    let find = |prefix: &str| -> Vec<&(String, Vec<u8>)> {
        all.iter()
            .filter(|(path, _)| path.starts_with(prefix))
            .collect()
    };

    for (path, content) in find("contact_card/") {
        assert!(ContactCard::decode(content).is_ok(), "{path}");
    }
    for (path, content) in find("contact_card_text/") {
        let text = core::str::from_utf8(content).unwrap();
        assert!(ContactCard::from_text(text).is_ok(), "{path}");
    }
    for (path, content) in find("base32/") {
        let text = core::str::from_utf8(content).unwrap();
        assert!(base32::decode(text, 256).is_ok(), "{path}");
    }

    let message_bodies = find("message_body/");
    assert_eq!(message_bodies.len(), MessageType::ALL.len());
    for (path, content) in message_bodies {
        let message_type = MessageType::ALL[usize::from(content[0])];
        assert!(
            Message::decode(message_type, &content[1..]).is_ok(),
            "{path}"
        );
    }

    for (path, content) in find("frame_plaintext/") {
        let state = STATES[usize::from(content[0])];
        let plaintext = if content[1] % 2 == 0 {
            content[2..].to_vec()
        } else {
            assert_eq!(&content[3..6], &[0, 0, 0], "{path} is damaged");
            let message_type = MessageType::ALL[usize::from(content[2])];
            encode_plaintext(&PARAMS, message_type, &content[6..]).unwrap()
        };
        let (message_type, body) = decode_plaintext(&PARAMS, state, &plaintext)
            .unwrap_or_else(|error| panic!("{path}: {error:?}"));
        assert!(Message::decode(message_type, body).is_ok(), "{path}");
    }
}

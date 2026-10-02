//! Property tests of the protocol core.
//!
//! These check statements that must hold for every input, not for chosen
//! examples: decoders are canonical, encoders round-trip, the frame decoder
//! stays bounded, the session logic never hands application data to the
//! caller before a session is confirmed, and identities that are not
//! contacts cannot tell their cases apart.
//!
//! Random bytes almost never get past the first check of a decoder. The
//! strategies here therefore start from valid values and damage them, or
//! draw from pools that hold the characters a rule is about.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::sync::LazyLock;

use monolith_identity::{
    EndpointEpoch, IdentityPublicKey, IdentitySecretKey, OnionServiceKey, TransportPublicKey,
    base32,
};
use monolith_protocol::body::{ContactRequest, FileChunk, Message, MessageId, TransferId};
use monolith_protocol::card::{
    CardChange, ContactCard, EndpointSet, InvitationCapability, evaluate_card,
};
use monolith_protocol::credential::{CredentialChange, Credentials};
use monolith_protocol::duplicate::{
    Contender, Initiator, ProbeOutcome, Resolution, after_probe, resolve,
};
use monolith_protocol::frame::{
    FrameParams, OuterDecoder, decode_plaintext, encode_outer, encode_plaintext,
};
use monolith_protocol::limits::{
    MAX_DISPLAY_NAME_LEN, MAX_DISPLAY_NAME_SCALARS, MAX_FILE_SIZE, MAX_FILENAME_LEN,
    MAX_FRAME_CIPHERTEXT_LEN,
};
use monolith_protocol::session::{Action, Admission, PeerRecord, Session, Standing};
use monolith_protocol::text::{ChatText, DisplayName, Filename, IntroductionText, ProfileText};
use monolith_protocol::{MessageType, ProtocolError, SessionState};
use proptest::collection::vec;
use proptest::prelude::*;

const PARAMS: FrameParams = FrameParams::PROVISIONAL;

fn identity(seed: [u8; 32]) -> IdentityPublicKey {
    IdentitySecretKey::from_seed(&seed).public_key()
}

fn endpoint(seed: [u8; 32]) -> OnionServiceKey {
    OnionServiceKey::from_bytes(identity(seed).as_bytes()).unwrap()
}

/// A transport key made from arbitrary bytes: the top bit is cleared, and
/// the few strings that are still not a valid key give `None`. The tests of
/// this crate need no private half for it.
fn transport(mut bytes: [u8; 32]) -> Option<TransportPublicKey> {
    bytes[31] &= 0x7f;
    TransportPublicKey::from_bytes(&bytes).ok()
}

fn arb_card(with_invitation: bool) -> impl Strategy<Value = ContactCard> {
    (
        any::<[u8; 32]>(),
        any::<[u8; 32]>(),
        any::<[u8; 32]>(),
        1..=u64::MAX,
        any::<[u8; 16]>(),
    )
        .prop_filter_map(
            "keys coincide",
            move |(identity_seed, transport_bytes, endpoint_seed, epoch, capability)| {
                ContactCard::sign(
                    &IdentitySecretKey::from_seed(&identity_seed),
                    transport(transport_bytes)?,
                    EndpointEpoch::new(epoch).unwrap(),
                    EndpointSet::single(endpoint(endpoint_seed)),
                    with_invitation.then(|| InvitationCapability::from_bytes(capability)),
                )
                .ok()
            },
        )
}

/// Characters that every text field accepts in the middle of a string. The
/// combining accent makes some of the generated names differ from their
/// normalized forms.
const PLAIN_CHARS: [char; 12] = [
    'a',
    'b',
    'Z',
    '0',
    '9',
    '-',
    '_',
    '.',
    '\u{e9}',
    '\u{4e2d}',
    '\u{1f600}',
    '\u{301}',
];

/// Characters that display names and filenames must reject: controls, line
/// breaks, bidirectional controls, invisible characters, whitespace other
/// than the ordinary space, separators and noncharacters.
const FORBIDDEN_IN_NAMES: [char; 34] = [
    '\u{0}',
    '\t',
    '\n',
    '\r',
    '\u{1b}',
    '\u{7f}',
    '\u{85}',
    '\u{61c}',
    '\u{200f}',
    '\u{202e}',
    '\u{2066}',
    '\u{ad}',
    '\u{200b}',
    '\u{200d}',
    '\u{2060}',
    '\u{2065}',
    '\u{206f}',
    '\u{feff}',
    '\u{115f}',
    '\u{3164}',
    '\u{2800}',
    '\u{fffc}',
    '\u{1d173}',
    '\u{e0001}',
    '\u{fff8}',
    '\u{1bca0}',
    '\u{e0080}',
    '\u{e01f0}',
    '\u{a0}',
    '\u{2003}',
    '\u{3000}',
    '\u{2028}',
    '\u{fdd0}',
    '\u{fffe}',
];

fn plain_char() -> impl Strategy<Value = char> {
    prop::sample::select(PLAIN_CHARS.to_vec())
}

fn arb_name(max_chars: usize) -> impl Strategy<Value = String> {
    vec(plain_char(), 0..=max_chars).prop_map(|chars| chars.into_iter().collect())
}

/// Text made of plain characters and spaces, with at most one character at
/// a random position that names must reject.
fn arb_name_candidate(max_chars: usize) -> impl Strategy<Value = String> {
    let mut pool = PLAIN_CHARS.to_vec();
    pool.extend([' ', ' ']);
    (
        vec(prop::sample::select(pool), 0..=max_chars),
        proptest::option::of((
            any::<prop::sample::Index>(),
            prop::sample::select(FORBIDDEN_IN_NAMES.to_vec()),
        )),
    )
        .prop_map(|(mut chars, insertion)| {
            if let Some((position, character)) = insertion {
                let at = position.index(chars.len() + 1);
                chars.insert(at, character);
            }
            chars.into_iter().collect()
        })
}

fn arb_message() -> impl Strategy<Value = Message> {
    let id = any::<[u8; 16]>();
    prop_oneof![
        Just(Message::Close),
        Just(Message::ContactAccept),
        any::<[u8; 8]>().prop_map(Message::Ping),
        any::<[u8; 8]>().prop_map(Message::Pong),
        (
            arb_card(false),
            proptest::option::of(any::<[u8; 16]>()),
            arb_name(20),
            vec(plain_char(), 0..100)
        )
            .prop_map(|(card, capability, name, intro)| {
                Message::ContactRequest(Box::new(ContactRequest {
                    card,
                    invitation: capability.map(InvitationCapability::from_bytes),
                    display_name: DisplayName::new(&name).unwrap(),
                    introduction: IntroductionText::new(&intro.into_iter().collect::<String>())
                        .unwrap(),
                }))
            }),
        (id, vec(plain_char(), 1..200)).prop_map(|(id, text)| Message::ChatMessage {
            id: MessageId::from_bytes(id),
            text: ChatText::new(&text.into_iter().collect::<String>()).unwrap(),
        }),
        id.prop_map(|id| Message::MessageAck(MessageId::from_bytes(id))),
        (arb_name(20), vec(plain_char(), 0..100)).prop_map(|(name, text)| Message::Profile {
            display_name: DisplayName::new(&name).unwrap(),
            profile_text: ProfileText::new(&text.into_iter().collect::<String>()).unwrap(),
        }),
        arb_card(false).prop_map(|card| Message::EndpointUpdate(Box::new(card))),
        (id, 0..=MAX_FILE_SIZE, vec(plain_char(), 1..40)).prop_filter_map(
            "filename",
            |(id, size, name)| {
                let filename = Filename::new(&name.into_iter().collect::<String>()).ok()?;
                Some(Message::FileOffer {
                    transfer: TransferId::from_bytes(id),
                    size,
                    filename,
                })
            }
        ),
        id.prop_map(|id| Message::FileAccept(TransferId::from_bytes(id))),
        id.prop_map(|id| Message::FileReject(TransferId::from_bytes(id))),
        id.prop_map(|id| Message::FileAbort(TransferId::from_bytes(id))),
        (id, vec(any::<u8>(), 1..2000)).prop_map(|(id, data)| Message::FileChunk(
            FileChunk::new(TransferId::from_bytes(id), data).unwrap()
        )),
        (id, any::<[u8; 32]>()).prop_map(|(id, digest)| Message::FileComplete {
            transfer: TransferId::from_bytes(id),
            digest,
        }),
    ]
}

fn arb_message_type() -> impl Strategy<Value = MessageType> {
    prop::sample::select(MessageType::ALL.to_vec())
}

fn arb_standing() -> impl Strategy<Value = Standing> {
    prop::sample::select(vec![
        Standing::None,
        Standing::Declined,
        Standing::Blocked,
        Standing::StaleCard,
        Standing::PendingSuccessor,
        Standing::Requested,
        Standing::Accepted,
    ])
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/// Seed of the identity of the peer of a test session.
const PEER: [u8; 32] = [0x51; 32];

/// Seed of an identity that is not the peer.
const STRANGER: [u8; 32] = [0x77; 32];

/// A card of the identity of `seed`. `variant` 0 is the card the identity
/// presents in the handshake. Variant 1 is a later card with the same
/// transport key and another endpoint. Variant 2 states another transport
/// key.
fn sample_card(seed: [u8; 32], variant: u8) -> ContactCard {
    let mut endpoint_seed = seed;
    endpoint_seed[0] ^= 0xff;
    endpoint_seed[1] ^= variant;
    let mut transport_bytes = seed;
    if variant == 2 {
        transport_bytes[0] ^= 0x01;
    }
    ContactCard::sign(
        &IdentitySecretKey::from_seed(&seed),
        transport(transport_bytes).unwrap(),
        EndpointEpoch::new(u64::from(variant) + 1).unwrap(),
        EndpointSet::single(endpoint(endpoint_seed)),
        None,
    )
    .unwrap()
}

/// One message of every type, in the order of `MessageType::ALL`. The card
/// inside them is the given one.
fn samples(card: ContactCard) -> Vec<Message> {
    let transfer = TransferId::from_bytes([3; 16]);
    let id = MessageId::from_bytes([4; 16]);
    let all = vec![
        Message::Close,
        Message::Ping([1; 8]),
        Message::Pong([1; 8]),
        Message::ContactRequest(Box::new(ContactRequest {
            card: card.clone(),
            invitation: None,
            display_name: DisplayName::new("Peer").unwrap(),
            introduction: IntroductionText::new("hello").unwrap(),
        })),
        Message::ContactAccept,
        Message::ChatMessage {
            id,
            text: ChatText::new("hi").unwrap(),
        },
        Message::MessageAck(id),
        Message::Profile {
            display_name: DisplayName::new("Peer").unwrap(),
            profile_text: ProfileText::new("").unwrap(),
        },
        Message::EndpointUpdate(Box::new(card)),
        Message::FileOffer {
            transfer,
            size: 1,
            filename: Filename::new("a.txt").unwrap(),
        },
        Message::FileAccept(transfer),
        Message::FileReject(transfer),
        Message::FileChunk(FileChunk::new(transfer, vec![1]).unwrap()),
        Message::FileComplete {
            transfer,
            digest: [5; 32],
        },
        Message::FileAbort(transfer),
    ];
    let types: Vec<MessageType> = all.iter().map(Message::message_type).collect();
    assert_eq!(types, MessageType::ALL);
    all
}

/// The card the peer presents in the handshake of the test sessions.
static PEER_CARD: LazyLock<ContactCard> = LazyLock::new(|| sample_card(PEER, 0));
static FROM_PEER: LazyLock<Vec<Message>> = LazyLock::new(|| samples(PEER_CARD.clone()));
/// Messages of the peer that carry a later card of it, with the same
/// transport key.
static FROM_PEER_LATER: LazyLock<Vec<Message>> = LazyLock::new(|| samples(sample_card(PEER, 1)));
/// Messages of the peer that carry a card of it with another transport key.
static FROM_PEER_REKEYED: LazyLock<Vec<Message>> = LazyLock::new(|| samples(sample_card(PEER, 2)));
static FROM_STRANGER: LazyLock<Vec<Message>> = LazyLock::new(|| samples(sample_card(STRANGER, 0)));
static PEER_IDENTITY: LazyLock<IdentityPublicKey> = LazyLock::new(|| identity(PEER));

/// The seed of the local identity of the test sessions.
const LOCAL: [u8; 32] = [0x10; 32];

/// The local identity of the test sessions.
static LOCAL_IDENTITY: LazyLock<IdentityPublicKey> = LazyLock::new(|| identity(LOCAL));

/// Returns true if the message carries a card that an inbound session with
/// the peer must refuse. This restates `docs/PROTOCOL.md` sections 8.3 and
/// 8.8: the card of a request is the card of the handshake; the card of an
/// endpoint update is any card of the peer's identity.
fn has_foreign_card(message: &Message) -> bool {
    match message {
        Message::ContactRequest(request) => request.card != *PEER_CARD,
        Message::EndpointUpdate(card) => card.identity() != PEER_CARD.identity(),
        _ => false,
    }
}

/// A message as the peer would send it.
///
/// Uniformly chosen message types end almost every session at the first
/// message, because most types are illegal before confirmation. The
/// messages that keep a session going, and the ones that carry a card of
/// another identity, are therefore drawn more often than the rest.
fn arb_session_message() -> impl Strategy<Value = Message> {
    let peer =
        |message_type: MessageType| -> Message { FROM_PEER[type_index(message_type)].clone() };
    let stranger =
        |message_type: MessageType| -> Message { FROM_STRANGER[type_index(message_type)].clone() };
    let later = |message_type: MessageType| -> Message {
        FROM_PEER_LATER[type_index(message_type)].clone()
    };
    let rekeyed = |message_type: MessageType| -> Message {
        FROM_PEER_REKEYED[type_index(message_type)].clone()
    };
    prop_oneof![
        6 => Just(peer(MessageType::ContactAccept)),
        6 => Just(peer(MessageType::ContactRequest)),
        2 => Just(stranger(MessageType::ContactRequest)),
        2 => Just(stranger(MessageType::EndpointUpdate)),
        1 => Just(later(MessageType::ContactRequest)),
        1 => Just(later(MessageType::EndpointUpdate)),
        1 => Just(rekeyed(MessageType::ContactRequest)),
        1 => Just(rekeyed(MessageType::EndpointUpdate)),
        8 => (0..MessageType::ALL.len()).prop_map(|index| FROM_PEER[index].clone()),
    ]
}

fn type_index(message_type: MessageType) -> usize {
    MessageType::ALL
        .iter()
        .position(|candidate| *candidate == message_type)
        .unwrap()
}

/// Something that happens to a session after it authenticated.
#[derive(Clone, Debug)]
enum Event {
    Receive(Message),
    Close,
    Block,
    Remove,
    StreamClosed,
}

fn arb_event() -> impl Strategy<Value = Event> {
    prop_oneof![
        16 => arb_session_message().prop_map(Event::Receive),
        1 => Just(Event::Close),
        1 => Just(Event::Block),
        1 => Just(Event::Remove),
        1 => Just(Event::StreamClosed),
    ]
}

fn apply(session: &mut Session, event: &Event) -> Result<Vec<Action>, ProtocolError> {
    match event {
        Event::Receive(message) => session.receive(message),
        Event::Close => Ok(session.close()),
        Event::Block => Ok(session.peer_blocked()),
        Event::Remove => Ok(session.contact_removed()),
        Event::StreamClosed => {
            session.stream_closed();
            Ok(Vec::new())
        }
    }
}

fn authenticated(standing: Standing) -> (Session, Vec<Action>) {
    let mut session = Session::inbound(*LOCAL_IDENTITY);
    session.stream_established().unwrap();
    session.handshake_completed().unwrap();
    let first = session.authenticated(PEER_CARD.clone(), standing).unwrap();
    (session, first)
}

fn visible(actions: &[Action]) -> Vec<Action> {
    actions
        .iter()
        .copied()
        .filter(|action| action.is_visible_to_peer())
        .collect()
}

type Transcript = (Vec<Vec<Action>>, Result<(), ProtocolError>, SessionState);

/// Everything the peer can observe of a session with the given standing
/// over a sequence of events.
fn transcript(standing: Standing, events: &[Event]) -> Transcript {
    let (mut session, first) = authenticated(standing);
    let mut seen = vec![visible(&first)];
    let mut outcome = Ok(());
    for event in events {
        match apply(&mut session, event) {
            Ok(actions) => seen.push(visible(&actions)),
            Err(error) => {
                outcome = Err(error);
                break;
            }
        }
    }
    (seen, outcome, session.state())
}

// ---------------------------------------------------------------------------
// Filenames
// ---------------------------------------------------------------------------

/// Every reserved device name, written out. Deliberately not the code that
/// `Filename::save_name` uses.
fn device_names() -> Vec<String> {
    let mut names: Vec<String> = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    for port in ["COM", "LPT"] {
        for unit in "0123456789\u{b9}\u{b2}\u{b3}".chars() {
            names.push(format!("{port}{unit}"));
        }
    }
    names
}

/// Checks that a suggested save name is one safe path component.
fn check_save_name(name: &str) -> Result<(), TestCaseError> {
    prop_assert!(!name.is_empty());
    prop_assert!(name.len() <= MAX_FILENAME_LEN);
    prop_assert!(name != "." && name != "..");
    prop_assert!(!name.contains(['/', '\\', '<', '>', ':', '"', '|', '?', '*']));
    prop_assert!(!name.starts_with(['.', ' ']));
    prop_assert!(!name.ends_with(['.', ' ']));
    let stem = name.split('.').next().unwrap().trim_end_matches(' ');
    for device in device_names() {
        prop_assert!(!stem.eq_ignore_ascii_case(&device), "{}", device);
    }
    Ok(())
}

/// A filename built around a reserved device name: the name in mixed case,
/// possibly behind a dot, followed by spaces and an extension.
fn arb_device_filename() -> impl Strategy<Value = String> {
    (
        prop::sample::select(device_names()),
        vec(any::<bool>(), 7),
        prop::sample::select(vec!["", ".", ".."]),
        prop::sample::select(vec!["", " ", "  "]),
        prop::sample::select(vec![
            "", ".txt", ".tar.gz", ".", "..", ". .", ".a b", ":x", "?", " .txt ",
        ]),
        0..=260_usize,
    )
        .prop_map(|(device, lower, lead, spaces, extension, fill)| {
            let cased: String = device
                .chars()
                .zip(lower.iter().chain(core::iter::repeat(&false)))
                .map(|(c, lower)| if *lower { c.to_ascii_lowercase() } else { c })
                .collect();
            let mut name = format!("{lead}{cased}{spaces}{extension}");
            // Sometimes pad the name up to the length limit, so that the
            // prefix a device name gets has to be cut off again.
            if fill > 200 && name.len() < MAX_FILENAME_LEN {
                let room = MAX_FILENAME_LEN - name.len();
                name.push('.');
                name.push_str(&"\u{4e2d}".repeat(room.saturating_sub(1) / 3));
                name.push_str(&"a".repeat(room.saturating_sub(1) % 3));
            }
            name
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    // -----------------------------------------------------------------------
    // Message bodies
    // -----------------------------------------------------------------------

    #[test]
    fn messages_round_trip(message in arb_message()) {
        let body = message.encode_body().unwrap();
        let decoded = Message::decode(message.message_type(), &body).unwrap();
        prop_assert_eq!(&decoded, &message);
        prop_assert_eq!(decoded.encode_body().unwrap(), body);
    }

    #[test]
    fn arbitrary_bodies_do_not_panic_the_decoder(
        message_type in arb_message_type(),
        body in vec(any::<u8>(), 0..1200),
    ) {
        // Arbitrary bytes almost never decode; this is a check for panics.
        // The properties below start from valid bodies.
        if let Ok(message) = Message::decode(message_type, &body) {
            prop_assert_eq!(message.encode_body().unwrap(), body);
        }
    }

    #[test]
    fn damaged_messages_are_rejected_or_canonical(
        message in arb_message(),
        position in any::<prop::sample::Index>(),
        value in any::<u8>(),
    ) {
        let mut body = message.encode_body().unwrap();
        if body.is_empty() {
            return Ok(());
        }
        let index = position.index(body.len());
        body[index] = value;
        if let Ok(decoded) = Message::decode(message.message_type(), &body) {
            prop_assert_eq!(decoded.encode_body().unwrap(), body);
        }
    }

    #[test]
    fn a_body_of_the_wrong_length_is_rejected(
        message in arb_message(),
        cut in any::<prop::sample::Index>(),
        extra in vec(any::<u8>(), 1..40),
    ) {
        // A valid body has exactly one length. Every proper prefix of it
        // and every extension of it is rejected.
        let body = message.encode_body().unwrap();
        if !body.is_empty() {
            let short = &body[..cut.index(body.len())];
            prop_assert!(Message::decode(message.message_type(), short).is_err());
        }
        let mut long = body;
        long.extend_from_slice(&extra);
        prop_assert!(Message::decode(message.message_type(), &long).is_err());
    }

    #[test]
    fn a_body_decodes_only_as_its_own_type_or_canonically(
        message in arb_message(),
        other in arb_message_type(),
    ) {
        // The bytes of one message offered as another type: rejected, or a
        // valid message of that type with exactly this encoding.
        let body = message.encode_body().unwrap();
        if let Ok(decoded) = Message::decode(other, &body) {
            prop_assert_eq!(decoded.message_type(), other);
            prop_assert_eq!(decoded.encode_body().unwrap(), body);
        }
    }

    // -----------------------------------------------------------------------
    // Contact cards
    // -----------------------------------------------------------------------

    #[test]
    fn cards_round_trip_in_both_forms(card in arb_card(true), plain in arb_card(false)) {
        for card in [card, plain] {
            let bytes = card.encode();
            prop_assert_eq!(&ContactCard::decode(&bytes).unwrap(), &card);
            let text = card.to_text();
            prop_assert_eq!(&ContactCard::from_text(&text).unwrap(), &card);
            prop_assert_eq!(&ContactCard::from_text(&text.to_ascii_lowercase()).unwrap(), &card);
        }
    }

    #[test]
    fn arbitrary_bytes_do_not_panic_the_card_decoder(
        bytes in prop_oneof![
            vec(any::<u8>(), 0..200),
            vec(any::<u8>(), 171),
            vec(any::<u8>(), 187),
        ],
    ) {
        // Arbitrary bytes stop at the length or the version byte. This is a
        // check for panics; the next property gets to the signature.
        prop_assert!(ContactCard::decode(&bytes).is_err());
    }

    #[test]
    fn a_well_formed_card_with_another_signature_is_rejected(
        identity_seed in any::<[u8; 32]>(),
        transport_bytes in any::<[u8; 32]>(),
        endpoint_seed in any::<[u8; 32]>(),
        epoch in 1..=u64::MAX,
        capability in proptest::option::of(any::<[u8; 16]>()),
        signature in prop_oneof![
            // Arbitrary bytes.
            (any::<[u8; 32]>(), any::<[u8; 32]>()).prop_map(|(r, s)| [r, s].concat()),
            // A real signature by the same identity over something else.
            vec(any::<u8>(), 0..120).prop_map(|message| vec![0xff; 64].into_iter().chain(message).collect()),
        ],
        as_text in any::<bool>(),
    ) {
        // Every field is valid: version, keys, epoch, count, flags, length.
        // Only the signature is not the right one, so the decoder runs
        // every check and fails at the last.
        prop_assume!(identity_seed != endpoint_seed);
        let secret = IdentitySecretKey::from_seed(&identity_seed);
        let transport_key = transport(transport_bytes);
        prop_assume!(transport_key.is_some());
        let transport_key = transport_key.unwrap();
        prop_assume!(!transport_key.is_montgomery_form_of(secret.public_key().as_bytes()));
        prop_assume!(!transport_key.is_montgomery_form_of(endpoint(endpoint_seed).as_bytes()));
        let mut bytes = vec![0x01];
        bytes.extend_from_slice(secret.public_key().as_bytes());
        bytes.extend_from_slice(transport_key.as_bytes());
        bytes.extend_from_slice(&epoch.to_be_bytes());
        bytes.push(1);
        bytes.extend_from_slice(endpoint(endpoint_seed).as_bytes());
        match capability {
            Some(capability) => {
                bytes.push(0x01);
                bytes.extend_from_slice(&capability);
            }
            None => bytes.push(0x00),
        }
        if signature.len() == 64 {
            bytes.extend_from_slice(&signature);
        } else {
            // The marker is followed by the message to sign instead.
            bytes.extend_from_slice(secret.sign(&signature[64..]).as_bytes());
        }

        let result = if as_text {
            ContactCard::from_text(&format!("MONOLITH1:{}", base32::encode(&bytes)))
        } else {
            ContactCard::decode(&bytes)
        };
        prop_assert_eq!(result, Err(ProtocolError::BadSignature));
    }

    #[test]
    fn a_card_with_a_forged_field_is_rejected(
        card in arb_card(true),
        other in arb_card(true),
        field in 0..7_usize,
    ) {
        // Every field of one valid card replaced by the same field of
        // another valid card, so that each field on its own is well formed
        // and only the signature can tell: identity key, transport key,
        // epoch, endpoint, capability, signature, half of the signature.
        let ranges = [1..33, 33..65, 65..73, 74..106, 107..123, 123..187, 123..155];
        let range = ranges[field].clone();
        let mut bytes = card.encode();
        let donor = other.encode();
        prop_assume!(bytes[range.clone()] != donor[range.clone()]);
        bytes[range.clone()].copy_from_slice(&donor[range]);
        prop_assert!(ContactCard::decode(&bytes).is_err());
    }

    #[test]
    fn a_damaged_card_is_rejected(
        card in arb_card(true),
        position in any::<prop::sample::Index>(),
        flip in 1..=255_u8,
    ) {
        let mut bytes = card.encode();
        let index = position.index(bytes.len());
        bytes[index] ^= flip;
        prop_assert!(ContactCard::decode(&bytes).is_err());
    }

    #[test]
    fn arbitrary_text_is_not_a_card(text in "\\PC{0,300}") {
        prop_assert!(ContactCard::from_text(&text).is_err());
    }

    #[test]
    fn well_formed_text_of_arbitrary_bytes_is_not_a_card(
        bytes in prop_oneof![vec(any::<u8>(), 171), vec(any::<u8>(), 187)],
        lower in any::<bool>(),
    ) {
        // The prefix and the base32 are right, so the text parser hands
        // the bytes to the card decoder, which stops early as above.
        let mut text = format!("MONOLITH1:{}", base32::encode(&bytes));
        if lower {
            text = text.to_ascii_lowercase();
        }
        prop_assert!(ContactCard::from_text(&text).is_err());
    }

    #[test]
    fn a_card_text_with_one_changed_symbol_is_rejected(
        card in arb_card(true),
        position in any::<prop::sample::Index>(),
        symbol in prop::sample::select("ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".chars().collect::<Vec<_>>()),
    ) {
        let text = card.to_text();
        let prefix = "MONOLITH1:".len();
        let index = prefix + position.index(text.len() - prefix);
        let mut chars: Vec<char> = text.chars().collect();
        prop_assume!(chars[index] != symbol);
        chars[index] = symbol;
        let changed: String = chars.into_iter().collect();
        prop_assert!(ContactCard::from_text(&changed).is_err());
    }

    #[test]
    fn the_standing_of_an_inbound_peer_follows_the_table(
        record in 0..5_usize,
        pinned_epoch in 1..6_u64,
        presented_epoch in 1..6_u64,
        other_transport in any::<bool>(),
        other_endpoint in any::<bool>(),
    ) {
        // PROTOCOL.md 6.2, restated: without a contact record the card is
        // not compared. With one, the active key with a newer or the same
        // card keeps the record, a newer key without continuity is pending,
        // and everything else is the stale-card standing.
        let card = |epoch: u64, changed_transport: bool, changed_endpoint: bool| {
            let mut transport_bytes = PEER;
            transport_bytes[0] ^= u8::from(changed_transport);
            let mut endpoint_seed = PEER;
            endpoint_seed[0] ^= 0xff;
            endpoint_seed[1] ^= u8::from(changed_endpoint);
            ContactCard::sign(
                &IdentitySecretKey::from_seed(&PEER),
                transport(transport_bytes).unwrap(),
                EndpointEpoch::new(epoch).unwrap(),
                EndpointSet::single(endpoint(endpoint_seed)),
                None,
            )
            .unwrap()
        };
        let pinned = card(pinned_epoch, false, false);
        let presented = card(presented_epoch, other_transport, other_endpoint);
        let same_content = !other_transport && !other_endpoint;

        let change = if presented_epoch > pinned_epoch {
            CardChange::Newer
        } else if presented_epoch < pinned_epoch {
            CardChange::Stale
        } else if same_content {
            CardChange::Unchanged
        } else {
            CardChange::Conflict
        };
        prop_assert_eq!(evaluate_card(&pinned, &presented), Ok(change));

        let mut credentials = Credentials::new(pinned.clone());
        let (mut record, as_recorded) = match record {
            0 => (PeerRecord::None, Standing::None),
            1 => (PeerRecord::Declined, Standing::Declined),
            2 => (PeerRecord::Blocked, Standing::Blocked),
            3 => (PeerRecord::Requested(&mut credentials), Standing::Requested),
            _ => (PeerRecord::Accepted(&mut credentials), Standing::Accepted),
        };
        let expected = if !as_recorded.is_contact_record() {
            Admission { standing: as_recorded, change: None }
        } else {
            let (standing, credential) = match change {
                CardChange::Unchanged => (as_recorded, CredentialChange::Unchanged),
                CardChange::Newer if !other_transport => (as_recorded, CredentialChange::Advanced),
                CardChange::Newer => (Standing::PendingSuccessor, CredentialChange::Pending),
                CardChange::Conflict => (Standing::StaleCard, CredentialChange::Conflict),
                CardChange::Stale => (Standing::StaleCard, CredentialChange::Stale),
            };
            Admission { standing, change: Some(credential) }
        };
        let admitted = record.admit(&presented);
        prop_assert_eq!(admitted, Ok(expected));

        // Nothing that is not the active key of a contact leaves the peer
        // with a standing that can become a contact session.
        if other_transport || matches!(change, CardChange::Stale | CardChange::Conflict) {
            prop_assert!(!admitted.unwrap().standing.is_contact_record());
        }
        // The record of another identity is refused.
        let stranger = sample_card(STRANGER, 0);
        if as_recorded.is_contact_record() {
            prop_assert_eq!(record.admit(&stranger), Err(ProtocolError::IdentityMismatch));
        }
    }

    #[test]
    fn whitespace_in_a_card_text_changes_nothing(
        card in arb_card(true),
        gaps in vec((any::<prop::sample::Index>(), prop::sample::select(vec![' ', '\t', '\r', '\n'])), 0..40),
    ) {
        let mut chars: Vec<char> = card.to_text().chars().collect();
        for (position, gap) in gaps {
            let at = position.index(chars.len() + 1);
            chars.insert(at, gap);
        }
        let spaced: String = chars.into_iter().collect();
        prop_assert_eq!(ContactCard::from_text(&spaced), Ok(card));
    }

}

// The properties below need no signature per case and run many more cases.
proptest! {
    #![proptest_config(ProptestConfig::with_cases(1024))]

    // -----------------------------------------------------------------------
    // Frames
    // -----------------------------------------------------------------------

    #[test]
    fn plaintext_round_trips(
        message_type in arb_message_type(),
        body in vec(any::<u8>(), 0..3000),
    ) {
        let plaintext = encode_plaintext(&PARAMS, message_type, &body).unwrap();
        prop_assert_eq!(plaintext.len() % 1024, 0);
        let (decoded_type, decoded_body) =
            decode_plaintext(&PARAMS, SessionState::AuthenticatedContact, &plaintext).unwrap();
        prop_assert_eq!(decoded_type, message_type);
        prop_assert_eq!(decoded_body, &body[..]);
    }

    #[test]
    fn damaged_plaintexts_are_rejected_or_canonical(
        message_type in arb_message_type(),
        body in vec(any::<u8>(), 0..2500),
        position in prop_oneof![
            // The header, where the type and the length are.
            (0..4_usize).prop_map(Some),
            Just(None),
        ],
        anywhere in any::<prop::sample::Index>(),
        value in any::<u8>(),
    ) {
        // A valid frame with one byte replaced. If it is still accepted,
        // encoding what was decoded gives the same bytes: no frame has two
        // readings.
        let mut plaintext = encode_plaintext(&PARAMS, message_type, &body).unwrap();
        let index = position.unwrap_or_else(|| anywhere.index(plaintext.len()));
        plaintext[index] = value;
        if let Ok((decoded_type, decoded_body)) =
            decode_plaintext(&PARAMS, SessionState::AuthenticatedContact, &plaintext)
        {
            prop_assert_eq!(
                encode_plaintext(&PARAMS, decoded_type, decoded_body).unwrap(),
                plaintext
            );
        }
    }

    #[test]
    fn any_nonzero_padding_byte_is_rejected(
        message_type in arb_message_type(),
        body in vec(any::<u8>(), 0..2500),
        position in any::<prop::sample::Index>(),
        value in 1..=255_u8,
    ) {
        let mut plaintext = encode_plaintext(&PARAMS, message_type, &body).unwrap();
        let padding_start = 4 + body.len();
        prop_assume!(padding_start < plaintext.len());
        let index = padding_start + position.index(plaintext.len() - padding_start);
        plaintext[index] = value;
        prop_assert_eq!(
            decode_plaintext(&PARAMS, SessionState::AuthenticatedContact, &plaintext),
            Err(ProtocolError::BadPadding)
        );
    }

    #[test]
    fn a_plaintext_of_the_wrong_length_is_rejected(
        body in vec(any::<u8>(), 0..2500),
        cut in 1..1024_usize,
        blocks in 1..3_usize,
    ) {
        let plaintext = encode_plaintext(&PARAMS, MessageType::FileChunk, &body).unwrap();
        // Not a whole number of blocks.
        let short = &plaintext[..plaintext.len() - cut];
        prop_assert_eq!(
            decode_plaintext(&PARAMS, SessionState::AuthenticatedContact, short),
            Err(ProtocolError::FrameLengthOutOfRange)
        );
        // More blocks than the body needs.
        let mut long = plaintext;
        long.resize(long.len() + blocks * 1024, 0);
        prop_assert_eq!(
            decode_plaintext(&PARAMS, SessionState::AuthenticatedContact, &long),
            Err(ProtocolError::BadPadding)
        );
    }

    #[test]
    fn frames_are_gated_by_the_session_state(
        message_type in arb_message_type(),
        body in vec(any::<u8>(), 0..700),
        state in prop::sample::select(vec![
            SessionState::Connecting,
            SessionState::CryptoHandshake,
            SessionState::IdentityAuth,
            SessionState::AuthenticatedUnknown,
            SessionState::AuthenticatedContact,
            SessionState::Closing,
            SessionState::Closed,
        ]),
    ) {
        // A well-formed one-block frame of any type in any state is
        // accepted exactly when the type is legal in that state.
        let plaintext = encode_plaintext(&PARAMS, message_type, &body).unwrap();
        let accepted = decode_plaintext(&PARAMS, state, &plaintext).is_ok();
        prop_assert_eq!(accepted, message_type.may_be_received_in(state));
    }

    #[test]
    fn the_frame_decoder_never_holds_more_than_one_frame(
        sizes in vec(1..=3_usize, 0..5),
        tail in vec(any::<u8>(), 0..40),
        damage in proptest::option::of((any::<prop::sample::Index>(), any::<u8>())),
        piece in 1..700_usize,
        contact in any::<bool>(),
    ) {
        // A stream of valid frames, followed by arbitrary bytes, with at
        // most one byte replaced anywhere, length prefixes included.
        // Whatever comes out of that, the decoder either fails or yields
        // frames of legal length, and what it holds is bounded by the
        // largest frame of the state.
        let state = if contact {
            SessionState::AuthenticatedContact
        } else {
            SessionState::AuthenticatedUnknown
        };
        let mut stream = Vec::new();
        for blocks in sizes {
            let payload = vec![0x5A_u8; blocks * 1024 + PARAMS.overhead()];
            stream.extend_from_slice(&encode_outer(&PARAMS, &payload).unwrap());
        }
        stream.extend_from_slice(&tail);
        if let Some((position, value)) = damage {
            if !stream.is_empty() {
                let index = position.index(stream.len());
                stream[index] = value;
            }
        }

        let limit = PARAMS.max_plaintext_len_in(state) + PARAMS.overhead();
        let mut decoder = OuterDecoder::new(PARAMS);
        'outer: for chunk in stream.chunks(piece) {
            let mut rest = chunk;
            while !rest.is_empty() {
                match decoder.feed(rest, state) {
                    Ok((used, frame)) => {
                        prop_assert!(used > 0);
                        rest = &rest[used..];
                        prop_assert!(decoder.buffered() <= limit);
                        if let Some(frame) = frame {
                            prop_assert!(frame.len() <= limit);
                            prop_assert!(PARAMS.check_outer_len(frame.len(), state).is_ok());
                        }
                    }
                    Err(_) => {
                        // A failed decoder stays failed.
                        prop_assert!(decoder.feed(&[0x04, 0x10], state).is_err());
                        break 'outer;
                    }
                }
            }
        }
        prop_assert!(decoder.buffered() <= limit);
        prop_assert!(limit <= MAX_FRAME_CIPHERTEXT_LEN);
    }

    #[test]
    fn the_frame_decoder_reassembles_any_split(
        sizes in vec(1..=3_usize, 1..5),
        piece in 1..3000_usize,
    ) {
        let payloads: Vec<Vec<u8>> = sizes
            .iter()
            .enumerate()
            .map(|(i, blocks)| vec![u8::try_from(i).unwrap(); blocks * 1024 + 16])
            .collect();
        let mut stream = Vec::new();
        for payload in &payloads {
            stream.extend_from_slice(&encode_outer(&PARAMS, payload).unwrap());
        }
        let mut decoder = OuterDecoder::new(PARAMS);
        let mut frames = Vec::new();
        for chunk in stream.chunks(piece) {
            let mut rest = chunk;
            while !rest.is_empty() {
                let (used, frame) = decoder
                    .feed(rest, SessionState::AuthenticatedContact)
                    .unwrap();
                rest = &rest[used..];
                frames.extend(frame);
            }
        }
        prop_assert_eq!(frames, payloads);
    }

    // -----------------------------------------------------------------------
    // Sessions
    // -----------------------------------------------------------------------

    #[test]
    fn session_invariants_hold_for_any_sequence_of_events(
        standing in arb_standing(),
        events in vec(arb_event(), 0..12),
    ) {
        let (mut session, _) = authenticated(standing);
        for event in &events {
            let before = session.state();
            let standing_before = session.standing();
            let result = apply(&mut session, event);
            let after = session.state();

            prop_assert!(before == after || before.can_transition_to(after));
            prop_assert_eq!(session.peer(), Some(&*PEER_IDENTITY));

            let live = matches!(
                before,
                SessionState::AuthenticatedUnknown | SessionState::AuthenticatedContact
            );
            if let Event::Receive(message) = event {
                let legal = message.message_type().may_be_received_in(before);
                if live && !legal {
                    prop_assert_eq!(&result, &Err(ProtocolError::MessageNotPermitted));
                }
                if legal && has_foreign_card(message) {
                    // Whatever the standing: the card check comes right
                    // after the state gate and before anything else.
                    prop_assert_eq!(&result, &Err(ProtocolError::IdentityMismatch));
                }
            }

            match result {
                Ok(actions) => {
                    if !live {
                        // After a Close or the end of the stream nothing is
                        // processed and nothing is sent.
                        prop_assert_eq!(&actions, &Vec::new());
                    }
                    if actions.contains(&Action::Deliver) {
                        prop_assert_eq!(before, SessionState::AuthenticatedContact);
                        prop_assert_eq!(standing_before, Standing::Accepted);
                    }
                    if actions.contains(&Action::MarkAccepted) {
                        // Only an identity the user asked for can become a
                        // contact through a message from the peer.
                        prop_assert_eq!(standing_before, Standing::Requested);
                        prop_assert_eq!(session.standing(), Standing::Accepted);
                    }
                    if actions.contains(&Action::ConsiderRequest) {
                        prop_assert_eq!(standing_before, Standing::None);
                        prop_assert_eq!(before, SessionState::AuthenticatedUnknown);
                        // The Close goes out before the request is looked at.
                        prop_assert_eq!(&actions, &vec![Action::SendClose, Action::ConsiderRequest]);
                    }
                    if actions.contains(&Action::Confirmed) {
                        prop_assert_eq!(before, SessionState::AuthenticatedUnknown);
                        prop_assert_eq!(after, SessionState::AuthenticatedContact);
                        prop_assert_eq!(session.standing(), Standing::Accepted);
                    }
                    for action in &actions {
                        // Whatever the session says to send, it also allows.
                        let sent = match action {
                            Action::SendContactAccept => MessageType::ContactAccept,
                            Action::SendContactRequest => MessageType::ContactRequest,
                            Action::SendClose => MessageType::Close,
                            _ => continue,
                        };
                        prop_assert!(session.may_send(sent), "{:?} in {:?}", action, after);
                    }
                }
                Err(_) => {
                    prop_assert!(matches!(event, Event::Receive(_)));
                    prop_assert_eq!(after, SessionState::Closed);
                    prop_assert_eq!(session.standing(), standing_before);
                }
            }

            // Application messages may be sent on a confirmed session only.
            if session.may_send(MessageType::ChatMessage) {
                prop_assert_eq!(after, SessionState::AuthenticatedContact);
            }
        }
    }

    #[test]
    fn a_session_that_is_not_authenticated_delivers_nothing(
        messages in vec(arb_session_message(), 1..12),
        steps in 0..=2_u8,
    ) {
        let mut session = Session::inbound(*LOCAL_IDENTITY);
        if steps >= 1 {
            session.stream_established().unwrap();
        }
        if steps >= 2 {
            session.handshake_completed().unwrap();
        }
        for message in &messages {
            if let Ok(actions) = session.receive(message) {
                prop_assert!(!actions.contains(&Action::Deliver));
                prop_assert!(!actions.contains(&Action::MarkAccepted));
                prop_assert!(!actions.contains(&Action::ConsiderRequest));
                prop_assert!(!actions.contains(&Action::Confirmed));
            }
            prop_assert!(!session.state().is_authenticated());
            prop_assert_eq!(session.peer(), None);
        }
    }

    #[test]
    fn a_session_is_authenticated_only_after_the_handshake_and_as_the_right_identity(
        steps in 0..=2_u8,
        outbound in any::<bool>(),
        claim in 0..3_u8,
        standing in arb_standing(),
    ) {
        // The handshake layer reports the peer, a stranger or the local
        // identity as authenticated, on a session that got some way into
        // the handshake. That is accepted in one case only: the handshake
        // finished, the identity is not the local one, and a session that
        // dialed the peer is not told about anybody else.
        let mut session = if outbound {
            Session::outbound(*LOCAL_IDENTITY, *PEER_IDENTITY)
        } else {
            Session::inbound(*LOCAL_IDENTITY)
        };
        if steps >= 1 {
            session.stream_established().unwrap();
        }
        if steps >= 2 {
            session.handshake_completed().unwrap();
        }
        let card = match claim {
            0 => PEER_CARD.clone(),
            1 => sample_card(STRANGER, 0),
            _ => sample_card(LOCAL, 0),
        };
        let claimed = *card.identity();
        let outcome = session.authenticated(card, standing);
        let accepted = steps == 2 && claim != 2 && !(outbound && claim == 1);
        if accepted {
            prop_assert!(outcome.is_ok());
            prop_assert_eq!(session.state(), SessionState::AuthenticatedUnknown);
            prop_assert_eq!(session.peer(), Some(&claimed));
            prop_assert_eq!(session.standing(), standing);
        } else {
            prop_assert!(outcome.is_err());
            prop_assert!(!session.state().is_authenticated());
            prop_assert_eq!(session.peer(), None);
            // Nothing can be received on it afterwards.
            if let Ok(actions) = session.receive(&FROM_PEER[type_index(MessageType::ContactAccept)]) {
                prop_assert!(actions.is_empty());
            }
            prop_assert!(!session.state().is_authenticated());
        }
    }

    #[test]
    fn identities_that_are_not_contacts_see_the_same_thing(
        events in vec(arb_event(), 0..10),
    ) {
        // T-ORACLE-1 for arbitrary peer behavior and local events.
        let reference = transcript(Standing::None, &events);
        prop_assert_eq!(&transcript(Standing::Declined, &events), &reference);
        prop_assert_eq!(&transcript(Standing::Blocked, &events), &reference);
        prop_assert_eq!(&transcript(Standing::StaleCard, &events), &reference);
        prop_assert_eq!(&transcript(Standing::PendingSuccessor, &events), &reference);
    }

    #[test]
    fn both_ends_keep_the_same_duplicate(
        seed_a in any::<[u8; 32]>(),
        seed_b in any::<[u8; 32]>(),
        a_sees_own_first in any::<bool>(),
        b_sees_own_first in any::<bool>(),
    ) {
        let a = identity(seed_a);
        let b = identity(seed_b);
        prop_assume!(a != b);
        // One session initiated by each side, both alive. Each side sees
        // them confirmed in its own order.
        let order = |own_first: bool| {
            if own_first {
                (Initiator::Local, Initiator::Remote)
            } else {
                (Initiator::Remote, Initiator::Local)
            }
        };
        let kept = |local: &IdentityPublicKey, remote: &IdentityPublicKey, own_first: bool| {
            let (older, newer) = order(own_first);
            match resolve(local, remote, Contender::current(older), Contender::current(newer))
                .unwrap()
            {
                Resolution::CloseOlder => newer,
                Resolution::ProbeOlder => match after_probe(true) {
                    ProbeOutcome::CloseNewer => older,
                    ProbeOutcome::CloseOlder => newer,
                },
                Resolution::CloseNewer | Resolution::CloseBoth => panic!("both are current"),
            }
        };
        let kept_by_a = kept(&a, &b, a_sees_own_first);
        let kept_by_b = kept(&b, &a, b_sees_own_first);
        // "Local" for one side is "Remote" for the other.
        prop_assert_ne!(kept_by_a, kept_by_b);
        let a_is_smaller = a < b;
        prop_assert_eq!(kept_by_a == Initiator::Local, a_is_smaller);
    }

    // -----------------------------------------------------------------------
    // Text
    // -----------------------------------------------------------------------

    #[test]
    fn text_validators_accept_or_reject_without_panicking(bytes in vec(any::<u8>(), 0..300)) {
        let _ = ChatText::from_bytes(&bytes);
        let _ = IntroductionText::from_bytes(&bytes);
        let _ = ProfileText::from_bytes(&bytes);
        let _ = DisplayName::from_bytes(&bytes);
        let _ = Filename::from_bytes(&bytes);
    }

    #[test]
    fn display_names_are_accepted_exactly_when_the_rules_say_so(
        text in arb_name_candidate(70),
    ) {
        // A length in bytes, a count of scalar values and a fixed list of
        // code points. Nothing else decides, and what is accepted is kept
        // byte for byte.
        let expected = !text.contains(FORBIDDEN_IN_NAMES)
            && !text.starts_with(' ')
            && !text.ends_with(' ')
            && text.chars().count() <= MAX_DISPLAY_NAME_SCALARS
            && text.len() <= MAX_DISPLAY_NAME_LEN;
        let result = DisplayName::new(&text);
        prop_assert_eq!(result.is_ok(), expected);
        if let Ok(name) = result {
            prop_assert_eq!(name.as_str(), &text);
        }
    }

    #[test]
    fn filenames_are_accepted_exactly_when_the_rules_say_so(
        text in arb_name_candidate(40),
        separator in proptest::option::weighted(0.2, (any::<prop::sample::Index>(), prop::sample::select(vec!['/', '\\']))),
    ) {
        let mut chars: Vec<char> = text.chars().collect();
        if let Some((position, character)) = separator {
            let at = position.index(chars.len() + 1);
            chars.insert(at, character);
        }
        let text: String = chars.into_iter().collect();
        let expected = !text.is_empty()
            && text.len() <= MAX_FILENAME_LEN
            && !text.contains(FORBIDDEN_IN_NAMES)
            && !text.contains(['/', '\\'])
            && !text.starts_with(' ')
            && !text.ends_with(' ')
            && text != "."
            && text != "..";
        let result = Filename::new(&text);
        prop_assert_eq!(result.is_ok(), expected);
        if let Ok(filename) = result {
            check_save_name(&filename.save_name())?;
        }
    }

    #[test]
    fn free_text_keeps_format_characters_and_rejects_controls(
        text in arb_name_candidate(60),
    ) {
        // Chat text allows tab, line feed, bidirectional and invisible
        // characters. It rejects the other controls, the line and paragraph
        // separators and noncharacters.
        let rejected = [
            '\u{0}', '\r', '\u{1b}', '\u{7f}', '\u{85}', '\u{2028}', '\u{fdd0}', '\u{fffe}',
        ];
        let expected = !text.is_empty() && !text.contains(rejected);
        prop_assert_eq!(ChatText::new(&text).is_ok(), expected);
        prop_assert_eq!(IntroductionText::new(&text).is_ok(), expected || text.is_empty());
    }

    #[test]
    fn save_names_of_device_names_are_safe(name in arb_device_filename()) {
        if let Ok(filename) = Filename::new(&name) {
            check_save_name(&filename.save_name())?;
        }
    }

    #[test]
    fn save_names_are_single_safe_components(
        chars in vec(
            prop::sample::select(vec![
                'a', 'B', '1', '0', '.', ' ', '<', '>', ':', '"', '|', '?', '*', '-', '$', 'C', 'O',
                'N', 'M', 'n', 'u', 'l', 'L', 'P', 'T', '\u{b9}', '\u{e9}',
            ]),
            1..40,
        ),
    ) {
        let text: String = chars.into_iter().collect();
        if let Ok(filename) = Filename::new(&text) {
            check_save_name(&filename.save_name())?;
        }
    }
}

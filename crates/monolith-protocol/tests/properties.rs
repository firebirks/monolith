//! Property tests of the protocol core.
//!
//! These check statements that must hold for every input, not for chosen
//! examples: decoders are canonical, encoders round-trip, the frame decoder
//! stays bounded, the session logic never hands application data to the
//! caller before a session is confirmed, and identities that are not
//! contacts cannot tell their cases apart.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use monolith_identity::{EndpointEpoch, IdentityPublicKey, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::body::{
    AuthProof, ContactRequest, FileChunk, Message, MessageId, TransferId,
};
use monolith_protocol::card::{ContactCard, EndpointSet, InvitationCapability};
use monolith_protocol::duplicate::{Initiator, ProbeOutcome, Resolution, after_probe, resolve};
use monolith_protocol::frame::{
    FrameParams, OuterDecoder, decode_plaintext, encode_outer, encode_plaintext,
};
use monolith_protocol::limits::{MAX_FILE_SIZE, MAX_FRAME_CIPHERTEXT_LEN};
use monolith_protocol::session::{Action, Session, Standing};
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

fn arb_card(with_invitation: bool) -> impl Strategy<Value = ContactCard> {
    (
        any::<[u8; 32]>(),
        any::<[u8; 32]>(),
        1..=u64::MAX,
        any::<[u8; 16]>(),
    )
        .prop_map(move |(identity_seed, endpoint_seed, epoch, capability)| {
            ContactCard::sign(
                &IdentitySecretKey::from_seed(&identity_seed),
                EndpointEpoch::new(epoch).unwrap(),
                EndpointSet::single(endpoint(endpoint_seed)),
                with_invitation.then(|| InvitationCapability::from_bytes(capability)),
            )
        })
}

/// Characters that every text field accepts in the middle of a string.
fn plain_char() -> impl Strategy<Value = char> {
    prop::sample::select(vec![
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
    ])
}

fn arb_name(max_chars: usize) -> impl Strategy<Value = String> {
    vec(plain_char(), 0..=max_chars).prop_map(|chars| chars.into_iter().collect())
}

fn arb_message() -> impl Strategy<Value = Message> {
    let id = any::<[u8; 16]>();
    prop_oneof![
        Just(Message::Close),
        Just(Message::ContactAccept),
        any::<[u8; 8]>().prop_map(Message::Ping),
        any::<[u8; 8]>().prop_map(Message::Pong),
        (any::<[u8; 32]>(), any::<u64>(), any::<[u8; 32]>()).prop_map(|(seed, features, s)| {
            let secret = IdentitySecretKey::from_seed(&seed);
            Message::AuthProof(Box::new(AuthProof {
                identity: secret.public_key(),
                features,
                signature: secret.sign(&s),
            }))
        }),
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
        Standing::Requested,
        Standing::Accepted,
    ])
}

fn authenticated(standing: Standing) -> (Session, Vec<Action>) {
    let mut session = Session::new();
    session.stream_established().unwrap();
    session.handshake_completed().unwrap();
    let first = session.identities_proven(standing).unwrap();
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

fn transcript(standing: Standing, messages: &[MessageType]) -> Transcript {
    let (mut session, first) = authenticated(standing);
    let mut seen = vec![visible(&first)];
    let mut outcome = Ok(());
    for message in messages {
        match session.receive(*message) {
            Ok(actions) => seen.push(visible(&actions)),
            Err(error) => {
                outcome = Err(error);
                break;
            }
        }
    }
    (seen, outcome, session.state())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn messages_round_trip(message in arb_message()) {
        let body = message.encode_body().unwrap();
        let decoded = Message::decode(message.message_type(), &body).unwrap();
        prop_assert_eq!(&decoded, &message);
        prop_assert_eq!(decoded.encode_body().unwrap(), body);
    }

    #[test]
    fn accepted_bodies_have_one_encoding(
        message_type in arb_message_type(),
        body in vec(any::<u8>(), 0..1200),
    ) {
        // Arbitrary bytes almost never decode. When they do, encoding the
        // result must give the same bytes back.
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
    fn arbitrary_bytes_are_not_a_card(bytes in vec(any::<u8>(), 0..200)) {
        // Without the signing key, a valid card cannot be produced. This
        // mainly checks that the decoder returns instead of panicking.
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
    fn plaintext_round_trips(
        message_type in arb_message_type(),
        body in vec(any::<u8>(), 0..3000),
    ) {
        prop_assume!(message_type != MessageType::AuthProof);
        let plaintext = encode_plaintext(&PARAMS, message_type, &body).unwrap();
        prop_assert_eq!(plaintext.len() % 1024, 0);
        let (decoded_type, decoded_body) =
            decode_plaintext(&PARAMS, SessionState::AuthenticatedContact, &plaintext).unwrap();
        prop_assert_eq!(decoded_type, message_type);
        prop_assert_eq!(decoded_body, &body[..]);
    }

    #[test]
    fn accepted_plaintexts_have_one_encoding(
        header in any::<[u8; 4]>(),
        content in vec(any::<u8>(), 0..64),
        blocks in 1..=2_usize,
        zero_tail in any::<bool>(),
    ) {
        // A frame with an arbitrary header and either an all-zero or a
        // partly random tail. If it is accepted, re-encoding what was
        // decoded gives the same bytes.
        let mut plaintext = vec![0_u8; blocks * 1024];
        plaintext[..4].copy_from_slice(&header);
        if !zero_tail {
            let end = (4 + content.len()).min(plaintext.len());
            plaintext[4..end].copy_from_slice(&content[..end - 4]);
        }
        if let Ok((message_type, body)) =
            decode_plaintext(&PARAMS, SessionState::AuthenticatedContact, &plaintext)
        {
            prop_assert_eq!(encode_plaintext(&PARAMS, message_type, body).unwrap(), plaintext);
        }
    }

    #[test]
    fn the_frame_decoder_never_holds_more_than_one_frame(
        stream in vec(any::<u8>(), 0..6000),
        piece in 1..700_usize,
        contact in any::<bool>(),
    ) {
        // Arbitrary bytes as a stream. Whatever they are, the decoder
        // either fails or yields frames of legal length, and what it holds
        // is bounded by the largest frame of the state.
        let state = if contact {
            SessionState::AuthenticatedContact
        } else {
            SessionState::AuthenticatedUnknown
        };
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
                    Err(_) => break 'outer,
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

    #[test]
    fn application_data_is_delivered_only_on_a_confirmed_session(
        standing in arb_standing(),
        messages in vec(arb_message_type(), 0..12),
    ) {
        let (mut session, _) = authenticated(standing);
        for message in messages {
            let before = session.state();
            let result = session.receive(message);
            let after = session.state();
            prop_assert!(before == after || before.can_transition_to(after));
            if let Ok(actions) = result {
                if actions.contains(&Action::Deliver) {
                    prop_assert_eq!(before, SessionState::AuthenticatedContact);
                }
                if actions.contains(&Action::MarkAccepted) {
                    // Only an identity the user asked for can become a
                    // contact through a message from the peer.
                    prop_assert_eq!(standing, Standing::Requested);
                }
            } else {
                prop_assert_eq!(after, SessionState::Closed);
            }
        }
    }

    #[test]
    fn a_session_without_identity_proof_delivers_nothing(
        messages in vec(arb_message_type(), 1..12),
        steps in 0..=2_u8,
    ) {
        let mut session = Session::new();
        if steps >= 1 {
            session.stream_established().unwrap();
        }
        if steps >= 2 {
            session.handshake_completed().unwrap();
        }
        for message in messages {
            if let Ok(actions) = session.receive(message) {
                prop_assert!(!actions.contains(&Action::Deliver));
                prop_assert!(!actions.contains(&Action::MarkAccepted));
                prop_assert!(!actions.contains(&Action::ConsiderRequest));
            }
            prop_assert!(!session.state().is_authenticated());
        }
    }

    #[test]
    fn identities_that_are_not_contacts_see_the_same_thing(
        messages in vec(arb_message_type(), 0..8),
    ) {
        // T-ORACLE-1 for arbitrary peer behavior.
        let reference = transcript(Standing::None, &messages);
        prop_assert_eq!(&transcript(Standing::Declined, &messages), &reference);
        prop_assert_eq!(&transcript(Standing::Blocked, &messages), &reference);
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
            match resolve(local, remote, older, newer).unwrap() {
                Resolution::CloseOlder => newer,
                Resolution::ProbeOlder => match after_probe(true) {
                    ProbeOutcome::CloseNewer => older,
                    ProbeOutcome::CloseOlder => newer,
                },
            }
        };
        let kept_by_a = kept(&a, &b, a_sees_own_first);
        let kept_by_b = kept(&b, &a, b_sees_own_first);
        // "Local" for one side is "Remote" for the other.
        prop_assert_ne!(kept_by_a, kept_by_b);
        let a_is_smaller = a < b;
        prop_assert_eq!(kept_by_a == Initiator::Local, a_is_smaller);
    }

    #[test]
    fn text_validators_accept_or_reject_without_panicking(bytes in vec(any::<u8>(), 0..300)) {
        let _ = ChatText::from_bytes(&bytes);
        let _ = IntroductionText::from_bytes(&bytes);
        let _ = ProfileText::from_bytes(&bytes);
        let _ = DisplayName::from_bytes(&bytes);
        let _ = Filename::from_bytes(&bytes);
    }

    #[test]
    fn accepted_names_hold_their_invariants(text in "\\PC{0,80}") {
        if let Ok(name) = DisplayName::new(&text) {
            let name = name.as_str();
            prop_assert!(name.len() <= 128);
            prop_assert!(name.chars().count() <= 64);
            prop_assert!(!name.starts_with(' ') && !name.ends_with(' '));
            prop_assert!(!name.chars().any(char::is_control));
            let hidden = name.contains(['\u{202e}', '\u{200b}', '\u{feff}', '\u{a0}']);
            prop_assert!(!hidden);
        }
    }

    #[test]
    fn save_names_are_single_safe_components(
        chars in vec(
            prop::sample::select(vec![
                'a', 'B', '1', '.', ' ', '<', '>', ':', '"', '|', '?', '*', '-', 'C', 'O', 'N',
                'n', 'u', 'l', '\u{e9}',
            ]),
            1..40,
        ),
    ) {
        let text: String = chars.into_iter().collect();
        if let Ok(filename) = Filename::new(&text) {
            let name = filename.save_name();
            prop_assert!(!name.is_empty());
            prop_assert!(name != "." && name != "..");
            prop_assert!(!name.contains(['/', '\\', '<', '>', ':', '"', '|', '?', '*']));
            prop_assert!(!name.starts_with(['.', ' ']));
            prop_assert!(!name.ends_with(['.', ' ']));
            let stem = name.split('.').next().unwrap().trim_end_matches(' ');
            for device in ["CON", "PRN", "AUX", "NUL", "COM1", "LPT1"] {
                prop_assert!(!stem.eq_ignore_ascii_case(device));
            }
        }
    }
}

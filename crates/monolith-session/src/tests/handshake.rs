//! The handshake against a peer that does not follow the rules.
//!
//! A hostile peer is played with the Noise library directly, so that it
//! can send what the types of this crate would never produce: another
//! prologue, somebody else's card, a key that is not a key.

use std::sync::atomic::{AtomicUsize, Ordering};

use monolith_identity::IdentityPublicKey;
use monolith_protocol::limits::{
    CONTACT_CARD_BASE_LEN, HANDSHAKE_MSG1_LEN, HANDSHAKE_MSG2_LEN, HANDSHAKE_MSG3_LEN,
    HANDSHAKE_TIMEOUT,
};
use monolith_protocol::session::{PeerRecord, Standing};
use monolith_protocol::{ProtocolError, SessionState};
use snow::params::{CipherChoice, DHChoice, HashChoice};
use snow::resolvers::CryptoResolver;
use snow::types::{Cipher, Dh, Hash, Random};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::resolver::Resolver;
use crate::testing::{
    ALICE, BOB, EPHEMERAL_I, EPHEMERAL_R, MALLORY, admit_outbound, after, card, card_of, card_with,
    handshake, handshake_with, identity_secret, party, start, transport_bytes,
};
use crate::{
    HandshakeInitiator, HandshakeResponder, HandshakeResponderFinal, LocalParty, SessionError,
};
use monolith_protocol::duplicate::Initiator;

const NAME: &str = "Noise_XK_25519_ChaChaPoly_SHA256";
const LABEL: &[u8] = b"MONOLITH-SESSION-V1";

/// The points of small order of `docs/PROTOCOL.md` section 10.2.
const SMALL_ORDER: [[u8; 32]; 5] = [
    [0; 32],
    [
        1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0,
    ],
    [
        0xe0, 0xeb, 0x7a, 0x7c, 0x3b, 0x41, 0xb8, 0xae, 0x16, 0x56, 0xe3, 0xfa, 0xf1, 0x9f, 0xc4,
        0x6a, 0xda, 0x09, 0x8d, 0xeb, 0x9c, 0x32, 0xb1, 0xfd, 0x86, 0x62, 0x05, 0x16, 0x5f, 0x49,
        0xb8, 0x00,
    ],
    [
        0x5f, 0x9c, 0x95, 0xbc, 0xa3, 0x50, 0x8c, 0x24, 0xb1, 0xd0, 0xb1, 0x55, 0x9c, 0x83, 0xef,
        0x5b, 0x04, 0x44, 0x5c, 0xc4, 0x58, 0x1c, 0x8e, 0x86, 0xd8, 0x22, 0x4e, 0xdd, 0xd0, 0x9f,
        0x11, 0x57,
    ],
    [
        0xec, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        0xff, 0x7f,
    ],
];

fn failed(error: ProtocolError) -> SessionError {
    SessionError::Protocol(error)
}

/// A prologue as the handshake builds it, from parts a test chooses.
fn prologue(label: &[u8], responder: &IdentityPublicKey) -> Vec<u8> {
    let mut bytes = label.to_vec();
    bytes.extend_from_slice(responder.as_bytes());
    bytes
}

fn identity(seed: u8) -> IdentityPublicKey {
    identity_secret(seed).public_key()
}

/// The transport public key of a seed, as bytes.
fn transport_public(seed: u8) -> [u8; 32] {
    PublicKey::from(&StaticSecret::from(transport_bytes(seed))).to_bytes()
}

/// A Noise initiator driven by hand.
fn raw_initiator(
    prologue: &[u8],
    static_secret: &[u8; 32],
    responder_static: &[u8; 32],
    ephemeral: [u8; 32],
) -> snow::HandshakeState {
    snow::Builder::with_resolver(
        NAME.parse().unwrap(),
        Box::new(Resolver::with_fixed_ephemeral(ephemeral)),
    )
    .local_private_key(static_secret)
    .unwrap()
    .remote_public_key(responder_static)
    .unwrap()
    .prologue(prologue)
    .unwrap()
    .build_initiator()
    .unwrap()
}

/// A Noise responder driven by hand.
fn raw_responder(
    prologue: &[u8],
    static_secret: &[u8; 32],
    ephemeral: [u8; 32],
) -> snow::HandshakeState {
    snow::Builder::with_resolver(
        NAME.parse().unwrap(),
        Box::new(Resolver::with_fixed_ephemeral(ephemeral)),
    )
    .local_private_key(static_secret)
    .unwrap()
    .prologue(prologue)
    .unwrap()
    .build_responder()
    .unwrap()
}

fn bob_waiting() -> HandshakeResponder {
    HandshakeResponder::new_with_ephemeral(&party(BOB), start(), EPHEMERAL_R).unwrap()
}

fn alice_dialing_bob() -> (HandshakeInitiator, [u8; HANDSHAKE_MSG1_LEN]) {
    HandshakeInitiator::start_with_ephemeral(&party(ALICE), &card(BOB), start(), EPHEMERAL_I)
        .unwrap()
}

/// An initiator that holds the transport key of `static_seed` runs the
/// handshake with Bob correctly up to the third message and puts `payload`
/// where its card belongs. Returns Bob, waiting for that message, and the
/// message.
fn third_message_with(
    static_seed: u8,
    payload: &[u8],
) -> (HandshakeResponderFinal, [u8; HANDSHAKE_MSG3_LEN]) {
    let mut raw = raw_initiator(
        &prologue(LABEL, &identity(BOB)),
        &transport_bytes(static_seed),
        &transport_public(BOB),
        EPHEMERAL_I,
    );
    let mut message_1 = [0_u8; HANDSHAKE_MSG1_LEN];
    assert_eq!(raw.write_message(&[], &mut message_1), Ok(48));
    let (waiting, message_2) = bob_waiting().read_message_1(&message_1, start()).unwrap();
    assert_eq!(raw.read_message(&message_2, &mut []), Ok(0));
    let mut message_3 = [0_u8; HANDSHAKE_MSG3_LEN];
    assert_eq!(raw.write_message(payload, &mut message_3), Ok(235));
    (waiting, message_3)
}

#[test]
fn a_handshake_authenticates_both_sides() {
    let (outbound, inbound, transcript) = handshake(&party(ALICE), &party(BOB));
    // The initiator knows whom it dialed; the responder learns the
    // initiator from the card in message 3.
    assert_eq!(outbound.card(), &card(BOB));
    assert_eq!(inbound.card(), &card(ALICE));
    assert_eq!(transcript.message_1.len(), 48);
    assert_eq!(transcript.message_2.len(), 48);
    assert_eq!(transcript.message_3.len(), 235);

    let (alice, first) = admit_outbound(outbound, Standing::None);
    assert_eq!(first, Vec::new());
    let (bob, admission, first) = inbound.admit(PeerRecord::None).unwrap();
    assert_eq!(first, Vec::new());
    assert_eq!(admission.standing, Standing::None);
    assert_eq!(admission.card, None);

    assert_eq!(alice.peer(), &identity(BOB));
    assert_eq!(alice.peer_card(), &card(BOB));
    assert_eq!(bob.peer(), &identity(ALICE));
    assert_eq!(bob.peer_card(), &card(ALICE));
    assert_eq!(alice.initiator(), Initiator::Local);
    assert_eq!(bob.initiator(), Initiator::Remote);
    assert_eq!(alice.state(), SessionState::AuthenticatedUnknown);
    assert_eq!(bob.state(), SessionState::AuthenticatedUnknown);
    assert_eq!(alice.handshake_hash(), bob.handshake_hash());
    assert_ne!(alice.handshake_hash(), &[0; 32]);
}

#[test]
fn handshakes_with_fresh_randomness_differ_and_complete() {
    // The production path: ephemeral keys from the operating system.
    let run = || {
        let (first, message_1) =
            HandshakeInitiator::start(&party(ALICE), &card(BOB), start()).unwrap();
        let waiting = HandshakeResponder::new(&party(BOB), start()).unwrap();
        let (waiting, message_2) = waiting.read_message_1(&message_1, start()).unwrap();
        let (outbound, message_3) = first.read_message_2(&message_2, start()).unwrap();
        let inbound = waiting.read_message_3(&message_3, start()).unwrap();
        let (alice, _) = admit_outbound(outbound, Standing::None);
        let (bob, _, _) = inbound.admit(PeerRecord::None).unwrap();
        assert_eq!(alice.handshake_hash(), bob.handshake_hash());
        (message_1, message_2, message_3, *alice.handshake_hash())
    };
    let (a1, a2, a3, a_hash) = run();
    let (b1, b2, b3, b_hash) = run();
    assert_ne!(a1, b1);
    assert_ne!(a2, b2);
    assert_ne!(a3, b3);
    assert_ne!(a_hash, b_hash);
    // The ephemeral keys differ, and none is the fixed test key.
    assert_ne!(a1[..32], b1[..32]);
    assert_ne!(a2[..32], b2[..32]);
    let fixed = PublicKey::from(&StaticSecret::from(EPHEMERAL_I)).to_bytes();
    assert_ne!(a1[..32], fixed);
}

/// Returns true if `needle` occurs in `haystack`.
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn no_long_term_key_appears_in_a_handshake_message() {
    let (_, _, transcript) = handshake(&party(ALICE), &party(BOB));
    let secrets_and_names: Vec<Vec<u8>> = [ALICE, BOB]
        .into_iter()
        .flat_map(|seed| {
            [
                identity(seed).as_bytes().to_vec(),
                transport_public(seed).to_vec(),
                card(seed).signature().as_bytes()[..32].to_vec(),
                card(seed).endpoints().first().as_bytes().to_vec(),
            ]
        })
        .collect();
    for message in [
        &transcript.message_1[..],
        &transcript.message_2[..],
        &transcript.message_3[..],
    ] {
        for value in &secrets_and_names {
            assert!(!contains(message, value));
        }
    }
    // The first message is an ephemeral key and a tag. It is the same
    // whoever the initiator is: nothing of the initiator has entered it.
    let (_, from_mallory) =
        HandshakeInitiator::start_with_ephemeral(&party(MALLORY), &card(BOB), start(), EPHEMERAL_I)
            .unwrap();
    assert_eq!(from_mallory, transcript.message_1);
}

#[test]
fn an_endpoint_without_the_transport_key_cannot_answer() {
    // Outbound pinning. Alice dials Bob and reaches Mallory.
    let (alice, message_1) = alice_dialing_bob();

    // Mallory, following the protocol with her own keys, cannot read the
    // first message, so she has nothing to reply with.
    let mallory =
        HandshakeResponder::new_with_ephemeral(&party(MALLORY), start(), EPHEMERAL_R).unwrap();
    assert_eq!(
        mallory.read_message_1(&message_1, start()).err(),
        Some(failed(ProtocolError::HandshakeFailed))
    );

    // She cannot read it either if she claims to be Bob in the prologue:
    // she does not hold Bob's transport key.
    let mut claiming = raw_responder(
        &prologue(LABEL, &identity(BOB)),
        &transport_bytes(MALLORY),
        EPHEMERAL_R,
    );
    assert!(claiming.read_message(&message_1, &mut []).is_err());

    // Whatever she sends back, Alice does not accept it: an ephemeral key
    // with a made-up tag.
    let mut forged = [0x5A_u8; HANDSHAKE_MSG2_LEN];
    forged[..32].copy_from_slice(&transport_public(MALLORY));
    assert_eq!(
        alice.read_message_2(&forged, start()).err(),
        Some(failed(ProtocolError::IdentityMismatch))
    );
}

#[test]
fn a_second_message_of_another_handshake_is_not_accepted() {
    // A genuine message 2 from Bob, made for another first message.
    let (_, other_message_1) =
        HandshakeInitiator::start_with_ephemeral(&party(ALICE), &card(BOB), start(), [0x33; 32])
            .unwrap();
    let (_, other_message_2) = bob_waiting()
        .read_message_1(&other_message_1, start())
        .unwrap();

    let (alice, _) = alice_dialing_bob();
    assert_eq!(
        alice.read_message_2(&other_message_2, start()).err(),
        Some(failed(ProtocolError::IdentityMismatch))
    );
}

#[test]
fn a_card_that_names_another_partys_transport_key_gives_no_session() {
    // Rule F2. Mallory signs a card of her own identity that states Bob's
    // transport key and Bob's endpoint. Alice imports it, dials it and
    // reaches Bob. If this produced a session, Alice would take Bob for
    // Mallory.
    let misbinding = card_with(MALLORY, BOB, 1, BOB, false);
    assert_eq!(misbinding.identity(), &identity(MALLORY));
    assert_eq!(misbinding.transport(), card(BOB).transport());

    let (alice, message_1) =
        HandshakeInitiator::start_with_ephemeral(&party(ALICE), &misbinding, start(), EPHEMERAL_I)
            .unwrap();
    assert_eq!(
        bob_waiting().read_message_1(&message_1, start()).err(),
        Some(failed(ProtocolError::HandshakeFailed))
    );
    assert!(handshake_with(&party(ALICE), &party(BOB), &misbinding).is_err());

    // What stops it is the identity key in the prologue and nothing else:
    // a responder with Bob's transport key that put Mallory's identity
    // into its prologue would be accepted by Alice.
    let mut wrong = raw_responder(
        &prologue(LABEL, &identity(MALLORY)),
        &transport_bytes(BOB),
        EPHEMERAL_R,
    );
    assert_eq!(wrong.read_message(&message_1, &mut []), Ok(0));
    let mut message_2 = [0_u8; HANDSHAKE_MSG2_LEN];
    assert_eq!(wrong.write_message(&[], &mut message_2), Ok(48));
    assert!(alice.read_message_2(&message_2, start()).is_ok());
}

#[test]
fn another_label_or_prologue_fails_the_first_message() {
    let bob = identity(BOB);
    let prologues: Vec<Vec<u8>> = vec![
        // A later version of the protocol.
        prologue(b"MONOLITH-SESSION-V2", &bob),
        // The label alone, as if the identity were not bound.
        LABEL.to_vec(),
        // No prologue at all.
        Vec::new(),
        // The right label and another identity.
        prologue(LABEL, &identity(MALLORY)),
        // The identity in front of the label.
        [bob.as_bytes().as_slice(), LABEL].concat(),
        // One bit of the label changed.
        prologue(b"MONOLITH-SESSION-V0", &bob),
    ];
    for other in &prologues {
        let mut raw = raw_initiator(
            other,
            &transport_bytes(ALICE),
            &transport_public(BOB),
            EPHEMERAL_I,
        );
        let mut message_1 = [0_u8; HANDSHAKE_MSG1_LEN];
        assert_eq!(raw.write_message(&[], &mut message_1), Ok(48));
        assert_eq!(
            bob_waiting().read_message_1(&message_1, start()).err(),
            Some(failed(ProtocolError::HandshakeFailed)),
            "prologue of {} bytes",
            other.len()
        );
    }
    // The right prologue is accepted from the same raw initiator.
    let mut raw = raw_initiator(
        &prologue(LABEL, &bob),
        &transport_bytes(ALICE),
        &transport_public(BOB),
        EPHEMERAL_I,
    );
    let mut message_1 = [0_u8; HANDSHAKE_MSG1_LEN];
    raw.write_message(&[], &mut message_1).unwrap();
    assert!(bob_waiting().read_message_1(&message_1, start()).is_ok());
}

#[test]
fn another_noise_pattern_fails_the_first_message() {
    // The same primitives in another pattern. NK also starts with an
    // ephemeral key and a tag, 48 bytes, made with the responder's
    // transport key. The protocol name is part of the handshake hash, so
    // the tag does not verify. Nothing is negotiated and nothing can be
    // downgraded: a peer that speaks anything else is a peer that fails.
    let bob_prologue = prologue(LABEL, &identity(BOB));
    let bob_transport = transport_public(BOB);
    for name in [
        "Noise_NK_25519_ChaChaPoly_SHA256",
        "Noise_XKpsk3_25519_ChaChaPoly_SHA256",
    ] {
        let mut builder = snow::Builder::with_resolver(
            name.parse().unwrap(),
            Box::new(Resolver::with_fixed_ephemeral(EPHEMERAL_I)),
        )
        .remote_public_key(&bob_transport)
        .unwrap()
        .prologue(&bob_prologue)
        .unwrap();
        let static_secret = transport_bytes(ALICE);
        if name.contains("XK") {
            builder = builder
                .local_private_key(&static_secret)
                .unwrap()
                .psk(3, &[7; 32])
                .unwrap();
        }
        let mut raw = builder.build_initiator().unwrap();
        let mut message_1 = [0_u8; HANDSHAKE_MSG1_LEN];
        assert_eq!(raw.write_message(&[], &mut message_1), Ok(48), "{name}");
        assert_eq!(
            bob_waiting().read_message_1(&message_1, start()).err(),
            Some(failed(ProtocolError::HandshakeFailed)),
            "{name}"
        );
    }
}

#[test]
fn a_pinned_card_with_an_invitation_can_be_dialed() {
    // The card a user imports may carry an invitation capability. The
    // handshake uses its identity key and its transport key; the
    // capability goes into the contact request, inside the session.
    let imported = card_of(BOB, BOB, 1, true);
    assert!(imported.invitation().is_some());
    let (outbound, inbound, transcript) =
        handshake_with(&party(ALICE), &party(BOB), &imported).unwrap();
    // It is the same handshake as with the card without the capability.
    let (_, _, plain) = handshake(&party(ALICE), &party(BOB));
    assert_eq!(transcript.message_1, plain.message_1);
    assert_eq!(transcript.message_3, plain.message_3);
    let (alice, first) = admit_outbound(outbound, Standing::Requested);
    assert_eq!(
        first,
        vec![monolith_protocol::session::Action::SendContactRequest]
    );
    assert_eq!(alice.peer_card(), &imported);
    assert_eq!(inbound.card(), &card(ALICE));
}

#[test]
fn an_older_pinned_card_with_the_current_transport_key_still_reaches_the_peer() {
    // Bob has issued a card of epoch 2 with a new endpoint and the same
    // transport key. Alice still holds epoch 1. The handshake depends on
    // the identity key and the transport key, which did not change.
    let bob_now = LocalParty::new(
        card_with(BOB, BOB, 2, MALLORY, false),
        crate::testing::transport_secret(BOB),
    )
    .unwrap();
    assert!(handshake_with(&party(ALICE), &bob_now, &card(BOB)).is_ok());

    // After Bob replaced his transport key, the old card reaches nobody:
    // to Alice this is an identity mismatch.
    let bob_rekeyed = LocalParty::new(
        card_of(BOB, MALLORY, 2, false),
        crate::testing::transport_secret(MALLORY),
    )
    .unwrap();
    let (alice, message_1) = alice_dialing_bob();
    let waiting =
        HandshakeResponder::new_with_ephemeral(&bob_rekeyed, start(), EPHEMERAL_R).unwrap();
    assert_eq!(
        waiting.read_message_1(&message_1, start()).err(),
        Some(failed(ProtocolError::HandshakeFailed))
    );
    drop(alice);
    // With the new card she reaches him.
    assert!(handshake_with(&party(ALICE), &bob_rekeyed, bob_rekeyed.card()).is_ok());
}

#[test]
fn a_responder_with_another_label_is_not_accepted() {
    // Both sides of a complete handshake under another label, with Bob's
    // real keys. Its second message means nothing to an initiator of this
    // version, and the initiator never retries under another label.
    let other = prologue(b"MONOLITH-SESSION-V2", &identity(BOB));
    let mut raw_i = raw_initiator(
        &other,
        &transport_bytes(ALICE),
        &transport_public(BOB),
        EPHEMERAL_I,
    );
    let mut raw_r = raw_responder(&other, &transport_bytes(BOB), EPHEMERAL_R);
    let mut message_1 = [0_u8; HANDSHAKE_MSG1_LEN];
    let mut message_2 = [0_u8; HANDSHAKE_MSG2_LEN];
    raw_i.write_message(&[], &mut message_1).unwrap();
    raw_r.read_message(&message_1, &mut []).unwrap();
    raw_r.write_message(&[], &mut message_2).unwrap();

    let (alice, _) = alice_dialing_bob();
    assert_eq!(
        alice.read_message_2(&message_2, start()).err(),
        Some(failed(ProtocolError::IdentityMismatch))
    );
}

#[test]
fn the_card_in_the_third_message_must_belong_to_the_key_holder() {
    // Rule F3. Each initiator below holds a transport key and completes
    // the Noise handshake with it. What differs is the card it presents.

    // A valid card of its own: accepted, whoever it is. Mallory is nobody
    // Bob knows.
    let (waiting, message_3) = third_message_with(MALLORY, &card(MALLORY).encode());
    let peer = waiting.read_message_3(&message_3, start()).unwrap();
    assert_eq!(peer.card(), &card(MALLORY));

    // Alice's genuine card, which is public, presented by Mallory with
    // Mallory's key.
    let (waiting, message_3) = third_message_with(MALLORY, &card(ALICE).encode());
    assert_eq!(
        waiting.read_message_3(&message_3, start()).err(),
        Some(failed(ProtocolError::AuthenticationFailed))
    );

    // Alice's card with Mallory's transport key written into it. The
    // signature no longer verifies.
    let mut altered = card(ALICE).encode();
    altered[33..65].copy_from_slice(&transport_public(MALLORY));
    let (waiting, message_3) = third_message_with(MALLORY, &altered);
    assert_eq!(
        waiting.read_message_3(&message_3, start()).err(),
        Some(failed(ProtocolError::BadSignature))
    );

    // A card of Mallory's identity that states Alice's transport key,
    // presented by Mallory with her own key.
    let (waiting, message_3) =
        third_message_with(MALLORY, &card_of(MALLORY, ALICE, 1, false).encode());
    assert_eq!(
        waiting.read_message_3(&message_3, start()).err(),
        Some(failed(ProtocolError::AuthenticationFailed))
    );

    // Bytes that are not a card: a wrong version, and the right version
    // followed by an epoch of zero.
    let mut zero_epoch = [0_u8; CONTACT_CARD_BASE_LEN];
    zero_epoch[0] = 1;
    for (payload, expected) in [
        (
            [0_u8; CONTACT_CARD_BASE_LEN],
            ProtocolError::UnsupportedVersion,
        ),
        (zero_epoch, ProtocolError::InvalidValue),
    ] {
        let (waiting, message_3) = third_message_with(MALLORY, &payload);
        assert_eq!(
            waiting.read_message_3(&message_3, start()).err(),
            Some(failed(expected))
        );
    }

    // The front of a card that carries an invitation capability. The card
    // of a handshake never has one.
    let with_invitation = card_of(MALLORY, MALLORY, 1, true).encode();
    let (waiting, message_3) =
        third_message_with(MALLORY, &with_invitation[..CONTACT_CARD_BASE_LEN]);
    assert_eq!(
        waiting.read_message_3(&message_3, start()).err(),
        Some(failed(ProtocolError::BadMessageLength))
    );
}

#[test]
fn an_initiator_cannot_present_the_responders_own_identity() {
    // A card of Bob's identity that states Mallory's transport key. Nobody
    // but Bob can sign one; the test does it with Bob's test key. The
    // transport key matches the key holder, so only the last check of
    // PROTOCOL.md 4.4 stands in the way.
    let own = card_of(BOB, MALLORY, 1, false);
    assert_eq!(own.identity(), &identity(BOB));
    let (waiting, message_3) = third_message_with(MALLORY, &own.encode());
    assert_eq!(
        waiting.read_message_3(&message_3, start()).err(),
        Some(failed(ProtocolError::AuthenticationFailed))
    );
}

#[test]
fn a_payload_of_another_size_is_not_a_third_message() {
    // Noise would carry any payload. The third message has one size, and
    // the reader takes exactly that many bytes, so a shorter or an empty
    // payload cannot be presented at all; a message of the right size
    // built for a shorter payload fails authentication.
    let mut raw = raw_initiator(
        &prologue(LABEL, &identity(BOB)),
        &transport_bytes(MALLORY),
        &transport_public(BOB),
        EPHEMERAL_I,
    );
    let mut message_1 = [0_u8; HANDSHAKE_MSG1_LEN];
    raw.write_message(&[], &mut message_1).unwrap();
    let (waiting, message_2) = bob_waiting().read_message_1(&message_1, start()).unwrap();
    raw.read_message(&message_2, &mut []).unwrap();
    let mut short = [0_u8; HANDSHAKE_MSG3_LEN];
    let written = raw
        .write_message(&card(MALLORY).encode()[..100], &mut short)
        .unwrap();
    assert_eq!(written, 164);
    assert_eq!(
        waiting.read_message_3(&short, start()).err(),
        Some(failed(ProtocolError::HandshakeFailed))
    );
}

/// A Diffie-Hellman object that presents a public key of the test's
/// choice. With a private key it computes X25519 as usual; without one its
/// result is all zeros, which is what every honest party computes with a
/// point of small order.
struct ForgedKey {
    public: [u8; 32],
    secret: Option<StaticSecret>,
}

impl Dh for ForgedKey {
    fn name(&self) -> &'static str {
        "25519"
    }
    fn pub_len(&self) -> usize {
        32
    }
    fn priv_len(&self) -> usize {
        32
    }
    fn set(&mut self, _: &[u8]) {}
    fn generate(&mut self, _: &mut dyn Random) -> Result<(), snow::Error> {
        Ok(())
    }
    fn pubkey(&self) -> &[u8] {
        &self.public
    }
    fn privkey(&self) -> &[u8] {
        &[]
    }
    fn dh(&self, pubkey: &[u8], out: &mut [u8]) -> Result<(), snow::Error> {
        let shared = match &self.secret {
            Some(secret) => {
                let their: [u8; 32] = pubkey[..32].try_into().unwrap();
                *secret.diffie_hellman(&PublicKey::from(their)).as_bytes()
            }
            None => [0; 32],
        };
        out[..32].copy_from_slice(&shared);
        Ok(())
    }
}

/// A resolver whose static key is a [`ForgedKey`]. Everything else is the
/// resolver of this crate.
struct ForgedStatic {
    inner: Resolver,
    public: [u8; 32],
    secret: Option<[u8; 32]>,
    resolved: AtomicUsize,
}

impl CryptoResolver for ForgedStatic {
    fn resolve_rng(&self) -> Option<Box<dyn Random>> {
        self.inner.resolve_rng()
    }
    fn resolve_dh(&self, choice: &DHChoice) -> Option<Box<dyn Dh>> {
        // snow asks for the static key object first, then the ephemeral.
        if self.resolved.fetch_add(1, Ordering::Relaxed) == 0 {
            Some(Box::new(ForgedKey {
                public: self.public,
                secret: self.secret.map(StaticSecret::from),
            }))
        } else {
            self.inner.resolve_dh(choice)
        }
    }
    fn resolve_hash(&self, choice: &HashChoice) -> Option<Box<dyn Hash>> {
        self.inner.resolve_hash(choice)
    }
    fn resolve_cipher(&self, choice: &CipherChoice) -> Option<Box<dyn Cipher>> {
        self.inner.resolve_cipher(choice)
    }
}

/// Runs the handshake with Bob as an initiator whose transport public key
/// is `public`, up to the third message, which carries `payload`.
fn third_message_with_forged_static(
    public: [u8; 32],
    secret: Option<[u8; 32]>,
    payload: &[u8],
) -> (HandshakeResponderFinal, [u8; HANDSHAKE_MSG3_LEN]) {
    let resolver = ForgedStatic {
        inner: Resolver::with_fixed_ephemeral(EPHEMERAL_I),
        public,
        secret,
        resolved: AtomicUsize::new(0),
    };
    let bob_prologue = prologue(LABEL, &identity(BOB));
    let mut raw = snow::Builder::with_resolver(NAME.parse().unwrap(), Box::new(resolver))
        .local_private_key(&[0; 32])
        .unwrap()
        .remote_public_key(&transport_public(BOB))
        .unwrap()
        .prologue(&bob_prologue)
        .unwrap()
        .build_initiator()
        .unwrap();
    let mut message_1 = [0_u8; HANDSHAKE_MSG1_LEN];
    raw.write_message(&[], &mut message_1).unwrap();
    let (waiting, message_2) = bob_waiting().read_message_1(&message_1, start()).unwrap();
    raw.read_message(&message_2, &mut []).unwrap();
    let mut message_3 = [0_u8; HANDSHAKE_MSG3_LEN];
    assert_eq!(raw.write_message(payload, &mut message_3), Ok(235));
    (waiting, message_3)
}

#[test]
fn a_transport_key_of_small_order_ends_the_handshake() {
    // Rule F5 for the key in message 3. With a point of small order as
    // its transport key an initiator knows the result of the
    // Diffie-Hellman operation without holding any private key: it is
    // zero. The message it builds is one that a responder without the
    // rule would decrypt.
    for point in SMALL_ORDER {
        let (waiting, message_3) =
            third_message_with_forged_static(point, None, &card(MALLORY).encode());
        assert_eq!(
            waiting.read_message_3(&message_3, start()).err(),
            Some(failed(ProtocolError::HandshakeFailed))
        );
    }
}

#[test]
fn a_transport_key_that_is_not_canonical_ends_the_handshake() {
    // Mallory's real key with the top bit set. X25519 ignores the bit, so
    // both sides would compute the same secret; the encoding is refused.
    let mut public = transport_public(MALLORY);
    public[31] |= 0x80;
    let (waiting, message_3) = third_message_with_forged_static(
        public,
        Some(transport_bytes(MALLORY)),
        &card(MALLORY).encode(),
    );
    assert_eq!(
        waiting.read_message_3(&message_3, start()).err(),
        Some(failed(ProtocolError::HandshakeFailed))
    );

    // The forged-key initiator itself is sound: with the canonical key
    // and its secret it is accepted.
    let (waiting, message_3) = third_message_with_forged_static(
        transport_public(MALLORY),
        Some(transport_bytes(MALLORY)),
        &card(MALLORY).encode(),
    );
    assert!(waiting.read_message_3(&message_3, start()).is_ok());
}

#[test]
fn an_ephemeral_key_that_is_not_valid_ends_the_handshake() {
    // Rule F5 for the keys in messages 1 and 2: each point of small order,
    // and a valid key with the top bit set.
    let (_, good_1) = alice_dialing_bob();
    let (_, good_2) = bob_waiting().read_message_1(&good_1, start()).unwrap();
    let mut top_bit = [0_u8; 32];
    top_bit.copy_from_slice(&good_1[..32]);
    top_bit[31] |= 0x80;

    for key in SMALL_ORDER.into_iter().chain([top_bit]) {
        let mut message_1 = good_1;
        message_1[..32].copy_from_slice(&key);
        assert_eq!(
            bob_waiting().read_message_1(&message_1, start()).err(),
            Some(failed(ProtocolError::HandshakeFailed))
        );

        let mut message_2 = good_2;
        message_2[..32].copy_from_slice(&key);
        let (alice, _) = alice_dialing_bob();
        assert_eq!(
            alice.read_message_2(&message_2, start()).err(),
            Some(failed(ProtocolError::IdentityMismatch))
        );
    }
}

#[test]
fn malformed_handshake_messages_are_rejected() {
    for filler in [0x00_u8, 0x01, 0x55, 0xff] {
        assert_eq!(
            bob_waiting()
                .read_message_1(&[filler; HANDSHAKE_MSG1_LEN], start())
                .err(),
            Some(failed(ProtocolError::HandshakeFailed)),
            "message 1 of {filler:#x}"
        );
        let (alice, _) = alice_dialing_bob();
        assert_eq!(
            alice
                .read_message_2(&[filler; HANDSHAKE_MSG2_LEN], start())
                .err(),
            Some(failed(ProtocolError::IdentityMismatch)),
            "message 2 of {filler:#x}"
        );
        let (_, message_1) = alice_dialing_bob();
        let (waiting, _) = bob_waiting().read_message_1(&message_1, start()).unwrap();
        assert_eq!(
            waiting
                .read_message_3(&[filler; HANDSHAKE_MSG3_LEN], start())
                .err(),
            Some(failed(ProtocolError::HandshakeFailed)),
            "message 3 of {filler:#x}"
        );
    }
}

#[test]
fn every_byte_of_every_handshake_message_is_authenticated() {
    // One bit flipped in each byte of each message, a different bit from
    // byte to byte. The property tests flip arbitrary bits.
    let (_, _, transcript) = handshake(&party(ALICE), &party(BOB));

    for index in 0..HANDSHAKE_MSG1_LEN {
        let mut message = transcript.message_1;
        message[index] ^= 1 << (index % 8);
        assert!(
            bob_waiting().read_message_1(&message, start()).is_err(),
            "message 1 byte {index}"
        );
    }
    for index in 0..HANDSHAKE_MSG2_LEN {
        let mut message = transcript.message_2;
        message[index] ^= 1 << (index % 8);
        let (alice, _) = alice_dialing_bob();
        assert_eq!(
            alice.read_message_2(&message, start()).err(),
            Some(failed(ProtocolError::IdentityMismatch)),
            "message 2 byte {index}"
        );
    }
    for index in 0..HANDSHAKE_MSG3_LEN {
        let mut message = transcript.message_3;
        message[index] ^= 1 << (index % 8);
        let (waiting, _) = bob_waiting()
            .read_message_1(&transcript.message_1, start())
            .unwrap();
        assert_eq!(
            waiting.read_message_3(&message, start()).err(),
            Some(failed(ProtocolError::HandshakeFailed)),
            "message 3 byte {index}"
        );
    }
}

#[test]
fn handshake_messages_cannot_be_replayed() {
    let (_, _, recorded) = handshake(&party(ALICE), &party(BOB));

    // A recorded first message gets a reply from a new responder: it
    // carries nothing that could be fresh. The reply is made with a new
    // ephemeral key, and whoever replays the message cannot use it.
    let fresh = HandshakeResponder::new(&party(BOB), start()).unwrap();
    let (waiting, reply) = fresh.read_message_1(&recorded.message_1, start()).unwrap();
    assert_ne!(reply, recorded.message_2);
    // The recorded third message does not fit the new handshake.
    assert_eq!(
        waiting.read_message_3(&recorded.message_3, start()).err(),
        Some(failed(ProtocolError::HandshakeFailed))
    );

    // A recorded second message does not fit a new first message.
    let (alice, _) = HandshakeInitiator::start(&party(ALICE), &card(BOB), start()).unwrap();
    assert_eq!(
        alice.read_message_2(&recorded.message_2, start()).err(),
        Some(failed(ProtocolError::IdentityMismatch))
    );

    // The third message is taken once. The state that read it is gone,
    // and what arrives afterwards on the stream is read as frames: the
    // same bytes again are not a frame.
    let (waiting, _) = bob_waiting()
        .read_message_1(&recorded.message_1, start())
        .unwrap();
    let inbound = waiting
        .read_message_3(&recorded.message_3, start())
        .unwrap();
    let (mut bob, _, _) = inbound.admit(PeerRecord::None).unwrap();
    assert!(bob.receive(&recorded.message_3, start()).is_err());
    assert_eq!(bob.state(), SessionState::Closed);
}

#[test]
fn handshake_messages_cannot_be_reflected() {
    let (alice, message_1) = alice_dialing_bob();
    let (_, message_2) = bob_waiting().read_message_1(&message_1, start()).unwrap();

    // The initiator's own first message sent back to it.
    assert_eq!(
        alice.read_message_2(&message_1, start()).err(),
        Some(failed(ProtocolError::IdentityMismatch))
    );
    // The responder's second message sent to a responder as a first one.
    assert_eq!(
        bob_waiting().read_message_1(&message_2, start()).err(),
        Some(failed(ProtocolError::HandshakeFailed))
    );
    // Alice dialing herself through a mirror: her first message returned
    // to a responder of her own.
    let own = HandshakeResponder::new_with_ephemeral(&party(ALICE), start(), EPHEMERAL_R).unwrap();
    assert_eq!(
        own.read_message_1(&message_1, start()).err(),
        Some(failed(ProtocolError::HandshakeFailed))
    );
}

#[test]
fn a_party_does_not_dial_its_own_identity() {
    let alice = party(ALICE);
    assert_eq!(
        HandshakeInitiator::start(&alice, &card(ALICE), start()).err(),
        Some(SessionError::OwnIdentity)
    );
    // Another card of the same identity, with another transport key.
    assert_eq!(
        HandshakeInitiator::start(&alice, &card_of(ALICE, MALLORY, 2, false), start()).err(),
        Some(SessionError::OwnIdentity)
    );
}

#[test]
fn a_stalled_handshake_times_out() {
    let just_in_time = after(HANDSHAKE_TIMEOUT);
    let too_late = after(HANDSHAKE_TIMEOUT + core::time::Duration::from_millis(1));
    let timed_out = Some(failed(ProtocolError::TimedOut));

    let (alice, message_1) = alice_dialing_bob();
    assert_eq!(alice.deadline(), Some(just_in_time));
    let bob = bob_waiting();
    assert_eq!(bob.deadline(), Some(just_in_time));

    // Message 1 arrives too late, or just in time.
    assert_eq!(bob.read_message_1(&message_1, too_late).err(), timed_out);
    let (waiting, message_2) = bob_waiting()
        .read_message_1(&message_1, just_in_time)
        .unwrap();
    // The clock of a handshake starts when the stream is accepted, not
    // when a message arrives.
    assert_eq!(waiting.deadline(), Some(just_in_time));

    // Message 2 arrives too late.
    assert_eq!(alice.read_message_2(&message_2, too_late).err(), timed_out);
    let (alice, _) = alice_dialing_bob();
    let (_, message_3) = alice.read_message_2(&message_2, just_in_time).unwrap();

    // Message 3 arrives too late: a peer that sends a valid first message
    // and then nothing holds a pending handshake for this long and no
    // longer.
    assert_eq!(
        waiting.read_message_3(&message_3, too_late).err(),
        timed_out
    );
    let (waiting, _) = bob_waiting().read_message_1(&message_1, start()).unwrap();
    assert!(waiting.read_message_3(&message_3, just_in_time).is_ok());

    // A clock that reads earlier than the start does not end a handshake.
    let (alice, message_1) = HandshakeInitiator::start_with_ephemeral(
        &party(ALICE),
        &card(BOB),
        after(core::time::Duration::from_secs(5)),
        EPHEMERAL_I,
    )
    .unwrap();
    let (_, message_2) = bob_waiting().read_message_1(&message_1, start()).unwrap();
    assert!(alice.read_message_2(&message_2, start()).is_ok());
}

#[test]
fn the_local_party_of_a_session_is_fixed_by_its_card() {
    // The responder's identity in the prologue comes from its card. A
    // party with Bob's transport key and another identity is not Bob.
    let impostor = LocalParty::new(
        card_of(MALLORY, BOB, 1, false),
        crate::testing::transport_secret(BOB),
    )
    .unwrap();
    let (_, message_1) = alice_dialing_bob();
    let waiting = HandshakeResponder::new_with_ephemeral(&impostor, start(), EPHEMERAL_R).unwrap();
    assert_eq!(
        waiting.read_message_1(&message_1, start()).err(),
        Some(failed(ProtocolError::HandshakeFailed))
    );
}

#[test]
fn debug_output_of_handshake_types_shows_nothing() {
    let (alice, message_1) = alice_dialing_bob();
    assert_eq!(format!("{alice:?}"), "HandshakeInitiator([redacted])");
    let bob = bob_waiting();
    assert_eq!(format!("{bob:?}"), "HandshakeResponder([redacted])");
    let (waiting, message_2) = bob.read_message_1(&message_1, start()).unwrap();
    assert_eq!(
        format!("{waiting:?}"),
        "HandshakeResponderFinal([redacted])"
    );
    let (outbound, message_3) = alice.read_message_2(&message_2, start()).unwrap();
    assert_eq!(format!("{outbound:?}"), "OutboundPeer([redacted])");
    let inbound = waiting.read_message_3(&message_3, start()).unwrap();
    assert_eq!(format!("{inbound:?}"), "InboundPeer([redacted])");

    let (session, _) = admit_outbound(outbound, Standing::Accepted);
    let text = format!("{session:?}");
    assert!(
        text.starts_with("AuthenticatedSession { state: AuthenticatedUnknown"),
        "{text}"
    );
    for secret in [
        crate::testing::hex(session.handshake_hash()),
        crate::testing::hex(identity(BOB).as_bytes()),
        crate::testing::hex(&transport_bytes(ALICE)),
    ] {
        assert!(!text.contains(&secret[..16]), "{text}");
    }
    assert!(
        !text.contains("Accepted"),
        "the standing is not printed: {text}"
    );
}

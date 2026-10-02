//! Known-answer tests: the vectors of `docs/PROTOCOL.md` section 16.1.
//!
//! The expected values were produced by an implementation that shares no
//! code with this workspace. This crate has to reproduce them byte for
//! byte: the handshake messages, the handshake hash and the first frames.
//! A resolver, a library version or a parameter that differs from the
//! specification fails here.

use std::fs;
use std::path::Path;

use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::SessionState;
use monolith_protocol::body::Message;
use monolith_protocol::card::EndpointSet;
use monolith_protocol::session::{Action, PeerRecord, Standing};
use sha2::{Digest, Sha256};

use crate::testing::{EPHEMERAL_I, EPHEMERAL_R, admit_outbound, deliver, hex, start, unhex};
use crate::{HandshakeInitiator, HandshakeResponder, LocalParty, TransportSecretKey};

const R_IDENTITY_SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const R_IDENTITY: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
const R_TRANSPORT_SECRET: &str = "77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a";
const R_TRANSPORT: &str = "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a";

const I_IDENTITY_SEED: &str = "4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb";
const I_IDENTITY: &str = "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c";
const I_TRANSPORT_SECRET: &str = "5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb";
const I_TRANSPORT: &str = "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f";

const I_EPHEMERAL: &str = "7b4e909bbe7ffe44c465a220037d608ee35897d31ef972f07f74892cb0f73f13";
const R_EPHEMERAL: &str = "0faa684ed28867b97f4a6a2dee5df8ce974e76b7018e3f22a1c4cf2678570f20";

const I_CARD: &str = concat!(
    "013d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af466",
    "0cde9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b",
    "4f000000000000000101ed4928c628d1c2c6eae90338905995612959273a5c63",
    "f93636c14614ac8737d1007c98b9a384ab4a8dd22e7b752249ae282a5d4072de",
    "c9c640f2d1d59de1844e5f3192e3263077fd11c77b5afb9f9b80d2e2d77b1648",
    "7b43119b9746b95f2e2f0d",
);

const MESSAGE_1: &str = concat!(
    "7b4e909bbe7ffe44c465a220037d608ee35897d31ef972f07f74892cb0f73f13",
    "c4613ae914614af97813408bce932efc",
);

const MESSAGE_2: &str = concat!(
    "0faa684ed28867b97f4a6a2dee5df8ce974e76b7018e3f22a1c4cf2678570f20",
    "04bdc426ae5793c3d04345705141dfbc",
);

const MESSAGE_3: &str = concat!(
    "30daabb09afb8c0ac3ab7397a17abdddbd1e51bb900dde33066249b26217d6e2",
    "9d6ab023301792b938bc608f195d252b8ab93333f1347d12417c36b28a88709f",
    "9bd9c35fc186e3137e0cf5821cc0fa8ecfed61861537004d0d158d40876abf47",
    "5886d070d79c147a6fdd424de135041ea7fb24b233841dc38611edb24427d4c5",
    "77d3025b18eba9a5467abb80bc931570f1055817fa7f4ab92d030b47fd845194",
    "3b221b2891bbd72e367b943a235f4972838c034bff66d46487e07c51fdb23786",
    "77a37b6f62792193522f75c3b4eb9aec237f8daf83e8ae9341f681ed2a618802",
    "db7d3684d5fc2968897ac9",
);

const HANDSHAKE_HASH: &str = "ef8f280b4bc4ccc6f2e5164984de6738f1b467a970841480f6ebdf61a69716a9";

const FIRST_FRAME_OF_I: &str = "b11fcacca95b6d25e038cc84eadeea2ede1e5e881fb6aa3a6138f743ec659785";
const FIRST_FRAME_OF_R: &str = "d2e82f5edebfb960a8ef4e58c29889a741c481f0990ad048662965c612c7fb19";
const SECOND_FRAME_OF_I: &str = "990ee883b76ee5ed77d00281c72569765ad8afdccd36d2736b629fa47cdf7c5a";

fn key32(text: &str) -> [u8; 32] {
    unhex(text).try_into().unwrap()
}

fn party(identity_seed: &str, transport_secret: &str, endpoint_seed: u8) -> LocalParty {
    let transport = TransportSecretKey::from_bytes(&key32(transport_secret)).unwrap();
    let endpoint_key = IdentitySecretKey::from_seed(&[endpoint_seed; 32]).public_key();
    LocalParty::issue(
        &IdentitySecretKey::from_seed(&key32(identity_seed)),
        transport,
        EndpointEpoch::FIRST,
        EndpointSet::single(OnionServiceKey::from_bytes(endpoint_key.as_bytes()).unwrap()),
    )
    .unwrap()
}

fn party_r() -> LocalParty {
    party(R_IDENTITY_SEED, R_TRANSPORT_SECRET, 2)
}

fn party_i() -> LocalParty {
    party(I_IDENTITY_SEED, I_TRANSPORT_SECRET, 3)
}

fn digest(frame: &[u8]) -> String {
    hex(&Sha256::digest(frame))
}

#[test]
fn the_parties_are_those_of_the_specification() {
    let responder = party_r();
    assert_eq!(hex(responder.identity().as_bytes()), R_IDENTITY);
    assert_eq!(hex(responder.card().transport().as_bytes()), R_TRANSPORT);
    let initiator = party_i();
    assert_eq!(hex(initiator.identity().as_bytes()), I_IDENTITY);
    assert_eq!(hex(initiator.card().transport().as_bytes()), I_TRANSPORT);
    assert_eq!(hex(&initiator.card().encode()), I_CARD);

    // The fixed ephemeral secrets have the public keys the document
    // states.
    for (secret, public) in [(EPHEMERAL_I, I_EPHEMERAL), (EPHEMERAL_R, R_EPHEMERAL)] {
        let key = TransportSecretKey::from_bytes(&secret).unwrap();
        assert_eq!(hex(key.public_key().as_bytes()), public);
    }
}

#[test]
fn the_handshake_reproduces_the_published_vectors() {
    let initiator = party_i();
    let responder = party_r();

    let (first, message_1) = HandshakeInitiator::start_with_ephemeral(
        &initiator,
        responder.card(),
        start(),
        EPHEMERAL_I,
    )
    .unwrap();
    assert_eq!(hex(&message_1), MESSAGE_1);

    let waiting = HandshakeResponder::new_with_ephemeral(&responder, start(), EPHEMERAL_R).unwrap();
    let (waiting, message_2) = waiting.read_message_1(&message_1, start()).unwrap();
    assert_eq!(hex(&message_2), MESSAGE_2);

    let (outbound, message_3) = first.read_message_2(&message_2, start()).unwrap();
    assert_eq!(hex(&message_3), MESSAGE_3);

    let inbound = waiting.read_message_3(&message_3, start()).unwrap();
    assert_eq!(inbound.card(), initiator.card());
    assert_eq!(outbound.card(), responder.card());

    // Both sides hold each other as accepted contacts, so the first frame
    // each sends is a ContactAccept.
    let (mut at_i, first_i) = admit_outbound(outbound, Standing::Accepted);
    let (mut at_r, admission, first_r) = inbound
        .admit(PeerRecord::Accepted(initiator.card()))
        .unwrap();
    assert_eq!(admission.standing, Standing::Accepted);
    assert_eq!(first_i, vec![Action::SendContactAccept]);
    assert_eq!(first_r, vec![Action::SendContactAccept]);
    assert_eq!(hex(at_i.handshake_hash()), HANDSHAKE_HASH);
    assert_eq!(hex(at_r.handshake_hash()), HANDSHAKE_HASH);

    let frame_i = at_i.send(&Message::ContactAccept, start()).unwrap();
    assert_eq!(frame_i.len(), 1042);
    assert_eq!(&frame_i[..2], &[0x04, 0x10]);
    assert_eq!(
        hex(&frame_i[..34]),
        "04103273bc8299d935f5154fc85fd061e8b973ac6f7fe701d3e990d205d2ac6971ed"
    );
    assert_eq!(digest(&frame_i), FIRST_FRAME_OF_I);

    let frame_r = at_r.send(&Message::ContactAccept, start()).unwrap();
    assert_eq!(frame_r.len(), 1042);
    assert_eq!(digest(&frame_r), FIRST_FRAME_OF_R);

    // Each side reads the other's frame, and the session is confirmed.
    let received = deliver(&mut at_r, &frame_i).unwrap().unwrap();
    assert_eq!(received.message, Message::ContactAccept);
    assert_eq!(received.actions, vec![Action::Confirmed]);
    let received = deliver(&mut at_i, &frame_r).unwrap().unwrap();
    assert_eq!(received.actions, vec![Action::Confirmed]);
    assert_eq!(at_i.state(), SessionState::AuthenticatedContact);
    assert_eq!(at_r.state(), SessionState::AuthenticatedContact);

    // The same plaintext again: the nonce has moved on, so the frame
    // differs.
    let second = at_i.send(&Message::ContactAccept, start()).unwrap();
    assert_eq!(digest(&second), SECOND_FRAME_OF_I);
    assert_ne!(second, frame_i);
    assert!(deliver(&mut at_r, &second).unwrap().is_some());
}

#[test]
fn the_vectors_are_the_ones_in_the_specification() {
    // The constants above are compared with the text of the document, so
    // that the two cannot drift apart unnoticed.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/PROTOCOL.md");
    let document: String = fs::read_to_string(path)
        .unwrap()
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    for (name, value) in [
        ("R identity seed", R_IDENTITY_SEED),
        ("R identity", R_IDENTITY),
        ("R transport secret", R_TRANSPORT_SECRET),
        ("R transport", R_TRANSPORT),
        ("I identity seed", I_IDENTITY_SEED),
        ("I identity", I_IDENTITY),
        ("I transport secret", I_TRANSPORT_SECRET),
        ("I transport", I_TRANSPORT),
        ("I ephemeral", I_EPHEMERAL),
        ("R ephemeral", R_EPHEMERAL),
        ("I card", I_CARD),
        ("message 1", MESSAGE_1),
        ("message 2", MESSAGE_2),
        ("message 3", MESSAGE_3),
        ("handshake hash", HANDSHAKE_HASH),
        ("first frame of I", FIRST_FRAME_OF_I),
        ("first frame of R", FIRST_FRAME_OF_R),
        ("second frame of I", SECOND_FRAME_OF_I),
    ] {
        assert!(document.contains(value), "{name} is not in PROTOCOL.md");
    }
    assert!(document.contains(&hex(&EPHEMERAL_I)));
    assert!(document.contains(&hex(&EPHEMERAL_R)));
    assert!(document.contains("Noise_XK_25519_ChaChaPoly_SHA256"));
    assert!(document.contains("MONOLITH-SESSION-V1"));
}

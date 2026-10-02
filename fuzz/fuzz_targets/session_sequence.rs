//! Session logic: an arbitrary sequence of events against a session of
//! arbitrary standing never produces application data before the session is
//! confirmed, never moves a session backwards, never accepts a card that
//! does not belong to the authenticated peer, and treats every identity
//! that is not a contact alike.
//!
//! Input: the low two bits of the first byte select how far the session got
//! before the events arrive, and the next bit whether the local side opened
//! it. The second byte selects the standing of the peer. Every further byte
//! is one event. Its low six bits select a message type or one of four
//! local events (close, block, remove, stream closed). Its two high bits
//! select the card inside the message: the card of the handshake, a later
//! card of the peer with the same transport key, a card of another
//! identity, or a card of the peer with another transport key.

#![no_main]

use std::sync::LazyLock;

use libfuzzer_sys::fuzz_target;
use monolith_identity::{
    EndpointEpoch, IdentityPublicKey, IdentitySecretKey, OnionServiceKey, TransportPublicKey,
};
use monolith_protocol::body::{ContactRequest, FileChunk, Message, MessageId, TransferId};
use monolith_protocol::card::{ContactCard, EndpointSet};
use monolith_protocol::session::{Action, Session, Standing};
use monolith_protocol::text::{ChatText, DisplayName, Filename, IntroductionText, ProfileText};
use monolith_protocol::{MessageType, ProtocolError, SessionState};

const STANDINGS: [Standing; 7] = [
    Standing::None,
    Standing::Declined,
    Standing::Blocked,
    Standing::Requested,
    Standing::Accepted,
    Standing::StaleCard,
    Standing::PendingSuccessor,
];

const PEER: [u8; 32] = [0x51; 32];
const STRANGER: [u8; 32] = [0x77; 32];

/// Number of local events that follow the message types in the selector.
const LOCAL_EVENTS: usize = 4;

/// A card of the identity of `seed`. Variant 0 is the card of the
/// handshake. Variant 1 is a later card with the same transport key and
/// another endpoint. Variant 2 states another transport key.
fn sample_card(seed: [u8; 32], variant: u8) -> ContactCard {
    let mut endpoint_seed = seed;
    endpoint_seed[0] ^= 0xff;
    endpoint_seed[1] ^= variant;
    let endpoint = OnionServiceKey::from_bytes(
        IdentitySecretKey::from_seed(&endpoint_seed)
            .public_key()
            .as_bytes(),
    )
    .unwrap();
    let mut transport = seed;
    transport[31] &= 0x7f;
    if variant == 2 {
        transport[0] ^= 0x01;
    }
    ContactCard::sign(
        &IdentitySecretKey::from_seed(&seed),
        TransportPublicKey::from_bytes(&transport).unwrap(),
        EndpointEpoch::new(u64::from(variant) + 1).unwrap(),
        EndpointSet::single(endpoint),
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

/// The card that stands for the peer: the one it presents in the handshake
/// of an inbound session, and the pinned one an outbound session dials.
static PEER_CARD: LazyLock<ContactCard> = LazyLock::new(|| sample_card(PEER, 0));

/// The four sources of messages, in the order the two high bits of an
/// event select them.
static SOURCES: LazyLock<[Vec<Message>; 4]> = LazyLock::new(|| {
    [
        samples(PEER_CARD.clone()),
        samples(sample_card(PEER, 1)),
        samples(sample_card(STRANGER, 0)),
        samples(sample_card(PEER, 2)),
    ]
});

static LOCAL_IDENTITY: LazyLock<IdentityPublicKey> =
    LazyLock::new(|| IdentitySecretKey::from_seed(&[0x10; 32]).public_key());

enum Event {
    Receive(&'static Message),
    Close,
    Block,
    Remove,
    StreamClosed,
}

fn event(byte: u8) -> Event {
    let source = &SOURCES[usize::from(byte >> 6)];
    let selector = usize::from(byte & 0x3f) % (MessageType::ALL.len() + LOCAL_EVENTS);
    match source.get(selector) {
        Some(message) => Event::Receive(message),
        None => match selector - MessageType::ALL.len() {
            0 => Event::Close,
            1 => Event::Block,
            2 => Event::Remove,
            _ => Event::StreamClosed,
        },
    }
}

/// Returns true if the message carries a card that the session must
/// refuse. This restates `docs/PROTOCOL.md` sections 8.3 and 8.8 without
/// using the code under test: a card of another identity is always
/// refused; the card of a request is the card of the handshake when the
/// peer opened the session, and states the authenticated transport key
/// when the local side did; an endpoint update may carry any card of the
/// peer.
fn has_foreign_card(message: &Message, outbound: bool) -> bool {
    let (card, is_request) = match message {
        Message::ContactRequest(request) => (&request.card, true),
        Message::EndpointUpdate(card) => (&**card, false),
        _ => return false,
    };
    if card.identity() != PEER_CARD.identity() {
        return true;
    }
    if !is_request {
        return false;
    }
    if outbound {
        card.transport() != PEER_CARD.transport()
    } else {
        *card != *PEER_CARD
    }
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

fn is_outbound(selector: u8) -> bool {
    selector & 0x04 != 0
}

fn start(selector: u8, standing: Standing) -> Session {
    let steps = selector & 0x03;
    let mut session = if is_outbound(selector) {
        Session::outbound(*LOCAL_IDENTITY, *PEER_CARD.identity())
    } else {
        Session::inbound(*LOCAL_IDENTITY)
    };
    if steps >= 1 {
        session.stream_established().unwrap();
    }
    if steps >= 2 {
        session.handshake_completed().unwrap();
    }
    if steps >= 3 {
        session.authenticated(PEER_CARD.clone(), standing).unwrap();
    }
    session
}

/// What the peer can observe: the visible actions per event and where the
/// session ends up.
fn observe(steps: u8, standing: Standing, bytes: &[u8]) -> (Vec<Vec<Action>>, bool, SessionState) {
    let mut session = start(steps, standing);
    let mut seen = Vec::new();
    let mut failed = false;
    for byte in bytes {
        match apply(&mut session, &event(*byte)) {
            Ok(actions) => seen.push(
                actions
                    .into_iter()
                    .filter(|action| action.is_visible_to_peer())
                    .collect(),
            ),
            Err(_) => {
                failed = true;
                break;
            }
        }
    }
    (seen, failed, session.state())
}

fuzz_target!(|data: &[u8]| {
    let [steps, standing_selector, rest @ ..] = data else {
        return;
    };
    let steps = *steps;
    let authenticated = steps & 0x03 == 3;
    let outbound = is_outbound(steps);
    let standing = STANDINGS[usize::from(*standing_selector) % STANDINGS.len()];

    let mut session = start(steps, standing);
    for byte in rest {
        let event = event(*byte);
        let before = session.state();
        let standing_before = session.standing();
        let result = apply(&mut session, &event);
        let after = session.state();

        assert!(before == after || before.can_transition_to(after));
        assert_eq!(session.peer().is_some(), authenticated);
        if authenticated {
            // The session keeps the card it was authenticated with,
            // whatever cards arrive in messages.
            assert_eq!(session.peer_card(), Some(&*PEER_CARD));
        }

        let live = matches!(
            before,
            SessionState::AuthenticatedUnknown | SessionState::AuthenticatedContact
        );
        if let Event::Receive(message) = &event {
            let legal = message.message_type().may_be_received_in(before);
            if live && !legal {
                assert_eq!(result, Err(ProtocolError::MessageNotPermitted));
            }
            if live && legal {
                let refused = result == Err(ProtocolError::IdentityMismatch);
                assert_eq!(refused, has_foreign_card(message, outbound));
            }
            if !live && !matches!(before, SessionState::Closing | SessionState::Closed) {
                // Before the peer is authenticated every message is a
                // violation.
                assert_eq!(result, Err(ProtocolError::MessageNotPermitted));
            }
        }

        match result {
            Ok(actions) => {
                if matches!(before, SessionState::Closing | SessionState::Closed) {
                    assert!(actions.is_empty());
                }
                if actions.contains(&Action::Deliver) {
                    assert_eq!(before, SessionState::AuthenticatedContact);
                    assert_eq!(standing_before, Standing::Accepted);
                }
                if actions.contains(&Action::MarkAccepted) {
                    assert_eq!(standing_before, Standing::Requested);
                }
                if actions.contains(&Action::ConsiderRequest) {
                    assert_eq!(standing_before, Standing::None);
                    assert_eq!(before, SessionState::AuthenticatedUnknown);
                    assert_eq!(actions, [Action::SendClose, Action::ConsiderRequest]);
                }
                if actions.contains(&Action::Confirmed) {
                    assert_eq!(after, SessionState::AuthenticatedContact);
                    assert_eq!(session.standing(), Standing::Accepted);
                }
                for action in &actions {
                    let sent = match action {
                        Action::SendContactAccept => MessageType::ContactAccept,
                        Action::SendContactRequest => MessageType::ContactRequest,
                        Action::SendClose => MessageType::Close,
                        _ => continue,
                    };
                    assert!(session.may_send(sent));
                }
            }
            Err(_) => {
                assert!(matches!(event, Event::Receive(_)));
                assert_eq!(after, SessionState::Closed);
                assert_eq!(session.standing(), standing_before);
            }
        }

        // Application messages may be sent on a confirmed session only, and
        // nothing at all before the peer is authenticated.
        for message_type in MessageType::ALL {
            if !session.may_send(message_type) {
                continue;
            }
            match after {
                SessionState::AuthenticatedContact => {}
                SessionState::AuthenticatedUnknown => assert!(matches!(
                    message_type,
                    MessageType::Close | MessageType::ContactAccept | MessageType::ContactRequest
                )),
                SessionState::Closing => assert_eq!(message_type, MessageType::Close),
                SessionState::Connecting
                | SessionState::CryptoHandshake
                | SessionState::IdentityAuth
                | SessionState::Closed => {
                    panic!("{message_type:?} may be sent in {after:?}")
                }
            }
        }
    }

    // Identities that are not contacts are indistinguishable to the peer,
    // and so is a contact that presented a stale card or a new key without
    // continuity.
    let reference = observe(steps, Standing::None, rest);
    assert_eq!(observe(steps, Standing::Declined, rest), reference);
    assert_eq!(observe(steps, Standing::Blocked, rest), reference);
    assert_eq!(observe(steps, Standing::StaleCard, rest), reference);
    assert_eq!(observe(steps, Standing::PendingSuccessor, rest), reference);
});

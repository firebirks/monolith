//! Session logic: an arbitrary sequence of events against a session of
//! arbitrary standing never produces application data before the session is
//! confirmed, never moves a session backwards, never accepts a card of
//! another identity, and treats every identity that is not a contact alike.
//!
//! Input: the low two bits of the first byte select how far the session got
//! before the events arrive, and the next bit whether the local side opened
//! it. The second byte selects the standing of the peer. Every further byte
//! is one event. Its low seven bits select a message type or one of four
//! local events (close, block, remove, stream closed). Its high bit makes
//! the message come from another identity: the card it carries, or the
//! identity an AuthProof names.

#![no_main]

use std::sync::LazyLock;

use libfuzzer_sys::fuzz_target;
use monolith_identity::{EndpointEpoch, IdentityPublicKey, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::body::{
    AuthProof, ContactRequest, FileChunk, Message, MessageId, TransferId,
};
use monolith_protocol::card::{ContactCard, EndpointSet};
use monolith_protocol::session::{Action, Session, Standing};
use monolith_protocol::text::{ChatText, DisplayName, Filename, IntroductionText, ProfileText};
use monolith_protocol::{MessageType, ProtocolError, SessionState};

const STANDINGS: [Standing; 5] = [
    Standing::None,
    Standing::Declined,
    Standing::Blocked,
    Standing::Requested,
    Standing::Accepted,
];

const PEER: [u8; 32] = [0x51; 32];
const STRANGER: [u8; 32] = [0x77; 32];

/// Number of local events that follow the message types in the selector.
const LOCAL_EVENTS: usize = 4;

/// One message of every type, in the order of `MessageType::ALL`. Cards
/// inside them are signed by the identity of `seed`.
fn samples(seed: [u8; 32]) -> Vec<Message> {
    let secret = IdentitySecretKey::from_seed(&seed);
    let mut endpoint_seed = seed;
    endpoint_seed[0] ^= 0xff;
    let endpoint = OnionServiceKey::from_bytes(
        IdentitySecretKey::from_seed(&endpoint_seed)
            .public_key()
            .as_bytes(),
    )
    .unwrap();
    let card = ContactCard::sign(
        &secret,
        EndpointEpoch::FIRST,
        EndpointSet::single(endpoint),
        None,
    )
    .unwrap();
    let transfer = TransferId::from_bytes([3; 16]);
    let id = MessageId::from_bytes([4; 16]);
    let all = vec![
        Message::AuthProof(Box::new(AuthProof {
            identity: secret.public_key(),
            features: 0,
            signature: secret.sign(b"stands in for a real proof"),
        })),
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

static FROM_PEER: LazyLock<Vec<Message>> = LazyLock::new(|| samples(PEER));
static FROM_STRANGER: LazyLock<Vec<Message>> = LazyLock::new(|| samples(STRANGER));
static PEER_IDENTITY: LazyLock<IdentityPublicKey> =
    LazyLock::new(|| IdentitySecretKey::from_seed(&PEER).public_key());
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
    let foreign = byte & 0x80 != 0;
    let selector = usize::from(byte & 0x7f) % (MessageType::ALL.len() + LOCAL_EVENTS);
    let source = if foreign { &FROM_STRANGER } else { &FROM_PEER };
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

fn has_foreign_card(message: &Message) -> bool {
    let signer = match message {
        Message::ContactRequest(request) => request.card.identity(),
        Message::EndpointUpdate(card) => card.identity(),
        _ => return false,
    };
    *signer != *PEER_IDENTITY
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

fn start(selector: u8, standing: Standing) -> Session {
    let steps = selector & 0x03;
    let mut session = if selector & 0x04 == 0 {
        Session::inbound(*LOCAL_IDENTITY)
    } else {
        Session::outbound(*LOCAL_IDENTITY, *PEER_IDENTITY)
    };
    if steps >= 1 {
        session.stream_established().unwrap();
    }
    if steps >= 2 {
        session.handshake_completed().unwrap();
    }
    if steps >= 3 {
        session.receive(&FROM_PEER[0]).unwrap();
        session.identities_proven(*PEER_IDENTITY, standing).unwrap();
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

        let live = matches!(
            before,
            SessionState::AuthenticatedUnknown | SessionState::AuthenticatedContact
        );
        if let Event::Receive(message) = &event {
            let legal = message.message_type().may_be_received_in(before);
            if live && !legal {
                assert_eq!(result, Err(ProtocolError::MessageNotPermitted));
            }
            if live && legal && has_foreign_card(message) {
                assert_eq!(result, Err(ProtocolError::IdentityMismatch));
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
        // nothing at all before the encrypted channel exists.
        for message_type in MessageType::ALL {
            if !session.may_send(message_type) {
                continue;
            }
            match after {
                SessionState::AuthenticatedContact => {}
                SessionState::IdentityAuth => assert_eq!(message_type, MessageType::AuthProof),
                SessionState::AuthenticatedUnknown => assert!(matches!(
                    message_type,
                    MessageType::Close | MessageType::ContactAccept | MessageType::ContactRequest
                )),
                SessionState::Closing => assert_eq!(message_type, MessageType::Close),
                SessionState::Connecting | SessionState::CryptoHandshake | SessionState::Closed => {
                    panic!("{message_type:?} may be sent in {after:?}")
                }
            }
        }
    }

    // Identities that are not contacts are indistinguishable to the peer.
    let reference = observe(steps, Standing::None, rest);
    assert_eq!(observe(steps, Standing::Declined, rest), reference);
    assert_eq!(observe(steps, Standing::Blocked, rest), reference);
});

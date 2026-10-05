//! Keys, cards and connected sessions for tests.
//!
//! Everything here is deterministic. Handshakes run with fixed ephemeral
//! keys, so that a failing test fails the same way every time.

use core::time::Duration;
use std::sync::LazyLock;
use std::time::Instant;

use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::body::{ContactRequest, FileChunk, Message, MessageId, TransferId};
use monolith_protocol::card::{ContactCard, EndpointSet, InvitationCapability};
use monolith_protocol::credential::Credentials;
use monolith_protocol::limits::{HANDSHAKE_MSG1_LEN, HANDSHAKE_MSG2_LEN, HANDSHAKE_MSG3_LEN};
use monolith_protocol::session::{Action, PeerRecord, Standing};
use monolith_protocol::text::{ChatText, DisplayName, Filename, IntroductionText, ProfileText};
use monolith_protocol::{MessageType, SessionState};

use crate::{
    Admitted, AuthenticatedSession, HandshakeInitiator, HandshakeResponder,
    HandshakeResponderFinal, InboundPeer, LocalParty, OutboundAdmission, OutboundPeer, Received,
    SessionError, TransportSecretKey,
};

/// Seed of the initiator in most tests.
pub(crate) const ALICE: u8 = 0x10;

/// Seed of the responder in most tests.
pub(crate) const BOB: u8 = 0x51;

/// Seed of a third party.
pub(crate) const MALLORY: u8 = 0x77;

/// Ephemeral secret of the initiator in deterministic handshakes.
pub(crate) const EPHEMERAL_I: [u8; 32] = [0x11; 32];

/// Ephemeral secret of the responder in deterministic handshakes.
pub(crate) const EPHEMERAL_R: [u8; 32] = [0x22; 32];

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
        .collect()
}

static START: LazyLock<Instant> = LazyLock::new(Instant::now);

/// The moment every test session begins.
pub(crate) fn start() -> Instant {
    *START
}

/// A moment the given time after [`start`].
pub(crate) fn after(elapsed: Duration) -> Instant {
    start() + elapsed
}

pub(crate) fn identity_secret(seed: u8) -> IdentitySecretKey {
    IdentitySecretKey::from_seed(&[seed; 32])
}

/// The bytes of the transport private key of a seed. They differ from the
/// identity seed, as the keys of an identity are generated independently.
pub(crate) fn transport_bytes(seed: u8) -> [u8; 32] {
    [seed ^ 0xA5; 32]
}

/// The transport key of a seed.
pub(crate) fn transport_secret(seed: u8) -> TransportSecretKey {
    TransportSecretKey::from_bytes(&transport_bytes(seed)).unwrap()
}

pub(crate) fn endpoint(seed: u8) -> OnionServiceKey {
    let key = identity_secret(seed.wrapping_add(100)).public_key();
    OnionServiceKey::from_bytes(key.as_bytes()).unwrap()
}

/// A card of the identity of `seed` that states the transport key of
/// `transport_seed` and the endpoint of `endpoint_seed`.
pub(crate) fn card_with(
    seed: u8,
    transport_seed: u8,
    epoch: u64,
    endpoint_seed: u8,
    invitation: bool,
) -> ContactCard {
    ContactCard::sign(
        &identity_secret(seed),
        *transport_secret(transport_seed).public_key(),
        EndpointEpoch::new(epoch).unwrap(),
        EndpointSet::single(endpoint(endpoint_seed)),
        invitation.then(|| InvitationCapability::from_bytes([0xC4; 16])),
    )
    .unwrap()
}

/// The usual card of the identity of `seed`, carrying the given invitation
/// capability: what a user hands to somebody it wants requests from.
pub(crate) fn card_inviting(seed: u8, capability: [u8; 16]) -> ContactCard {
    ContactCard::sign(
        &identity_secret(seed),
        *transport_secret(seed).public_key(),
        EndpointEpoch::FIRST,
        EndpointSet::single(endpoint(seed)),
        Some(InvitationCapability::from_bytes(capability)),
    )
    .unwrap()
}

/// A card of the identity of `seed` with its usual endpoint.
pub(crate) fn card_of(seed: u8, transport_seed: u8, epoch: u64, invitation: bool) -> ContactCard {
    card_with(seed, transport_seed, epoch, seed, invitation)
}

/// The card an identity uses in tests unless a test says otherwise.
pub(crate) fn card(seed: u8) -> ContactCard {
    card_of(seed, seed, 1, false)
}

/// The local side of the identity of `seed` with the transport key of
/// `transport_seed`, at the given epoch and with its usual endpoint. Its
/// card is `card_of(seed, transport_seed, epoch, false)`.
pub(crate) fn party_with(seed: u8, transport_seed: u8, epoch: u64) -> LocalParty {
    LocalParty::issue(
        &identity_secret(seed),
        transport_secret(transport_seed),
        EndpointEpoch::new(epoch).unwrap(),
        EndpointSet::single(endpoint(seed)),
    )
    .unwrap()
}

/// The local side of an identity: its usual card and its transport key.
pub(crate) fn party(seed: u8) -> LocalParty {
    party_with(seed, seed, 1)
}

/// The three messages of a handshake.
#[derive(Clone)]
pub(crate) struct Transcript {
    pub(crate) message_1: [u8; HANDSHAKE_MSG1_LEN],
    pub(crate) message_2: [u8; HANDSHAKE_MSG2_LEN],
    pub(crate) message_3: [u8; HANDSHAKE_MSG3_LEN],
}

/// A handshake with fixed ephemeral keys that has reached the admission of
/// the responder: message 2 was taken, and message 3 comes only from a
/// granted admission.
pub(crate) struct Handshake {
    pub(crate) outbound: OutboundPeer,
    pub(crate) waiting: HandshakeResponderFinal,
    pub(crate) message_1: [u8; HANDSHAKE_MSG1_LEN],
    pub(crate) message_2: [u8; HANDSHAKE_MSG2_LEN],
}

impl Handshake {
    /// The initiator admits the responder with `record`. The admission has
    /// to let the responder learn the local identity; the responder then
    /// reads message 3. Returns the initiator's admission, the responder's
    /// authenticated peer and the transcript.
    pub(crate) fn admit(
        self,
        record: PeerRecord<'_>,
    ) -> Result<(Admitted, InboundPeer, Transcript), SessionError> {
        let OutboundAdmission::Granted {
            session,
            admission,
            first,
            message_3,
        } = self.outbound.admit(record)?
        else {
            panic!("the initiator refused the responder");
        };
        let message_3 = *message_3.as_bytes();
        let inbound = self.waiting.read_message_3(&message_3, start())?;
        let transcript = Transcript {
            message_1: self.message_1,
            message_2: self.message_2,
            message_3,
        };
        Ok(((session, admission, first), inbound, transcript))
    }

    /// The initiator admits the responder holding exactly the dialed card
    /// with the given standing, which has to be that of a contact.
    pub(crate) fn admit_as(
        self,
        standing: Standing,
    ) -> (AuthenticatedSession, Vec<Action>, InboundPeer, Transcript) {
        let mut held = Credentials::new(self.outbound.card().clone());
        let record = match standing {
            Standing::Requested => PeerRecord::Requested(&mut held),
            Standing::Accepted => PeerRecord::Accepted(&mut held),
            _ => panic!("an initiator sends message 3 only to a contact"),
        };
        let ((session, admission, first), inbound, transcript) = self.admit(record).unwrap();
        assert_eq!(admission.standing, standing);
        (session, first, inbound, transcript)
    }
}

/// Runs a handshake with fixed ephemeral keys up to the admission of the
/// responder. The initiator dials `dialed`, which is normally the
/// responder's card.
pub(crate) fn handshake_with(
    initiator: &LocalParty,
    responder: &LocalParty,
    dialed: &ContactCard,
) -> Result<Handshake, SessionError> {
    let (first, message_1) =
        HandshakeInitiator::start_with_ephemeral(initiator, dialed, start(), EPHEMERAL_I)?;
    let waiting = HandshakeResponder::new_with_ephemeral(responder, start(), EPHEMERAL_R)?;
    let (waiting, message_2) = waiting.read_message_1(&message_1, start())?;
    let outbound = first.read_message_2(&message_2, start())?;
    Ok(Handshake {
        outbound,
        waiting,
        message_1,
        message_2,
    })
}

/// Runs a handshake between two parties up to the admission of the
/// responder.
pub(crate) fn handshake(initiator: &LocalParty, responder: &LocalParty) -> Handshake {
    handshake_with(initiator, responder, responder.card()).unwrap()
}

/// The transcript of a handshake in which Alice holds Bob as accepted.
pub(crate) fn transcript(initiator: &LocalParty, responder: &LocalParty) -> Transcript {
    handshake(initiator, responder)
        .admit_as(Standing::Accepted)
        .3
}

/// Two sessions connected to each other, with what each side was told to
/// do when its session authenticated.
pub(crate) struct Pair {
    /// The side that opened the session.
    pub(crate) initiator: AuthenticatedSession,
    /// The side that accepted it.
    pub(crate) responder: AuthenticatedSession,
    pub(crate) initiator_first: Vec<Action>,
    pub(crate) responder_first: Vec<Action>,
}

/// Alice dials Bob. Alice holds Bob with the given standing, which is that
/// of a contact, since an initiator sends message 3 to nobody else; Bob
/// holds the given record of Alice.
pub(crate) fn connect(alice_holds_bob: Standing, bob_holds_alice: PeerRecord<'_>) -> Pair {
    let (initiator, initiator_first, inbound, _) =
        handshake(&party(ALICE), &party(BOB)).admit_as(alice_holds_bob);
    let (responder, _, responder_first) = inbound.admit(bob_holds_alice).unwrap();
    Pair {
        initiator,
        responder,
        initiator_first,
        responder_first,
    }
}

/// Feeds a whole frame to a session and returns what it produced. The
/// frame must be consumed completely.
pub(crate) fn deliver(
    session: &mut AuthenticatedSession,
    frame: &[u8],
) -> Result<Option<Received>, SessionError> {
    let (used, received) = session.receive(frame, start())?;
    assert_eq!(used, frame.len(), "one frame is consumed in one call");
    Ok(received)
}

/// Two sessions between accepted contacts, confirmed on both sides.
pub(crate) fn confirmed() -> Pair {
    let mut pair = connect(
        Standing::Accepted,
        PeerRecord::Accepted(&mut Credentials::new(card(ALICE))),
    );
    assert_eq!(pair.initiator_first, vec![Action::SendContactAccept]);
    assert_eq!(pair.responder_first, vec![Action::SendContactAccept]);
    let from_alice = pair
        .initiator
        .send(&Message::ContactAccept, start())
        .unwrap();
    let from_bob = pair
        .responder
        .send(&Message::ContactAccept, start())
        .unwrap();
    let at_bob = deliver(&mut pair.responder, &from_alice).unwrap().unwrap();
    let at_alice = deliver(&mut pair.initiator, &from_bob).unwrap().unwrap();
    assert_eq!(at_bob.actions, vec![Action::Confirmed]);
    assert_eq!(at_alice.actions, vec![Action::Confirmed]);
    assert_eq!(pair.initiator.state(), SessionState::AuthenticatedContact);
    assert_eq!(pair.responder.state(), SessionState::AuthenticatedContact);
    pair
}

/// A chat message with the given text.
pub(crate) fn chat(text: &str) -> Message {
    Message::ChatMessage {
        id: MessageId::from_bytes([4; 16]),
        text: ChatText::new(text).unwrap(),
    }
}

/// A contact request that carries the given card.
pub(crate) fn request_with(card: ContactCard, invitation: Option<InvitationCapability>) -> Message {
    Message::ContactRequest(Box::new(ContactRequest {
        card,
        invitation,
        display_name: DisplayName::new("Peer").unwrap(),
        introduction: IntroductionText::new("hello").unwrap(),
    }))
}

/// One message of every type, as the identity of `seed` would send it.
pub(crate) fn sample(message_type: MessageType, seed: u8) -> Message {
    let transfer = TransferId::from_bytes([3; 16]);
    let id = MessageId::from_bytes([4; 16]);
    match message_type {
        MessageType::Close => Message::Close,
        MessageType::Ping => Message::Ping([1; 8]),
        MessageType::Pong => Message::Pong([1; 8]),
        MessageType::ContactRequest => request_with(card(seed), None),
        MessageType::ContactAccept => Message::ContactAccept,
        MessageType::ChatMessage => chat("hi"),
        MessageType::MessageAck => Message::MessageAck(id),
        MessageType::Profile => Message::Profile {
            display_name: DisplayName::new("Peer").unwrap(),
            profile_text: ProfileText::new("").unwrap(),
        },
        MessageType::EndpointUpdate => Message::EndpointUpdate(Box::new(card(seed))),
        MessageType::FileOffer => Message::FileOffer {
            transfer,
            size: 1,
            filename: Filename::new("a.txt").unwrap(),
        },
        MessageType::FileAccept => Message::FileAccept(transfer),
        MessageType::FileReject => Message::FileReject(transfer),
        MessageType::FileChunk => Message::FileChunk(FileChunk::new(transfer, vec![1]).unwrap()),
        MessageType::FileComplete => Message::FileComplete {
            transfer,
            digest: [5; 32],
        },
        MessageType::FileAbort => Message::FileAbort(transfer),
    }
}

//! Two parties with fixed keys for the session fuzz targets, and the
//! handshake between them with fixed ephemeral keys.
//!
//! The values are those of the tests of `monolith-session`, which also
//! generate the seeds under `fuzz/seeds/`. A handshake with fixed
//! ephemeral keys exists only in builds made by `cargo fuzz` and in the
//! tests of that crate.

// Each target uses a part of this file.
#![allow(dead_code)]

use std::sync::LazyLock;
use std::time::Instant;

use monolith_identity::{EndpointEpoch, IdentitySecretKey, OnionServiceKey};
use monolith_protocol::card::{ContactCard, EndpointSet};
use monolith_protocol::credential::Credentials;
use monolith_protocol::session::PeerRecord;
use monolith_session::{
    AuthenticatedSession, HandshakeInitiator, HandshakeResponder, InboundPeer, LocalParty,
    MessageBuffer, OutboundAdmission, TransportSecretKey,
};

/// Seed of the initiator.
pub const ALICE: u8 = 0x10;

/// Seed of the responder.
pub const BOB: u8 = 0x51;

/// Ephemeral secret of the initiator.
pub const EPHEMERAL_I: [u8; 32] = [0x11; 32];

/// Ephemeral secret of the responder.
pub const EPHEMERAL_R: [u8; 32] = [0x22; 32];

static START: LazyLock<Instant> = LazyLock::new(Instant::now);

/// The moment every session of a target begins. The handshake targets
/// never let time pass, so no timeout and no age limit is reached there.
/// The frame target keeps a clock of its own that starts here.
pub fn start() -> Instant {
    *START
}

fn transport(seed: u8) -> TransportSecretKey {
    TransportSecretKey::from_bytes(&[seed ^ 0xA5; 32]).unwrap()
}

/// The contact card of a party.
pub fn card(seed: u8) -> ContactCard {
    let endpoint = IdentitySecretKey::from_seed(&[seed.wrapping_add(100); 32]).public_key();
    ContactCard::sign(
        &IdentitySecretKey::from_seed(&[seed; 32]),
        *transport(seed).public_key(),
        EndpointEpoch::FIRST,
        EndpointSet::single(OnionServiceKey::from_bytes(endpoint.as_bytes()).unwrap()),
        None,
    )
    .unwrap()
}

/// The local side of a party: its card and its transport key.
pub fn party(seed: u8) -> LocalParty {
    let endpoint = IdentitySecretKey::from_seed(&[seed.wrapping_add(100); 32]).public_key();
    LocalParty::issue(
        &IdentitySecretKey::from_seed(&[seed; 32]),
        transport(seed),
        EndpointEpoch::FIRST,
        EndpointSet::single(OnionServiceKey::from_bytes(endpoint.as_bytes()).unwrap()),
    )
    .unwrap()
}

/// Alice, about to dial Bob, and her first message.
pub fn alice_dialing() -> (HandshakeInitiator, [u8; 48]) {
    HandshakeInitiator::start_with_ephemeral(&party(ALICE), &card(BOB), start(), EPHEMERAL_I)
        .unwrap()
}

/// Bob, waiting for a first message.
pub fn bob_waiting() -> HandshakeResponder {
    HandshakeResponder::new_with_ephemeral(&party(BOB), start(), EPHEMERAL_R).unwrap()
}

/// The three messages of the handshake between Alice and Bob.
pub struct Transcript {
    pub message_1: [u8; 48],
    pub message_2: [u8; 48],
    pub message_3: [u8; 235],
}

/// Runs the handshake between Alice and Bob. Alice holds Bob as an
/// accepted contact, so her admission makes message 3. Returns her
/// session, Bob's authenticated peer and the transcript.
pub fn handshake() -> (AuthenticatedSession, InboundPeer, Transcript) {
    let (alice, message_1) = alice_dialing();
    let (bob, message_2) = bob_waiting().read_message_1(&message_1, start()).unwrap();
    let outbound = alice.read_message_2(&message_2, start()).unwrap();
    let OutboundAdmission::Granted {
        session: at_alice,
        message_3,
        ..
    } = outbound
        .admit(PeerRecord::Accepted(&mut Credentials::new(card(BOB))))
        .unwrap()
    else {
        panic!("Alice refused Bob");
    };
    let message_3 = *message_3.as_bytes();
    let inbound = bob.read_message_3(&message_3, start()).unwrap();
    let transcript = Transcript {
        message_1,
        message_2,
        message_3,
    };
    (at_alice, inbound, transcript)
}

/// The messages of that handshake. They are the same on every run.
pub static TRANSCRIPT: LazyLock<Transcript> = LazyLock::new(|| handshake().2);

/// A stream that arrives in pieces of a fixed size.
pub struct Pieces<'a> {
    chunks: core::slice::Chunks<'a, u8>,
    pending: &'a [u8],
}

impl<'a> Pieces<'a> {
    /// Cuts `stream` into pieces of `piece` bytes.
    pub fn new(stream: &'a [u8], piece: usize) -> Self {
        Self {
            chunks: stream.chunks(piece),
            pending: &[],
        }
    }

    /// Reads one handshake message of exactly `N` bytes, as the stream
    /// delivers it. `None` if the stream ends before the message is
    /// complete. Nothing after the message is consumed.
    pub fn message<const N: usize>(&mut self) -> Option<[u8; N]> {
        let mut buffer = MessageBuffer::<N>::new();
        loop {
            if self.pending.is_empty() {
                self.pending = self.chunks.next()?;
            }
            let (used, message) = buffer.feed(self.pending);
            self.pending = &self.pending[used..];
            if let Some(message) = message {
                return Some(*message);
            }
        }
    }

    /// Returns the next piece of what is left of the stream.
    pub fn rest(&mut self) -> Option<&'a [u8]> {
        if self.pending.is_empty() {
            self.pending = self.chunks.next()?;
        }
        Some(core::mem::take(&mut self.pending))
    }
}

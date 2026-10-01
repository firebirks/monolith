//! The transport key and what the local side brings to a handshake.

use core::fmt;

use monolith_identity::redact::REDACTED;
use monolith_identity::{IdentityPublicKey, TransportPublicKey};
use monolith_protocol::card::ContactCard;
use x25519_dalek::{PublicKey, StaticSecret};
use zeroize::Zeroizing;

use crate::SessionError;
use crate::session::SessionLimits;

/// Length in bytes of an X25519 private key.
pub const TRANSPORT_SECRET_KEY_LEN: usize = 32;

/// The secret half of a transport key: the X25519 key with which an
/// identity authenticates its sessions.
///
/// It is generated from the random source of the operating system,
/// independently of the identity key and of the onion keys, and is used for
/// nothing but the handshake of `docs/PROTOCOL.md` section 4. The key
/// material is erased when the value is dropped. The type prints nothing
/// about it and has no way to export it.
pub struct TransportSecretKey {
    secret: StaticSecret,
    public: TransportPublicKey,
}

impl TransportSecretKey {
    /// Generates a new key from the random source of the operating system.
    pub fn generate() -> Result<Self, SessionError> {
        let mut bytes = Zeroizing::new([0_u8; TRANSPORT_SECRET_KEY_LEN]);
        getrandom::fill(bytes.as_mut_slice()).map_err(|_| SessionError::Randomness)?;
        Self::from_bytes(&bytes)
    }

    /// Builds the key from 32 bytes, for a key that was generated earlier
    /// and stored.
    ///
    /// The bytes must come from a cryptographically secure random source
    /// and must not be derived from another key of the identity. The caller
    /// keeps responsibility for its own copy of them.
    pub fn from_bytes(bytes: &[u8; TRANSPORT_SECRET_KEY_LEN]) -> Result<Self, SessionError> {
        let secret = StaticSecret::from(*bytes);
        // X25519 clamps the scalar, so the public key of any 32 bytes is a
        // point of large order in canonical encoding. The check is kept so
        // that a value of this type never holds a public key that a
        // contact card would reject.
        let public = TransportPublicKey::from_bytes(PublicKey::from(&secret).as_bytes())
            .map_err(|_| SessionError::Internal)?;
        Ok(Self { secret, public })
    }

    /// Returns the public half.
    pub const fn public_key(&self) -> &TransportPublicKey {
        &self.public
    }

    /// Returns the private key for the Noise library, which takes it by
    /// reference and copies it into the resolver's key object.
    pub(crate) fn expose(&self) -> &[u8; TRANSPORT_SECRET_KEY_LEN] {
        self.secret.as_bytes()
    }
}

impl fmt::Debug for TransportSecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TransportSecretKey({REDACTED})")
    }
}

/// What the local side brings to every handshake: its contact card and the
/// secret half of the transport key that the card states.
///
/// The card is the one without invitation capability. An initiator sends
/// it in the third handshake message; a responder takes its identity key
/// from it for the prologue.
pub struct LocalParty {
    card: ContactCard,
    transport: TransportSecretKey,
    limits: SessionLimits,
}

impl LocalParty {
    /// Puts a card and a transport key together.
    ///
    /// Fails with [`SessionError::LocalCard`] if the card carries an
    /// invitation capability, or if the transport key in the card is not
    /// the public half of `transport`. A handshake with such a pair could
    /// never be accepted by a peer.
    pub fn new(card: ContactCard, transport: TransportSecretKey) -> Result<Self, SessionError> {
        if card.invitation().is_some() || card.transport() != transport.public_key() {
            return Err(SessionError::LocalCard);
        }
        Ok(Self {
            card,
            transport,
            limits: SessionLimits::PROTOCOL,
        })
    }

    /// Replaces the limits that sessions of this party are created with.
    /// The default is [`SessionLimits::PROTOCOL`].
    #[must_use]
    pub const fn with_limits(mut self, limits: SessionLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Returns the local contact card.
    pub const fn card(&self) -> &ContactCard {
        &self.card
    }

    /// Returns the local identity.
    pub const fn identity(&self) -> &IdentityPublicKey {
        self.card.identity()
    }

    pub(crate) const fn transport(&self) -> &TransportSecretKey {
        &self.transport
    }

    pub(crate) const fn limits(&self) -> SessionLimits {
        self.limits
    }
}

impl fmt::Debug for LocalParty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LocalParty({REDACTED})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{card_of, identity_secret, transport_secret};

    /// The key pairs of RFC 7748 section 6.1.
    const ALICE_SECRET: [u8; 32] = [
        0x77, 0x07, 0x6d, 0x0a, 0x73, 0x18, 0xa5, 0x7d, 0x3c, 0x16, 0xc1, 0x72, 0x51, 0xb2, 0x66,
        0x45, 0xdf, 0x4c, 0x2f, 0x87, 0xeb, 0xc0, 0x99, 0x2a, 0xb1, 0x77, 0xfb, 0xa5, 0x1d, 0xb9,
        0x2c, 0x2a,
    ];
    const ALICE_PUBLIC: [u8; 32] = [
        0x85, 0x20, 0xf0, 0x09, 0x89, 0x30, 0xa7, 0x54, 0x74, 0x8b, 0x7d, 0xdc, 0xb4, 0x3e, 0xf7,
        0x5a, 0x0d, 0xbf, 0x3a, 0x0d, 0x26, 0x38, 0x1a, 0xf4, 0xeb, 0xa4, 0xa9, 0x8e, 0xaa, 0x9b,
        0x4e, 0x6a,
    ];

    #[test]
    fn public_key_matches_rfc_7748() {
        let key = TransportSecretKey::from_bytes(&ALICE_SECRET).unwrap();
        assert_eq!(key.public_key().as_bytes(), &ALICE_PUBLIC);
        assert_eq!(key.expose(), &ALICE_SECRET);
    }

    #[test]
    fn every_seed_gives_a_valid_public_key() {
        // Clamping makes the scalar a non-zero multiple of the cofactor,
        // also for the extreme byte strings.
        for bytes in [[0_u8; 32], [0xff; 32], [0x80; 32], [1; 32]] {
            let key = TransportSecretKey::from_bytes(&bytes).unwrap();
            assert!(TransportPublicKey::from_bytes(key.public_key().as_bytes()).is_ok());
        }
    }

    #[test]
    fn generated_keys_differ() {
        let first = TransportSecretKey::generate().unwrap();
        let second = TransportSecretKey::generate().unwrap();
        assert_ne!(first.public_key(), second.public_key());
        assert_ne!(first.expose(), second.expose());
        assert_ne!(first.expose(), &[0_u8; 32]);
    }

    #[test]
    fn a_local_party_needs_a_card_that_states_its_transport_key() {
        let good = LocalParty::new(card_of(1, 1, 1, false), transport_secret(1)).unwrap();
        assert_eq!(good.identity(), &identity_secret(1).public_key());
        assert_eq!(good.card(), &card_of(1, 1, 1, false));
        assert_eq!(good.limits(), SessionLimits::PROTOCOL);

        // The card states the transport key of seed 1; the secret key is
        // another one.
        assert_eq!(
            LocalParty::new(card_of(1, 1, 1, false), transport_secret(2)).err(),
            Some(SessionError::LocalCard)
        );
        // A card with an invitation capability is not the card that goes
        // into a handshake.
        assert_eq!(
            LocalParty::new(card_of(1, 1, 1, true), transport_secret(1)).err(),
            Some(SessionError::LocalCard)
        );
    }

    #[test]
    fn debug_output_shows_no_key_material() {
        let key = TransportSecretKey::from_bytes(&ALICE_SECRET).unwrap();
        assert_eq!(format!("{key:?}"), "TransportSecretKey([redacted])");
        let party = LocalParty::new(card_of(1, 1, 1, false), transport_secret(1)).unwrap();
        assert_eq!(format!("{party:?}"), "LocalParty([redacted])");
    }
}

//! The transport key and what the local side brings to a handshake.

use core::fmt;

use monolith_identity::redact::REDACTED;
use monolith_identity::{EndpointEpoch, IdentityPublicKey, IdentitySecretKey, TransportPublicKey};
use monolith_protocol::card::{ContactCard, EndpointSet};
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
    ///
    /// Fails with [`SessionError::InvalidSecretKey`] for 32 zero bytes. No
    /// random source produces them; a buffer that was never filled does,
    /// and the key made from it would be known to everybody.
    pub fn from_bytes(bytes: &[u8; TRANSPORT_SECRET_KEY_LEN]) -> Result<Self, SessionError> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(SessionError::InvalidSecretKey);
        }
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
/// A party is made from the local secret keys and nothing else. The card
/// is signed here, by the identity key that is passed in, for the
/// transport key that is passed in. A card that came from outside cannot
/// become the local one: there is no constructor that takes a card. So a
/// party never pairs the transport key of one identity with a card signed
/// by another, and the identity in the prologue of every handshake it
/// takes part in is the identity that holds its transport key (rule F2 of
/// `docs/CRYPTOGRAPHY.md` section 5.2).
///
/// ```compile_fail
/// use monolith_protocol::card::ContactCard;
/// use monolith_session::{LocalParty, TransportSecretKey};
///
/// // An imported card cannot be adopted as the local one.
/// fn adopt(card: ContactCard, transport: TransportSecretKey) -> LocalParty {
///     LocalParty::new(card, transport).unwrap()
/// }
/// ```
///
/// The identity key is used while the party is made and is not kept. The
/// card has no invitation capability. An initiator sends it in the third
/// handshake message; a responder takes its identity key from it for the
/// prologue.
pub struct LocalParty {
    card: ContactCard,
    transport: TransportSecretKey,
    limits: SessionLimits,
}

impl LocalParty {
    /// Signs the local card with `identity` for `transport`, `epoch` and
    /// `endpoints`, and keeps it with the transport key.
    ///
    /// Fails with [`SessionError::LocalCard`] if the card cannot be made:
    /// the transport key or an endpoint is the identity key in another
    /// form, which key separation forbids (`docs/PROTOCOL.md` section
    /// 11.2). The card is decoded again as a receiver would decode it, and
    /// it has to come out the same; a card that would not verify is never
    /// used.
    pub fn issue(
        identity: &IdentitySecretKey,
        transport: TransportSecretKey,
        epoch: EndpointEpoch,
        endpoints: EndpointSet,
    ) -> Result<Self, SessionError> {
        let card = ContactCard::sign(identity, *transport.public_key(), epoch, endpoints, None)
            .map_err(|_| SessionError::LocalCard)?;
        // `sign` puts the public half of `identity` and of `transport` into
        // the card. Decoding checks the signature under that identity key.
        let verified = ContactCard::decode(&card.encode()).map_err(|_| SessionError::LocalCard)?;
        if verified != card {
            return Err(SessionError::LocalCard);
        }
        Ok(Self {
            card: verified,
            transport,
            limits: SessionLimits::PROTOCOL,
        })
    }

    /// Replaces the limits that sessions of this party are created with,
    /// so that a test can reach one. Both ends of a test session get the
    /// same limits. Outside tests the limits are those of the protocol.
    #[cfg(test)]
    #[must_use]
    pub(crate) const fn with_limits(mut self, limits: SessionLimits) -> Self {
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
    use crate::testing::{card, card_of, endpoint, identity_secret, transport_secret};
    use monolith_identity::OnionServiceKey;
    use sha2::{Digest, Sha512};

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
        let mut one_bit = [0_u8; 32];
        one_bit[17] = 0x04;
        for bytes in [one_bit, [0xff; 32], [0x80; 32], [1; 32]] {
            let key = TransportSecretKey::from_bytes(&bytes).unwrap();
            assert!(TransportPublicKey::from_bytes(key.public_key().as_bytes()).is_ok());
        }
    }

    #[test]
    fn a_buffer_of_zeros_is_not_a_key() {
        assert_eq!(
            TransportSecretKey::from_bytes(&[0; 32]).err(),
            Some(SessionError::InvalidSecretKey)
        );
    }

    #[test]
    fn generated_keys_differ() {
        let first = TransportSecretKey::generate().unwrap();
        let second = TransportSecretKey::generate().unwrap();
        assert_ne!(first.public_key(), second.public_key());
        assert_ne!(first.expose(), second.expose());
        assert_ne!(first.expose(), &[0_u8; 32]);
    }

    fn issue(identity: u8, transport: u8) -> Result<LocalParty, SessionError> {
        LocalParty::issue(
            &identity_secret(identity),
            transport_secret(transport),
            EndpointEpoch::FIRST,
            EndpointSet::single(endpoint(identity)),
        )
    }

    #[test]
    fn a_local_party_signs_its_own_card() {
        let party = issue(1, 1).unwrap();
        assert_eq!(party.identity(), &identity_secret(1).public_key());
        assert_eq!(party.card(), &card(1));
        assert_eq!(party.card().transport(), transport_secret(1).public_key());
        assert!(party.card().invitation().is_none());
        assert_eq!(party.limits(), SessionLimits::PROTOCOL);
        // The card verifies as a receiver would decode it.
        assert_eq!(
            &ContactCard::decode(&party.card().encode()).unwrap(),
            party.card()
        );
        // Another epoch and endpoint go into the card as given.
        let later = LocalParty::issue(
            &identity_secret(1),
            transport_secret(2),
            EndpointEpoch::new(4).unwrap(),
            EndpointSet::single(endpoint(3)),
        )
        .unwrap();
        assert_eq!(later.card(), &crate::testing::card_with(1, 2, 4, 3, false));
    }

    #[test]
    fn a_card_from_outside_cannot_become_the_local_card() {
        // Rule F2, the local half. Mallory signs a card that states Bob's
        // transport key. Bob's side holds that card and his transport
        // secret. The only way to make a party is from an identity secret,
        // and the card of the party is always signed by it: with Bob's
        // identity key the party is Bob, and Mallory's card is not used.
        let mallory = identity_secret(0x77).public_key();
        let imported = card_of(0x77, 0x51, 1, false);
        assert_eq!(imported.identity(), &mallory);
        assert_eq!(imported.transport(), transport_secret(0x51).public_key());

        let party = issue(0x51, 0x51).unwrap();
        assert_ne!(party.card(), &imported);
        assert_ne!(party.identity(), &mallory);
        assert_eq!(party.identity(), &identity_secret(0x51).public_key());
        // Whatever identity key is used, the card is that identity's and
        // states the transport key that came with it.
        for (identity, transport) in [(0x51, 0x51), (0x77, 0x51), (0x51, 0x77), (0x10, 0x10)] {
            let party = issue(identity, transport).unwrap();
            assert_eq!(party.identity(), &identity_secret(identity).public_key());
            assert_eq!(party.card().identity(), party.identity());
            assert_eq!(
                party.card().transport(),
                transport_secret(transport).public_key()
            );
        }
    }

    #[test]
    fn key_separation_is_checked_when_the_party_is_made() {
        // A transport key that is the identity key in Montgomery form. The
        // X25519 secret is the clamped Ed25519 scalar of the identity seed.
        let digest = Sha512::digest([5_u8; 32]);
        let scalar: [u8; 32] = digest[..32].try_into().unwrap();
        let reused = TransportSecretKey::from_bytes(&scalar).unwrap();
        assert!(
            reused
                .public_key()
                .is_montgomery_form_of(identity_secret(5).public_key().as_bytes())
        );
        assert_eq!(
            LocalParty::issue(
                &identity_secret(5),
                reused,
                EndpointEpoch::FIRST,
                EndpointSet::single(endpoint(5)),
            )
            .err(),
            Some(SessionError::LocalCard)
        );

        // An endpoint that is the identity key itself.
        let own = OnionServiceKey::from_bytes(identity_secret(5).public_key().as_bytes()).unwrap();
        assert_eq!(
            LocalParty::issue(
                &identity_secret(5),
                transport_secret(5),
                EndpointEpoch::FIRST,
                EndpointSet::single(own),
            )
            .err(),
            Some(SessionError::LocalCard)
        );
    }

    #[test]
    fn debug_output_shows_no_key_material() {
        let key = TransportSecretKey::from_bytes(&ALICE_SECRET).unwrap();
        assert_eq!(format!("{key:?}"), "TransportSecretKey([redacted])");
        let party = issue(1, 1).unwrap();
        assert_eq!(format!("{party:?}"), "LocalParty([redacted])");
    }
}

//! Contact cards and endpoint sets.
//!
//! A contact card is a signed statement by an identity: "as of this epoch,
//! my sessions are authenticated by this transport key, and I can be reached
//! at this set of endpoints". The format is defined in `docs/PROTOCOL.md`
//! section 11. Version 1 allows one endpoint per card; the format, the
//! signed bytes and the epoch rule are those of a set.
//!
//! The card is the certificate of the transport key: the session handshake
//! authenticates transport keys, and a card says which identity one belongs
//! to (`docs/PROTOCOL.md` section 4.6).
//!
//! The bytes that are signed come from [`ContactCard::signed_bytes`], which
//! is separate from the transport encoder [`ContactCard::encode`], so that a
//! change to the transport layout cannot change what a signature covers.

use core::fmt;

use monolith_identity::redact::REDACTED;
use monolith_identity::{
    EndpointEpoch, IdentityPublicKey, IdentitySecretKey, OnionServiceKey, Signature,
    TransportPublicKey, base32,
};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::ProtocolError;
use crate::codec::{Reader, Writer};
use crate::limits::{
    CONTACT_CARD_FIXED_LEN, INVITATION_CAPABILITY_LEN, MAX_ACTIVE_ENDPOINTS, MAX_CONTACT_CARD_LEN,
    MAX_CONTACT_CARD_TEXT_LEN,
};

/// Version byte of the binary card format.
const CARD_VERSION: u8 = 0x01;

/// Flag bit: an invitation capability is present.
const FLAG_INVITATION: u8 = 0x01;

/// Domain separation prefix of the card signature.
const SIGNING_PREFIX: &[u8; 24] = b"MONOLITH-CONTACT-CARD-V1";

/// Prefix of the text form. The digit is the version of the text encoding.
const TEXT_PREFIX: &str = "MONOLITH1:";

/// A high-entropy token that a contact card may carry.
///
/// It is an anti-spam capability: in the default mode a contact request is
/// shown to the user only if it carries one that is currently valid. It is
/// not an identity and authenticates nobody.
///
/// A capability has a fixed size. Two capabilities are compared with
/// `subtle`, which looks at every byte whatever the result, so that
/// comparing a received one with the valid ones does not tell the sender
/// how many leading bytes it got right. That is a best-effort property of
/// the software: it removes the early exit, and it does not promise that no
/// timing difference exists on any compiler or hardware. The type has no
/// `Hash` and no ordering: a capability is looked up by comparing it with
/// each valid one.
///
/// A capability is a secret of the user who issued it and of the people it
/// was handed to. The type is not `Copy`, so that no copy is made without a
/// visible call, and its bytes are overwritten with zeros when a value is
/// dropped. That is best effort: it does not reach copies the compiler
/// made, the bytes a decoder read before they became a capability, or the
/// text form of a card that was handed out.
pub struct InvitationCapability([u8; INVITATION_CAPABILITY_LEN]);

/// Written out instead of derived, so that every copy of a capability is
/// made here. Two owners need one: a contact card that carries a capability
/// is cloned where a session keeps the card that stands for its peer, and
/// a session keeps its own copy of the capability that a request to its
/// peer carries, because on an inbound session that comes from a card the
/// caller only lends. Each copy is erased when it is dropped.
impl Clone for InvitationCapability {
    fn clone(&self) -> Self {
        Self(self.0)
    }
}

impl Zeroize for InvitationCapability {
    fn zeroize(&mut self) {
        self.0.zeroize();
    }
}

impl Drop for InvitationCapability {
    fn drop(&mut self) {
        self.zeroize();
    }
}

impl ZeroizeOnDrop for InvitationCapability {}

impl PartialEq for InvitationCapability {
    fn eq(&self, other: &Self) -> bool {
        self.0.as_slice().ct_eq(other.0.as_slice()).into()
    }
}

impl Eq for InvitationCapability {}

impl InvitationCapability {
    /// Wraps capability bytes. They must come from a CSPRNG.
    pub const fn from_bytes(bytes: [u8; INVITATION_CAPABILITY_LEN]) -> Self {
        Self(bytes)
    }

    /// Returns the capability bytes. Callers must not log them.
    pub const fn expose(&self) -> &[u8; INVITATION_CAPABILITY_LEN] {
        &self.0
    }
}

impl fmt::Debug for InvitationCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "InvitationCapability({REDACTED})")
    }
}

/// The endpoints at which an identity can be reached.
///
/// Never empty, never more than [`MAX_ACTIVE_ENDPOINTS`] members, and no
/// member twice. The order is the order in the card and is kept, because the
/// signature covers it. Two sets are equal if they have the same members, in
/// whatever order.
#[derive(Clone, Debug)]
pub struct EndpointSet {
    first: OnionServiceKey,
    rest: Vec<OnionServiceKey>,
}

impl PartialEq for EndpointSet {
    fn eq(&self, other: &Self) -> bool {
        // No member occurs twice, so equal counts and inclusion one way
        // are enough.
        self.count() == other.count() && self.iter().all(|endpoint| other.contains(endpoint))
    }
}

impl Eq for EndpointSet {}

impl EndpointSet {
    /// Builds a set with one endpoint.
    pub const fn single(endpoint: OnionServiceKey) -> Self {
        Self {
            first: endpoint,
            rest: Vec::new(),
        }
    }

    /// Builds a set from a list of endpoints.
    ///
    /// Fails if the list is empty, longer than [`MAX_ACTIVE_ENDPOINTS`], or
    /// contains an endpoint twice.
    pub fn new(endpoints: &[OnionServiceKey]) -> Result<Self, ProtocolError> {
        if endpoints.len() > MAX_ACTIVE_ENDPOINTS {
            return Err(ProtocolError::InvalidValue);
        }
        let (first, rest) = endpoints.split_first().ok_or(ProtocolError::InvalidValue)?;
        let mut seen: Vec<OnionServiceKey> = Vec::with_capacity(endpoints.len());
        for endpoint in endpoints {
            if seen.contains(endpoint) {
                return Err(ProtocolError::InvalidValue);
            }
            seen.push(*endpoint);
        }
        Ok(Self {
            first: *first,
            rest: rest.to_vec(),
        })
    }

    /// Returns the first endpoint of the set.
    pub const fn first(&self) -> &OnionServiceKey {
        &self.first
    }

    /// Returns the endpoints in card order.
    pub fn iter(&self) -> impl Iterator<Item = &OnionServiceKey> {
        core::iter::once(&self.first).chain(self.rest.iter())
    }

    /// Returns the number of endpoints, which is at least 1.
    pub fn count(&self) -> usize {
        self.rest.len().saturating_add(1)
    }

    /// Returns true if `endpoint` is a member of the set.
    pub fn contains(&self, endpoint: &OnionServiceKey) -> bool {
        self.iter().any(|member| member == endpoint)
    }

    /// Returns true if one of the endpoints is byte for byte the same key
    /// as `identity`, which key separation forbids.
    fn reuses(&self, identity: &IdentityPublicKey) -> bool {
        self.iter()
            .any(|endpoint| endpoint.as_bytes() == identity.as_bytes())
    }
}

/// Returns true if `transport` is the identity key, or one of the endpoint
/// keys, carried over to the Montgomery form. Key separation forbids that
/// (`docs/PROTOCOL.md` section 11.2, invariant S33).
///
/// The endpoints are taken as raw bytes so that a decoder can apply this
/// check before it validates them, in the order the specification gives.
fn transport_reuses<'a>(
    transport: &TransportPublicKey,
    identity: &IdentityPublicKey,
    mut endpoints: impl Iterator<Item = &'a [u8; 32]>,
) -> bool {
    transport.is_montgomery_form_of(identity.as_bytes())
        || endpoints.any(|endpoint| transport.is_montgomery_form_of(endpoint))
}

/// A verified contact card.
///
/// A value of this type has passed every check of `docs/PROTOCOL.md` section
/// 11.2, signature included. There is no unverified card type: decoding and
/// verifying are one step.
#[derive(Clone, PartialEq, Eq)]
pub struct ContactCard {
    identity: IdentityPublicKey,
    transport: TransportPublicKey,
    epoch: EndpointEpoch,
    endpoints: EndpointSet,
    invitation: Option<InvitationCapability>,
    signature: Signature,
}

impl ContactCard {
    /// Creates and signs a card for the identity that `secret` belongs to.
    /// `transport` is the X25519 key with which that identity authenticates
    /// its sessions.
    ///
    /// Fails if one of the endpoints is the identity key itself, or if the
    /// transport key is the identity key or an endpoint key in Montgomery
    /// form. That is key separation (`docs/PROTOCOL.md` section 11.2,
    /// invariant S33): the identity key, the transport key and the master
    /// key of an Onion Service belong to different cryptographic domains
    /// and are never the same key. No receiver accepts such a card.
    pub fn sign(
        secret: &IdentitySecretKey,
        transport: TransportPublicKey,
        epoch: EndpointEpoch,
        endpoints: EndpointSet,
        invitation: Option<InvitationCapability>,
    ) -> Result<Self, ProtocolError> {
        let identity = secret.public_key();
        if endpoints.reuses(&identity) {
            return Err(ProtocolError::InvalidValue);
        }
        if transport_reuses(
            &transport,
            &identity,
            endpoints.iter().map(OnionServiceKey::as_bytes),
        ) {
            return Err(ProtocolError::InvalidValue);
        }
        let message = signed_bytes(
            &identity,
            &transport,
            epoch,
            &endpoints,
            invitation.as_ref(),
        );
        Ok(Self {
            identity,
            transport,
            epoch,
            endpoints,
            invitation,
            signature: secret.sign(&message),
        })
    }

    /// Returns the identity that signed the card.
    pub const fn identity(&self) -> &IdentityPublicKey {
        &self.identity
    }

    /// Returns the transport key the identity states in this card.
    pub const fn transport(&self) -> &TransportPublicKey {
        &self.transport
    }

    /// Returns the epoch of the card. It covers the transport key and the
    /// endpoint set.
    pub const fn epoch(&self) -> EndpointEpoch {
        self.epoch
    }

    /// Returns the endpoint set.
    pub const fn endpoints(&self) -> &EndpointSet {
        &self.endpoints
    }

    /// Returns the invitation capability, if the card carries one.
    pub const fn invitation(&self) -> Option<&InvitationCapability> {
        self.invitation.as_ref()
    }

    /// Returns the signature.
    pub const fn signature(&self) -> &Signature {
        &self.signature
    }

    /// Returns the bytes the signature covers. See `docs/PROTOCOL.md`
    /// section 11.1.1.
    pub fn signed_bytes(&self) -> Vec<u8> {
        signed_bytes(
            &self.identity,
            &self.transport,
            self.epoch,
            &self.endpoints,
            self.invitation.as_ref(),
        )
    }

    /// Returns the binary transport form.
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::with_capacity(MAX_CONTACT_CARD_LEN);
        write_fields(
            &mut writer,
            &self.identity,
            &self.transport,
            self.epoch,
            &self.endpoints,
            self.invitation.as_ref(),
        );
        writer.raw(self.signature.as_bytes());
        writer.into_bytes()
    }

    /// Decodes and verifies the binary transport form.
    ///
    /// The checks are those of `docs/PROTOCOL.md` section 11.2, in that
    /// order. The input must be exactly one card; trailing bytes are an
    /// error.
    pub fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        if bytes.len() < CONTACT_CARD_FIXED_LEN || bytes.len() > MAX_CONTACT_CARD_LEN {
            return Err(ProtocolError::BadMessageLength);
        }
        let mut reader = Reader::new(bytes);

        if reader.u8()? != CARD_VERSION {
            return Err(ProtocolError::UnsupportedVersion);
        }
        let identity_bytes: [u8; 32] = reader.array()?;
        let transport_bytes: [u8; 32] = reader.array()?;
        let epoch = EndpointEpoch::new(reader.u64()?).map_err(|_| ProtocolError::InvalidValue)?;

        let count = usize::from(reader.u8()?);
        if count == 0 || count > MAX_ACTIVE_ENDPOINTS {
            return Err(ProtocolError::InvalidValue);
        }
        let mut endpoint_bytes: Vec<[u8; 32]> = Vec::with_capacity(count);
        for _ in 0..count {
            endpoint_bytes.push(reader.array()?);
        }

        let flags = reader.u8()?;
        if flags & !FLAG_INVITATION != 0 {
            return Err(ProtocolError::InvalidValue);
        }
        let invitation = if flags & FLAG_INVITATION != 0 {
            Some(InvitationCapability::from_bytes(reader.array()?))
        } else {
            None
        };
        let signature = Signature::from_bytes(reader.array()?);
        reader.finish()?;

        let identity = IdentityPublicKey::from_bytes(&identity_bytes)?;
        let transport = TransportPublicKey::from_bytes(&transport_bytes)?;
        if transport_reuses(&transport, &identity, endpoint_bytes.iter()) {
            return Err(ProtocolError::InvalidValue);
        }
        let mut endpoints: Vec<OnionServiceKey> = Vec::with_capacity(count);
        for raw in &endpoint_bytes {
            endpoints.push(OnionServiceKey::from_bytes(raw)?);
        }
        let endpoints = EndpointSet::new(&endpoints)?;
        if endpoints.reuses(&identity) {
            return Err(ProtocolError::InvalidValue);
        }

        let message = signed_bytes(
            &identity,
            &transport,
            epoch,
            &endpoints,
            invitation.as_ref(),
        );
        identity.verify(&message, &signature)?;

        Ok(Self {
            identity,
            transport,
            epoch,
            endpoints,
            invitation,
            signature,
        })
    }

    /// Returns the text form: `MONOLITH1:` followed by unpadded upper-case
    /// base32 of the binary form.
    pub fn to_text(&self) -> String {
        let mut text = String::from(TEXT_PREFIX);
        text.push_str(&base32::encode(&self.encode()));
        text
    }

    /// Parses and verifies the text form.
    ///
    /// Input longer than [`MAX_CONTACT_CARD_TEXT_LEN`] bytes is rejected
    /// before anything else is looked at. ASCII space, tab, carriage return
    /// and line feed are removed wherever they occur. Letters are accepted
    /// in either case. Anything else that is not part of the format rejects
    /// the input.
    pub fn from_text(input: &str) -> Result<Self, ProtocolError> {
        if input.len() > MAX_CONTACT_CARD_TEXT_LEN {
            return Err(ProtocolError::FieldTooLong);
        }
        let mut compact = String::with_capacity(input.len());
        for character in input.chars() {
            match character {
                ' ' | '\t' | '\r' | '\n' => {}
                c if c.is_ascii_graphic() => compact.push(c),
                _ => return Err(ProtocolError::InvalidEncoding),
            }
        }
        let (prefix, encoded) = compact
            .split_at_checked(TEXT_PREFIX.len())
            .ok_or(ProtocolError::InvalidEncoding)?;
        if !prefix.eq_ignore_ascii_case(TEXT_PREFIX) {
            return Err(ProtocolError::InvalidEncoding);
        }
        let bytes = base32::decode(encoded, MAX_CONTACT_CARD_LEN)?;
        Self::decode(&bytes)
    }
}

impl fmt::Debug for ContactCard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContactCard({REDACTED})")
    }
}

/// Writes every field of a card except the signature, in transport order.
fn write_fields(
    writer: &mut Writer,
    identity: &IdentityPublicKey,
    transport: &TransportPublicKey,
    epoch: EndpointEpoch,
    endpoints: &EndpointSet,
    invitation: Option<&InvitationCapability>,
) {
    writer.u8(CARD_VERSION);
    writer.raw(identity.as_bytes());
    writer.raw(transport.as_bytes());
    writer.u64(epoch.get());
    // An EndpointSet holds at most MAX_ACTIVE_ENDPOINTS members, and limits.rs
    // asserts that this maximum fits in one byte.
    writer.u8(u8::try_from(endpoints.count()).unwrap_or(u8::MAX));
    for endpoint in endpoints.iter() {
        writer.raw(endpoint.as_bytes());
    }
    writer.u8(if invitation.is_some() {
        FLAG_INVITATION
    } else {
        0
    });
    if let Some(capability) = invitation {
        writer.raw(capability.expose());
    }
}

/// Builds the bytes a card signature covers.
///
/// This is deliberately its own function and not "the encoded card minus
/// the signature". The field list is that of `docs/PROTOCOL.md` section
/// 11.1.1. A unit test pins the relation to the transport form that holds in
/// version 1.
fn signed_bytes(
    identity: &IdentityPublicKey,
    transport: &TransportPublicKey,
    epoch: EndpointEpoch,
    endpoints: &EndpointSet,
    invitation: Option<&InvitationCapability>,
) -> Vec<u8> {
    let mut writer =
        Writer::with_capacity(SIGNING_PREFIX.len().saturating_add(MAX_CONTACT_CARD_LEN));
    writer.raw(SIGNING_PREFIX);
    writer.u8(CARD_VERSION);
    writer.raw(identity.as_bytes());
    writer.raw(transport.as_bytes());
    writer.u64(epoch.get());
    writer.u8(u8::try_from(endpoints.count()).unwrap_or(u8::MAX));
    for endpoint in endpoints.iter() {
        writer.raw(endpoint.as_bytes());
    }
    writer.u8(if invitation.is_some() {
        FLAG_INVITATION
    } else {
        0
    });
    if let Some(capability) = invitation {
        writer.raw(capability.expose());
    }
    writer.into_bytes()
}

/// What a card of a contact means for what is held of that contact: its
/// transport key and its endpoint set, as of an epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CardChange {
    /// The epoch is greater than that of the newest card held. The card
    /// becomes the newest card held at once, and the pinned card once the
    /// change is confirmed.
    Newer,
    /// Same epoch, same transport key, same endpoint set. Nothing to do;
    /// this is the normal case.
    Unchanged,
    /// Same epoch and another transport key or endpoint set: the owner
    /// signed two statements with one epoch. Nothing changes; the user is
    /// told.
    Conflict,
    /// The epoch is lower than that of the newest card held. Nothing
    /// changes.
    Stale,
}

/// Compares a card of a contact with the newest card that is held of that
/// contact: the pinned one, or a pending one with a greater epoch. See
/// `docs/PROTOCOL.md` sections 6.2, 8.8 and 11.4.
///
/// Fails with [`ProtocolError::IdentityMismatch`] if the card was signed by
/// another identity than the held one. Nothing but [`CardChange::Newer`]
/// ever leads to a change of what is held or pinned. An invitation
/// capability in either card plays no part.
pub fn evaluate_card(held: &ContactCard, card: &ContactCard) -> Result<CardChange, ProtocolError> {
    if card.identity() != held.identity() {
        return Err(ProtocolError::IdentityMismatch);
    }
    if held.epoch().is_superseded_by(card.epoch()) {
        return Ok(CardChange::Newer);
    }
    if card.epoch() == held.epoch() {
        if card.transport() == held.transport() && card.endpoints() == held.endpoints() {
            return Ok(CardChange::Unchanged);
        }
        return Ok(CardChange::Conflict);
    }
    Ok(CardChange::Stale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::CONTACT_CARD_BASE_LEN;

    fn secret(seed: u8) -> IdentitySecretKey {
        IdentitySecretKey::from_seed(&[seed; 32])
    }

    fn endpoint(seed: u8) -> OnionServiceKey {
        let key = *secret(seed).public_key().as_bytes();
        OnionServiceKey::from_bytes(&key).unwrap()
    }

    fn epoch(value: u64) -> EndpointEpoch {
        EndpointEpoch::new(value).unwrap()
    }

    /// A valid X25519 public key that depends on the seed. The tests of
    /// this crate need no private half for it.
    fn transport(seed: u8) -> TransportPublicKey {
        let mut bytes = [seed; 32];
        bytes[31] = 0x40;
        TransportPublicKey::from_bytes(&bytes).unwrap()
    }

    fn card(seed: u8, epoch_value: u64, invitation: bool) -> ContactCard {
        ContactCard::sign(
            &secret(seed),
            transport(seed),
            epoch(epoch_value),
            EndpointSet::single(endpoint(seed.wrapping_add(100))),
            invitation.then(|| InvitationCapability::from_bytes([0xC4; 16])),
        )
        .unwrap()
    }

    /// Builds the bytes of a card from raw fields and signs them with the
    /// key of `seed`, without any of the checks that [`ContactCard::sign`]
    /// and the typed fields apply.
    fn raw_card(seed: u8, endpoint_bytes: &[u8; 32]) -> Vec<u8> {
        raw_card_with_transport(seed, transport(seed).as_bytes(), endpoint_bytes)
    }

    /// The same, with the transport key given as raw bytes.
    fn raw_card_with_transport(
        seed: u8,
        transport_bytes: &[u8; 32],
        endpoint_bytes: &[u8; 32],
    ) -> Vec<u8> {
        let mut fields = vec![CARD_VERSION];
        fields.extend_from_slice(secret(seed).public_key().as_bytes());
        fields.extend_from_slice(transport_bytes);
        fields.extend_from_slice(&7_u64.to_be_bytes());
        fields.push(1);
        fields.extend_from_slice(endpoint_bytes);
        fields.push(0);

        let mut signed = SIGNING_PREFIX.to_vec();
        signed.extend_from_slice(&fields);
        let signature = secret(seed).sign(&signed);

        fields.extend_from_slice(signature.as_bytes());
        fields
    }

    #[test]
    fn raw_card_helper_builds_valid_cards() {
        // The helper is only useful if a card it builds from good fields is
        // accepted, so that a rejection is due to the field under test.
        let bytes = raw_card(1, endpoint(101).as_bytes());
        assert_eq!(bytes, card(1, 7, false).encode());
        assert!(ContactCard::decode(&bytes).is_ok());
    }

    #[test]
    fn a_signed_card_with_an_invalid_endpoint_is_rejected() {
        // The signature is good; the endpoint is not a valid onion service
        // key. First the neutral element, then a canonical point outside
        // the prime-order subgroup.
        let mut neutral = [0_u8; 32];
        neutral[0] = 1;
        assert_eq!(
            ContactCard::decode(&raw_card(1, &neutral)),
            Err(ProtocolError::InvalidKey)
        );

        let mut order_two = [0xff_u8; 32];
        order_two[0] = 0xec;
        order_two[31] = 0x7f;
        let honest = ed25519_dalek::VerifyingKey::from_bytes(endpoint(101).as_bytes())
            .unwrap()
            .to_edwards();
        let torsion = ed25519_dalek::VerifyingKey::from_bytes(&order_two)
            .unwrap()
            .to_edwards();
        let mixed = (honest + torsion).compress().to_bytes();
        assert_eq!(
            ContactCard::decode(&raw_card(1, &mixed)),
            Err(ProtocolError::InvalidKey)
        );

        // Not a point at all.
        let mut off_curve = [0_u8; 32];
        off_curve[0] = 2;
        assert_eq!(
            ContactCard::decode(&raw_card(1, &off_curve)),
            Err(ProtocolError::InvalidKey)
        );
    }

    #[test]
    fn an_endpoint_must_not_be_the_identity_key() {
        let identity_key = *secret(1).public_key().as_bytes();
        assert_eq!(
            ContactCard::decode(&raw_card(1, &identity_key)),
            Err(ProtocolError::InvalidValue)
        );
        assert_eq!(
            ContactCard::sign(
                &secret(1),
                transport(1),
                epoch(7),
                EndpointSet::single(OnionServiceKey::from_bytes(&identity_key).unwrap()),
                None,
            ),
            Err(ProtocolError::InvalidValue)
        );
    }

    /// The Montgomery form of the Ed25519 public key of `seed`.
    fn montgomery(seed: u8) -> [u8; 32] {
        ed25519_dalek::VerifyingKey::from_bytes(secret(seed).public_key().as_bytes())
            .unwrap()
            .to_montgomery()
            .to_bytes()
    }

    #[test]
    fn a_signed_card_with_an_invalid_transport_key_is_rejected() {
        // The signature is good; the transport key is not a valid X25519
        // key. A point of small order, a value that is not below the field
        // prime, and a key with the top bit set.
        let mut order_eight = [0_u8; 32];
        order_eight[..31].copy_from_slice(&[
            0xe0, 0xeb, 0x7a, 0x7c, 0x3b, 0x41, 0xb8, 0xae, 0x16, 0x56, 0xe3, 0xfa, 0xf1, 0x9f,
            0xc4, 0x6a, 0xda, 0x09, 0x8d, 0xeb, 0x9c, 0x32, 0xb1, 0xfd, 0x86, 0x62, 0x05, 0x16,
            0x5f, 0x49, 0xb8,
        ]);
        let mut prime = [0xff_u8; 32];
        prime[0] = 0xed;
        prime[31] = 0x7f;
        let mut top_bit = *transport(1).as_bytes();
        top_bit[31] |= 0x80;
        for bad in [[0_u8; 32], order_eight, prime, top_bit] {
            assert_eq!(
                ContactCard::decode(&raw_card_with_transport(1, &bad, endpoint(101).as_bytes())),
                Err(ProtocolError::InvalidKey)
            );
        }
        // The helper builds a valid card from good fields.
        assert!(
            ContactCard::decode(&raw_card_with_transport(
                1,
                transport(9).as_bytes(),
                endpoint(101).as_bytes()
            ))
            .is_ok()
        );
    }

    #[test]
    fn the_transport_key_must_not_be_the_identity_key_in_montgomery_form() {
        let converted = montgomery(1);
        assert_eq!(
            ContactCard::decode(&raw_card_with_transport(
                1,
                &converted,
                endpoint(101).as_bytes()
            )),
            Err(ProtocolError::InvalidValue)
        );
        assert_eq!(
            ContactCard::sign(
                &secret(1),
                TransportPublicKey::from_bytes(&converted).unwrap(),
                epoch(7),
                EndpointSet::single(endpoint(101)),
                None,
            ),
            Err(ProtocolError::InvalidValue)
        );
        // The same key is fine in the card of another identity.
        assert!(
            ContactCard::sign(
                &secret(2),
                TransportPublicKey::from_bytes(&converted).unwrap(),
                epoch(7),
                EndpointSet::single(endpoint(101)),
                None,
            )
            .is_ok()
        );
    }

    #[test]
    fn the_transport_key_must_not_be_an_endpoint_key_in_montgomery_form() {
        // endpoint(101) is the Ed25519 public key of seed 101.
        let converted = montgomery(101);
        assert_eq!(
            ContactCard::decode(&raw_card_with_transport(
                1,
                &converted,
                endpoint(101).as_bytes()
            )),
            Err(ProtocolError::InvalidValue)
        );
        assert_eq!(
            ContactCard::sign(
                &secret(1),
                TransportPublicKey::from_bytes(&converted).unwrap(),
                epoch(7),
                EndpointSet::single(endpoint(101)),
                None,
            ),
            Err(ProtocolError::InvalidValue)
        );
        // With another endpoint the same transport key is accepted.
        assert!(
            ContactCard::decode(&raw_card_with_transport(
                1,
                &converted,
                endpoint(102).as_bytes()
            ))
            .is_ok()
        );
    }

    #[test]
    fn key_separation_is_checked_before_the_endpoints_are_validated() {
        // PROTOCOL.md 11.2 puts the transport key (step 8) before the
        // endpoints (step 9). A card that fails both reports the first:
        // the reused key, not the invalid endpoint.
        let mut neutral = [0_u8; 32];
        neutral[0] = 1;
        assert_eq!(
            ContactCard::decode(&raw_card(1, &neutral)),
            Err(ProtocolError::InvalidKey)
        );
        assert_eq!(
            ContactCard::decode(&raw_card_with_transport(1, &montgomery(1), &neutral)),
            Err(ProtocolError::InvalidValue)
        );
    }

    #[test]
    fn endpoint_sets_compare_as_sets() {
        // Version 1 cannot build a set with two members through the public
        // constructors, so this builds them directly.
        let ab = EndpointSet {
            first: endpoint(1),
            rest: vec![endpoint(2)],
        };
        let ba = EndpointSet {
            first: endpoint(2),
            rest: vec![endpoint(1)],
        };
        let ac = EndpointSet {
            first: endpoint(1),
            rest: vec![endpoint(3)],
        };
        assert_eq!(ab, ba);
        assert_ne!(ab, ac);
        assert_ne!(ab, EndpointSet::single(endpoint(1)));
        assert_ne!(EndpointSet::single(endpoint(1)), ab);
        assert!(ab.contains(&endpoint(2)));
        assert!(!ab.contains(&endpoint(3)));
        // The order is still the order of the card.
        assert_eq!(ab.first(), &endpoint(1));
        assert_eq!(ba.first(), &endpoint(2));
    }

    #[test]
    fn invitation_capabilities_compare_by_value() {
        let a = InvitationCapability::from_bytes([1; 16]);
        let mut other = [1_u8; 16];
        assert_eq!(a, InvitationCapability::from_bytes(other));
        for index in 0..16 {
            other = [1; 16];
            other[index] ^= 0x80;
            assert_ne!(a, InvitationCapability::from_bytes(other), "byte {index}");
        }
    }

    #[test]
    fn an_invitation_capability_is_erased_and_copied_only_on_purpose() {
        let mut capability = InvitationCapability::from_bytes([0xC4; 16]);
        let copy = capability.clone();
        assert_eq!(copy, capability);
        assert_eq!(copy.expose(), &[0xC4; 16]);

        // Erasing one owner leaves the other as it was.
        capability.zeroize();
        assert_eq!(capability.expose(), &[0; 16]);
        assert_eq!(copy.expose(), &[0xC4; 16]);

        // Dropping erases. The memory of a dropped value cannot be read
        // without unsafe code, so this checks that the type promises it.
        fn erased_on_drop<T: ZeroizeOnDrop>() {}
        erased_on_drop::<InvitationCapability>();

        // The type is not Copy. If it were, both impls below would apply
        // and the call would be ambiguous, which does not compile.
        trait AmbiguousIfCopy<A> {
            fn check() {}
        }
        impl<T> AmbiguousIfCopy<()> for T {}
        impl<T: Copy> AmbiguousIfCopy<u8> for T {}
        <InvitationCapability as AmbiguousIfCopy<_>>::check();
        <ContactCard as AmbiguousIfCopy<_>>::check();
    }

    #[test]
    fn sizes_match_the_specification() {
        assert_eq!(card(1, 1, false).encode().len(), 171);
        assert_eq!(card(1, 1, true).encode().len(), 187);
        assert_eq!(card(1, 1, false).encode().len(), CONTACT_CARD_BASE_LEN);
        assert_eq!(card(1, 1, true).encode().len(), MAX_CONTACT_CARD_LEN);
        assert_eq!(card(1, 1, false).signed_bytes().len(), 131);
        assert_eq!(card(1, 1, true).signed_bytes().len(), 147);
    }

    #[test]
    fn binary_form_round_trips() {
        for invitation in [false, true] {
            let original = card(1, 7, invitation);
            let bytes = original.encode();
            let decoded = ContactCard::decode(&bytes).unwrap();
            assert_eq!(decoded, original);
            assert_eq!(decoded.encode(), bytes);
        }
    }

    #[test]
    fn field_layout_is_as_specified() {
        let original = card(1, 0x0102_0304_0506_0708, true);
        let bytes = original.encode();
        assert_eq!(bytes[0], 0x01);
        assert_eq!(&bytes[1..33], original.identity().as_bytes());
        assert_eq!(&bytes[33..65], original.transport().as_bytes());
        assert_eq!(&bytes[65..73], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(bytes[73], 1);
        assert_eq!(&bytes[74..106], original.endpoints().first().as_bytes());
        assert_eq!(bytes[106], 0x01);
        assert_eq!(&bytes[107..123], &[0xC4; 16]);
        assert_eq!(&bytes[123..187], original.signature().as_bytes());
    }

    #[test]
    fn signed_bytes_are_the_prefix_and_the_unsigned_fields() {
        for invitation in [false, true] {
            let original = card(1, 7, invitation);
            let encoded = original.encode();
            let unsigned = &encoded[..encoded.len() - 64];
            let mut expected = b"MONOLITH-CONTACT-CARD-V1".to_vec();
            expected.extend_from_slice(unsigned);
            assert_eq!(original.signed_bytes(), expected);
        }
    }

    #[test]
    fn signature_verifies_over_the_signed_bytes() {
        let original = card(1, 7, true);
        assert!(
            original
                .identity()
                .verify(&original.signed_bytes(), original.signature())
                .is_ok()
        );
    }

    #[test]
    fn every_single_bit_flip_is_rejected() {
        for invitation in [false, true] {
            let good = card(1, 7, invitation).encode();
            for byte in 0..good.len() {
                for bit in 0..8 {
                    let mut bad = good.clone();
                    bad[byte] ^= 1 << bit;
                    assert!(
                        ContactCard::decode(&bad).is_err(),
                        "byte {byte} bit {bit} accepted"
                    );
                }
            }
        }
    }

    #[test]
    fn wrong_lengths_are_rejected() {
        let good = card(1, 7, false).encode();
        assert_eq!(
            ContactCard::decode(&good[..good.len() - 1]),
            Err(ProtocolError::BadMessageLength)
        );
        let mut long = good.clone();
        long.push(0);
        assert_eq!(
            ContactCard::decode(&long),
            Err(ProtocolError::BadMessageLength)
        );
        assert_eq!(
            ContactCard::decode(&[]),
            Err(ProtocolError::BadMessageLength)
        );
        assert_eq!(
            ContactCard::decode(&[0; MAX_CONTACT_CARD_LEN + 1]),
            Err(ProtocolError::BadMessageLength)
        );
    }

    #[test]
    fn length_must_match_the_invitation_flag() {
        // A card with the flag set but no capability bytes, and the reverse.
        // Both fail on the length, before any key or signature is looked at.
        let without = card(1, 7, false).encode();
        let mut flag_set = without.clone();
        flag_set[106] = 0x01;
        assert_eq!(
            ContactCard::decode(&flag_set),
            Err(ProtocolError::BadMessageLength)
        );

        let with = card(1, 7, true).encode();
        let mut flag_clear = with.clone();
        flag_clear[106] = 0x00;
        assert_eq!(
            ContactCard::decode(&flag_clear),
            Err(ProtocolError::BadMessageLength)
        );
    }

    #[test]
    fn wrong_version_is_rejected() {
        let mut bytes = card(1, 7, false).encode();
        bytes[0] = 0x02;
        assert_eq!(
            ContactCard::decode(&bytes),
            Err(ProtocolError::UnsupportedVersion)
        );
    }

    #[test]
    fn epoch_zero_is_rejected() {
        let mut bytes = card(1, 7, false).encode();
        bytes[65..73].copy_from_slice(&[0; 8]);
        assert_eq!(
            ContactCard::decode(&bytes),
            Err(ProtocolError::InvalidValue)
        );
    }

    #[test]
    fn endpoint_count_must_be_exactly_one_in_version_1() {
        for count in [0_u8, 2, 3, 255] {
            let mut bytes = card(1, 7, false).encode();
            bytes[73] = count;
            assert_eq!(
                ContactCard::decode(&bytes),
                Err(ProtocolError::InvalidValue),
                "count {count}"
            );
        }
    }

    #[test]
    fn reserved_flag_bits_are_rejected() {
        for flags in [0x02_u8, 0x04, 0x80, 0xfe] {
            let mut bytes = card(1, 7, false).encode();
            bytes[106] = flags;
            assert_eq!(
                ContactCard::decode(&bytes),
                Err(ProtocolError::InvalidValue),
                "flags {flags:#x}"
            );
        }
    }

    #[test]
    fn invalid_identity_key_is_rejected() {
        let mut bytes = card(1, 7, false).encode();
        // The neutral element: a small-order point.
        bytes[1..33].copy_from_slice(&{
            let mut key = [0_u8; 32];
            key[0] = 1;
            key
        });
        assert_eq!(ContactCard::decode(&bytes), Err(ProtocolError::InvalidKey));
    }

    #[test]
    fn a_card_signed_by_another_identity_is_rejected() {
        let mine = card(1, 7, false).encode();
        let other = card(2, 7, false).encode();
        let mut forged = mine.clone();
        forged[107..].copy_from_slice(&other[107..]);
        assert_eq!(
            ContactCard::decode(&forged),
            Err(ProtocolError::BadSignature)
        );
    }

    #[test]
    fn endpoint_set_rejects_empty_duplicate_and_oversized_input() {
        assert_eq!(EndpointSet::new(&[]), Err(ProtocolError::InvalidValue));
        assert!(EndpointSet::new(&[endpoint(1)]).is_ok());
        assert_eq!(
            EndpointSet::new(&[endpoint(1), endpoint(1)]),
            Err(ProtocolError::InvalidValue)
        );
        let too_many: Vec<OnionServiceKey> = (0..=MAX_ACTIVE_ENDPOINTS)
            .map(|i| endpoint(u8::try_from(i).unwrap()))
            .collect();
        assert_eq!(
            EndpointSet::new(&too_many),
            Err(ProtocolError::InvalidValue)
        );
        assert_eq!(EndpointSet::single(endpoint(1)).count(), 1);
    }

    #[test]
    fn text_form_round_trips_and_is_upper_case() {
        for invitation in [false, true] {
            let original = card(1, 7, invitation);
            let text = original.to_text();
            assert!(text.starts_with("MONOLITH1:"));
            assert!(!text.chars().any(|c| c.is_ascii_lowercase()));
            assert_eq!(ContactCard::from_text(&text).unwrap(), original);
        }
        assert_eq!(card(1, 7, false).to_text().len(), 284);
        assert_eq!(card(1, 7, true).to_text().len(), 310);
    }

    #[test]
    fn text_form_accepts_lower_case_and_whitespace() {
        let original = card(1, 7, true);
        let text = original.to_text();

        assert_eq!(
            ContactCard::from_text(&text.to_ascii_lowercase()).unwrap(),
            original
        );

        let wrapped: String = text
            .as_bytes()
            .chunks(40)
            .map(|chunk| format!("  {}\r\n", core::str::from_utf8(chunk).unwrap()))
            .collect();
        assert_eq!(ContactCard::from_text(&wrapped).unwrap(), original);

        let grouped: String = text
            .as_bytes()
            .chunks(4)
            .map(|chunk| format!("{} ", core::str::from_utf8(chunk).unwrap()))
            .collect();
        assert_eq!(ContactCard::from_text(&grouped).unwrap(), original);
    }

    #[test]
    fn text_form_rejects_bad_input() {
        let text = card(1, 7, false).to_text();
        let body = &text["MONOLITH1:".len()..];

        for bad in [
            String::new(),
            "MONOLITH1:".to_owned(),
            body.to_owned(),
            format!("MONOLITH2:{body}"),
            format!("monolith:{body}"),
            format!("{text}="),
            format!("{text}A"),
            format!("{text}\u{e9}"),
            format!("{}0{}", &text[..20], &text[21..]),
            text[..text.len() - 1].to_owned(),
        ] {
            assert!(ContactCard::from_text(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn text_form_rejects_oversized_input_before_decoding() {
        let long = "A".repeat(MAX_CONTACT_CARD_TEXT_LEN + 1);
        assert_eq!(
            ContactCard::from_text(&long),
            Err(ProtocolError::FieldTooLong)
        );
        let spaces = " ".repeat(MAX_CONTACT_CARD_TEXT_LEN + 1);
        assert_eq!(
            ContactCard::from_text(&spaces),
            Err(ProtocolError::FieldTooLong)
        );
    }

    #[test]
    fn text_form_rejects_nonzero_trailing_bits() {
        // 171 bytes are 1368 bits and 274 symbols, so the last symbol has
        // two unused bits. Setting one of them must be rejected.
        let text = card(1, 7, false).to_text();
        let last = text.chars().last().unwrap();
        let index = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".find(last).unwrap();
        assert_eq!(index % 4, 0, "canonical form has zero trailing bits");
        let altered = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"
            .chars()
            .nth(index + 1)
            .unwrap();
        let bad = format!("{}{altered}", &text[..text.len() - 1]);
        assert_eq!(
            ContactCard::from_text(&bad),
            Err(ProtocolError::InvalidEncoding)
        );
    }

    #[test]
    fn known_answer() {
        // The card of R in docs/PROTOCOL.md section 16.1. Identity seed:
        // RFC 8032 test vector 1. Transport key: the first public key of
        // RFC 7748 section 6.1. Endpoint: the public key of the all-0x02
        // seed. Epoch 1, no invitation.
        let seed: [u8; 32] = [
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
            0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
            0x1c, 0xae, 0x7f, 0x60,
        ];
        let transport_key: [u8; 32] = [
            0x85, 0x20, 0xf0, 0x09, 0x89, 0x30, 0xa7, 0x54, 0x74, 0x8b, 0x7d, 0xdc, 0xb4, 0x3e,
            0xf7, 0x5a, 0x0d, 0xbf, 0x3a, 0x0d, 0x26, 0x38, 0x1a, 0xf4, 0xeb, 0xa4, 0xa9, 0x8e,
            0xaa, 0x9b, 0x4e, 0x6a,
        ];
        let known = ContactCard::sign(
            &IdentitySecretKey::from_seed(&seed),
            TransportPublicKey::from_bytes(&transport_key).unwrap(),
            EndpointEpoch::FIRST,
            EndpointSet::single(endpoint(2)),
            None,
        )
        .unwrap();
        assert_eq!(known.to_text(), KNOWN_CARD_TEXT);
        assert_eq!(ContactCard::from_text(KNOWN_CARD_TEXT).unwrap(), known);
        assert_eq!(hex(known.signature().as_bytes()), KNOWN_CARD_SIGNATURE);
        assert_eq!(
            hex(&known.signed_bytes()),
            concat!(
                "4d4f4e4f4c4954482d434f4e544143542d434152442d563101",
                "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a",
                "8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a",
                "000000000000000101",
                "8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394",
                "00",
            )
        );
    }

    #[test]
    fn known_answer_of_the_initiator_card() {
        // The card of I in docs/PROTOCOL.md section 16.1, as it appears in
        // the third handshake message.
        let bytes: Vec<u8> = (0..KNOWN_INITIATOR_CARD.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&KNOWN_INITIATOR_CARD[index..index + 2], 16).unwrap())
            .collect();
        assert_eq!(bytes.len(), 171);
        let decoded = ContactCard::decode(&bytes).unwrap();
        assert_eq!(decoded.encode(), bytes);
        assert_eq!(
            hex(decoded.identity().as_bytes()),
            "3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c"
        );
        assert_eq!(
            hex(decoded.transport().as_bytes()),
            "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f"
        );
        assert_eq!(decoded.epoch(), EndpointEpoch::FIRST);
        assert_eq!(decoded.endpoints(), &EndpointSet::single(endpoint(3)));
        assert!(decoded.invitation().is_none());

        // Signing the same fields gives the same bytes.
        let seed: [u8; 32] = [
            0x4c, 0xcd, 0x08, 0x9b, 0x28, 0xff, 0x96, 0xda, 0x9d, 0xb6, 0xc3, 0x46, 0xec, 0x11,
            0x4e, 0x0f, 0x5b, 0x8a, 0x31, 0x9f, 0x35, 0xab, 0xa6, 0x24, 0xda, 0x8c, 0xf6, 0xed,
            0x4f, 0xb8, 0xa6, 0xfb,
        ];
        let signed = ContactCard::sign(
            &IdentitySecretKey::from_seed(&seed),
            *decoded.transport(),
            EndpointEpoch::FIRST,
            EndpointSet::single(endpoint(3)),
            None,
        )
        .unwrap();
        assert_eq!(signed.encode(), bytes);
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Reproduced with an independent Ed25519 implementation written from
    /// RFC 8032, following the field list of `docs/PROTOCOL.md` 11.1.1.
    const KNOWN_CARD_TEXT: &str = "MONOLITH1:AHLVVGABQKYQVN6VJP7NHSLEA45A5YLS6PNKMIZFV4BBU2HXA5IRVBJA6AEYSMFHKR2IW7O4WQ7POWQNX45A2JRYDL2OXJFJR2VJWTTKAAAAAAAAAAAACAMBHF3Q5KD5C5PVNI2UM3BUY7WMZOGYVENU5Y32EXPWB5NY7SNTSQAN4L47JWL3SCBZOLLAFXTZ6JK2RASOSQAL43SXXBA7ALY33GZ4R6QTXCN2RKXEPYRJ4PSJ34NEEL2XOAWXID2XQRJPDV3EHRZSDSKMAM";

    const KNOWN_CARD_SIGNATURE: &str = concat!(
        "de2f9f4d97b9083972d602de79f255a8824e9400be6e57b841f02f1bd9b3c8fa",
        "13b89ba8aae47e229e3e49df1a422f57702d740f578452f1d7643c7321c94c03",
    );

    const KNOWN_INITIATOR_CARD: &str = concat!(
        "013d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af466",
        "0cde9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b",
        "4f000000000000000101ed4928c628d1c2c6eae90338905995612959273a5c63",
        "f93636c14614ac8737d1007c98b9a384ab4a8dd22e7b752249ae282a5d4072de",
        "c9c640f2d1d59de1844e5f3192e3263077fd11c77b5afb9f9b80d2e2d77b1648",
        "7b43119b9746b95f2e2f0d",
    );

    #[test]
    fn card_change_outcomes() {
        let pinned = card(1, 5, false);
        let pinned_set = pinned.endpoints().clone();
        let evaluate = |candidate: &ContactCard| evaluate_card(&pinned, candidate);

        assert_eq!(evaluate(&pinned), Ok(CardChange::Unchanged));

        let signed = |epoch_value: u64, key: TransportPublicKey, endpoints: EndpointSet| {
            ContactCard::sign(&secret(1), key, epoch(epoch_value), endpoints, None).unwrap()
        };
        let other_set = || EndpointSet::single(endpoint(50));

        let newer = signed(6, transport(1), other_set());
        assert_eq!(evaluate(&newer), Ok(CardChange::Newer));

        let newer_same_content = signed(6, transport(1), pinned_set.clone());
        assert_eq!(evaluate(&newer_same_content), Ok(CardChange::Newer));

        let newer_transport = signed(6, transport(9), pinned_set.clone());
        assert_eq!(evaluate(&newer_transport), Ok(CardChange::Newer));

        // Same epoch: any difference in what the card states is a conflict.
        let conflict = signed(5, transport(1), other_set());
        assert_eq!(evaluate(&conflict), Ok(CardChange::Conflict));
        let conflict_transport = signed(5, transport(9), pinned_set.clone());
        assert_eq!(evaluate(&conflict_transport), Ok(CardChange::Conflict));
        let conflict_both = signed(5, transport(9), other_set());
        assert_eq!(evaluate(&conflict_both), Ok(CardChange::Conflict));

        let stale = signed(4, transport(1), other_set());
        assert_eq!(evaluate(&stale), Ok(CardChange::Stale));

        // A lower epoch is stale even if it states what is pinned.
        let stale_same_content = signed(4, transport(1), pinned_set.clone());
        assert_eq!(evaluate(&stale_same_content), Ok(CardChange::Stale));
        let stale_transport = signed(4, transport(9), pinned_set.clone());
        assert_eq!(evaluate(&stale_transport), Ok(CardChange::Stale));

        // An invitation capability is not part of what is pinned.
        assert_eq!(evaluate(&card(1, 5, true)), Ok(CardChange::Unchanged));
        assert_eq!(
            evaluate_card(&card(1, 5, true), &pinned),
            Ok(CardChange::Unchanged)
        );

        let other_identity = card(2, 9, false);
        assert_eq!(
            evaluate(&other_identity),
            Err(ProtocolError::IdentityMismatch)
        );
    }

    #[test]
    fn debug_output_shows_nothing() {
        assert_eq!(format!("{:?}", card(1, 7, true)), "ContactCard([redacted])");
        assert_eq!(
            format!("{:?}", InvitationCapability::from_bytes([1; 16])),
            "InvitationCapability([redacted])"
        );
    }
}

//! Contact cards and endpoint sets.
//!
//! A contact card is a signed statement by an identity: "as of this epoch, I
//! can be reached at this set of endpoints". The format is defined in
//! `docs/PROTOCOL.md` section 11. Version 1 allows one endpoint per card;
//! the format, the signed bytes and the epoch rule are those of a set.
//!
//! The bytes that are signed come from [`ContactCard::signed_bytes`], which
//! is separate from the transport encoder [`ContactCard::encode`], so that a
//! change to the transport layout cannot change what a signature covers.

use core::fmt;

use monolith_identity::redact::REDACTED;
use monolith_identity::{
    EndpointEpoch, IdentityPublicKey, IdentitySecretKey, OnionServiceKey, Signature, base32,
};

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
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct InvitationCapability([u8; INVITATION_CAPABILITY_LEN]);

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
/// member twice. The order is the order in the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointSet {
    first: OnionServiceKey,
    rest: Vec<OnionServiceKey>,
}

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
}

/// A verified contact card.
///
/// A value of this type has passed every check of `docs/PROTOCOL.md` section
/// 11.2, signature included. There is no unverified card type: decoding and
/// verifying are one step.
#[derive(Clone, PartialEq, Eq)]
pub struct ContactCard {
    identity: IdentityPublicKey,
    epoch: EndpointEpoch,
    endpoints: EndpointSet,
    invitation: Option<InvitationCapability>,
    signature: Signature,
}

impl ContactCard {
    /// Creates and signs a card for the identity that `secret` belongs to.
    pub fn sign(
        secret: &IdentitySecretKey,
        epoch: EndpointEpoch,
        endpoints: EndpointSet,
        invitation: Option<InvitationCapability>,
    ) -> Self {
        let identity = secret.public_key();
        let message = signed_bytes(&identity, epoch, &endpoints, invitation.as_ref());
        Self {
            identity,
            epoch,
            endpoints,
            invitation,
            signature: secret.sign(&message),
        }
    }

    /// Returns the identity that signed the card.
    pub const fn identity(&self) -> &IdentityPublicKey {
        &self.identity
    }

    /// Returns the epoch of the endpoint set.
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
        let mut endpoints: Vec<OnionServiceKey> = Vec::with_capacity(count);
        for raw in &endpoint_bytes {
            endpoints.push(OnionServiceKey::from_bytes(raw)?);
        }
        let endpoints = EndpointSet::new(&endpoints)?;

        let message = signed_bytes(&identity, epoch, &endpoints, invitation.as_ref());
        identity.verify(&message, &signature)?;

        Ok(Self {
            identity,
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
    /// ASCII space, tab, carriage return and line feed are removed wherever
    /// they occur. Letters are accepted in either case. Anything else that
    /// is not part of the format rejects the input.
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
    epoch: EndpointEpoch,
    endpoints: &EndpointSet,
    invitation: Option<&InvitationCapability>,
) {
    writer.u8(CARD_VERSION);
    writer.raw(identity.as_bytes());
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
    epoch: EndpointEpoch,
    endpoints: &EndpointSet,
    invitation: Option<&InvitationCapability>,
) -> Vec<u8> {
    let mut writer =
        Writer::with_capacity(SIGNING_PREFIX.len().saturating_add(MAX_CONTACT_CARD_LEN));
    writer.raw(SIGNING_PREFIX);
    writer.u8(CARD_VERSION);
    writer.raw(identity.as_bytes());
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

/// What a card from a contact means for the endpoint set pinned for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointUpdate {
    /// The epoch is greater than the pinned one. The card's set replaces the
    /// pinned set once the change is confirmed.
    Newer,
    /// Same epoch, same set. Nothing to do; this is the normal case.
    Unchanged,
    /// Same epoch, different set: the owner signed two statements with one
    /// epoch. Nothing changes; the user is told.
    Conflict,
    /// The epoch is lower than the pinned one. Nothing changes.
    Stale,
}

/// Compares a card received from a contact with what is pinned for that
/// contact. See `docs/PROTOCOL.md` section 8.8.
///
/// Fails with [`ProtocolError::IdentityMismatch`] if the card was signed by
/// another identity than the pinned one. Nothing but [`EndpointUpdate::Newer`]
/// ever leads to a change of the pinned set.
pub fn evaluate_endpoint_update(
    pinned_identity: &IdentityPublicKey,
    pinned_epoch: EndpointEpoch,
    pinned_endpoints: &EndpointSet,
    card: &ContactCard,
) -> Result<EndpointUpdate, ProtocolError> {
    if card.identity() != pinned_identity {
        return Err(ProtocolError::IdentityMismatch);
    }
    if pinned_epoch.is_superseded_by(card.epoch()) {
        return Ok(EndpointUpdate::Newer);
    }
    if card.epoch() == pinned_epoch {
        if card.endpoints() == pinned_endpoints {
            return Ok(EndpointUpdate::Unchanged);
        }
        return Ok(EndpointUpdate::Conflict);
    }
    Ok(EndpointUpdate::Stale)
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

    fn card(seed: u8, epoch_value: u64, invitation: bool) -> ContactCard {
        ContactCard::sign(
            &secret(seed),
            epoch(epoch_value),
            EndpointSet::single(endpoint(seed.wrapping_add(100))),
            invitation.then(|| InvitationCapability::from_bytes([0xC4; 16])),
        )
    }

    #[test]
    fn sizes_match_the_specification() {
        assert_eq!(card(1, 1, false).encode().len(), 139);
        assert_eq!(card(1, 1, true).encode().len(), 155);
        assert_eq!(card(1, 1, false).encode().len(), CONTACT_CARD_BASE_LEN);
        assert_eq!(card(1, 1, true).encode().len(), MAX_CONTACT_CARD_LEN);
        assert_eq!(card(1, 1, false).signed_bytes().len(), 99);
        assert_eq!(card(1, 1, true).signed_bytes().len(), 115);
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
        assert_eq!(&bytes[33..41], &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(bytes[41], 1);
        assert_eq!(&bytes[42..74], original.endpoints().first().as_bytes());
        assert_eq!(bytes[74], 0x01);
        assert_eq!(&bytes[75..91], &[0xC4; 16]);
        assert_eq!(&bytes[91..155], original.signature().as_bytes());
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
        let without = card(1, 7, false).encode();
        let mut flag_set = without.clone();
        flag_set[74] = 0x01;
        assert!(ContactCard::decode(&flag_set).is_err());

        let with = card(1, 7, true).encode();
        let mut flag_clear = with.clone();
        flag_clear[74] = 0x00;
        assert!(ContactCard::decode(&flag_clear).is_err());
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
        bytes[33..41].copy_from_slice(&[0; 8]);
        assert_eq!(
            ContactCard::decode(&bytes),
            Err(ProtocolError::InvalidValue)
        );
    }

    #[test]
    fn endpoint_count_must_be_exactly_one_in_version_1() {
        for count in [0_u8, 2, 3, 255] {
            let mut bytes = card(1, 7, false).encode();
            bytes[41] = count;
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
            bytes[74] = flags;
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
        forged[75..].copy_from_slice(&other[75..]);
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
        assert_eq!(card(1, 7, false).to_text().len(), 233);
        assert_eq!(card(1, 7, true).to_text().len(), 258);
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
        // 139 bytes are 1112 bits and 223 symbols, so the last symbol has
        // three unused bits. Setting one of them must be rejected.
        let text = card(1, 7, false).to_text();
        let last = text.chars().last().unwrap();
        let index = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567".find(last).unwrap();
        assert_eq!(index % 8, 0, "canonical form has zero trailing bits");
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
        // Identity seed: RFC 8032 test vector 1. Endpoint: the public key of
        // the all-0x02 seed. Epoch 1, no invitation.
        let seed: [u8; 32] = [
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
            0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
            0x1c, 0xae, 0x7f, 0x60,
        ];
        let known = ContactCard::sign(
            &IdentitySecretKey::from_seed(&seed),
            EndpointEpoch::FIRST,
            EndpointSet::single(endpoint(2)),
            None,
        );
        assert_eq!(known.to_text(), KNOWN_CARD_TEXT);
        assert_eq!(ContactCard::from_text(KNOWN_CARD_TEXT).unwrap(), known);
    }

    /// Reproduced with an independent Ed25519 implementation written from
    /// RFC 8032, following the field list of `docs/PROTOCOL.md` 11.1.1.
    const KNOWN_CARD_TEXT: &str = "MONOLITH1:AHLVVGABQKYQVN6VJP7NHSLEA45A5YLS6PNKMIZFV4BBU2HXA5IRUAAAAAAAAAAAAEAYCOLXB2UH2F27K2RVIZWDJR7MZS4NRKI3J3RXUJO7MD23R7E3HFAAPAOTVCXFGWOLXUJQKXMVEHXRLWB4ALO2N5O2YREPLVGZMPS7QATQKPJDFFX5FHCDFCZUYVWNCERTGPIKNJQ226UI4SPJN7T5UD2DEAA";

    #[test]
    fn endpoint_update_outcomes() {
        let pinned = card(1, 5, false);
        let pinned_set = pinned.endpoints().clone();
        let evaluate = |candidate: &ContactCard| {
            evaluate_endpoint_update(pinned.identity(), pinned.epoch(), &pinned_set, candidate)
        };

        assert_eq!(evaluate(&pinned), Ok(EndpointUpdate::Unchanged));

        let newer = ContactCard::sign(
            &secret(1),
            epoch(6),
            EndpointSet::single(endpoint(50)),
            None,
        );
        assert_eq!(evaluate(&newer), Ok(EndpointUpdate::Newer));

        let newer_same_set = ContactCard::sign(&secret(1), epoch(6), pinned_set.clone(), None);
        assert_eq!(evaluate(&newer_same_set), Ok(EndpointUpdate::Newer));

        let conflict = ContactCard::sign(
            &secret(1),
            epoch(5),
            EndpointSet::single(endpoint(50)),
            None,
        );
        assert_eq!(evaluate(&conflict), Ok(EndpointUpdate::Conflict));

        let stale = ContactCard::sign(
            &secret(1),
            epoch(4),
            EndpointSet::single(endpoint(50)),
            None,
        );
        assert_eq!(evaluate(&stale), Ok(EndpointUpdate::Stale));

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

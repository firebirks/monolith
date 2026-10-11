//! The payload of a vault, version 1: typed records, one set per local
//! identity (`docs/STORAGE.md` sections 1.1 and 3.4).
//!
//! The layout follows the wire format: big-endian integers, presence bytes
//! for optional fields, a 16-bit length before every field of variable
//! size, and one valid encoding for every value.
//!
//! ```text
//! payload    u16 version (1), u32 records length, records, zero padding
//! records    u16 identity count, identity*
//! identity   u8 tag (1), identity seed [32], transport secret [32],
//!            presence + onion secret [64], epoch u64, endpoint [32],
//!            presence + rotation, request mode u8, presence + label,
//!            u8 invitation count, invitation*, u16 contact count, contact*,
//!            u16 blocked count, identity key [32]*,
//!            u16 declined count, identity key [32]* (oldest first)
//! rotation   transport secret [32], epoch u64, switched u8
//! invitation capability [16], presence + label
//! contact    kind u8 (1 requested, 2 accepted), active card,
//!            presence + authorized card, presence + pending card,
//!            imported u8, presence + retired key [32],
//!            presence + capability [16], dial card, verified u8,
//!            presence + alias, announced u8, promoted u8
//! card       u16 length, the card as on the wire
//! label      u16 length, display-name text
//! ```
//!
//! Every record is built from validated typed values and read back through
//! the same validation: cards are decoded and their signatures verified,
//! credentials are rebuilt with `Credentials::restore`, texts with the
//! display-name rules, and every count is checked against its limit. A
//! reader that meets a version or a record tag it does not know refuses
//! the payload; it never skips. Nothing peer-supplied is ever read as
//! structure (S28).

use core::fmt;
use std::collections::HashSet;

use monolith_identity::redact::Redacted;
use monolith_identity::{EndpointEpoch, IdentityPublicKey, OnionServiceKey, TransportPublicKey};
use monolith_protocol::ProtocolError;
use monolith_protocol::card::{ContactCard, InvitationCapability};
use monolith_protocol::codec::{Reader, Writer};
use monolith_protocol::contact::{RecordKind, RequestMode};
use monolith_protocol::credential::Credentials;
use monolith_protocol::limits::{
    INVITATION_CAPABILITY_LEN, MAX_ACTIVE_INVITATIONS, MAX_BLOCKED_IDENTITIES,
    MAX_CONTACT_CARD_LEN, MAX_CONTACTS, MAX_DECLINED_IDENTITIES, MAX_DISPLAY_NAME_LEN,
    MAX_LOCAL_IDENTITIES,
};
use monolith_protocol::text::DisplayName;
use zeroize::Zeroizing;

use crate::StorageError;

/// The payload version this build writes and reads.
pub const PAYLOAD_VERSION: u16 = 1;

/// The tag of an identity record.
const IDENTITY_TAG: u8 = 1;

/// Length of the secret of an Onion Service key in Tor's format.
pub const ONION_SECRET_LEN: usize = 64;

/// Everything a vault holds.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Contents {
    /// The local identities, each with everything that belongs to it.
    pub identities: Vec<StoredIdentity>,
}

/// A local identity and everything that belongs to it.
///
/// `Debug` prints no secret: the identity seed, the transport secret and
/// the Onion Service secret are redacted (S18).
#[derive(PartialEq, Eq)]
pub struct StoredIdentity {
    /// The 32-byte seed of the identity key.
    pub identity_seed: Zeroizing<[u8; 32]>,
    /// The 32 bytes of the transport secret key that is active.
    pub transport_secret: Zeroizing<[u8; 32]>,
    /// The secret key of the Onion Service, in Tor's format, once Tor has
    /// returned it. Absent for an identity that has not been published.
    pub onion_secret: Option<Zeroizing<[u8; ONION_SECRET_LEN]>>,
    /// The epoch of the local card.
    pub epoch: EndpointEpoch,
    /// The endpoint of the local card.
    pub endpoint: OnionServiceKey,
    /// A change of transport key in progress.
    pub rotation: Option<StoredRotation>,
    /// Which requests from strangers are considered.
    pub request_mode: RequestMode,
    /// The local label of the identity. Never sent.
    pub label: Option<DisplayName>,
    /// The active set of invitation capabilities.
    pub invitations: Vec<StoredInvitation>,
    /// Requested and accepted contacts.
    pub contacts: Vec<StoredContact>,
    /// Blocked identities.
    pub blocked: Vec<IdentityPublicKey>,
    /// Declined identities, oldest first.
    pub declined: Vec<IdentityPublicKey>,
}

/// A change of the local transport key in progress (`docs/PROTOCOL.md`
/// section 11.4, the steps of changing the transport key). `Debug` prints
/// no secret.
#[derive(PartialEq, Eq)]
pub struct StoredRotation {
    /// The 32 bytes of the new transport secret key.
    pub transport_secret: Zeroizing<[u8; 32]>,
    /// The epoch of the successor card.
    pub epoch: EndpointEpoch,
    /// The identity answers with the new key.
    pub switched: bool,
}

impl fmt::Debug for StoredIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredIdentity")
            .field("identity_seed", &Redacted::new(()))
            .field("transport_secret", &Redacted::new(()))
            .field(
                "onion_secret",
                &self.onion_secret.as_ref().map(|_| Redacted::new(())),
            )
            .field("epoch", &self.epoch)
            .field("endpoint", &self.endpoint)
            .field("rotation", &self.rotation)
            .field("request_mode", &self.request_mode)
            .field("label", &self.label)
            .field("invitations", &self.invitations)
            .field("contacts", &self.contacts)
            .field("blocked", &self.blocked)
            .field("declined", &self.declined)
            .finish()
    }
}

impl fmt::Debug for StoredRotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StoredRotation")
            .field("transport_secret", &Redacted::new(()))
            .field("epoch", &self.epoch)
            .field("switched", &self.switched)
            .finish()
    }
}

/// One member of the active set of invitation capabilities.
#[derive(Debug, PartialEq, Eq)]
pub struct StoredInvitation {
    /// The capability.
    pub capability: InvitationCapability,
    /// Its local label. Never part of a card or sent.
    pub label: Option<DisplayName>,
}

/// A requested or accepted contact.
#[derive(Debug, PartialEq, Eq)]
pub struct StoredContact {
    /// [`RecordKind::Requested`] or [`RecordKind::Accepted`].
    pub kind: RecordKind,
    /// Which transport key stands for the contact.
    pub credentials: Credentials,
    /// The card the user confirmed for dialing.
    pub dial: ContactCard,
    /// The user verified the fingerprint out of band.
    pub verified: bool,
    /// The user's local name for the contact. Never sent.
    pub alias: Option<DisplayName>,
    /// The successor card of a local rotation was sent to this contact on
    /// a confirmed session of the old key.
    pub successor_announced: bool,
    /// This contact confirmed a session made with the new local key.
    pub successor_promoted: bool,
}

fn format(_: ProtocolError) -> StorageError {
    StorageError::BadFormat
}

fn put_label(writer: &mut Writer, label: Option<&DisplayName>) -> Result<(), StorageError> {
    writer.presence(label.is_some());
    if let Some(label) = label {
        writer
            .bytes(label.as_bytes(), MAX_DISPLAY_NAME_LEN)
            .map_err(|_| StorageError::TooLarge)?;
    }
    Ok(())
}

fn get_label(reader: &mut Reader<'_>) -> Result<Option<DisplayName>, StorageError> {
    if !reader.presence().map_err(format)? {
        return Ok(None);
    }
    let bytes = reader.bytes(0, MAX_DISPLAY_NAME_LEN).map_err(format)?;
    DisplayName::from_bytes(bytes).map(Some).map_err(format)
}

fn put_card(writer: &mut Writer, card: &ContactCard) -> Result<(), StorageError> {
    // A card held of a contact may carry its capability.
    writer
        .bytes(&Zeroizing::new(card.encode()), MAX_CONTACT_CARD_LEN)
        .map_err(|_| StorageError::TooLarge)
}

fn get_card(reader: &mut Reader<'_>) -> Result<ContactCard, StorageError> {
    let bytes = reader.bytes(1, MAX_CONTACT_CARD_LEN).map_err(format)?;
    ContactCard::decode(bytes).map_err(format)
}

fn put_optional_card(writer: &mut Writer, card: Option<&ContactCard>) -> Result<(), StorageError> {
    writer.presence(card.is_some());
    card.map_or(Ok(()), |card| put_card(writer, card))
}

fn get_optional_card(reader: &mut Reader<'_>) -> Result<Option<ContactCard>, StorageError> {
    if reader.presence().map_err(format)? {
        get_card(reader).map(Some)
    } else {
        Ok(None)
    }
}

fn get_flag(reader: &mut Reader<'_>) -> Result<bool, StorageError> {
    reader.presence().map_err(format)
}

fn count_u16(count: usize, max: usize) -> Result<u16, StorageError> {
    if count > max {
        return Err(StorageError::TooLarge);
    }
    u16::try_from(count).map_err(|_| StorageError::TooLarge)
}

fn get_count(value: usize, max: usize) -> Result<usize, StorageError> {
    if value > max {
        return Err(StorageError::BadFormat);
    }
    Ok(value)
}

fn get_epoch(reader: &mut Reader<'_>) -> Result<EndpointEpoch, StorageError> {
    EndpointEpoch::new(reader.u64().map_err(format)?).map_err(|_| StorageError::BadFormat)
}

/// Whether `bytes` can be a stored secret: not all zero. No random source
/// produces only zero bytes; a buffer that was never filled does. The
/// vault refuses such a secret when it is read, so whoever writes one
/// checks it with this first.
pub fn is_usable_secret(bytes: &[u8]) -> bool {
    bytes.iter().any(|byte| *byte != 0)
}

fn get_secret<const N: usize>(reader: &mut Reader<'_>) -> Result<Zeroizing<[u8; N]>, StorageError> {
    let bytes = Zeroizing::new(reader.array::<N>().map_err(format)?);
    if !is_usable_secret(bytes.as_slice()) {
        return Err(StorageError::BadFormat);
    }
    Ok(bytes)
}

fn request_mode_code(mode: RequestMode) -> u8 {
    match mode {
        RequestMode::Invitation => 0,
        RequestMode::Open => 1,
        RequestMode::Closed => 2,
    }
}

fn kind_code(kind: RecordKind) -> Result<u8, StorageError> {
    match kind {
        RecordKind::Requested => Ok(1),
        RecordKind::Accepted => Ok(2),
        RecordKind::None | RecordKind::Declined | RecordKind::Blocked => {
            Err(StorageError::Internal)
        }
    }
}

impl Contents {
    /// Encodes the payload, without padding.
    ///
    /// Fails with [`StorageError::TooLarge`] if a count is above its limit,
    /// and with [`StorageError::Internal`] for a contact whose kind is not
    /// requested or accepted.
    pub fn encode(&self) -> Result<Zeroizing<Vec<u8>>, StorageError> {
        let mut records = Writer::with_capacity(1024);
        records.u16(count_u16(self.identities.len(), MAX_LOCAL_IDENTITIES)?);
        for identity in &self.identities {
            identity.encode(&mut records)?;
        }
        let records = Zeroizing::new(records.into_bytes());
        let mut payload = Writer::with_capacity(records.len().saturating_add(6));
        payload.u16(PAYLOAD_VERSION);
        payload.raw(
            &u32::try_from(records.len())
                .map_err(|_| StorageError::TooLarge)?
                .to_be_bytes(),
        );
        payload.raw(&records);
        Ok(Zeroizing::new(payload.into_bytes()))
    }

    /// Decodes a payload. Padding after the records must be zero.
    ///
    /// Fails with [`StorageError::UnsupportedVersion`] for a payload
    /// version this build does not know and with [`StorageError::BadFormat`]
    /// for anything that does not validate.
    pub fn decode(payload: &[u8]) -> Result<Self, StorageError> {
        let mut reader = Reader::new(payload);
        let version = reader.u16().map_err(format)?;
        if version != PAYLOAD_VERSION {
            return Err(StorageError::UnsupportedVersion);
        }
        let len = u32::from_be_bytes(reader.array().map_err(format)?);
        let len = usize::try_from(len).map_err(|_| StorageError::BadFormat)?;
        let records = reader.take(len).map_err(format)?;
        if reader.peek().iter().any(|byte| *byte != 0) {
            return Err(StorageError::BadFormat);
        }
        let mut reader = Reader::new(records);
        let count = get_count(
            usize::from(reader.u16().map_err(format)?),
            MAX_LOCAL_IDENTITIES,
        )?;
        let mut identities = Vec::with_capacity(count);
        for _ in 0..count {
            identities.push(StoredIdentity::decode(&mut reader)?);
        }
        reader.finish().map_err(format)?;
        Ok(Self { identities })
    }
}

impl StoredIdentity {
    fn encode(&self, writer: &mut Writer) -> Result<(), StorageError> {
        writer.u8(IDENTITY_TAG);
        writer.raw(self.identity_seed.as_slice());
        writer.raw(self.transport_secret.as_slice());
        writer.presence(self.onion_secret.is_some());
        if let Some(onion) = &self.onion_secret {
            writer.raw(onion.as_slice());
        }
        writer.u64(self.epoch.get());
        writer.raw(self.endpoint.as_bytes());
        writer.presence(self.rotation.is_some());
        if let Some(rotation) = &self.rotation {
            writer.raw(rotation.transport_secret.as_slice());
            writer.u64(rotation.epoch.get());
            writer.presence(rotation.switched);
        }
        writer.u8(request_mode_code(self.request_mode));
        put_label(writer, self.label.as_ref())?;
        writer.u8(u8::try_from(self.invitations.len())
            .ok()
            .filter(|count| usize::from(*count) <= MAX_ACTIVE_INVITATIONS)
            .ok_or(StorageError::TooLarge)?);
        for invitation in &self.invitations {
            writer.raw(invitation.capability.expose());
            put_label(writer, invitation.label.as_ref())?;
        }
        writer.u16(count_u16(self.contacts.len(), MAX_CONTACTS)?);
        for contact in &self.contacts {
            contact.encode(writer)?;
        }
        writer.u16(count_u16(self.blocked.len(), MAX_BLOCKED_IDENTITIES)?);
        for identity in &self.blocked {
            writer.raw(identity.as_bytes());
        }
        writer.u16(count_u16(self.declined.len(), MAX_DECLINED_IDENTITIES)?);
        for identity in &self.declined {
            writer.raw(identity.as_bytes());
        }
        Ok(())
    }

    fn decode(reader: &mut Reader<'_>) -> Result<Self, StorageError> {
        if reader.u8().map_err(format)? != IDENTITY_TAG {
            return Err(StorageError::BadFormat);
        }
        let identity_seed = get_secret::<32>(reader)?;
        let transport_secret = get_secret::<32>(reader)?;
        let onion_secret = if get_flag(reader)? {
            Some(get_secret::<ONION_SECRET_LEN>(reader)?)
        } else {
            None
        };
        let epoch = get_epoch(reader)?;
        let endpoint = OnionServiceKey::from_bytes(&reader.array().map_err(format)?)
            .map_err(|_| StorageError::BadFormat)?;
        let rotation = if get_flag(reader)? {
            Some(StoredRotation {
                transport_secret: get_secret::<32>(reader)?,
                epoch: get_epoch(reader)?,
                switched: get_flag(reader)?,
            })
        } else {
            None
        };
        if rotation.as_ref().is_some_and(|rotation| {
            !epoch.is_superseded_by(rotation.epoch) || rotation.transport_secret == transport_secret
        }) {
            return Err(StorageError::BadFormat);
        }
        let request_mode = match reader.u8().map_err(format)? {
            0 => RequestMode::Invitation,
            1 => RequestMode::Open,
            2 => RequestMode::Closed,
            _ => return Err(StorageError::BadFormat),
        };
        let label = get_label(reader)?;
        let count = get_count(
            usize::from(reader.u8().map_err(format)?),
            MAX_ACTIVE_INVITATIONS,
        )?;
        let mut invitations: Vec<StoredInvitation> = Vec::with_capacity(count);
        for _ in 0..count {
            let capability = InvitationCapability::from_bytes(
                reader
                    .array::<INVITATION_CAPABILITY_LEN>()
                    .map_err(format)?,
            );
            if invitations
                .iter()
                .any(|held| held.capability.expose() == capability.expose())
            {
                return Err(StorageError::BadFormat);
            }
            invitations.push(StoredInvitation {
                capability,
                label: get_label(reader)?,
            });
        }
        let mut seen = HashSet::new();
        let count = get_count(usize::from(reader.u16().map_err(format)?), MAX_CONTACTS)?;
        let mut contacts = Vec::with_capacity(count);
        for _ in 0..count {
            let contact = StoredContact::decode(reader)?;
            if !seen.insert(*contact.credentials.identity()) {
                return Err(StorageError::BadFormat);
            }
            contacts.push(contact);
        }
        let mut lists = [Vec::new(), Vec::new()];
        for (list, max) in lists
            .iter_mut()
            .zip([MAX_BLOCKED_IDENTITIES, MAX_DECLINED_IDENTITIES])
        {
            let count = get_count(usize::from(reader.u16().map_err(format)?), max)?;
            list.reserve(count);
            for _ in 0..count {
                let identity = IdentityPublicKey::from_bytes(&reader.array().map_err(format)?)
                    .map_err(|_| StorageError::BadFormat)?;
                // An identity is a contact, blocked or declined, never two
                // of them.
                if !seen.insert(identity) {
                    return Err(StorageError::BadFormat);
                }
                list.push(identity);
            }
        }
        let [blocked, declined] = lists;
        Ok(Self {
            identity_seed,
            transport_secret,
            onion_secret,
            epoch,
            endpoint,
            rotation,
            request_mode,
            label,
            invitations,
            contacts,
            blocked,
            declined,
        })
    }
}

impl StoredContact {
    fn encode(&self, writer: &mut Writer) -> Result<(), StorageError> {
        let credentials = &self.credentials;
        writer.u8(kind_code(self.kind)?);
        put_card(writer, credentials.active())?;
        put_optional_card(writer, credentials.authorized_successor())?;
        put_optional_card(writer, credentials.pending_successor())?;
        writer.presence(credentials.pending_was_imported());
        writer.presence(credentials.retired().is_some());
        if let Some(retired) = credentials.retired() {
            writer.raw(retired.as_bytes());
        }
        writer.presence(credentials.invitation().is_some());
        if let Some(invitation) = credentials.invitation() {
            writer.raw(invitation.expose());
        }
        put_card(writer, &self.dial)?;
        writer.presence(self.verified);
        put_label(writer, self.alias.as_ref())?;
        writer.presence(self.successor_announced);
        writer.presence(self.successor_promoted);
        Ok(())
    }

    fn decode(reader: &mut Reader<'_>) -> Result<Self, StorageError> {
        let kind = match reader.u8().map_err(format)? {
            1 => RecordKind::Requested,
            2 => RecordKind::Accepted,
            _ => return Err(StorageError::BadFormat),
        };
        let active = get_card(reader)?;
        let authorized = get_optional_card(reader)?;
        let pending = get_optional_card(reader)?;
        let imported = get_flag(reader)?;
        let retired = if get_flag(reader)? {
            Some(
                TransportPublicKey::from_bytes(&reader.array().map_err(format)?)
                    .map_err(|_| StorageError::BadFormat)?,
            )
        } else {
            None
        };
        let invitation = if get_flag(reader)? {
            Some(InvitationCapability::from_bytes(
                reader.array().map_err(format)?,
            ))
        } else {
            None
        };
        let credentials =
            Credentials::restore(active, authorized, pending, imported, retired, invitation)
                .map_err(format)?;
        let dial = get_card(reader)?;
        // The dial card is a card of the contact; which key it states is
        // checked when it is used.
        if dial.identity() != credentials.identity() {
            return Err(StorageError::BadFormat);
        }
        Ok(Self {
            kind,
            credentials,
            dial,
            verified: get_flag(reader)?,
            alias: get_label(reader)?,
            successor_announced: get_flag(reader)?,
            successor_promoted: get_flag(reader)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use monolith_identity::IdentitySecretKey;
    use monolith_protocol::card::EndpointSet;

    fn secret(seed: u8) -> IdentitySecretKey {
        IdentitySecretKey::from_seed(&[seed; 32])
    }

    fn onion(seed: u8) -> OnionServiceKey {
        OnionServiceKey::from_bytes(secret(seed.wrapping_add(100)).public_key().as_bytes()).unwrap()
    }

    fn transport(seed: u8) -> TransportPublicKey {
        let mut bytes = [seed; 32];
        bytes[31] = 0x40;
        TransportPublicKey::from_bytes(&bytes).unwrap()
    }

    fn card(identity: u8, key: u8, epoch: u64, capability: Option<u8>) -> ContactCard {
        ContactCard::sign(
            &secret(identity),
            transport(key),
            EndpointEpoch::new(epoch).unwrap(),
            EndpointSet::single(onion(identity)),
            capability.map(|byte| InvitationCapability::from_bytes([byte; 16])),
        )
        .unwrap()
    }

    fn contact(identity: u8) -> StoredContact {
        let mut credentials = Credentials::new(card(identity, identity, 2, Some(7)));
        credentials
            .announce(
                &card(identity, 0x61, 3, None),
                &card(identity, identity, 2, None),
            )
            .unwrap();
        credentials.admit(&card(identity, 0x62, 5, None)).unwrap();
        StoredContact {
            kind: RecordKind::Accepted,
            credentials,
            dial: card(identity, identity, 1, None),
            verified: true,
            alias: Some(DisplayName::new("Bob at work").unwrap()),
            successor_announced: true,
            successor_promoted: false,
        }
    }

    fn identity(seed: u8) -> StoredIdentity {
        StoredIdentity {
            identity_seed: Zeroizing::new([seed; 32]),
            transport_secret: Zeroizing::new([seed ^ 0xA5; 32]),
            onion_secret: Some(Zeroizing::new([seed; ONION_SECRET_LEN])),
            epoch: EndpointEpoch::new(4).unwrap(),
            endpoint: onion(seed),
            rotation: Some(StoredRotation {
                transport_secret: Zeroizing::new([seed ^ 0x5A; 32]),
                epoch: EndpointEpoch::new(5).unwrap(),
                switched: false,
            }),
            request_mode: RequestMode::Open,
            label: Some(DisplayName::new("Personal").unwrap()),
            invitations: vec![
                StoredInvitation {
                    capability: InvitationCapability::from_bytes([1; 16]),
                    label: Some(DisplayName::new("Website").unwrap()),
                },
                StoredInvitation {
                    capability: InvitationCapability::from_bytes([2; 16]),
                    label: None,
                },
            ],
            contacts: vec![contact(0x30), contact(0x31)],
            blocked: vec![secret(0x40).public_key()],
            declined: vec![secret(0x41).public_key(), secret(0x42).public_key()],
        }
    }

    #[test]
    fn contents_survive_a_round_trip() {
        let contents = Contents {
            identities: vec![identity(1), identity(2)],
        };
        let bytes = contents.encode().unwrap();
        assert_eq!(Contents::decode(&bytes).unwrap(), contents);
        // Zero padding is accepted, as the vault adds it.
        let padded = [bytes.as_slice(), &[0; 100]].concat();
        assert_eq!(Contents::decode(&padded).unwrap(), contents);
        // An empty vault is valid.
        let empty = Contents::default().encode().unwrap();
        assert_eq!(Contents::decode(&empty).unwrap(), Contents::default());
    }

    #[test]
    fn anything_unknown_or_left_over_is_refused() {
        let bytes = Contents {
            identities: vec![identity(1)],
        }
        .encode()
        .unwrap();
        // Another payload version.
        let mut changed = bytes.to_vec();
        changed[1] = 2;
        assert_eq!(
            Contents::decode(&changed),
            Err(StorageError::UnsupportedVersion)
        );
        // Padding that is not zero.
        let padded = [bytes.as_slice(), &[0, 1]].concat();
        assert_eq!(Contents::decode(&padded), Err(StorageError::BadFormat));
        // An unknown record tag.
        let mut changed = bytes.to_vec();
        changed[8] = 2;
        assert_eq!(Contents::decode(&changed), Err(StorageError::BadFormat));
        // A truncated payload.
        assert_eq!(
            Contents::decode(&bytes[..bytes.len() - 1]),
            Err(StorageError::BadFormat)
        );
        // Every truncation fails, and none panics.
        for len in 0..bytes.len() {
            assert!(Contents::decode(&bytes[..len]).is_err(), "{len}");
        }
    }

    #[test]
    fn limits_are_checked_both_ways() {
        let mut too_many = Contents::default();
        for seed in 0..=u8::try_from(MAX_LOCAL_IDENTITIES).unwrap() {
            too_many.identities.push(StoredIdentity {
                contacts: Vec::new(),
                ..identity(seed.wrapping_add(1))
            });
        }
        assert_eq!(too_many.encode().err(), Some(StorageError::TooLarge));
        let mut invitations = identity(1);
        invitations.invitations = (0..=u8::try_from(MAX_ACTIVE_INVITATIONS).unwrap())
            .map(|byte| StoredInvitation {
                capability: InvitationCapability::from_bytes([byte; 16]),
                label: None,
            })
            .collect();
        assert_eq!(
            Contents {
                identities: vec![invitations]
            }
            .encode()
            .err(),
            Some(StorageError::TooLarge)
        );
        // A count above the limit in a stored payload is refused before
        // anything is allocated for it.
        let mut bytes = Contents::default().encode().unwrap().to_vec();
        let max = u16::try_from(MAX_LOCAL_IDENTITIES + 1)
            .unwrap()
            .to_be_bytes();
        bytes[6] = max[0];
        bytes[7] = max[1];
        assert_eq!(Contents::decode(&bytes), Err(StorageError::BadFormat));
    }

    #[test]
    fn duplicates_and_inconsistent_records_are_refused() {
        let encode = |identity: StoredIdentity| {
            Contents {
                identities: vec![identity],
            }
            .encode()
            .unwrap()
        };
        // The same contact twice, a contact that is also blocked, a
        // blocked identity that is also declined, a capability twice.
        let mut twice = identity(1);
        twice.contacts.push(contact(0x30));
        let mut blocked_contact = identity(1);
        blocked_contact
            .blocked
            .push(*contact(0x30).credentials.identity());
        let mut blocked_declined = identity(1);
        blocked_declined.declined.push(secret(0x40).public_key());
        let mut same_capability = identity(1);
        same_capability.invitations.push(StoredInvitation {
            capability: InvitationCapability::from_bytes([1; 16]),
            label: None,
        });
        // A rotation that does not move the epoch forward, or reuses the key.
        let mut stale_rotation = identity(1);
        stale_rotation.rotation.as_mut().unwrap().epoch = EndpointEpoch::new(4).unwrap();
        let mut same_key = identity(1);
        same_key.rotation.as_mut().unwrap().transport_secret = Zeroizing::new([1 ^ 0xA5; 32]);
        // A dial card of another identity.
        let mut other_dial = identity(1);
        other_dial.contacts[0].dial = card(0x31, 0x31, 1, None);
        // Secrets that are all zero.
        let mut zero = identity(1);
        zero.transport_secret = Zeroizing::new([0; 32]);
        for broken in [
            twice,
            blocked_contact,
            blocked_declined,
            same_capability,
            stale_rotation,
            same_key,
            other_dial,
            zero,
        ] {
            assert_eq!(
                Contents::decode(&encode(broken)),
                Err(StorageError::BadFormat)
            );
        }
    }

    #[test]
    fn stored_credentials_keep_their_invariants() {
        // A retired key that is also the active one cannot be stored and
        // read back: restore refuses it.
        let bytes = Contents {
            identities: vec![identity(1)],
        }
        .encode()
        .unwrap();
        let decoded = Contents::decode(&bytes).unwrap();
        let held = &decoded.identities[0].contacts[0].credentials;
        assert_eq!(held.active(), &card(0x30, 0x30, 2, Some(7)));
        assert_eq!(
            held.authorized_successor(),
            Some(&card(0x30, 0x61, 3, None))
        );
        assert_eq!(held.pending_successor(), Some(&card(0x30, 0x62, 5, None)));
        assert_eq!(
            held.invitation(),
            Some(&InvitationCapability::from_bytes([7; 16]))
        );
    }

    #[test]
    fn debug_output_shows_no_secret() {
        // Secrets of bytes that appear nowhere else, so that any of them
        // in the output, in a list as `Debug` prints an array, is a leak.
        let mut stored = identity(1);
        stored.identity_seed = Zeroizing::new([0xB1; 32]);
        stored.transport_secret = Zeroizing::new([0xB2; 32]);
        stored.onion_secret = Some(Zeroizing::new([0xB3; ONION_SECRET_LEN]));
        stored.rotation.as_mut().unwrap().transport_secret = Zeroizing::new([0xB4; 32]);
        let rotation = stored.rotation.as_ref().unwrap();
        let mut texts = vec![
            format!("{rotation:?}"),
            format!("{rotation:#?}"),
            format!("{:?}", stored.rotation),
            format!("{stored:?}"),
            format!("{stored:#?}"),
            format!("{:?}", vec![&stored]),
        ];
        let contents = Contents {
            identities: vec![identity(2), stored],
        };
        texts.push(format!("{contents:?}"));
        texts.push(format!("{contents:#?}"));
        for text in &texts {
            for byte in [0xB1_u8, 0xB2, 0xB3, 0xB4] {
                for after in [",", "]", "\n"] {
                    assert!(!text.contains(&format!("{byte}{after}")), "{text}");
                }
            }
        }
    }
}

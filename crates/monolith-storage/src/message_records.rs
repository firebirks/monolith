//! The payload of the message store, version 1
//! (`docs/adr/0008-message-store.md`).
//!
//! ```text
//! payload       u16 version (1), u32 records length, records, zero padding
//! records       u8 identity count, identity*
//! identity      identity id [16], u16 conversation count, conversation*
//! conversation  contact id [16], peer identity key [32], u64 next sequence,
//!               u16 forgotten count, message id [16]*,
//!               u32 entry count, entry*
//! entry         u64 sequence, u8 kind, message id [16], u64 time,
//!               u16 length, text
//! kind          1 outbound queued, 2 outbound delivered, 3 inbound
//! ```
//!
//! Reading checks everything before it is used, and every count before
//! anything is allocated for it: the version, the zero padding, the limits
//! of `docs/RESOURCE_LIMITS.md` section 5, identities in increasing order
//! of identifier and the conversations of each in increasing order of
//! contact identifier, identifiers that are unique where they have to be,
//! entries in increasing sequence below the next sequence, known kinds,
//! and every text against the rules of `ChatText`.
//! A payload that fails any check is refused as a whole; nothing is
//! repaired. What was read encodes to the same bytes, so a payload has one
//! valid encoding.

use std::collections::HashSet;

use monolith_identity::IdentityPublicKey;
use monolith_protocol::ProtocolError;
use monolith_protocol::body::MessageId;
use monolith_protocol::codec::{Reader, Writer};
use monolith_protocol::limits::{
    MAX_CHAT_TEXT_LEN, MAX_CONVERSATION_STORE_LEN, MAX_CONVERSATIONS_PER_IDENTITY,
    MAX_LOCAL_IDENTITIES, MAX_QUEUED_MESSAGE_BYTES, MAX_QUEUED_MESSAGES_PER_CONTACT,
    MAX_QUEUED_MESSAGES_PER_IDENTITY, MAX_STORED_MESSAGES, MESSAGE_DEDUP_WINDOW, MESSAGE_ID_LEN,
};
use monolith_protocol::text::ChatText;
use zeroize::Zeroizing;

use crate::StorageError;
use crate::record::{ContactId, IdentityId, RECORD_ID_LEN};

/// The payload version this build writes and reads.
pub const MESSAGE_PAYLOAD_VERSION: u16 = 1;

/// Bytes an entry takes in the payload besides its text.
pub const ENTRY_OVERHEAD_LEN: usize = 8 + 1 + MESSAGE_ID_LEN + 8 + 2;

/// Bytes a conversation takes in the payload besides its forgotten
/// identifiers and its entries.
pub const CONVERSATION_OVERHEAD_LEN: usize = RECORD_ID_LEN + 32 + 8 + 2 + 4;

/// Bytes an identity takes in the payload besides its conversations.
pub const IDENTITY_OVERHEAD_LEN: usize = RECORD_ID_LEN + 2;

/// The most of each count that payload version 1 of the message store
/// holds (`docs/DESIGN_QUESTIONS.md` P7-5): the limits of
/// `RESOURCE_LIMITS.md` section 5 that the format was made with. A store is
/// read and written up to these, whatever the limits are in the build that
/// reads it; the limits bound only what is added to it. A later build may
/// lower a limit: a store above it is then still read, and nothing more is
/// added past it. A limit above its ceiling needs a new payload version.
pub mod ceiling {
    /// Local identities with a section in the store.
    pub const LOCAL_IDENTITIES: usize = 8;
    /// Conversations of one local identity.
    pub const CONVERSATIONS_PER_IDENTITY: usize = 2048;
    /// Queued messages of one local identity.
    pub const QUEUED_PER_IDENTITY: usize = 1024;
    /// Queued messages of one conversation.
    pub const QUEUED_PER_CONTACT: usize = 256;
    /// Bytes of queued text, all identities together.
    pub const QUEUED_BYTES: usize = 8 * 1024 * 1024;
    /// Identifiers of deleted received messages kept for one conversation.
    pub const FORGOTTEN_PER_CONVERSATION: usize = 1024;
    /// Bytes the entries of one conversation take.
    pub const CONVERSATION_LEN: usize = 4 * 1024 * 1024;
    /// Messages, all conversations together.
    pub const MESSAGES: usize = 262_144;
}

// A limit may be lowered by a later build, never raised above what the
// format holds.
const _: () = assert!(
    MAX_LOCAL_IDENTITIES <= ceiling::LOCAL_IDENTITIES
        && MAX_CONVERSATIONS_PER_IDENTITY <= ceiling::CONVERSATIONS_PER_IDENTITY
        && MAX_QUEUED_MESSAGES_PER_IDENTITY <= ceiling::QUEUED_PER_IDENTITY
        && MAX_QUEUED_MESSAGES_PER_CONTACT <= ceiling::QUEUED_PER_CONTACT
        && MAX_QUEUED_MESSAGE_BYTES <= ceiling::QUEUED_BYTES
        && MESSAGE_DEDUP_WINDOW <= ceiling::FORGOTTEN_PER_CONVERSATION
        && MAX_CONVERSATION_STORE_LEN <= ceiling::CONVERSATION_LEN
        && MAX_STORED_MESSAGES <= ceiling::MESSAGES
);

/// Bytes the payload takes besides its identities: the version, the
/// records length and the identity count.
pub const PAYLOAD_OVERHEAD_LEN: usize = 2 + 4 + 1;

/// Bytes one entry with a text of `text_len` bytes takes in the payload.
pub const fn entry_len(text_len: usize) -> usize {
    ENTRY_OVERHEAD_LEN.saturating_add(text_len)
}

/// What an entry of a conversation is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntryKind {
    /// Sent by the local side, not acknowledged: to be delivered.
    Queued,
    /// Sent by the local side and acknowledged by the contact.
    Delivered,
    /// Received from the contact and acknowledged to it.
    Received,
}

impl EntryKind {
    const fn code(self) -> u8 {
        match self {
            Self::Queued => 1,
            Self::Delivered => 2,
            Self::Received => 3,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Queued),
            2 => Some(Self::Delivered),
            3 => Some(Self::Received),
            _ => None,
        }
    }

    /// Returns true for a message the local side sent.
    pub const fn is_outbound(self) -> bool {
        matches!(self, Self::Queued | Self::Delivered)
    }
}

/// One message of a conversation. `Debug` prints neither the text nor the
/// identifier.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredEntry {
    /// Its place in the conversation, in the order entries were stored.
    pub sequence: u64,
    /// What it is.
    pub kind: EntryKind,
    /// The identifier on the wire: chosen by the local side for an
    /// outbound message, by the contact for an inbound one.
    pub id: MessageId,
    /// The local time it was queued or received, in seconds since the Unix
    /// epoch. For display only; nothing is ordered by it.
    pub time: u64,
    /// The text.
    pub text: ChatText,
}

/// The messages of one contact record of one local identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredConversation {
    /// The contact record the conversation belongs to.
    pub contact: ContactId,
    /// The remote identity of that record, so that the conversation can be
    /// shown after the record is gone.
    pub peer: IdentityPublicKey,
    /// The sequence the next entry gets. Above every stored one.
    pub next_sequence: u64,
    /// Identifiers of received messages the user deleted, oldest first, so
    /// that a late retry of one is acknowledged and not shown again.
    pub forgotten: Vec<MessageId>,
    /// The entries, in increasing sequence.
    pub entries: Vec<StoredEntry>,
}

/// The conversations of one local identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredMessages {
    /// The local identity.
    pub identity: IdentityId,
    /// Its conversations.
    pub conversations: Vec<StoredConversation>,
}

/// Everything a message store holds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MessageContents {
    /// The conversations of each local identity that has any.
    pub identities: Vec<StoredMessages>,
}

fn format(_: ProtocolError) -> StorageError {
    StorageError::BadFormat
}

fn too_large<T>(_: T) -> StorageError {
    StorageError::TooLarge
}

/// The number of items a payload of `remaining` bytes can hold at most,
/// each at least `size` bytes, below `count`: what may be allocated for a
/// count read from it.
fn capacity(count: usize, remaining: usize, size: usize) -> usize {
    count.min(remaining.checked_div(size).unwrap_or(0))
}

impl MessageContents {
    /// Encodes the payload, without padding.
    ///
    /// Fails with [`StorageError::TooLarge`] if a count is above its limit.
    pub fn encode(&self) -> Result<Zeroizing<Vec<u8>>, StorageError> {
        let mut records = Writer::with_capacity(1024);
        records.u8(u8::try_from(self.identities.len())
            .ok()
            .filter(|count| usize::from(*count) <= ceiling::LOCAL_IDENTITIES)
            .ok_or(StorageError::TooLarge)?);
        for identity in &self.identities {
            records.raw(identity.identity.as_bytes());
            if identity.conversations.len() > ceiling::CONVERSATIONS_PER_IDENTITY {
                return Err(StorageError::TooLarge);
            }
            records.u16(u16::try_from(identity.conversations.len()).map_err(too_large)?);
            for conversation in &identity.conversations {
                conversation.encode(&mut records)?;
            }
        }
        let records = Zeroizing::new(records.into_bytes());
        let mut payload = Writer::with_capacity(records.len().saturating_add(6));
        payload.u16(MESSAGE_PAYLOAD_VERSION);
        payload.raw(
            &u32::try_from(records.len())
                .map_err(too_large)?
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
        if reader.u16().map_err(format)? != MESSAGE_PAYLOAD_VERSION {
            return Err(StorageError::UnsupportedVersion);
        }
        let len = u32::from_be_bytes(reader.array().map_err(format)?);
        let len = usize::try_from(len).map_err(|_| StorageError::BadFormat)?;
        let records = reader.take(len).map_err(format)?;
        if reader.peek().iter().any(|byte| *byte != 0) {
            return Err(StorageError::BadFormat);
        }
        let mut reader = Reader::new(records);
        let count = usize::from(reader.u8().map_err(format)?);
        if count > ceiling::LOCAL_IDENTITIES {
            return Err(StorageError::BadFormat);
        }
        let mut totals = Totals::default();
        let mut identities: Vec<StoredMessages> = Vec::with_capacity(count);
        let mut contacts = HashSet::new();
        for _ in 0..count {
            let identity = IdentityId::from_bytes(reader.array().map_err(format)?)?;
            // In increasing order, so that contents have one encoding.
            if identities
                .last()
                .is_some_and(|held| held.identity >= identity)
            {
                return Err(StorageError::BadFormat);
            }
            let conversations = usize::from(reader.u16().map_err(format)?);
            if conversations > ceiling::CONVERSATIONS_PER_IDENTITY {
                return Err(StorageError::BadFormat);
            }
            let mut held = Vec::with_capacity(capacity(
                conversations,
                reader.peek().len(),
                CONVERSATION_OVERHEAD_LEN,
            ));
            let mut queued = 0_usize;
            for _ in 0..conversations {
                let conversation = StoredConversation::decode(&mut reader, &mut totals)?;
                let ordered = held
                    .last()
                    .is_none_or(|last: &StoredConversation| last.contact < conversation.contact);
                if !ordered || !contacts.insert(conversation.contact) {
                    return Err(StorageError::BadFormat);
                }
                queued = queued.saturating_add(conversation.queued());
                held.push(conversation);
            }
            if queued > ceiling::QUEUED_PER_IDENTITY {
                return Err(StorageError::BadFormat);
            }
            identities.push(StoredMessages {
                identity,
                conversations: held,
            });
        }
        reader.finish().map_err(format)?;
        Ok(Self { identities })
    }
}

/// What the whole store holds, counted while it is read.
#[derive(Default)]
struct Totals {
    entries: usize,
    queued_bytes: usize,
}

impl StoredConversation {
    /// The number of queued entries.
    pub fn queued(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.kind == EntryKind::Queued)
            .count()
    }

    /// The bytes the entries take in the payload, which
    /// `ceiling::CONVERSATION_LEN` bounds.
    pub fn entries_len(&self) -> usize {
        self.entries
            .iter()
            .map(|entry| entry_len(entry.text.as_bytes().len()))
            .fold(0, usize::saturating_add)
    }

    fn encode(&self, writer: &mut Writer) -> Result<(), StorageError> {
        writer.raw(self.contact.as_bytes());
        writer.raw(self.peer.as_bytes());
        writer.u64(self.next_sequence);
        if self.forgotten.len() > ceiling::FORGOTTEN_PER_CONVERSATION {
            return Err(StorageError::TooLarge);
        }
        writer.u16(u16::try_from(self.forgotten.len()).map_err(too_large)?);
        for id in &self.forgotten {
            writer.raw(id.as_bytes());
        }
        if self.entries.len() > ceiling::MESSAGES {
            return Err(StorageError::TooLarge);
        }
        writer.raw(
            &u32::try_from(self.entries.len())
                .map_err(too_large)?
                .to_be_bytes(),
        );
        for entry in &self.entries {
            writer.u64(entry.sequence);
            writer.u8(entry.kind.code());
            writer.raw(entry.id.as_bytes());
            writer.u64(entry.time);
            writer
                .bytes(entry.text.as_bytes(), MAX_CHAT_TEXT_LEN)
                .map_err(too_large)?;
        }
        Ok(())
    }

    fn decode(reader: &mut Reader<'_>, totals: &mut Totals) -> Result<Self, StorageError> {
        let contact = ContactId::from_bytes(reader.array().map_err(format)?)?;
        let peer = IdentityPublicKey::from_bytes(&reader.array().map_err(format)?)
            .map_err(|_| StorageError::BadFormat)?;
        let next_sequence = reader.u64().map_err(format)?;
        let count = usize::from(reader.u16().map_err(format)?);
        if count > ceiling::FORGOTTEN_PER_CONVERSATION {
            return Err(StorageError::BadFormat);
        }
        let mut inbound = HashSet::new();
        let mut forgotten =
            Vec::with_capacity(capacity(count, reader.peek().len(), MESSAGE_ID_LEN));
        for _ in 0..count {
            let id = MessageId::from_bytes(reader.array().map_err(format)?);
            if !inbound.insert(id) {
                return Err(StorageError::BadFormat);
            }
            forgotten.push(id);
        }
        let count = u32::from_be_bytes(reader.array().map_err(format)?);
        let count = usize::try_from(count).map_err(|_| StorageError::BadFormat)?;
        totals.entries = totals.entries.saturating_add(count);
        if totals.entries > ceiling::MESSAGES {
            return Err(StorageError::BadFormat);
        }
        let mut outbound = HashSet::new();
        let mut entries: Vec<StoredEntry> =
            Vec::with_capacity(capacity(count, reader.peek().len(), entry_len(1)));
        let mut queued = 0_usize;
        let mut bytes = 0_usize;
        for _ in 0..count {
            let sequence = reader.u64().map_err(format)?;
            // Increasing, and below the next sequence.
            if entries.last().is_some_and(|last| last.sequence >= sequence)
                || sequence >= next_sequence
            {
                return Err(StorageError::BadFormat);
            }
            let kind = EntryKind::from_code(reader.u8().map_err(format)?)
                .ok_or(StorageError::BadFormat)?;
            let id = MessageId::from_bytes(reader.array().map_err(format)?);
            let fresh = if kind.is_outbound() {
                outbound.insert(id)
            } else {
                inbound.insert(id)
            };
            if !fresh {
                return Err(StorageError::BadFormat);
            }
            let time = reader.u64().map_err(format)?;
            let text = ChatText::from_bytes(reader.bytes(1, MAX_CHAT_TEXT_LEN).map_err(format)?)
                .map_err(format)?;
            let len = text.as_bytes().len();
            bytes = bytes.saturating_add(entry_len(len));
            if kind == EntryKind::Queued {
                queued = queued.saturating_add(1);
                totals.queued_bytes = totals.queued_bytes.saturating_add(len);
            }
            entries.push(StoredEntry {
                sequence,
                kind,
                id,
                time,
                text,
            });
        }
        if queued > ceiling::QUEUED_PER_CONTACT
            || bytes > ceiling::CONVERSATION_LEN
            || totals.queued_bytes > ceiling::QUEUED_BYTES
        {
            return Err(StorageError::BadFormat);
        }
        Ok(Self {
            contact,
            peer,
            next_sequence,
            forgotten,
            entries,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects
    )]

    use super::*;
    use monolith_identity::IdentitySecretKey;

    fn peer(seed: u8) -> IdentityPublicKey {
        IdentitySecretKey::from_seed(&[seed; 32]).public_key()
    }

    fn entry(sequence: u64, kind: EntryKind, id: u8, text: &str) -> StoredEntry {
        StoredEntry {
            sequence,
            kind,
            id: MessageId::from_bytes([id; MESSAGE_ID_LEN]),
            time: 1_700_000_000 + sequence,
            text: ChatText::new(text).unwrap(),
        }
    }

    fn conversation(contact: u8) -> StoredConversation {
        StoredConversation {
            contact: ContactId::from_bytes([contact; RECORD_ID_LEN]).unwrap(),
            peer: peer(contact),
            next_sequence: 9,
            forgotten: vec![MessageId::from_bytes([0xF0; MESSAGE_ID_LEN])],
            entries: vec![
                entry(1, EntryKind::Delivered, 1, "first"),
                entry(
                    2,
                    EntryKind::Received,
                    1,
                    "the same identifier, the other way",
                ),
                entry(5, EntryKind::Queued, 2, "line one\nline two"),
                entry(8, EntryKind::Received, 3, "last"),
            ],
        }
    }

    fn contents() -> MessageContents {
        MessageContents {
            identities: vec![
                StoredMessages {
                    identity: IdentityId::from_bytes([1; RECORD_ID_LEN]).unwrap(),
                    conversations: vec![conversation(0x10), conversation(0x11)],
                },
                StoredMessages {
                    identity: IdentityId::from_bytes([2; RECORD_ID_LEN]).unwrap(),
                    conversations: vec![conversation(0x20)],
                },
            ],
        }
    }

    #[test]
    fn contents_survive_a_round_trip() {
        let bytes = contents().encode().unwrap();
        assert_eq!(MessageContents::decode(&bytes).unwrap(), contents());
        let padded = [bytes.as_slice(), &[0; 64]].concat();
        assert_eq!(MessageContents::decode(&padded).unwrap(), contents());
        let empty = MessageContents::default().encode().unwrap();
        assert_eq!(
            MessageContents::decode(&empty).unwrap(),
            MessageContents::default()
        );
    }

    #[test]
    fn anything_unknown_truncated_or_left_over_is_refused() {
        let bytes = contents().encode().unwrap();
        let mut changed = bytes.to_vec();
        changed[1] = 2;
        assert_eq!(
            MessageContents::decode(&changed),
            Err(StorageError::UnsupportedVersion)
        );
        let padded = [bytes.as_slice(), &[0, 1]].concat();
        assert_eq!(
            MessageContents::decode(&padded),
            Err(StorageError::BadFormat)
        );
        for len in 0..bytes.len() {
            assert!(MessageContents::decode(&bytes[..len]).is_err(), "{len}");
        }
    }

    fn refused(contents: &MessageContents) {
        assert_eq!(
            MessageContents::decode(&contents.encode().unwrap()),
            Err(StorageError::BadFormat)
        );
    }

    #[test]
    fn impossible_states_are_refused() {
        // Entries out of sequence, or at or above the next sequence.
        let mut changed = contents();
        changed.identities[0].conversations[0].entries.swap(0, 1);
        refused(&changed);
        let mut changed = contents();
        changed.identities[0].conversations[0].next_sequence = 8;
        refused(&changed);
        // One identifier twice in one direction; a forgotten identifier
        // that is also received.
        let mut changed = contents();
        changed.identities[0].conversations[0].entries[2].id =
            MessageId::from_bytes([1; MESSAGE_ID_LEN]);
        refused(&changed);
        let mut changed = contents();
        changed.identities[0].conversations[0].forgotten =
            vec![MessageId::from_bytes([3; MESSAGE_ID_LEN])];
        refused(&changed);
        // Two sections of one identity, two conversations of one record,
        // and sections or conversations out of order.
        let mut changed = contents();
        changed.identities[1].identity = changed.identities[0].identity;
        refused(&changed);
        let mut changed = contents();
        changed.identities[1].conversations[0].contact =
            changed.identities[0].conversations[0].contact;
        refused(&changed);
        let mut changed = contents();
        changed.identities.swap(0, 1);
        refused(&changed);
        let mut changed = contents();
        changed.identities[0].conversations.swap(0, 1);
        refused(&changed);
    }

    #[test]
    fn bad_fields_are_refused() {
        let bytes = contents().encode().unwrap().to_vec();
        // The kind of the first entry: version, length, count, identity id,
        // conversation count, contact id, peer, next sequence, forgotten
        // count and one identifier, entry count, sequence.
        let kind = 2 + 4 + 1 + 16 + 2 + 16 + 32 + 8 + 2 + 16 + 4 + 8;
        assert_eq!(bytes[kind], EntryKind::Delivered.code());
        for bad in [0, 4, 0xFF] {
            let mut changed = bytes.clone();
            changed[kind] = bad;
            assert_eq!(
                MessageContents::decode(&changed),
                Err(StorageError::BadFormat)
            );
        }
        // A text that is not UTF-8, or empty.
        let text = kind + 1 + 16 + 8 + 2;
        let mut changed = bytes.clone();
        changed[text] = 0xFF;
        assert_eq!(
            MessageContents::decode(&changed),
            Err(StorageError::BadFormat)
        );
        let mut empty = bytes.clone();
        empty[text - 2] = 0;
        empty[text - 1] = 0;
        assert_eq!(
            MessageContents::decode(&empty),
            Err(StorageError::BadFormat)
        );
        // A contact identifier of zero bytes.
        let contact = 2 + 4 + 1 + 16 + 2;
        let mut changed = bytes.clone();
        changed[contact..contact + 16].fill(0);
        assert_eq!(
            MessageContents::decode(&changed),
            Err(StorageError::BadFormat)
        );
        // A peer that is not a valid identity key.
        let mut changed = bytes;
        changed[contact + 16..contact + 48].fill(0);
        assert_eq!(
            MessageContents::decode(&changed),
            Err(StorageError::BadFormat)
        );
    }

    #[test]
    fn counts_above_their_limits_are_refused_before_allocating() {
        let bytes = MessageContents::default().encode().unwrap().to_vec();
        // The identity count.
        let mut changed = bytes.clone();
        changed[6] = u8::try_from(ceiling::LOCAL_IDENTITIES + 1).unwrap();
        assert_eq!(
            MessageContents::decode(&changed),
            Err(StorageError::BadFormat)
        );
        // An entry count far beyond what the payload holds fails without
        // allocating for it.
        let mut one = contents();
        one.identities.truncate(1);
        let mut bytes = one.encode().unwrap().to_vec();
        let count = 2 + 4 + 1 + 16 + 2 + 16 + 32 + 8 + 2 + 16;
        bytes[count..count + 4].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(
            MessageContents::decode(&bytes),
            Err(StorageError::BadFormat)
        );
    }

    #[test]
    fn queue_limits_hold_when_read() {
        let identity = IdentityId::from_bytes([1; RECORD_ID_LEN]).unwrap();
        let mut queued = conversation(0x10);
        queued.entries = (0..=ceiling::QUEUED_PER_CONTACT as u64)
            .map(|n| StoredEntry {
                sequence: n,
                kind: EntryKind::Queued,
                id: MessageId::from_bytes((n as u128).to_be_bytes()),
                time: 0,
                text: ChatText::new("x").unwrap(),
            })
            .collect();
        queued.next_sequence = ceiling::QUEUED_PER_CONTACT as u64 + 1;
        let contents = MessageContents {
            identities: vec![StoredMessages {
                identity,
                conversations: vec![queued.clone()],
            }],
        };
        refused(&contents);
        // One fewer is accepted.
        queued.entries.pop();
        let contents = MessageContents {
            identities: vec![StoredMessages {
                identity,
                conversations: vec![queued],
            }],
        };
        assert_eq!(
            MessageContents::decode(&contents.encode().unwrap()).unwrap(),
            contents
        );
    }

    #[test]
    fn debug_shows_no_text_and_no_identifier() {
        let mut stored = contents();
        stored.identities[0].conversations[0].entries[0].text =
            ChatText::new("very secret words").unwrap();
        stored.identities[0].conversations[0].entries[0].id =
            MessageId::from_bytes([0xB7; MESSAGE_ID_LEN]);
        let text = format!("{stored:?} {stored:#?}");
        assert!(!text.contains("secret"), "{text}");
        assert!(!text.contains("183"), "{text}");
    }
}

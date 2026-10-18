//! The messages of the local identities: the outbound queue of every
//! conversation, what was delivered, what was received, and the
//! identifiers already taken in (`docs/adr/0007-delivery-semantics.md`,
//! `docs/adr/0008-message-store.md`).
//!
//! [`MessageState`] is the state machine: plain data and synchronous
//! transitions, no I/O, no clock, no random source. A conversation is the
//! messages of one contact record (its `ContactId`) of one local identity
//! (its `IdentityId`); nothing in one conversation is found through
//! another. Its rules:
//!
//! - an outbound message enters as `Queued` and becomes `Delivered` only by
//!   the acknowledgement of its identifier in its own conversation, never
//!   back;
//! - the queue is first in, first out: [`MessageState::head`] is the oldest
//!   queued message;
//! - an inbound message is stored once per identifier: the identifier is
//!   looked up and the message inserted in one transition, and the
//!   identifiers of received messages the user deleted are remembered in
//!   a bounded window;
//! - every limit of `docs/RESOURCE_LIMITS.md` section 5 for the store is
//!   checked before anything changes; nothing stored is dropped to make
//!   room.
//!
//! [`Messages`] holds the state of one installation under one lock, stamps
//! every change with a generation of its own and writes it to the message
//! store with the write job of [`crate::persist`]. A change is used (an
//! acknowledgement sent, a command reported done, an event raised) only
//! once its generation is durable.

use core::fmt;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use monolith_identity::IdentityPublicKey;
use monolith_protocol::body::MessageId;
use monolith_protocol::limits::{
    MAX_CONVERSATION_STORE_LEN, MAX_CONVERSATIONS_PER_IDENTITY, MAX_LOCAL_IDENTITIES,
    MAX_QUEUED_MESSAGE_BYTES, MAX_QUEUED_MESSAGES_PER_CONTACT, MAX_QUEUED_MESSAGES_PER_IDENTITY,
    MAX_STORED_MESSAGES, MESSAGE_DEDUP_WINDOW, MESSAGE_ID_LEN,
};
use monolith_protocol::text::ChatText;
use monolith_storage::StorageError;
use monolith_storage::message_records::{
    CONVERSATION_OVERHEAD_LEN, EntryKind, IDENTITY_OVERHEAD_LEN, MessageContents,
    PAYLOAD_OVERHEAD_LEN, StoredConversation, StoredEntry, StoredMessages, entry_len,
};
use monolith_storage::messages::MAX_PLAINTEXT_LEN;
use monolith_storage::record::{ContactId, IdentityId};
use tokio::sync::watch;

use crate::persist::{CommitError, Durability, Store, Write};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Every transition is one step that cannot panic halfway, so a
    // poisoned lock still guards a consistent state.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Why a message was not stored. Nothing changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Refusal {
    /// `MAX_QUEUED_MESSAGES_PER_CONTACT` messages are queued for the
    /// contact.
    ContactQueueFull,
    /// `MAX_QUEUED_MESSAGES_PER_IDENTITY` messages are queued for the
    /// identity.
    IdentityQueueFull,
    /// `MAX_QUEUED_MESSAGE_BYTES` bytes of text are queued.
    QueueBytesFull,
    /// The conversation holds `MAX_CONVERSATION_STORE_LEN` bytes.
    ConversationFull,
    /// The identity has `MAX_CONVERSATIONS_PER_IDENTITY` conversations.
    TooManyConversations,
    /// The store holds `MAX_STORED_MESSAGES` messages, or the messages of
    /// `MAX_LOCAL_IDENTITIES` identities, or would not fit in the largest
    /// message store file.
    StoreFull,
    /// The identifier of an outbound message is taken in its conversation.
    /// The caller draws another one.
    IdTaken,
    /// The conversation of this record belongs to another remote identity.
    /// It cannot happen through the core; a store that says so is refused.
    OtherPeer,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ContactQueueFull => "too many messages queued for this contact",
            Self::IdentityQueueFull => "too many messages queued for this identity",
            Self::QueueBytesFull => "too much text queued",
            Self::ConversationFull => "conversation full",
            Self::TooManyConversations => "too many conversations",
            Self::StoreFull => "message store full",
            Self::IdTaken => "message identifier taken",
            Self::OtherPeer => "conversation of another peer",
        })
    }
}

/// What became of a received message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Receipt {
    /// It was stored, with this sequence.
    Stored(u64),
    /// Its identifier was taken in before: nothing was stored. It is
    /// acknowledged again.
    Duplicate,
}

/// What an acknowledgement did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Acknowledgement {
    /// The queued message of this sequence is delivered now.
    Delivered(u64),
    /// The message was delivered already.
    AlreadyDelivered,
    /// No outbound message of this conversation has the identifier. Ignored.
    Unknown,
}

/// One message, for a query.
#[derive(Clone, PartialEq, Eq)]
pub struct EntryView {
    /// Its place in the conversation.
    pub sequence: u64,
    /// Queued, delivered or received.
    pub kind: EntryKind,
    /// The identifier on the wire.
    pub id: MessageId,
    /// The local time it was queued or received, in seconds since the Unix
    /// epoch.
    pub time: u64,
    /// The text.
    pub text: ChatText,
}

impl fmt::Debug for EntryView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EntryView")
            .field("sequence", &self.sequence)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// One conversation, for a query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConversationView {
    /// The contact record it belongs to.
    pub contact: ContactId,
    /// The remote identity of that record.
    pub peer: IdentityPublicKey,
    /// Messages stored.
    pub messages: usize,
    /// Of those, queued.
    pub queued: usize,
    /// The sequence of the newest message, if there is one.
    pub latest: Option<u64>,
}

#[derive(Clone, PartialEq, Eq)]
struct Entry {
    kind: EntryKind,
    id: MessageId,
    time: u64,
    text: ChatText,
}

#[derive(Clone, PartialEq, Eq)]
struct Conversation {
    peer: IdentityPublicKey,
    next_sequence: u64,
    /// Identifiers of received messages the user deleted, oldest first.
    forgotten: VecDeque<MessageId>,
    entries: BTreeMap<u64, Entry>,
    /// The sequence of every outbound message, by identifier.
    outbound: HashMap<MessageId, u64>,
    /// The identifiers of the received messages that are stored.
    inbound: HashSet<MessageId>,
    /// The sequences of the queued messages, oldest first.
    queued: VecDeque<u64>,
    /// The bytes the entries take in the payload.
    entries_len: usize,
    /// The bytes of text queued.
    queued_bytes: usize,
}

impl Conversation {
    fn new(peer: IdentityPublicKey) -> Self {
        Self {
            peer,
            next_sequence: 1,
            forgotten: VecDeque::new(),
            entries: BTreeMap::new(),
            outbound: HashMap::new(),
            inbound: HashSet::new(),
            queued: VecDeque::new(),
            entries_len: 0,
            queued_bytes: 0,
        }
    }

    /// The bytes this conversation takes in the payload.
    fn payload_len(&self) -> usize {
        CONVERSATION_OVERHEAD_LEN
            .saturating_add(self.forgotten.len().saturating_mul(MESSAGE_ID_LEN))
            .saturating_add(self.entries_len)
    }

    fn knows_inbound(&self, id: &MessageId) -> bool {
        self.inbound.contains(id) || self.forgotten.contains(id)
    }
}

#[derive(Clone, Default, PartialEq, Eq)]
struct IdentityMessages {
    conversations: BTreeMap<ContactId, Conversation>,
    queued: usize,
}

/// The messages of every local identity of one installation. See the
/// module documentation for its rules.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct MessageState {
    identities: BTreeMap<IdentityId, IdentityMessages>,
    /// What bounds what is added.
    limits: Limits,
    /// Messages stored.
    entries: usize,
    /// Bytes of text queued.
    queued_bytes: usize,
    /// The exact length of the encoded payload, without padding.
    payload_len: usize,
}

impl fmt::Debug for MessageState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageState")
            .field("identities", &self.identities.len())
            .field("entries", &self.entries)
            .finish_non_exhaustive()
    }
}

/// The limits of `RESOURCE_LIMITS.md` section 5 that bound what is added
/// to the store. What it holds is bounded by the ceilings of its format
/// (`monolith_storage::message_records::ceiling`), which are at least
/// these: a store above a limit, as one written before a later build
/// lowered it, is read and kept, and nothing is added past the limit until
/// it is below it again (`DESIGN_QUESTIONS.md` P7-5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
    pub(crate) local_identities: usize,
    pub(crate) conversations_per_identity: usize,
    pub(crate) queued_per_identity: usize,
    pub(crate) queued_per_contact: usize,
    pub(crate) queued_bytes: usize,
    pub(crate) forgotten: usize,
    pub(crate) conversation_len: usize,
    pub(crate) messages: usize,
}

impl Default for Limits {
    /// The limits of this build.
    fn default() -> Self {
        Self {
            local_identities: MAX_LOCAL_IDENTITIES,
            conversations_per_identity: MAX_CONVERSATIONS_PER_IDENTITY,
            queued_per_identity: MAX_QUEUED_MESSAGES_PER_IDENTITY,
            queued_per_contact: MAX_QUEUED_MESSAGES_PER_CONTACT,
            queued_bytes: MAX_QUEUED_MESSAGE_BYTES,
            forgotten: MESSAGE_DEDUP_WINDOW,
            conversation_len: MAX_CONVERSATION_STORE_LEN,
            messages: MAX_STORED_MESSAGES,
        }
    }
}

/// What a new entry would take, checked against every limit of the store
/// before anything changes.
struct Room {
    new_identity: bool,
    new_conversation: bool,
    entry: usize,
}

impl MessageState {
    /// An empty state.
    pub fn new() -> Self {
        Self {
            payload_len: PAYLOAD_OVERHEAD_LEN,
            ..Self::default()
        }
    }

    /// The same state, with `limits` for what is added: those a later
    /// build could have.
    #[cfg(test)]
    pub(crate) fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// The state of a message store as read. The payload was validated
    /// when it was decoded; what it holds is indexed here.
    pub fn load(contents: MessageContents) -> Self {
        let mut state = Self::new();
        for stored in contents.identities {
            let mut identity = IdentityMessages::default();
            state.payload_len = state.payload_len.saturating_add(IDENTITY_OVERHEAD_LEN);
            for held in stored.conversations {
                let mut conversation = Conversation::new(held.peer);
                conversation.next_sequence = held.next_sequence;
                conversation.forgotten = held.forgotten.into_iter().collect();
                for entry in held.entries {
                    let len = entry_len(entry.text.as_bytes().len());
                    conversation.entries_len = conversation.entries_len.saturating_add(len);
                    match entry.kind {
                        EntryKind::Queued => {
                            conversation.queued.push_back(entry.sequence);
                            conversation.queued_bytes = conversation
                                .queued_bytes
                                .saturating_add(entry.text.as_bytes().len());
                            conversation.outbound.insert(entry.id, entry.sequence);
                        }
                        EntryKind::Delivered => {
                            conversation.outbound.insert(entry.id, entry.sequence);
                        }
                        EntryKind::Received => {
                            conversation.inbound.insert(entry.id);
                        }
                    }
                    state.entries = state.entries.saturating_add(1);
                    conversation.entries.insert(
                        entry.sequence,
                        Entry {
                            kind: entry.kind,
                            id: entry.id,
                            time: entry.time,
                            text: entry.text,
                        },
                    );
                }
                identity.queued = identity.queued.saturating_add(conversation.queued.len());
                state.queued_bytes = state.queued_bytes.saturating_add(conversation.queued_bytes);
                state.payload_len = state.payload_len.saturating_add(conversation.payload_len());
                identity.conversations.insert(held.contact, conversation);
            }
            state.identities.insert(stored.identity, identity);
        }
        state
    }

    /// What the message store holds, to write.
    pub fn contents(&self) -> MessageContents {
        MessageContents {
            identities: self
                .identities
                .iter()
                .map(|(identity, held)| StoredMessages {
                    identity: *identity,
                    conversations: held
                        .conversations
                        .iter()
                        .map(|(contact, conversation)| StoredConversation {
                            contact: *contact,
                            peer: conversation.peer,
                            next_sequence: conversation.next_sequence,
                            forgotten: conversation.forgotten.iter().copied().collect(),
                            entries: conversation
                                .entries
                                .iter()
                                .map(|(sequence, entry)| StoredEntry {
                                    sequence: *sequence,
                                    kind: entry.kind,
                                    id: entry.id,
                                    time: entry.time,
                                    text: entry.text.clone(),
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// The length of the payload this state encodes to, without padding.
    pub const fn payload_len(&self) -> usize {
        self.payload_len
    }

    /// Messages stored, all conversations together.
    pub const fn stored(&self) -> usize {
        self.entries
    }

    fn conversation(&self, identity: &IdentityId, contact: &ContactId) -> Option<&Conversation> {
        self.identities
            .get(identity)
            .and_then(|held| held.conversations.get(contact))
    }

    /// Checks that a new entry fits every limit of the whole store.
    fn fits(&self, room: &Room) -> Result<(), Refusal> {
        if self.entries >= self.limits.messages {
            return Err(Refusal::StoreFull);
        }
        let mut grows = room.entry;
        if room.new_conversation {
            grows = grows.saturating_add(CONVERSATION_OVERHEAD_LEN);
        }
        if room.new_identity {
            grows = grows.saturating_add(IDENTITY_OVERHEAD_LEN);
        }
        if self.payload_len.saturating_add(grows) > MAX_PLAINTEXT_LEN {
            return Err(Refusal::StoreFull);
        }
        Ok(())
    }

    /// Checks what a new entry of `len` bytes of text in the conversation
    /// of `contact` needs, and returns the room it takes.
    fn room_for(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
        peer: &IdentityPublicKey,
        len: usize,
    ) -> Result<Room, Refusal> {
        let held = self.identities.get(identity);
        // A payload holds the messages of at most `MAX_LOCAL_IDENTITIES`
        // identities; the state never holds what it could not write.
        if held.is_none() && self.identities.len() >= self.limits.local_identities {
            return Err(Refusal::StoreFull);
        }
        let conversation = held.and_then(|held| held.conversations.get(contact));
        if let Some(conversation) = conversation {
            if conversation.peer != *peer {
                return Err(Refusal::OtherPeer);
            }
            // Every sequence was given out: nothing more fits.
            if conversation.next_sequence == u64::MAX {
                return Err(Refusal::ConversationFull);
            }
        } else if held
            .is_some_and(|held| held.conversations.len() >= self.limits.conversations_per_identity)
        {
            return Err(Refusal::TooManyConversations);
        }
        let entry = entry_len(len);
        let used = conversation.map_or(0, |conversation| conversation.entries_len);
        if used.saturating_add(entry) > self.limits.conversation_len {
            return Err(Refusal::ConversationFull);
        }
        let room = Room {
            new_identity: held.is_none(),
            new_conversation: conversation.is_none(),
            entry,
        };
        self.fits(&room)?;
        Ok(room)
    }

    /// Adds an entry, after its room was checked.
    fn insert(
        &mut self,
        identity: IdentityId,
        contact: ContactId,
        peer: IdentityPublicKey,
        room: &Room,
        entry: Entry,
    ) -> u64 {
        let held = self.identities.entry(identity).or_default();
        let conversation = held
            .conversations
            .entry(contact)
            .or_insert_with(|| Conversation::new(peer));
        let sequence = conversation.next_sequence;
        conversation.next_sequence = sequence.saturating_add(1);
        conversation.entries_len = conversation.entries_len.saturating_add(room.entry);
        let len = entry.text.as_bytes().len();
        match entry.kind {
            EntryKind::Queued => {
                conversation.queued.push_back(sequence);
                conversation.queued_bytes = conversation.queued_bytes.saturating_add(len);
                conversation.outbound.insert(entry.id, sequence);
                held.queued = held.queued.saturating_add(1);
                self.queued_bytes = self.queued_bytes.saturating_add(len);
            }
            EntryKind::Delivered => {
                conversation.outbound.insert(entry.id, sequence);
            }
            EntryKind::Received => {
                conversation.inbound.insert(entry.id);
            }
        }
        conversation.entries.insert(sequence, entry);
        self.entries = self.entries.saturating_add(1);
        let mut grows = room.entry;
        if room.new_conversation {
            grows = grows.saturating_add(CONVERSATION_OVERHEAD_LEN);
        }
        if room.new_identity {
            grows = grows.saturating_add(IDENTITY_OVERHEAD_LEN);
        }
        self.payload_len = self.payload_len.saturating_add(grows);
        sequence
    }

    /// Queues `text` with the identifier `id` for the contact record
    /// `contact` of `identity`, whose remote identity is `peer`. Returns
    /// its sequence. Every limit is checked first: nothing changes on a
    /// refusal, and nothing stored is dropped to make room.
    #[allow(clippy::too_many_arguments)]
    pub fn queue(
        &mut self,
        identity: IdentityId,
        contact: ContactId,
        peer: IdentityPublicKey,
        id: MessageId,
        text: ChatText,
        time: u64,
    ) -> Result<u64, Refusal> {
        let len = text.as_bytes().len();
        if let Some(conversation) = self.conversation(&identity, &contact) {
            if conversation.outbound.contains_key(&id) {
                return Err(Refusal::IdTaken);
            }
            if conversation.queued.len() >= self.limits.queued_per_contact {
                return Err(Refusal::ContactQueueFull);
            }
        }
        if self
            .identities
            .get(&identity)
            .is_some_and(|held| held.queued >= self.limits.queued_per_identity)
        {
            return Err(Refusal::IdentityQueueFull);
        }
        if self.queued_bytes.saturating_add(len) > self.limits.queued_bytes {
            return Err(Refusal::QueueBytesFull);
        }
        let room = self.room_for(&identity, &contact, &peer, len)?;
        Ok(self.insert(
            identity,
            contact,
            peer,
            &room,
            Entry {
                kind: EntryKind::Queued,
                id,
                time,
                text,
            },
        ))
    }

    /// Takes in a message with the identifier `id` that arrived on a
    /// session of the contact record `contact` of `identity`. The lookup of
    /// the identifier and the insertion are one transition: a message is
    /// stored once, however often it arrives.
    pub fn receive(
        &mut self,
        identity: IdentityId,
        contact: ContactId,
        peer: IdentityPublicKey,
        id: MessageId,
        text: ChatText,
        time: u64,
    ) -> Result<Receipt, Refusal> {
        if let Some(conversation) = self.conversation(&identity, &contact) {
            if conversation.peer != peer {
                return Err(Refusal::OtherPeer);
            }
            if conversation.knows_inbound(&id) {
                return Ok(Receipt::Duplicate);
            }
        }
        let room = self.room_for(&identity, &contact, &peer, text.as_bytes().len())?;
        Ok(Receipt::Stored(self.insert(
            identity,
            contact,
            peer,
            &room,
            Entry {
                kind: EntryKind::Received,
                id,
                time,
                text,
            },
        )))
    }

    /// Applies an acknowledgement of `id` that arrived on a session of the
    /// contact record `contact` of `identity`: only an outbound message of
    /// that conversation can be delivered by it.
    pub fn acknowledge(
        &mut self,
        identity: &IdentityId,
        contact: &ContactId,
        id: &MessageId,
    ) -> Acknowledgement {
        let Some(held) = self.identities.get_mut(identity) else {
            return Acknowledgement::Unknown;
        };
        let Some(conversation) = held.conversations.get_mut(contact) else {
            return Acknowledgement::Unknown;
        };
        let Some(sequence) = conversation.outbound.get(id).copied() else {
            return Acknowledgement::Unknown;
        };
        let Some(entry) = conversation.entries.get_mut(&sequence) else {
            return Acknowledgement::Unknown;
        };
        if entry.kind != EntryKind::Queued {
            return Acknowledgement::AlreadyDelivered;
        }
        entry.kind = EntryKind::Delivered;
        let len = entry.text.as_bytes().len();
        conversation.queued.retain(|queued| *queued != sequence);
        conversation.queued_bytes = conversation.queued_bytes.saturating_sub(len);
        held.queued = held.queued.saturating_sub(1);
        self.queued_bytes = self.queued_bytes.saturating_sub(len);
        Acknowledgement::Delivered(sequence)
    }

    /// The oldest queued message of the conversation, if any: the one to
    /// send next.
    pub fn head(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
    ) -> Option<(u64, MessageId, ChatText)> {
        let conversation = self.conversation(identity, contact)?;
        let sequence = *conversation.queued.front()?;
        let entry = conversation.entries.get(&sequence)?;
        Some((sequence, entry.id, entry.text.clone()))
    }

    /// The number of queued messages of the conversation.
    pub fn queued(&self, identity: &IdentityId, contact: &ContactId) -> usize {
        self.conversation(identity, contact)
            .map_or(0, |conversation| conversation.queued.len())
    }

    /// The conversations of `identity` that have queued messages.
    pub fn with_queue(&self, identity: &IdentityId) -> Vec<(ContactId, IdentityPublicKey)> {
        self.identities
            .get(identity)
            .map(|held| {
                held.conversations
                    .iter()
                    .filter(|(_, conversation)| !conversation.queued.is_empty())
                    .map(|(contact, conversation)| (*contact, conversation.peer))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Deletes the messages of a conversation, queued ones included. The
    /// identifiers of the received ones are remembered, the newest
    /// `MESSAGE_DEDUP_WINDOW` of them, so that a late retry of one is not
    /// shown again. With `keep` false the conversation goes as a whole,
    /// window included: its record is gone. Returns true if anything
    /// changed.
    pub fn delete_conversation(
        &mut self,
        identity: &IdentityId,
        contact: &ContactId,
        keep: bool,
    ) -> bool {
        let forgotten = self.limits.forgotten;
        let Some(held) = self.identities.get_mut(identity) else {
            return false;
        };
        let Some(conversation) = held.conversations.get_mut(contact) else {
            return false;
        };
        let before = conversation.payload_len();
        let entries = conversation.entries.len();
        held.queued = held.queued.saturating_sub(conversation.queued.len());
        self.queued_bytes = self.queued_bytes.saturating_sub(conversation.queued_bytes);
        self.entries = self.entries.saturating_sub(entries);
        if keep {
            let received: Vec<MessageId> = conversation
                .entries
                .values()
                .filter(|entry| entry.kind == EntryKind::Received)
                .map(|entry| entry.id)
                .collect();
            conversation.forgotten.extend(received);
            while conversation.forgotten.len() > forgotten {
                conversation.forgotten.pop_front();
            }
            conversation.entries.clear();
            conversation.outbound.clear();
            conversation.inbound.clear();
            conversation.queued.clear();
            conversation.entries_len = 0;
            conversation.queued_bytes = 0;
            let after = conversation.payload_len();
            self.payload_len = self
                .payload_len
                .saturating_sub(before)
                .saturating_add(after);
            return entries > 0;
        }
        held.conversations.remove(contact);
        self.payload_len = self.payload_len.saturating_sub(before);
        if held.conversations.is_empty() {
            self.identities.remove(identity);
            self.payload_len = self.payload_len.saturating_sub(IDENTITY_OVERHEAD_LEN);
        }
        true
    }

    /// Removes everything of `identity`. Returns true if anything was held.
    pub fn remove_identity(&mut self, identity: &IdentityId) -> bool {
        let Some(held) = self.identities.remove(identity) else {
            return false;
        };
        for conversation in held.conversations.values() {
            self.entries = self.entries.saturating_sub(conversation.entries.len());
            self.queued_bytes = self.queued_bytes.saturating_sub(conversation.queued_bytes);
            self.payload_len = self.payload_len.saturating_sub(conversation.payload_len());
        }
        self.payload_len = self.payload_len.saturating_sub(IDENTITY_OVERHEAD_LEN);
        true
    }

    /// The local identities that have messages.
    pub fn identities(&self) -> Vec<IdentityId> {
        self.identities.keys().copied().collect()
    }

    /// The conversations of `identity`.
    pub fn conversations(&self, identity: &IdentityId) -> Vec<ConversationView> {
        self.identities
            .get(identity)
            .map(|held| {
                held.conversations
                    .iter()
                    .map(|(contact, conversation)| ConversationView {
                        contact: *contact,
                        peer: conversation.peer,
                        messages: conversation.entries.len(),
                        queued: conversation.queued.len(),
                        latest: conversation.entries.keys().next_back().copied(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Up to `limit` messages of a conversation, newest last, those with a
    /// sequence below `before` if it is given.
    pub fn entries(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
        before: Option<u64>,
        limit: usize,
    ) -> Vec<EntryView> {
        let Some(conversation) = self.conversation(identity, contact) else {
            return Vec::new();
        };
        let end = before.unwrap_or(u64::MAX);
        let mut page: Vec<EntryView> = conversation
            .entries
            .range(..end)
            .rev()
            .take(limit)
            .map(|(sequence, entry)| EntryView {
                sequence: *sequence,
                kind: entry.kind,
                id: entry.id,
                time: entry.time,
                text: entry.text.clone(),
            })
            .collect();
        page.reverse();
        page
    }

    /// One message, by its sequence.
    pub fn entry(
        &self,
        identity: &IdentityId,
        contact: &ContactId,
        sequence: u64,
    ) -> Option<EntryView> {
        let entry = self
            .conversation(identity, contact)?
            .entries
            .get(&sequence)?;
        Some(EntryView {
            sequence,
            kind: entry.kind,
            id: entry.id,
            time: entry.time,
            text: entry.text.clone(),
        })
    }

    /// Matches the state with the vault it belongs to
    /// (`docs/adr/0008-message-store.md`): the section of an identity that
    /// is not in the vault is dropped, what a crash left of a deletion; a
    /// conversation of a record the vault does not hold is kept, ended; a
    /// conversation of a record of another remote identity or of another
    /// local identity is refused as corrupt. `records` lists, for every
    /// identity in the vault, its contact records. Returns true if
    /// anything was dropped.
    pub fn reconcile(
        &mut self,
        records: &HashMap<IdentityId, Vec<(ContactId, IdentityPublicKey)>>,
    ) -> Result<bool, StorageError> {
        let mut owners: HashMap<ContactId, (IdentityId, IdentityPublicKey)> = HashMap::new();
        for (identity, contacts) in records {
            for (contact, peer) in contacts {
                owners.insert(*contact, (*identity, *peer));
            }
        }
        for (identity, held) in &self.identities {
            for (contact, conversation) in &held.conversations {
                if let Some((owner, peer)) = owners.get(contact) {
                    if owner != identity || *peer != conversation.peer {
                        return Err(StorageError::BadFormat);
                    }
                }
            }
        }
        let gone: Vec<IdentityId> = self
            .identities
            .keys()
            .filter(|identity| !records.contains_key(identity))
            .copied()
            .collect();
        for identity in &gone {
            self.remove_identity(identity);
        }
        Ok(!gone.is_empty())
    }
}

/// The message state of one installation, and where it is written.
pub(crate) struct Messages {
    state: Mutex<MessageState>,
    durability: Arc<Durability>,
    store: Store,
    /// Turns over after every change, so that the tasks of sessions look
    /// at their queues again.
    changes: watch::Sender<u64>,
}

impl fmt::Debug for Messages {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Messages").finish_non_exhaustive()
    }
}

impl Messages {
    pub(crate) fn new(store: Store, state: MessageState) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(state),
            durability: Durability::new(),
            store,
            changes: watch::Sender::new(0),
        })
    }

    /// Runs `change` on the state under its lock. If it returns a change,
    /// the change is stamped with the next generation in the same step and
    /// the tasks waiting for changes are woken. Returns what `change`
    /// returned, with the generation that what it decided depends on.
    pub(crate) fn change<R>(
        &self,
        change: impl FnOnce(&mut MessageState) -> (R, bool),
    ) -> Result<(R, u64), CommitError> {
        let generation;
        let result;
        let changed;
        {
            let mut state = lock(&self.state);
            if self.durability.is_failed() {
                return Err(CommitError::Failed);
            }
            let value;
            (value, changed) = change(&mut state);
            generation = if changed {
                self.durability.bump()
            } else {
                self.durability.applied()
            };
            result = value;
        }
        // Only a change wakes the tasks that follow them: an acknowledgement
        // or a copy that changes nothing wakes nobody.
        if changed {
            self.changes
                .send_modify(|count| *count = count.wrapping_add(1));
        }
        Ok((result, generation))
    }

    /// Reads the state under its lock.
    pub(crate) fn read<R>(&self, read: impl FnOnce(&MessageState) -> R) -> R {
        read(&lock(&self.state))
    }

    /// Reads the state under its lock, with the generation of the last
    /// change it holds: once that generation is durable, so is what was
    /// read.
    pub(crate) fn read_applied<R>(&self, read: impl FnOnce(&MessageState) -> R) -> (R, u64) {
        let state = lock(&self.state);
        (read(&state), self.durability.applied())
    }

    /// Follows the changes.
    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Returns true once a write failed: nothing more is stored.
    pub(crate) fn is_failed(&self) -> bool {
        self.durability.is_failed()
    }

    /// Waits until `generation` is durable.
    pub(crate) async fn commit(self: &Arc<Self>, generation: u64) -> Result<(), CommitError> {
        self.store
            .wait(&self.durability, generation, || self.write())
            .await
    }

    /// What a write of the message store needs.
    fn write(self: &Arc<Self>) -> Write {
        let reading: Weak<Self> = Arc::downgrade(self);
        let failing: Weak<Self> = Arc::downgrade(self);
        Write {
            snapshot: Box::new(move || {
                let messages = reading.upgrade().ok_or(StorageError::Internal)?;
                // The generation first: the state read after it holds every
                // change up to it.
                let covered = messages.durability.applied();
                let contents = lock(&messages.state).contents();
                Ok((covered, contents.encode()?))
            }),
            failed: Box::new(move || {
                // Nothing more is stored or acknowledged; the tasks that
                // wait for a change look again and find it failed.
                if let Some(messages) = failing.upgrade() {
                    messages
                        .changes
                        .send_modify(|count| *count = count.wrapping_add(1));
                }
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        clippy::cast_possible_truncation
    )]

    use super::*;
    use monolith_identity::IdentitySecretKey;
    use monolith_protocol::limits::MAX_CHAT_TEXT_LEN;
    use monolith_storage::record::RECORD_ID_LEN;

    fn identity(byte: u8) -> IdentityId {
        IdentityId::from_bytes([byte; RECORD_ID_LEN]).unwrap()
    }

    fn contact(byte: u8) -> ContactId {
        ContactId::from_bytes([byte; RECORD_ID_LEN]).unwrap()
    }

    fn peer(byte: u8) -> IdentityPublicKey {
        IdentitySecretKey::from_seed(&[byte; 32]).public_key()
    }

    fn id(n: u64) -> MessageId {
        MessageId::from_bytes(u128::from(n).to_be_bytes())
    }

    fn text(text: &str) -> ChatText {
        ChatText::new(text).unwrap()
    }

    fn roundtrip(state: &MessageState) -> MessageState {
        let bytes = state.contents().encode().unwrap();
        assert_eq!(
            bytes.len(),
            state.payload_len(),
            "the payload length is exact"
        );
        MessageState::load(MessageContents::decode(&bytes).unwrap())
    }

    #[test]
    fn a_queued_message_is_delivered_by_its_acknowledgement_only() {
        let mut state = MessageState::new();
        let (a, c) = (identity(1), contact(2));
        let first = state.queue(a, c, peer(3), id(1), text("one"), 10).unwrap();
        let second = state.queue(a, c, peer(3), id(2), text("two"), 11).unwrap();
        assert!(first < second);
        assert_eq!(state.head(&a, &c).unwrap().1, id(1));
        // Another identity, another record, another identifier: nothing.
        assert_eq!(
            state.acknowledge(&identity(9), &c, &id(1)),
            Acknowledgement::Unknown
        );
        assert_eq!(
            state.acknowledge(&a, &contact(9), &id(1)),
            Acknowledgement::Unknown
        );
        assert_eq!(state.acknowledge(&a, &c, &id(7)), Acknowledgement::Unknown);
        assert_eq!(state.head(&a, &c).unwrap().1, id(1));
        assert_eq!(
            state.acknowledge(&a, &c, &id(1)),
            Acknowledgement::Delivered(first)
        );
        assert_eq!(
            state.acknowledge(&a, &c, &id(1)),
            Acknowledgement::AlreadyDelivered
        );
        assert_eq!(state.head(&a, &c).unwrap().1, id(2));
        // A restart keeps both states.
        let state = roundtrip(&state);
        assert_eq!(
            state.entry(&a, &c, first).unwrap().kind,
            EntryKind::Delivered
        );
        assert_eq!(state.entry(&a, &c, second).unwrap().kind, EntryKind::Queued);
        assert_eq!(state.head(&a, &c).unwrap().1, id(2));
    }

    #[test]
    fn a_received_message_is_stored_once() {
        let mut state = MessageState::new();
        let (a, c) = (identity(1), contact(2));
        let stored = state.receive(a, c, peer(3), id(5), text("hi"), 1).unwrap();
        assert!(matches!(stored, Receipt::Stored(_)));
        assert_eq!(
            state.receive(a, c, peer(3), id(5), text("hi again"), 2),
            Ok(Receipt::Duplicate)
        );
        // After a restart as well.
        let mut state = roundtrip(&state);
        assert_eq!(
            state.receive(a, c, peer(3), id(5), text("hi"), 3),
            Ok(Receipt::Duplicate)
        );
        assert_eq!(state.entries(&a, &c, None, 10).len(), 1);
        // The same identifier in another conversation is another message.
        assert!(matches!(
            state.receive(a, contact(4), peer(3), id(5), text("hi"), 4),
            Ok(Receipt::Stored(_))
        ));
        // A conversation is never taken for another remote identity's.
        assert_eq!(
            state.receive(a, c, peer(8), id(6), text("x"), 5),
            Err(Refusal::OtherPeer)
        );
    }

    #[test]
    fn deleted_messages_are_remembered_and_not_taken_in_again() {
        let mut state = MessageState::new();
        let (a, c) = (identity(1), contact(2));
        state.receive(a, c, peer(3), id(5), text("hi"), 1).unwrap();
        state.queue(a, c, peer(3), id(6), text("out"), 1).unwrap();
        assert!(state.delete_conversation(&a, &c, true));
        assert_eq!(state.queued(&a, &c), 0);
        assert!(state.entries(&a, &c, None, 10).is_empty());
        let mut state = roundtrip(&state);
        assert_eq!(
            state.receive(a, c, peer(3), id(5), text("hi"), 2),
            Ok(Receipt::Duplicate)
        );
        // A conversation removed as a whole forgets.
        assert!(state.delete_conversation(&a, &c, false));
        assert!(matches!(
            state.receive(a, c, peer(3), id(5), text("hi"), 3),
            Ok(Receipt::Stored(_))
        ));
        assert_eq!(roundtrip(&state), state);
    }

    #[test]
    fn queue_limits_refuse_without_changing_anything() {
        let mut state = MessageState::new();
        let (a, c) = (identity(1), contact(2));
        for n in 0..MAX_QUEUED_MESSAGES_PER_CONTACT as u64 {
            state.queue(a, c, peer(3), id(n), text("m"), 0).unwrap();
        }
        let before = state.clone();
        assert_eq!(
            state.queue(a, c, peer(3), id(9999), text("m"), 0),
            Err(Refusal::ContactQueueFull)
        );
        assert_eq!(state, before);
        // An identifier taken in the conversation.
        let mut state = MessageState::new();
        state.queue(a, c, peer(3), id(1), text("m"), 0).unwrap();
        assert_eq!(
            state.queue(a, c, peer(3), id(1), text("m"), 0),
            Err(Refusal::IdTaken)
        );
        // The queue of an identity, over several contacts.
        let mut state = MessageState::new();
        let mut n = 0;
        'fill: for record in 1..=255_u8 {
            for _ in 0..MAX_QUEUED_MESSAGES_PER_CONTACT {
                if n == MAX_QUEUED_MESSAGES_PER_IDENTITY as u64 {
                    break 'fill;
                }
                state
                    .queue(a, contact(record), peer(3), id(n), text("m"), 0)
                    .unwrap();
                n += 1;
            }
        }
        assert_eq!(
            state.queue(a, contact(200), peer(3), id(n), text("m"), 0),
            Err(Refusal::IdentityQueueFull)
        );
        // Another identity still queues.
        assert!(
            state
                .queue(identity(2), c, peer(3), id(n), text("m"), 0)
                .is_ok()
        );
    }

    #[test]
    fn a_conversation_is_bounded_in_bytes() {
        let mut state = MessageState::new();
        let (a, c) = (identity(1), contact(2));
        let long = "x".repeat(MAX_CHAT_TEXT_LEN);
        let per = entry_len(MAX_CHAT_TEXT_LEN);
        let fit = MAX_CONVERSATION_STORE_LEN / per;
        for n in 0..fit as u64 {
            state.receive(a, c, peer(3), id(n), text(&long), 0).unwrap();
        }
        let before = state.clone();
        assert_eq!(
            state.receive(a, c, peer(3), id(fit as u64), text(&long), 0),
            Err(Refusal::ConversationFull)
        );
        assert_eq!(state, before);
        // Another conversation is not affected.
        assert!(
            state
                .receive(a, contact(3), peer(3), id(0), text(&long), 0)
                .is_ok()
        );
    }

    #[test]
    fn reconciling_drops_deleted_identities_and_refuses_impossible_owners() {
        let mut state = MessageState::new();
        state
            .queue(identity(1), contact(2), peer(3), id(1), text("a"), 0)
            .unwrap();
        state
            .queue(identity(4), contact(5), peer(6), id(1), text("b"), 0)
            .unwrap();
        // Identity 4 is not in the vault; the record of identity 1 is gone.
        let mut records = HashMap::new();
        records.insert(identity(1), Vec::new());
        let mut kept = state.clone();
        assert_eq!(kept.reconcile(&records), Ok(true));
        assert_eq!(kept.identities(), vec![identity(1)]);
        assert_eq!(kept.queued(&identity(1), &contact(2)), 1);
        assert_eq!(roundtrip(&kept), kept);
        // A record of another remote identity, or of another identity.
        let mut records = HashMap::new();
        records.insert(identity(1), vec![(contact(2), peer(7))]);
        assert_eq!(
            state.clone().reconcile(&records),
            Err(StorageError::BadFormat)
        );
        let mut records = HashMap::new();
        records.insert(identity(1), Vec::new());
        records.insert(identity(9), vec![(contact(2), peer(3))]);
        assert_eq!(state.reconcile(&records), Err(StorageError::BadFormat));
    }

    #[test]
    fn pages_come_newest_last() {
        let mut state = MessageState::new();
        let (a, c) = (identity(1), contact(2));
        for n in 0..10 {
            state.receive(a, c, peer(3), id(n), text("m"), n).unwrap();
        }
        let last = state.entries(&a, &c, None, 3);
        assert_eq!(
            last.iter().map(|entry| entry.sequence).collect::<Vec<_>>(),
            vec![8, 9, 10]
        );
        let before = state.entries(&a, &c, Some(8), 3);
        assert_eq!(
            before
                .iter()
                .map(|entry| entry.sequence)
                .collect::<Vec<_>>(),
            vec![5, 6, 7]
        );
        let view = state.conversations(&a);
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].messages, 10);
        assert_eq!(view[0].latest, Some(10));
    }

    #[test]
    fn debug_shows_no_text() {
        let mut state = MessageState::new();
        state
            .queue(
                identity(1),
                contact(2),
                peer(3),
                id(1),
                text("very secret"),
                0,
            )
            .unwrap();
        let view = state.entries(&identity(1), &contact(2), None, 1);
        let text = format!("{state:?} {view:?}");
        assert!(!text.contains("secret"), "{text}");
    }

    #[test]
    fn the_state_holds_the_messages_of_as_many_identities_as_a_payload_can() {
        // A payload has room for MAX_LOCAL_IDENTITIES sections: the state
        // never holds one more, which it could not write.
        let mut state = MessageState::new();
        for n in 0..MAX_LOCAL_IDENTITIES {
            let n = u8::try_from(n).unwrap() + 1;
            state
                .queue(identity(n), contact(n), peer(n), id(1), text("x"), 0)
                .unwrap();
        }
        let before = state.clone();
        assert_eq!(
            state.queue(identity(99), contact(99), peer(99), id(1), text("x"), 0),
            Err(Refusal::StoreFull)
        );
        assert_eq!(
            state.receive(identity(99), contact(99), peer(99), id(1), text("x"), 0),
            Err(Refusal::StoreFull)
        );
        assert!(state == before);
        assert!(roundtrip(&state) == state);
    }

    #[test]
    fn a_conversation_whose_sequences_ran_out_takes_nothing_more() {
        let mut state = MessageState::new();
        state
            .queue(identity(1), contact(2), peer(3), id(1), text("x"), 0)
            .unwrap();
        let mut contents = state.contents();
        contents.identities[0].conversations[0].next_sequence = u64::MAX;
        let mut state =
            MessageState::load(MessageContents::decode(&contents.encode().unwrap()).unwrap());
        let before = state.clone();
        assert_eq!(
            state.queue(identity(1), contact(2), peer(3), id(2), text("x"), 0),
            Err(Refusal::ConversationFull)
        );
        assert_eq!(
            state.receive(identity(1), contact(2), peer(3), id(3), text("x"), 0),
            Err(Refusal::ConversationFull)
        );
        assert!(state == before);
        assert!(roundtrip(&state) == state);
    }

    #[test]
    fn only_a_change_wakes_the_tasks_that_follow_the_messages() {
        let messages = Messages::new(crate::persist::Store::Ephemeral, MessageState::new());
        let changes = messages.subscribe();
        // An acknowledgement of nothing changes nothing.
        let (acknowledgement, _) = messages
            .change(|state| {
                let acknowledgement = state.acknowledge(&identity(1), &contact(2), &id(1));
                (acknowledgement, false)
            })
            .unwrap();
        assert_eq!(acknowledgement, Acknowledgement::Unknown);
        assert!(!changes.has_changed().unwrap());
        let (queued, _) = messages
            .change(|state| {
                let queued = state.queue(identity(1), contact(2), peer(3), id(1), text("x"), 0);
                (queued, true)
            })
            .unwrap();
        assert_eq!(queued, Ok(1));
        assert!(changes.has_changed().unwrap());
    }

    /// Limits a later build could have, each lowered to `lowered` where it
    /// is named, the others as now.
    fn lowered(change: impl FnOnce(&mut Limits)) -> Limits {
        let mut limits = Limits::default();
        change(&mut limits);
        limits
    }

    #[test]
    fn a_store_above_a_lowered_limit_is_read_and_kept_and_grows_no_further() {
        // Ten messages queued under the limits of now; a later build lowers
        // the queue of a contact to five (P7-5).
        let (a, c) = (identity(1), contact(2));
        let mut state = MessageState::new();
        for n in 0..10 {
            state.queue(a, c, peer(3), id(n), text("m"), 0).unwrap();
        }
        let bytes = state.contents().encode().unwrap();
        let mut later = MessageState::load(MessageContents::decode(&bytes).unwrap())
            .with_limits(lowered(|limits| limits.queued_per_contact = 5));
        // Read whole, written back whole.
        assert_eq!(later.queued(&a, &c), 10);
        assert_eq!(later.contents().encode().unwrap(), bytes);
        // Nothing more is queued until the queue is below the new limit.
        assert_eq!(
            later.queue(a, c, peer(3), id(100), text("m"), 0),
            Err(Refusal::ContactQueueFull)
        );
        for n in 0..5 {
            later.acknowledge(&a, &c, &id(n));
        }
        assert_eq!(
            later.queue(a, c, peer(3), id(100), text("m"), 0),
            Err(Refusal::ContactQueueFull)
        );
        later.acknowledge(&a, &c, &id(5));
        assert!(later.queue(a, c, peer(3), id(100), text("m"), 0).is_ok());
    }

    #[test]
    fn a_store_with_more_messages_than_a_lowered_limit_takes_none_in() {
        let (a, c) = (identity(1), contact(2));
        let mut state = MessageState::new();
        for n in 0..8 {
            assert!(matches!(
                state.receive(a, c, peer(3), id(n), text("m"), 0),
                Ok(Receipt::Stored(_))
            ));
        }
        let bytes = state.contents().encode().unwrap();
        let mut later = MessageState::load(MessageContents::decode(&bytes).unwrap())
            .with_limits(lowered(|limits| limits.messages = 4));
        let before = later.clone();
        // Refused, and the sender keeps it; nothing held is dropped.
        assert_eq!(
            later.receive(a, c, peer(3), id(50), text("m"), 0),
            Err(Refusal::StoreFull)
        );
        assert_eq!(later, before);
        // A message already held is still known as one.
        assert_eq!(
            later.receive(a, c, peer(3), id(1), text("m"), 0),
            Ok(Receipt::Duplicate)
        );
    }

    #[test]
    fn a_lowered_window_of_forgotten_messages_holds_from_the_next_deletion() {
        let (a, c) = (identity(1), contact(2));
        let mut state = MessageState::new();
        for n in 0..6 {
            state.receive(a, c, peer(3), id(n), text("m"), 0).unwrap();
        }
        state.delete_conversation(&a, &c, true);
        let bytes = state.contents().encode().unwrap();
        let mut later = MessageState::load(MessageContents::decode(&bytes).unwrap())
            .with_limits(lowered(|limits| limits.forgotten = 2));
        // All six are still remembered as read.
        for n in 0..6 {
            assert_eq!(
                later.receive(a, c, peer(3), id(n), text("m"), 0),
                Ok(Receipt::Duplicate)
            );
        }
        later.receive(a, c, peer(3), id(10), text("m"), 0).unwrap();
        later.delete_conversation(&a, &c, true);
        // The newest two, the one just deleted among them, are kept.
        assert_eq!(
            later.receive(a, c, peer(3), id(10), text("m"), 0),
            Ok(Receipt::Duplicate)
        );
        assert!(matches!(
            later.receive(a, c, peer(3), id(0), text("m"), 0),
            Ok(Receipt::Stored(_))
        ));
    }
}

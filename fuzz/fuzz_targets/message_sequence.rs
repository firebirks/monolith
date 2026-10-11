//! The message state machine under an arbitrary sequence of operations on
//! two local identities with two contact records each: queueing, taking
//! in, acknowledging, deleting a conversation, removing an identity, and a
//! restart through the encoded payload. After every operation the state
//! holds what a model written here, apart from `MessageState`, holds; its
//! outcome is the model's; the head of every queue is its oldest queued
//! message; its encoding has the length it accounts for; and a delivered
//! message never becomes queued again while its conversation lives.
//!
//! Every operation is three bytes. The first selects it, modulo 7: queue
//! (0), receive (1), acknowledge (2), delete a conversation (3), remove an
//! identity (4), restart (5), queue naming another peer (6). The second
//! selects the identity (bit 0) and the record (bit 1). The third gives
//! the message identifier (its low three bits, so that identifiers repeat)
//! and the length of the text (the next two bits) or, for a deletion,
//! whether the record is kept (bit 0).

#![no_main]

use std::collections::{BTreeMap, VecDeque};

use libfuzzer_sys::fuzz_target;
use monolith_core::messages::{Acknowledgement, MessageState, Receipt, Refusal};
use monolith_identity::{IdentityPublicKey, IdentitySecretKey};
use monolith_protocol::body::MessageId;
use monolith_protocol::limits::{
    MAX_QUEUED_MESSAGES_PER_CONTACT, MAX_QUEUED_MESSAGES_PER_IDENTITY,
};
use monolith_protocol::text::ChatText;
use monolith_storage::message_records::{EntryKind, MessageContents};
use monolith_storage::record::{ContactId, IdentityId};

fn identity(index: usize) -> IdentityId {
    IdentityId::from_bytes([1 + u8::try_from(index).unwrap(); 16]).unwrap()
}

fn record(identity: usize, contact: usize) -> ContactId {
    ContactId::from_bytes([0x10 + u8::try_from(identity * 2 + contact).unwrap(); 16]).unwrap()
}

fn peer(index: usize) -> IdentityPublicKey {
    IdentitySecretKey::from_seed(&[0x40 + u8::try_from(index).unwrap(); 32]).public_key()
}

fn id(byte: u8) -> MessageId {
    MessageId::from_bytes([byte; 16])
}

/// One message as the model holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Held {
    kind: EntryKind,
    id: u8,
    len: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Conversation {
    peer: usize,
    next: u64,
    entries: BTreeMap<u64, Held>,
    forgotten: VecDeque<u8>,
}

/// The model: the conversations, by identity and record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Model {
    conversations: BTreeMap<(usize, usize), Conversation>,
}

impl Model {
    fn queued(&self, identity: usize) -> usize {
        self.conversations
            .iter()
            .filter(|((held, _), _)| *held == identity)
            .map(|(_, conversation)| {
                conversation
                    .entries
                    .values()
                    .filter(|held| held.kind == EntryKind::Queued)
                    .count()
            })
            .sum()
    }

    fn queue(
        &mut self,
        i: usize,
        c: usize,
        p: usize,
        message: u8,
        len: usize,
    ) -> Result<u64, Refusal> {
        let identity_queued = self.queued(i);
        if let Some(conversation) = self.conversations.get(&(i, c)) {
            if conversation
                .entries
                .values()
                .any(|held| held.kind != EntryKind::Received && held.id == message)
            {
                return Err(Refusal::IdTaken);
            }
            let queued = conversation
                .entries
                .values()
                .filter(|held| held.kind == EntryKind::Queued)
                .count();
            if queued >= MAX_QUEUED_MESSAGES_PER_CONTACT {
                return Err(Refusal::ContactQueueFull);
            }
        }
        if identity_queued >= MAX_QUEUED_MESSAGES_PER_IDENTITY {
            return Err(Refusal::IdentityQueueFull);
        }
        self.insert(i, c, p, EntryKind::Queued, message, len)
    }

    fn insert(
        &mut self,
        i: usize,
        c: usize,
        p: usize,
        kind: EntryKind,
        message: u8,
        len: usize,
    ) -> Result<u64, Refusal> {
        let conversation = self.conversations.entry((i, c)).or_insert(Conversation {
            peer: p,
            next: 1,
            ..Conversation::default()
        });
        if conversation.peer != p {
            return Err(Refusal::OtherPeer);
        }
        let sequence = conversation.next;
        conversation.next += 1;
        conversation.entries.insert(
            sequence,
            Held {
                kind,
                id: message,
                len,
            },
        );
        Ok(sequence)
    }

    fn receive(
        &mut self,
        i: usize,
        c: usize,
        p: usize,
        message: u8,
        len: usize,
    ) -> Result<Receipt, Refusal> {
        if let Some(conversation) = self.conversations.get(&(i, c)) {
            if conversation.peer != p {
                return Err(Refusal::OtherPeer);
            }
            let known = conversation
                .entries
                .values()
                .any(|held| held.kind == EntryKind::Received && held.id == message)
                || conversation.forgotten.contains(&message);
            if known {
                return Ok(Receipt::Duplicate);
            }
        }
        self.insert(i, c, p, EntryKind::Received, message, len)
            .map(Receipt::Stored)
    }

    fn acknowledge(&mut self, i: usize, c: usize, message: u8) -> Acknowledgement {
        let Some(conversation) = self.conversations.get_mut(&(i, c)) else {
            return Acknowledgement::Unknown;
        };
        let Some((sequence, held)) = conversation
            .entries
            .iter_mut()
            .find(|(_, held)| held.kind != EntryKind::Received && held.id == message)
        else {
            return Acknowledgement::Unknown;
        };
        if held.kind == EntryKind::Delivered {
            return Acknowledgement::AlreadyDelivered;
        }
        held.kind = EntryKind::Delivered;
        Acknowledgement::Delivered(*sequence)
    }

    fn delete(&mut self, i: usize, c: usize, keep: bool) -> bool {
        let Some(conversation) = self.conversations.get_mut(&(i, c)) else {
            return false;
        };
        if keep {
            let had = !conversation.entries.is_empty();
            let received: Vec<u8> = conversation
                .entries
                .values()
                .filter(|held| held.kind == EntryKind::Received)
                .map(|held| held.id)
                .collect();
            conversation.forgotten.extend(received);
            conversation.entries.clear();
            had
        } else {
            self.conversations.remove(&(i, c));
            true
        }
    }

    fn remove(&mut self, i: usize) -> bool {
        let before = self.conversations.len();
        self.conversations.retain(|(held, _), _| *held != i);
        before != self.conversations.len()
    }

    fn head(&self, i: usize, c: usize) -> Option<u8> {
        self.conversations.get(&(i, c)).and_then(|conversation| {
            conversation
                .entries
                .values()
                .find(|held| held.kind == EntryKind::Queued)
                .map(|held| held.id)
        })
    }
}

/// The state as the model would hold it.
fn project(state: &MessageState) -> Model {
    let peers: Vec<IdentityPublicKey> = (0..3).map(peer).collect();
    let contents = state.contents();
    let mut model = Model::default();
    for stored in contents.identities {
        let i = usize::from(stored.identity.as_bytes()[0] - 1);
        for held in stored.conversations {
            let c = usize::from(held.contact.as_bytes()[0] - 0x10) - i * 2;
            let p = peers.iter().position(|known| *known == held.peer).unwrap();
            let conversation = Conversation {
                peer: p,
                next: held.next_sequence,
                entries: held
                    .entries
                    .iter()
                    .map(|entry| {
                        (
                            entry.sequence,
                            Held {
                                kind: entry.kind,
                                id: entry.id.as_bytes()[0],
                                len: entry.text.as_bytes().len(),
                            },
                        )
                    })
                    .collect(),
                forgotten: held.forgotten.iter().map(|id| id.as_bytes()[0]).collect(),
            };
            model.conversations.insert((i, c), conversation);
        }
    }
    model
}

fn text(len: usize) -> ChatText {
    ChatText::new(&"m".repeat(len)).unwrap()
}

fuzz_target!(|data: &[u8]| {
    let mut state = MessageState::new();
    let mut model = Model::default();
    for operation in data.chunks_exact(3) {
        let (op, target, arg) = (operation[0] % 7, operation[1], operation[2]);
        let i = usize::from(target & 1);
        let c = usize::from((target >> 1) & 1);
        let message = arg & 7;
        let len = usize::from((arg >> 3) & 3) + 1;
        let delivered_before: Vec<(usize, usize, u64)> = model
            .conversations
            .iter()
            .flat_map(|(key, conversation)| {
                conversation
                    .entries
                    .iter()
                    .filter(|(_, held)| held.kind == EntryKind::Delivered)
                    .map(move |(sequence, _)| (key.0, key.1, *sequence))
            })
            .collect();
        match op {
            0 | 6 => {
                let p = if op == 0 { c } else { 2 };
                let expected = model.queue(i, c, p, message, len);
                let got = state.queue(
                    identity(i),
                    record(i, c),
                    peer(p),
                    id(message),
                    text(len),
                    0,
                );
                assert_eq!(got, expected);
            }
            1 => {
                let expected = model.receive(i, c, c, message, len);
                let got = state.receive(
                    identity(i),
                    record(i, c),
                    peer(c),
                    id(message),
                    text(len),
                    0,
                );
                assert_eq!(got, expected);
            }
            2 => {
                let expected = model.acknowledge(i, c, message);
                let got = state.acknowledge(&identity(i), &record(i, c), &id(message));
                assert_eq!(got, expected);
            }
            3 => {
                let keep = arg & 1 == 1;
                let expected = model.delete(i, c, keep);
                let got = state.delete_conversation(&identity(i), &record(i, c), keep);
                assert_eq!(got, expected);
            }
            4 => {
                let expected = model.remove(i);
                assert_eq!(state.remove_identity(&identity(i)), expected);
            }
            _ => {
                let encoded = state.contents().encode().unwrap();
                state = MessageState::load(MessageContents::decode(&encoded).unwrap());
            }
        }
        // The state is the model's.
        assert_eq!(project(&state), model);
        let encoded = state.contents().encode().unwrap();
        assert_eq!(encoded.len(), state.payload_len());
        for i in 0..2 {
            for c in 0..2 {
                let head = state
                    .head(&identity(i), &record(i, c))
                    .map(|(_, held, _)| held.as_bytes()[0]);
                assert_eq!(head, model.head(i, c));
            }
        }
        // A delivered message of a conversation that lives stays delivered.
        for (i, c, sequence) in delivered_before {
            if let Some(conversation) = model.conversations.get(&(i, c)) {
                if let Some(held) = conversation.entries.get(&sequence) {
                    assert_eq!(held.kind, EntryKind::Delivered);
                }
            }
        }
    }
});

//! The message state machine against a model written here, apart from it
//! (`docs/TEST_PLAN.md` T-MSG-11): two local identities with two contact
//! records each, and any sequence of queueing, receiving, acknowledging,
//! deleting a conversation, removing an identity and restarting through
//! the encoded payload. After every operation the state holds what the
//! model holds, every outcome is the model's, the head of every queue is
//! its oldest queued message, and a delivered message never becomes queued
//! again while its conversation lives. The fuzz target `message_sequence`
//! runs the same comparison on inputs a fuzzer finds.

// Test code builds its own inputs.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::collections::{BTreeMap, VecDeque};

use monolith_core::messages::{Acknowledgement, MessageState, Receipt, Refusal};
use monolith_identity::{IdentityPublicKey, IdentitySecretKey};
use monolith_protocol::body::MessageId;
use monolith_protocol::limits::{
    MAX_QUEUED_MESSAGES_PER_CONTACT, MAX_QUEUED_MESSAGES_PER_IDENTITY,
};
use monolith_protocol::text::ChatText;
use monolith_storage::message_records::{EntryKind, MessageContents};
use monolith_storage::record::{ContactId, IdentityId};
use proptest::prelude::*;

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

fn text(len: usize) -> ChatText {
    ChatText::new(&"m".repeat(len)).unwrap()
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

fn is_queued(held: &Held) -> bool {
    held.kind == EntryKind::Queued
}

impl Model {
    fn queued_at(&self, identity: usize) -> usize {
        self.conversations
            .iter()
            .filter(|((held, _), _)| *held == identity)
            .map(|(_, conversation)| {
                conversation
                    .entries
                    .values()
                    .filter(|held| is_queued(held))
                    .count()
            })
            .sum()
    }

    fn store(&mut self, i: usize, c: usize, p: usize, held: Held) -> Result<u64, Refusal> {
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
        conversation.entries.insert(sequence, held);
        Ok(sequence)
    }

    fn queue(
        &mut self,
        i: usize,
        c: usize,
        p: usize,
        message: u8,
        len: usize,
    ) -> Result<u64, Refusal> {
        let at_identity = self.queued_at(i);
        if let Some(conversation) = self.conversations.get(&(i, c)) {
            let outbound = |held: &&Held| held.kind != EntryKind::Received;
            if conversation
                .entries
                .values()
                .filter(outbound)
                .any(|held| held.id == message)
            {
                return Err(Refusal::IdTaken);
            }
            if conversation
                .entries
                .values()
                .filter(|held| is_queued(held))
                .count()
                >= MAX_QUEUED_MESSAGES_PER_CONTACT
            {
                return Err(Refusal::ContactQueueFull);
            }
        }
        if at_identity >= MAX_QUEUED_MESSAGES_PER_IDENTITY {
            return Err(Refusal::IdentityQueueFull);
        }
        self.store(
            i,
            c,
            p,
            Held {
                kind: EntryKind::Queued,
                id: message,
                len,
            },
        )
    }

    fn receive(&mut self, i: usize, c: usize, message: u8, len: usize) -> Result<Receipt, Refusal> {
        if let Some(conversation) = self.conversations.get(&(i, c)) {
            if conversation.peer != c {
                return Err(Refusal::OtherPeer);
            }
            let held = conversation
                .entries
                .values()
                .any(|held| held.kind == EntryKind::Received && held.id == message);
            if held || conversation.forgotten.contains(&message) {
                return Ok(Receipt::Duplicate);
            }
        }
        self.store(
            i,
            c,
            c,
            Held {
                kind: EntryKind::Received,
                id: message,
                len,
            },
        )
        .map(Receipt::Stored)
    }

    fn acknowledge(&mut self, i: usize, c: usize, message: u8) -> Acknowledgement {
        let Some(conversation) = self.conversations.get_mut(&(i, c)) else {
            return Acknowledgement::Unknown;
        };
        let found = conversation
            .entries
            .iter_mut()
            .find(|(_, held)| held.kind != EntryKind::Received && held.id == message);
        match found {
            None => Acknowledgement::Unknown,
            Some((_, held)) if held.kind == EntryKind::Delivered => {
                Acknowledgement::AlreadyDelivered
            }
            Some((sequence, held)) => {
                held.kind = EntryKind::Delivered;
                Acknowledgement::Delivered(*sequence)
            }
        }
    }

    fn delete(&mut self, i: usize, c: usize, keep: bool) -> bool {
        let Some(conversation) = self.conversations.get_mut(&(i, c)) else {
            return false;
        };
        if !keep {
            self.conversations.remove(&(i, c));
            return true;
        }
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
    }

    fn remove(&mut self, i: usize) -> bool {
        let before = self.conversations.len();
        self.conversations.retain(|(held, _), _| *held != i);
        before != self.conversations.len()
    }

    fn head(&self, i: usize, c: usize) -> Option<u8> {
        self.conversations
            .get(&(i, c))
            .and_then(|conversation| conversation.entries.values().find(|held| is_queued(held)))
            .map(|held| held.id)
    }
}

/// The state as the model would hold it.
fn project(state: &MessageState) -> Model {
    let peers: Vec<IdentityPublicKey> = (0..3).map(peer).collect();
    let mut model = Model::default();
    for stored in state.contents().identities {
        let i = usize::from(stored.identity.as_bytes()[0] - 1);
        for held in stored.conversations {
            let c = usize::from(held.contact.as_bytes()[0] - 0x10) - i * 2;
            let conversation = Conversation {
                peer: peers.iter().position(|known| *known == held.peer).unwrap(),
                next: held.next_sequence,
                entries: held
                    .entries
                    .iter()
                    .map(|entry| {
                        let held = Held {
                            kind: entry.kind,
                            id: entry.id.as_bytes()[0],
                            len: entry.text.as_bytes().len(),
                        };
                        (entry.sequence, held)
                    })
                    .collect(),
                forgotten: held.forgotten.iter().map(|id| id.as_bytes()[0]).collect(),
            };
            model.conversations.insert((i, c), conversation);
        }
    }
    model
}

#[derive(Clone, Copy, Debug)]
enum Operation {
    Queue {
        i: usize,
        c: usize,
        id: u8,
        len: usize,
    },
    QueueOtherPeer {
        i: usize,
        c: usize,
        id: u8,
    },
    Receive {
        i: usize,
        c: usize,
        id: u8,
        len: usize,
    },
    Acknowledge {
        i: usize,
        c: usize,
        id: u8,
    },
    Delete {
        i: usize,
        c: usize,
        keep: bool,
    },
    Remove {
        i: usize,
    },
    Restart,
}

fn operation() -> impl Strategy<Value = Operation> {
    let at = (0..2_usize, 0..2_usize);
    prop_oneof![
        3 => (at.clone(), 0..8_u8, 1..5_usize).prop_map(|((i, c), id, len)| Operation::Queue { i, c, id, len }),
        1 => (at.clone(), 0..8_u8).prop_map(|((i, c), id)| Operation::QueueOtherPeer { i, c, id }),
        3 => (at.clone(), 0..8_u8, 1..5_usize).prop_map(|((i, c), id, len)| Operation::Receive { i, c, id, len }),
        3 => (at.clone(), 0..8_u8).prop_map(|((i, c), id)| Operation::Acknowledge { i, c, id }),
        1 => (at, any::<bool>()).prop_map(|((i, c), keep)| Operation::Delete { i, c, keep }),
        1 => (0..2_usize).prop_map(|i| Operation::Remove { i }),
        1 => Just(Operation::Restart),
    ]
}

fn delivered(model: &Model) -> Vec<(usize, usize, u64)> {
    model
        .conversations
        .iter()
        .flat_map(|(key, conversation)| {
            conversation
                .entries
                .iter()
                .filter(|(_, held)| held.kind == EntryKind::Delivered)
                .map(move |(sequence, _)| (key.0, key.1, *sequence))
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn the_state_machine_holds_what_the_model_holds(operations in prop::collection::vec(operation(), 1..80)) {
        let mut state = MessageState::new();
        let mut model = Model::default();
        for operation in operations {
            let before = delivered(&model);
            match operation {
                Operation::Queue { i, c, id: message, len } => {
                    let expected = model.queue(i, c, c, message, len);
                    let got = state.queue(identity(i), record(i, c), peer(c), id(message), text(len), 0);
                    prop_assert_eq!(got, expected);
                }
                Operation::QueueOtherPeer { i, c, id: message } => {
                    let expected = model.queue(i, c, 2, message, 1);
                    let got = state.queue(identity(i), record(i, c), peer(2), id(message), text(1), 0);
                    prop_assert_eq!(got, expected);
                }
                Operation::Receive { i, c, id: message, len } => {
                    let expected = model.receive(i, c, message, len);
                    let got = state.receive(identity(i), record(i, c), peer(c), id(message), text(len), 0);
                    prop_assert_eq!(got, expected);
                }
                Operation::Acknowledge { i, c, id: message } => {
                    let expected = model.acknowledge(i, c, message);
                    prop_assert_eq!(state.acknowledge(&identity(i), &record(i, c), &id(message)), expected);
                }
                Operation::Delete { i, c, keep } => {
                    let expected = model.delete(i, c, keep);
                    prop_assert_eq!(state.delete_conversation(&identity(i), &record(i, c), keep), expected);
                }
                Operation::Remove { i } => {
                    let expected = model.remove(i);
                    prop_assert_eq!(state.remove_identity(&identity(i)), expected);
                }
                Operation::Restart => {
                    let encoded = state.contents().encode().unwrap();
                    state = MessageState::load(MessageContents::decode(&encoded).unwrap());
                }
            }
            prop_assert_eq!(project(&state), model.clone());
            prop_assert_eq!(state.contents().encode().unwrap().len(), state.payload_len());
            for i in 0..2 {
                for c in 0..2 {
                    let head = state.head(&identity(i), &record(i, c)).map(|(_, held, _)| held.as_bytes()[0]);
                    prop_assert_eq!(head, model.head(i, c));
                }
            }
            for (i, c, sequence) in before {
                if let Some(held) = model.conversations.get(&(i, c)).and_then(|conversation| conversation.entries.get(&sequence)) {
                    prop_assert_eq!(held.kind, EntryKind::Delivered);
                }
            }
        }
    }
}

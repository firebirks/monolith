//! The payload of the message store: `MessageContents::decode` on
//! arbitrary bytes either refuses them or gives contents within every
//! ceiling of the format (`message_records::ceiling`, which the limits of
//! `docs/RESOURCE_LIMITS.md` section 5 never exceed: a store written under
//! higher limits is read, P7-5), in increasing order of
//! identity and of conversation, with entries in increasing sequence below
//! the next one; their encoding, with the zero padding the store adds, is
//! the input itself and decodes to the same contents; and the state loaded
//! from them encodes to the same contents, with the exact length it
//! accounts for. The payload is authenticated before it is read, so this
//! is the parser a changed or damaged store would reach only with the key;
//! it must still never panic or allocate by what it reads.
//!
//! The input is the payload.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_core::messages::MessageState;
use monolith_storage::message_records::{MessageContents, ceiling};

fuzz_target!(|data: &[u8]| {
    let Ok(contents) = MessageContents::decode(data) else {
        return;
    };
    assert!(contents.identities.len() <= ceiling::LOCAL_IDENTITIES);
    assert!(
        contents
            .identities
            .windows(2)
            .all(|pair| pair[0].identity < pair[1].identity)
    );
    let mut entries = 0;
    for identity in &contents.identities {
        assert!(identity.conversations.len() <= ceiling::CONVERSATIONS_PER_IDENTITY);
        assert!(
            identity
                .conversations
                .windows(2)
                .all(|pair| pair[0].contact < pair[1].contact)
        );
        let mut queued = 0;
        for conversation in &identity.conversations {
            assert!(conversation.forgotten.len() <= ceiling::FORGOTTEN_PER_CONVERSATION);
            assert!(
                conversation
                    .entries
                    .windows(2)
                    .all(|pair| pair[0].sequence < pair[1].sequence)
            );
            assert!(
                conversation
                    .entries
                    .iter()
                    .all(|entry| entry.sequence < conversation.next_sequence)
            );
            assert!(conversation.queued() <= ceiling::QUEUED_PER_CONTACT);
            assert!(conversation.entries_len() <= ceiling::CONVERSATION_LEN);
            queued += conversation.queued();
            entries += conversation.entries.len();
        }
        assert!(queued <= ceiling::QUEUED_PER_IDENTITY);
    }
    assert!(entries <= ceiling::MESSAGES);
    // One valid encoding: what was read is the encoding, then zeros.
    let encoded = contents.encode().unwrap();
    assert!(data.starts_with(&encoded));
    assert!(data[encoded.len()..].iter().all(|byte| *byte == 0));
    assert_eq!(MessageContents::decode(&encoded).unwrap(), contents);
    // The state holds what was read, and knows its length.
    let state = MessageState::load(contents.clone());
    assert_eq!(state.contents(), contents);
    assert_eq!(state.payload_len(), encoded.len());
    assert_eq!(state.stored(), entries);
});

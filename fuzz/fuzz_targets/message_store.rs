//! The payload of the message store: `MessageContents::decode` on
//! arbitrary bytes either refuses them or gives contents within every
//! limit of `docs/RESOURCE_LIMITS.md` section 5, in increasing order of
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
use monolith_protocol::limits::{
    MAX_CONVERSATION_STORE_LEN, MAX_CONVERSATIONS_PER_IDENTITY, MAX_LOCAL_IDENTITIES,
    MAX_QUEUED_MESSAGES_PER_CONTACT, MAX_QUEUED_MESSAGES_PER_IDENTITY, MAX_STORED_MESSAGES,
    MESSAGE_DEDUP_WINDOW,
};
use monolith_storage::message_records::MessageContents;

fuzz_target!(|data: &[u8]| {
    let Ok(contents) = MessageContents::decode(data) else {
        return;
    };
    assert!(contents.identities.len() <= MAX_LOCAL_IDENTITIES);
    assert!(
        contents
            .identities
            .windows(2)
            .all(|pair| pair[0].identity < pair[1].identity)
    );
    let mut entries = 0;
    for identity in &contents.identities {
        assert!(identity.conversations.len() <= MAX_CONVERSATIONS_PER_IDENTITY);
        assert!(
            identity
                .conversations
                .windows(2)
                .all(|pair| pair[0].contact < pair[1].contact)
        );
        let mut queued = 0;
        for conversation in &identity.conversations {
            assert!(conversation.forgotten.len() <= MESSAGE_DEDUP_WINDOW);
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
            assert!(conversation.queued() <= MAX_QUEUED_MESSAGES_PER_CONTACT);
            assert!(conversation.entries_len() <= MAX_CONVERSATION_STORE_LEN);
            queued += conversation.queued();
            entries += conversation.entries.len();
        }
        assert!(queued <= MAX_QUEUED_MESSAGES_PER_IDENTITY);
    }
    assert!(entries <= MAX_STORED_MESSAGES);
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

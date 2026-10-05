//! The payload of a vault: `Contents::decode` on arbitrary bytes either
//! refuses them or gives contents within every limit whose encoding, with
//! the zero padding the vault adds, is the input itself, and decodes to
//! the same contents. The payload is authenticated before it is read, so
//! this is the parser a changed or damaged vault would reach only with
//! the key; it must still never panic or allocate by what it reads.
//!
//! The input is the payload.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_protocol::limits::{
    MAX_ACTIVE_INVITATIONS, MAX_BLOCKED_IDENTITIES, MAX_CONTACTS, MAX_DECLINED_IDENTITIES,
    MAX_LOCAL_IDENTITIES,
};
use monolith_storage::record::Contents;

fuzz_target!(|data: &[u8]| {
    let Ok(contents) = Contents::decode(data) else {
        return;
    };
    assert!(contents.identities.len() <= MAX_LOCAL_IDENTITIES);
    for identity in &contents.identities {
        assert!(identity.invitations.len() <= MAX_ACTIVE_INVITATIONS);
        assert!(identity.contacts.len() <= MAX_CONTACTS);
        assert!(identity.blocked.len() <= MAX_BLOCKED_IDENTITIES);
        assert!(identity.declined.len() <= MAX_DECLINED_IDENTITIES);
        for contact in &identity.contacts {
            assert_eq!(contact.dial.identity(), contact.credentials.identity());
        }
    }
    // One valid encoding: what was read is the encoding, then zeros.
    let encoded = contents.encode().unwrap();
    assert!(data.starts_with(&encoded));
    assert!(data[encoded.len()..].iter().all(|byte| *byte == 0));
    assert_eq!(Contents::decode(&encoded).unwrap(), contents);
});

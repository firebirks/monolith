//! Base32 decoder: arbitrary text either fails or decodes to bytes that
//! encode back to the same text in upper case, within the stated limit.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_identity::base32;

const LIMIT: usize = 256;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = core::str::from_utf8(data) else {
        return;
    };
    if let Ok(bytes) = base32::decode(text, LIMIT) {
        assert!(bytes.len() <= LIMIT);
        assert_eq!(base32::encode(&bytes), text.to_ascii_uppercase());
    }
});

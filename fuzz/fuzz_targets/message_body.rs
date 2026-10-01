//! Message body decoder: for every message type, arbitrary bytes either fail
//! or decode to a message that encodes back to exactly the same bytes.
//!
//! The first input byte selects the message type.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_protocol::MessageType;
use monolith_protocol::body::Message;

fuzz_target!(|data: &[u8]| {
    let Some((selector, body)) = data.split_first() else {
        return;
    };
    let types = MessageType::ALL;
    let message_type = types[usize::from(*selector) % types.len()];
    if let Ok(message) = Message::decode(message_type, body) {
        assert_eq!(message.message_type(), message_type);
        assert_eq!(message.encode_body().as_deref(), Ok(body));
    }
});

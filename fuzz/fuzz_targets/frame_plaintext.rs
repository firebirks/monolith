//! Frame plaintext decoder: arbitrary bytes in any session state either fail
//! or yield a message type that is legal in that state and a body, and
//! encoding them again gives exactly the input.
//!
//! The first input byte selects the session state.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_protocol::SessionState;
use monolith_protocol::body::Message;
use monolith_protocol::frame::{FrameParams, decode_plaintext, encode_plaintext};

const STATES: [SessionState; 7] = [
    SessionState::Connecting,
    SessionState::CryptoHandshake,
    SessionState::IdentityAuth,
    SessionState::AuthenticatedUnknown,
    SessionState::AuthenticatedContact,
    SessionState::Closing,
    SessionState::Closed,
];

fuzz_target!(|data: &[u8]| {
    let Some((selector, plaintext)) = data.split_first() else {
        return;
    };
    let state = STATES[usize::from(*selector) % STATES.len()];
    let params = FrameParams::PROVISIONAL;

    if let Ok((message_type, body)) = decode_plaintext(&params, state, plaintext) {
        assert!(message_type.may_be_received_in(state));
        assert!(plaintext.len() <= params.max_plaintext_len_in(state));
        assert_eq!(
            encode_plaintext(&params, message_type, body).as_deref(),
            Ok(plaintext)
        );
        // The body goes on to the message decoder, which must not panic and
        // must be canonical as well.
        if let Ok(message) = Message::decode(message_type, body) {
            assert_eq!(message.encode_body().as_deref(), Ok(body));
        }
    }
});

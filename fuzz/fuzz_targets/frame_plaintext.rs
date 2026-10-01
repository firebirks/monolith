//! Frame plaintext decoder: a plaintext in any session state either fails or
//! yields a message type that is legal in that state and a body, and
//! encoding them again gives exactly the input.
//!
//! Input: the first byte selects the session state, the second the mode.
//!
//! - Even mode: the rest is the plaintext as it is.
//! - Odd mode: the rest is a type selector, a damage position (two bytes), a
//!   damage value, and a body. A valid frame is built from the type and the
//!   body. If the damage value is not zero, the byte at the damage position
//!   (modulo the frame length) is XORed with it. This gets the fuzzer past
//!   the padding check, which random bytes almost never pass.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_protocol::body::Message;
use monolith_protocol::frame::{FrameParams, decode_plaintext, encode_plaintext};
use monolith_protocol::{MessageType, SessionState};

const STATES: [SessionState; 7] = [
    SessionState::Connecting,
    SessionState::CryptoHandshake,
    SessionState::IdentityAuth,
    SessionState::AuthenticatedUnknown,
    SessionState::AuthenticatedContact,
    SessionState::Closing,
    SessionState::Closed,
];

const PARAMS: FrameParams = FrameParams::PROVISIONAL;

fn check(state: SessionState, plaintext: &[u8]) -> bool {
    let Ok((message_type, body)) = decode_plaintext(&PARAMS, state, plaintext) else {
        return false;
    };
    assert!(message_type.may_be_received_in(state));
    assert!(plaintext.len() <= PARAMS.max_plaintext_len_in(state));
    assert_eq!(
        encode_plaintext(&PARAMS, message_type, body).as_deref(),
        Ok(plaintext)
    );
    // The body goes on to the message decoder, which must not panic and
    // must be canonical as well.
    if let Ok(message) = Message::decode(message_type, body) {
        assert_eq!(message.message_type(), message_type);
        assert_eq!(message.encode_body().as_deref(), Ok(body));
    }
    true
}

fuzz_target!(|data: &[u8]| {
    let [state_selector, mode, rest @ ..] = data else {
        return;
    };
    let state = STATES[usize::from(*state_selector) % STATES.len()];

    if mode % 2 == 0 {
        check(state, rest);
        return;
    }

    let [
        type_selector,
        position_high,
        position_low,
        damage,
        body @ ..,
    ] = rest
    else {
        return;
    };
    let types = MessageType::ALL;
    let message_type = types[usize::from(*type_selector) % types.len()];
    let Ok(mut plaintext) = encode_plaintext(&PARAMS, message_type, body) else {
        return;
    };
    if *damage == 0 {
        // An undamaged frame is accepted exactly when its type is legal in
        // the state and its length is within the limit of the state.
        let expected = message_type.may_be_received_in(state)
            && plaintext.len() <= PARAMS.max_plaintext_len_in(state);
        assert_eq!(check(state, &plaintext), expected);
        return;
    }
    let position = usize::from(u16::from_be_bytes([*position_high, *position_low]));
    let index = position % plaintext.len();
    plaintext[index] ^= damage;
    check(state, &plaintext);
});

//! Outer frame decoder: an arbitrary byte stream, delivered in arbitrary
//! pieces, either fails or yields payloads of legal length, and the decoder
//! never holds more than one frame of the size the session state allows.
//!
//! The first input byte selects the session state, the second the piece
//! size. Each payload is then passed down the rest of the receive path with
//! the session overhead stripped, as if the session layer had removed it.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_protocol::SessionState;
use monolith_protocol::body::Message;
use monolith_protocol::frame::{FrameParams, OuterDecoder, decode_plaintext};

fuzz_target!(|data: &[u8]| {
    let [state_selector, piece_selector, stream @ ..] = data else {
        return;
    };
    let state = if state_selector % 2 == 0 {
        SessionState::AuthenticatedContact
    } else {
        SessionState::AuthenticatedUnknown
    };
    let piece = usize::from(*piece_selector) + 1;
    let params = FrameParams::PROVISIONAL;
    let limit = params.max_plaintext_len_in(state) + params.overhead();

    let mut decoder = OuterDecoder::new(params);
    for chunk in stream.chunks(piece) {
        let mut rest = chunk;
        while !rest.is_empty() {
            let Ok((used, frame)) = decoder.feed(rest, state) else {
                return;
            };
            assert!(used > 0 && used <= rest.len());
            rest = &rest[used..];
            assert!(decoder.buffered() <= limit);

            if let Some(payload) = frame {
                assert!(payload.len() <= limit);
                assert!(params.check_outer_len(payload.len(), state).is_ok());
                let plaintext = &payload[..payload.len() - params.overhead()];
                if let Ok((message_type, body)) = decode_plaintext(&params, state, plaintext) {
                    let _ = Message::decode(message_type, body);
                }
            }
        }
    }
});

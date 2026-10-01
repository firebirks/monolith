//! Outer frame decoder: a byte stream, delivered in arbitrary pieces, either
//! fails or yields payloads of legal length, and the decoder never holds
//! more than one frame of the size the session state allows.
//!
//! Input: the first byte selects the session state, the second the piece
//! size, the third the mode.
//!
//! - Even mode: the rest is the stream as it is.
//! - Odd mode: the rest is a damage position (two bytes), a damage value,
//!   and records of up to 48 bytes. Each record becomes one valid frame: its
//!   first byte selects the message type, the others are the body. If the
//!   damage value is not zero, one byte of the resulting stream is XORed
//!   with it. Undamaged, every frame must come out again.
//!
//! Each payload is then passed down the rest of the receive path with the
//! session overhead stripped, as if the session layer had removed it.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_protocol::body::Message;
use monolith_protocol::frame::{
    FrameParams, OuterDecoder, decode_plaintext, encode_outer, encode_plaintext,
};
use monolith_protocol::{MessageType, SessionState};

const PARAMS: FrameParams = FrameParams::PROVISIONAL;

/// Longest record in odd mode.
const RECORD_LEN: usize = 48;

/// Feeds the stream in pieces. Returns the number of frames that came out,
/// or `None` if the decoder failed.
fn run(state: SessionState, piece: usize, stream: &[u8]) -> Option<usize> {
    let limit = PARAMS.max_plaintext_len_in(state) + PARAMS.overhead();
    let mut decoder = OuterDecoder::new(PARAMS);
    let mut frames = 0_usize;
    for chunk in stream.chunks(piece) {
        let mut rest = chunk;
        while !rest.is_empty() {
            let Ok((used, frame)) = decoder.feed(rest, state) else {
                // A failed decoder stays failed.
                assert!(decoder.feed(&[0x04, 0x10], state).is_err());
                return None;
            };
            assert!(used > 0 && used <= rest.len());
            rest = &rest[used..];
            assert!(decoder.buffered() <= limit);

            if let Some(payload) = frame {
                frames += 1;
                assert!(payload.len() <= limit);
                assert!(PARAMS.check_outer_len(payload.len(), state).is_ok());
                let plaintext = &payload[..payload.len() - PARAMS.overhead()];
                if let Ok((message_type, body)) = decode_plaintext(&PARAMS, state, plaintext) {
                    if let Ok(message) = Message::decode(message_type, body) {
                        assert_eq!(message.encode_body().as_deref(), Ok(body));
                    }
                }
            }
        }
    }
    Some(frames)
}

fuzz_target!(|data: &[u8]| {
    let [state_selector, piece_selector, mode, rest @ ..] = data else {
        return;
    };
    let state = if state_selector % 2 == 0 {
        SessionState::AuthenticatedContact
    } else {
        SessionState::AuthenticatedUnknown
    };
    let piece = usize::from(*piece_selector) + 1;

    if mode % 2 == 0 {
        run(state, piece, rest);
        return;
    }

    let [position_high, position_low, damage, records @ ..] = rest else {
        return;
    };
    let types = MessageType::ALL;
    let mut stream = Vec::new();
    let mut count = 0_usize;
    for record in records.chunks(RECORD_LEN) {
        let Some((type_selector, body)) = record.split_first() else {
            continue;
        };
        let message_type = types[usize::from(*type_selector) % types.len()];
        let mut payload = encode_plaintext(&PARAMS, message_type, body).unwrap();
        // What the session layer would add.
        payload.resize(payload.len() + PARAMS.overhead(), 0);
        stream.extend_from_slice(&encode_outer(&PARAMS, &payload).unwrap());
        count += 1;
    }
    if stream.is_empty() {
        return;
    }
    if *damage == 0 {
        // The outer decoder does not look at message types, and every
        // record fits in one block, which every receiving state allows.
        assert_eq!(run(state, piece, &stream), Some(count));
        return;
    }
    let position = usize::from(u16::from_be_bytes([*position_high, *position_low]));
    let index = position % stream.len();
    stream[index] ^= damage;
    run(state, piece, &stream);
});

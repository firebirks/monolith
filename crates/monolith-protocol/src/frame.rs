//! Frames.
//!
//! Two layers, both defined in `docs/PROTOCOL.md` section 5:
//!
//! - The frame plaintext: message type, body length, body, and zero padding
//!   up to a multiple of the padding block. See [`encode_plaintext`] and
//!   [`decode_plaintext`].
//! - The outer frame on the stream: a 16-bit length and that many bytes of
//!   payload. The payload is the plaintext as the session layer transformed
//!   it. This module does not know how; it only knows how many bytes the
//!   session layer adds. See [`encode_outer`] and [`OuterDecoder`].
//!
//! Nothing here does I/O or cryptography. The padding block and the session
//! overhead are parameters ([`FrameParams`]), because neither is decided:
//! see `docs/adr/0003-wire-format.md` and `docs/adr/0002-session-protocol.md`.

use core::fmt;

use crate::limits::{
    FIELD_LENGTH_PREFIX_LEN, FRAME_LENGTH_PREFIX_LEN, FRAME_PADDING_BLOCK_LEN,
    FRAME_SESSION_OVERHEAD_LEN, MAX_FILE_CHUNK_LEN, MAX_FRAME_LENGTH_VALUE, MAX_NON_CHUNK_BODY_LEN,
    MAX_UNCONFIRMED_BODY_LEN, MESSAGE_HEADER_LEN, TRANSFER_ID_LEN,
};
use crate::{MessageType, ProtocolError, SessionState};

/// The two numbers the frame format depends on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameParams {
    padding_block: usize,
    overhead: usize,
}

impl FrameParams {
    /// The working values: a 1024-byte padding block and 16 bytes of session
    /// overhead. Both are provisional.
    pub const PROVISIONAL: Self = Self {
        padding_block: FRAME_PADDING_BLOCK_LEN,
        overhead: FRAME_SESSION_OVERHEAD_LEN,
    };

    /// Builds a parameter set.
    ///
    /// `padding_block` is the block the plaintext is padded to. `overhead`
    /// is what the session layer adds to each frame. Returns `None` if a
    /// block cannot hold a message header, if one block plus the overhead
    /// does not fit in a 16-bit length, or if the largest frame cannot hold
    /// every message other than a file chunk at its largest size. File
    /// chunks are cut to the frame; see [`Self::max_file_chunk_len`].
    pub const fn new(padding_block: usize, overhead: usize) -> Option<Self> {
        if padding_block <= MESSAGE_HEADER_LEN {
            return None;
        }
        let params = match padding_block.checked_add(overhead) {
            Some(smallest) if smallest <= MAX_FRAME_LENGTH_VALUE => Self {
                padding_block,
                overhead,
            },
            _ => return None,
        };
        // This covers the messages that are legal before confirmation as
        // well: none of them is larger.
        if params.padded_len(MAX_NON_CHUNK_BODY_LEN).is_none() {
            return None;
        }
        Some(params)
    }

    /// Returns the padding block.
    pub const fn padding_block(&self) -> usize {
        self.padding_block
    }

    /// Returns the session overhead per frame.
    pub const fn overhead(&self) -> usize {
        self.overhead
    }

    /// Returns the largest plaintext: the largest multiple of the padding
    /// block that, with the overhead, still fits in a 16-bit length.
    pub const fn max_plaintext_len(&self) -> usize {
        let room = MAX_FRAME_LENGTH_VALUE.saturating_sub(self.overhead);
        match room.checked_div(self.padding_block) {
            Some(blocks) => blocks.saturating_mul(self.padding_block),
            None => 0,
        }
    }

    /// Returns true if `len` is a whole number of padding blocks.
    const fn is_whole_blocks(&self, len: usize) -> bool {
        matches!(len.checked_rem(self.padding_block), Some(0))
    }

    /// Returns the largest message body.
    pub const fn max_body_len(&self) -> usize {
        self.max_plaintext_len().saturating_sub(MESSAGE_HEADER_LEN)
    }

    /// Returns the size of a full FileChunk: what is left of the largest
    /// body after the transfer identifier and the length of the data field,
    /// and never more than [`MAX_FILE_CHUNK_LEN`], which is what the body
    /// codec accepts.
    ///
    /// This is the chunk size of `docs/PROTOCOL.md` section 13.1 for these
    /// parameters. It is at least 1 for every parameter set that
    /// [`Self::new`] accepts.
    pub const fn max_file_chunk_len(&self) -> usize {
        let fits = self
            .max_body_len()
            .saturating_sub(TRANSFER_ID_LEN)
            .saturating_sub(FIELD_LENGTH_PREFIX_LEN);
        if fits < MAX_FILE_CHUNK_LEN {
            fits
        } else {
            MAX_FILE_CHUNK_LEN
        }
    }

    /// Returns the plaintext length of a message with a body of `body_len`
    /// bytes: header plus body, rounded up to a multiple of the padding
    /// block. `None` if that exceeds the largest plaintext.
    pub const fn padded_len(&self, body_len: usize) -> Option<usize> {
        let Some(unpadded) = body_len.checked_add(MESSAGE_HEADER_LEN) else {
            return None;
        };
        let blocks = unpadded.div_ceil(self.padding_block);
        let Some(padded) = blocks.checked_mul(self.padding_block) else {
            return None;
        };
        if padded > self.max_plaintext_len() {
            None
        } else {
            Some(padded)
        }
    }

    /// Returns the largest plaintext a peer may send in `state`.
    ///
    /// Before a session is confirmed it is the padded size of the largest
    /// message that is legal then. In states where nothing may be received
    /// it is zero.
    pub const fn max_plaintext_len_in(&self, state: SessionState) -> usize {
        match state {
            SessionState::IdentityAuth | SessionState::AuthenticatedUnknown => {
                match self.padded_len(MAX_UNCONFIRMED_BODY_LEN) {
                    Some(len) => len,
                    // Not reached: `new` refuses such parameters.
                    None => self.max_plaintext_len(),
                }
            }
            SessionState::AuthenticatedContact => self.max_plaintext_len(),
            SessionState::Connecting
            | SessionState::CryptoHandshake
            | SessionState::Closing
            | SessionState::Closed => 0,
        }
    }

    /// Checks the value of an outer length prefix for a frame received in
    /// `state` and returns the plaintext length it implies.
    ///
    /// This is the check that runs before a single byte of the frame is
    /// read or any buffer for it exists.
    pub const fn check_outer_len(
        &self,
        outer_len: usize,
        state: SessionState,
    ) -> Result<usize, ProtocolError> {
        let Some(plaintext_len) = outer_len.checked_sub(self.overhead) else {
            return Err(ProtocolError::FrameLengthOutOfRange);
        };
        if plaintext_len < self.padding_block
            || !self.is_whole_blocks(plaintext_len)
            || plaintext_len > self.max_plaintext_len_in(state)
        {
            return Err(ProtocolError::FrameLengthOutOfRange);
        }
        Ok(plaintext_len)
    }
}

/// Builds the plaintext of one frame: message type, body length, body, and
/// zero padding to a multiple of the padding block.
pub fn encode_plaintext(
    params: &FrameParams,
    message_type: MessageType,
    body: &[u8],
) -> Result<Vec<u8>, ProtocolError> {
    let total = params
        .padded_len(body.len())
        .ok_or(ProtocolError::BadMessageLength)?;
    let body_len = u16::try_from(body.len()).map_err(|_| ProtocolError::BadMessageLength)?;

    let mut plaintext = Vec::with_capacity(total);
    plaintext.extend_from_slice(&message_type.code().to_be_bytes());
    plaintext.extend_from_slice(&body_len.to_be_bytes());
    plaintext.extend_from_slice(body);
    plaintext.resize(total, 0);
    Ok(plaintext)
}

/// Takes the plaintext of one frame apart and returns the message type and
/// the body.
///
/// Checks, in this order: the length is a legal plaintext length for the
/// state; the header is present; the padding is exactly minimal for the
/// declared body length and consists of zero bytes; the type code is
/// assigned; the message is legal in `state`. The body is not parsed here,
/// and it is not looked at before the state check has passed.
pub fn decode_plaintext<'a>(
    params: &FrameParams,
    state: SessionState,
    plaintext: &'a [u8],
) -> Result<(MessageType, &'a [u8]), ProtocolError> {
    if plaintext.len() < params.padding_block()
        || !params.is_whole_blocks(plaintext.len())
        || plaintext.len() > params.max_plaintext_len_in(state)
    {
        return Err(ProtocolError::FrameLengthOutOfRange);
    }

    let (header, rest) = plaintext
        .split_at_checked(MESSAGE_HEADER_LEN)
        .ok_or(ProtocolError::FrameLengthOutOfRange)?;
    let (code_bytes, len_bytes) = header
        .split_at_checked(2)
        .ok_or(ProtocolError::FrameLengthOutOfRange)?;
    let code = u16::from_be_bytes(
        <[u8; 2]>::try_from(code_bytes).map_err(|_| ProtocolError::FrameLengthOutOfRange)?,
    );
    let body_len = usize::from(u16::from_be_bytes(
        <[u8; 2]>::try_from(len_bytes).map_err(|_| ProtocolError::FrameLengthOutOfRange)?,
    ));

    // The frame must be exactly as long as this body requires. That rules
    // out a body length that points past the frame and padding that is
    // longer than necessary.
    if params.padded_len(body_len) != Some(plaintext.len()) {
        return Err(ProtocolError::BadPadding);
    }
    let (body, padding) = rest
        .split_at_checked(body_len)
        .ok_or(ProtocolError::BadPadding)?;
    if padding.iter().any(|byte| *byte != 0) {
        return Err(ProtocolError::BadPadding);
    }

    let message_type = MessageType::from_code(code).ok_or(ProtocolError::UnknownMessageType)?;
    if !message_type.may_be_received_in(state) {
        return Err(ProtocolError::MessageNotPermitted);
    }
    Ok((message_type, body))
}

/// Puts the length prefix in front of a frame payload.
///
/// `payload` is a plaintext as the session layer transformed it, so its
/// length must be a legal plaintext length plus the session overhead.
pub fn encode_outer(params: &FrameParams, payload: &[u8]) -> Result<Vec<u8>, ProtocolError> {
    // A sender is not bound by the receiver's state-dependent limit; the
    // format limit is that of a confirmed session.
    params.check_outer_len(payload.len(), SessionState::AuthenticatedContact)?;
    let prefix = u16::try_from(payload.len()).map_err(|_| ProtocolError::FrameLengthOutOfRange)?;

    let mut frame = Vec::with_capacity(FRAME_LENGTH_PREFIX_LEN.saturating_add(payload.len()));
    frame.extend_from_slice(&prefix.to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

/// Splits a byte stream into frame payloads without ever holding more than
/// one frame.
///
/// Bytes are fed in whatever pieces they arrive. The decoder first collects
/// the two bytes of the length prefix, checks the declared length against
/// the limit for the current session state, and only then makes room for
/// the payload. A length that fails the check is an error before a single
/// payload byte is consumed, and after an error the decoder accepts nothing
/// more.
///
/// Once a Close was sent or received the caller stops feeding the decoder
/// and drops whatever is still in flight. The session is then in a state
/// where every length is refused, so feeding it would turn the tail of an
/// orderly close into an error.
///
/// The `Debug` output states sizes only, never buffered bytes.
pub struct OuterDecoder {
    params: FrameParams,
    prefix: [u8; FRAME_LENGTH_PREFIX_LEN],
    prefix_len: usize,
    expected: Option<usize>,
    payload: Vec<u8>,
    failed: bool,
}

impl fmt::Debug for OuterDecoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OuterDecoder")
            .field("params", &self.params)
            .field("expected", &self.expected)
            .field("buffered", &self.payload.len())
            .field("failed", &self.failed)
            .finish()
    }
}

impl OuterDecoder {
    /// Creates a decoder with nothing buffered.
    pub const fn new(params: FrameParams) -> Self {
        Self {
            params,
            prefix: [0; FRAME_LENGTH_PREFIX_LEN],
            prefix_len: 0,
            expected: None,
            payload: Vec::new(),
            failed: false,
        }
    }

    /// Returns the number of payload bytes currently buffered. Never more
    /// than one frame.
    pub fn buffered(&self) -> usize {
        self.payload.len()
    }

    /// Consumes bytes from `input` up to the end of one frame.
    ///
    /// Returns how many bytes were consumed and, if they completed a frame,
    /// its payload. The caller feeds the rest of `input` again. `state` is
    /// the session state at the time the length prefix completes; it decides
    /// how long the frame may be.
    ///
    /// A call never goes past the end of one frame, and the caller must
    /// pass that frame through the session before it feeds the rest: the
    /// message in it may change the state, and with it the limit for the
    /// frame that follows. A caller that collected several frames first
    /// would check the later ones against a state that is out of date.
    pub fn feed(
        &mut self,
        input: &[u8],
        state: SessionState,
    ) -> Result<(usize, Option<Vec<u8>>), ProtocolError> {
        if self.failed {
            return Err(ProtocolError::FrameLengthOutOfRange);
        }
        let mut rest = input;

        let expected = match self.expected {
            Some(expected) => expected,
            None => {
                let missing = FRAME_LENGTH_PREFIX_LEN.saturating_sub(self.prefix_len);
                let (head, tail) = rest.split_at(missing.min(rest.len()));
                for byte in head {
                    if let Some(slot) = self.prefix.get_mut(self.prefix_len) {
                        *slot = *byte;
                    }
                    self.prefix_len = self.prefix_len.saturating_add(1);
                }
                rest = tail;
                if self.prefix_len < FRAME_LENGTH_PREFIX_LEN {
                    return Ok((consumed(input, rest), None));
                }

                let declared = usize::from(u16::from_be_bytes(self.prefix));
                if let Err(error) = self.params.check_outer_len(declared, state) {
                    self.failed = true;
                    return Err(error);
                }
                // The declared length passed the check for this state, so
                // this reservation is bounded by the largest legal frame.
                self.payload = Vec::with_capacity(declared);
                self.expected = Some(declared);
                declared
            }
        };

        let missing = expected.saturating_sub(self.payload.len());
        let (head, tail) = rest.split_at(missing.min(rest.len()));
        self.payload.extend_from_slice(head);
        rest = tail;

        if self.payload.len() < expected {
            return Ok((consumed(input, rest), None));
        }
        self.prefix_len = 0;
        self.expected = None;
        let payload = core::mem::take(&mut self.payload);
        Ok((consumed(input, rest), Some(payload)))
    }
}

fn consumed(input: &[u8], rest: &[u8]) -> usize {
    input.len().saturating_sub(rest.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::{
        MAX_FILE_CHUNK_LEN, MAX_FRAME_CIPHERTEXT_LEN, MAX_FRAME_PLAINTEXT_LEN,
        MAX_MESSAGE_BODY_LEN, MIN_FRAME_CIPHERTEXT_LEN,
    };

    const P: FrameParams = FrameParams::PROVISIONAL;
    const CONTACT: SessionState = SessionState::AuthenticatedContact;
    const UNKNOWN: SessionState = SessionState::AuthenticatedUnknown;

    #[test]
    fn provisional_parameters_give_the_documented_sizes() {
        assert_eq!(P.max_plaintext_len(), MAX_FRAME_PLAINTEXT_LEN);
        assert_eq!(P.max_body_len(), MAX_MESSAGE_BODY_LEN);
        assert_eq!(P.max_plaintext_len_in(UNKNOWN), 1024);
        assert_eq!(P.max_plaintext_len_in(SessionState::IdentityAuth), 1024);
        assert_eq!(P.max_plaintext_len_in(CONTACT), 64_512);
        assert_eq!(P.max_plaintext_len_in(SessionState::Closing), 0);
        assert_eq!(
            P.max_body_len(),
            MAX_FILE_CHUNK_LEN + 16 + 2,
            "a full chunk fills a maximum frame"
        );
        assert_eq!(P.max_file_chunk_len(), MAX_FILE_CHUNK_LEN);
        // A frame with room to spare does not make chunks larger than the
        // body codec accepts, and a smaller frame makes them smaller.
        assert_eq!(
            FrameParams::new(65_535, 0).unwrap().max_file_chunk_len(),
            MAX_FILE_CHUNK_LEN
        );
        let small = FrameParams::new(1024, 48_000).unwrap();
        assert_eq!(small.max_body_len(), 17 * 1024 - 4);
        assert_eq!(small.max_file_chunk_len(), 17 * 1024 - 4 - 16 - 2);
        assert_eq!(
            FrameParams::new(FRAME_PADDING_BLOCK_LEN, FRAME_SESSION_OVERHEAD_LEN),
            Some(P),
            "the provisional values pass the checks of the constructor"
        );
    }

    #[test]
    fn parameters_are_validated() {
        assert_eq!(FrameParams::new(4, 0), None);
        assert_eq!(FrameParams::new(0, 0), None);
        assert_eq!(FrameParams::new(65_535, 1), None);
        assert!(FrameParams::new(65_535, 0).is_some());
        assert!(FrameParams::new(480, 16).is_some());
        assert_eq!(FrameParams::new(usize::MAX, 16), None);
        // One block of 500 with this overhead fits the length prefix, but no
        // second one does, and the largest contact request needs two.
        assert_eq!(FrameParams::new(500, 65_000), None);
        // Room for a contact request, but not for the longest chat message.
        assert_eq!(FrameParams::new(500, 64_000), None);
        assert_eq!(FrameParams::new(1024, 49_000), None);
        assert!(FrameParams::new(1024, 48_000).is_some());
        assert!(FrameParams::new(500, 40_000).is_some());
        // Whatever the constructor accepts can carry every message that is
        // legal before confirmation, in every state that receives any.
        for (block, overhead) in [(5, 0), (5, 65_000), (480, 16), (804, 0), (65_535, 0)] {
            let Some(params) = FrameParams::new(block, overhead) else {
                continue;
            };
            let needed = params.padded_len(MAX_UNCONFIRMED_BODY_LEN).unwrap();
            assert_eq!(params.max_plaintext_len_in(UNKNOWN), needed);
            assert!(needed <= params.max_plaintext_len());
            assert!(needed + overhead <= MAX_FRAME_LENGTH_VALUE);
            // And the longest chat message and at least one byte of file
            // data on a confirmed session.
            assert!(params.max_body_len() >= MAX_NON_CHUNK_BODY_LEN);
            assert!(params.max_file_chunk_len() >= 1);
            let chat = vec![0_u8; MAX_NON_CHUNK_BODY_LEN];
            assert!(encode_plaintext(&params, MessageType::ChatMessage, &chat).is_ok());
        }
    }

    #[test]
    fn other_parameters_follow_the_same_rules() {
        let small = FrameParams::new(480, 16).unwrap();
        assert_eq!(small.max_plaintext_len(), 136 * 480);
        // The largest unconfirmed message is 804 bytes with its header: two
        // blocks of 480.
        assert_eq!(small.max_plaintext_len_in(UNKNOWN), 960);
        let unprotected = FrameParams::new(1024, 0).unwrap();
        assert_eq!(unprotected.max_plaintext_len(), 63 * 1024);
        assert_eq!(unprotected.check_outer_len(1024, UNKNOWN), Ok(1024));
    }

    #[test]
    fn outer_length_rules() {
        // T-FRAME-1 to T-FRAME-4.
        assert_eq!(
            P.check_outer_len(MIN_FRAME_CIPHERTEXT_LEN, CONTACT),
            Ok(1024)
        );
        assert_eq!(
            P.check_outer_len(MAX_FRAME_CIPHERTEXT_LEN, CONTACT),
            Ok(64_512)
        );
        for bad in [
            0,
            1,
            15,
            16,
            1039,
            1041,
            2063,
            MAX_FRAME_CIPHERTEXT_LEN + 1,
            MAX_FRAME_CIPHERTEXT_LEN + 1024,
            65_535,
        ] {
            assert_eq!(
                P.check_outer_len(bad, CONTACT),
                Err(ProtocolError::FrameLengthOutOfRange),
                "{bad}"
            );
        }
        // Before confirmation only one block is allowed, and the same
        // before the identities are proven.
        for state in [UNKNOWN, SessionState::IdentityAuth] {
            assert_eq!(P.check_outer_len(1040, state), Ok(1024));
            for bad in [2064, MAX_FRAME_CIPHERTEXT_LEN] {
                assert_eq!(
                    P.check_outer_len(bad, state),
                    Err(ProtocolError::FrameLengthOutOfRange),
                    "{bad} in {state:?}"
                );
            }
        }
        // In states that receive nothing, every length is refused.
        for state in [
            SessionState::Connecting,
            SessionState::CryptoHandshake,
            SessionState::Closing,
            SessionState::Closed,
        ] {
            assert_eq!(
                P.check_outer_len(1040, state),
                Err(ProtocolError::FrameLengthOutOfRange)
            );
        }
    }

    #[test]
    fn plaintext_round_trips() {
        for body_len in [0, 1, 1019, 1020, 1021, 2044, 64_508] {
            let body = vec![0xA7_u8; body_len];
            let plaintext = encode_plaintext(&P, MessageType::FileChunk, &body).unwrap();
            assert_eq!(plaintext.len() % 1024, 0);
            assert!(plaintext.len() >= body_len + 4);
            assert!(plaintext.len() < body_len + 4 + 1024);
            let (message_type, decoded) = decode_plaintext(&P, CONTACT, &plaintext).unwrap();
            assert_eq!(message_type, MessageType::FileChunk);
            assert_eq!(decoded, &body[..]);
        }
    }

    #[test]
    fn plaintext_layout() {
        let plaintext = encode_plaintext(&P, MessageType::Ping, &[1, 2, 3]).unwrap();
        assert_eq!(plaintext.len(), 1024);
        assert_eq!(&plaintext[..7], &[0x00, 0x03, 0x00, 0x03, 1, 2, 3]);
        assert!(plaintext[7..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn oversized_body_is_refused_by_the_encoder() {
        let body = vec![0_u8; MAX_MESSAGE_BODY_LEN + 1];
        assert_eq!(
            encode_plaintext(&P, MessageType::FileChunk, &body),
            Err(ProtocolError::BadMessageLength)
        );
    }

    #[test]
    fn the_largest_body_is_the_boundary_of_the_decoder() {
        // A full frame whose body takes every byte after the header.
        let body = vec![0xA7_u8; MAX_MESSAGE_BODY_LEN];
        let mut plaintext = encode_plaintext(&P, MessageType::FileChunk, &body).unwrap();
        assert_eq!(plaintext.len(), MAX_FRAME_PLAINTEXT_LEN);
        assert_eq!(&plaintext[2..4], &[0xfb, 0xfc]);
        assert!(decode_plaintext(&P, CONTACT, &plaintext).is_ok());

        // The same frame claiming one byte more than it can hold.
        plaintext[2..4].copy_from_slice(&[0xfb, 0xfd]);
        assert_eq!(
            decode_plaintext(&P, CONTACT, &plaintext),
            Err(ProtocolError::BadPadding)
        );
        // Claiming one byte less leaves a non-zero byte in the padding.
        plaintext[2..4].copy_from_slice(&[0xfb, 0xfb]);
        assert_eq!(
            decode_plaintext(&P, CONTACT, &plaintext),
            Err(ProtocolError::BadPadding)
        );
    }

    #[test]
    fn debug_output_of_the_decoder_shows_no_payload() {
        let mut decoder = OuterDecoder::new(P);
        decoder
            .feed(&[0x04, 0x10, 0xAB, 0xAB, 0xAB], CONTACT)
            .unwrap();
        let text = format!("{decoder:?}");
        assert!(text.contains("buffered: 3"), "{text}");
        assert!(
            !text.contains("171") && !text.to_lowercase().contains("ab,"),
            "{text}"
        );
    }

    #[test]
    fn padding_must_be_zero() {
        // T-FRAME-5.
        let mut plaintext = encode_plaintext(&P, MessageType::Ping, &[1; 8]).unwrap();
        *plaintext.last_mut().unwrap() = 1;
        assert_eq!(
            decode_plaintext(&P, CONTACT, &plaintext),
            Err(ProtocolError::BadPadding)
        );
        let mut plaintext = encode_plaintext(&P, MessageType::Ping, &[1; 8]).unwrap();
        plaintext[12] = 0x80;
        assert_eq!(
            decode_plaintext(&P, CONTACT, &plaintext),
            Err(ProtocolError::BadPadding)
        );
    }

    #[test]
    fn padding_must_be_minimal() {
        // T-FRAME-5. A short body in a two-block frame.
        let mut plaintext = encode_plaintext(&P, MessageType::Ping, &[1; 8]).unwrap();
        plaintext.resize(2048, 0);
        assert_eq!(
            decode_plaintext(&P, CONTACT, &plaintext),
            Err(ProtocolError::BadPadding)
        );
    }

    #[test]
    fn body_length_must_fit_the_frame() {
        // T-FRAME-6. Body length claims more than the frame holds.
        let mut plaintext = vec![0_u8; 1024];
        plaintext[..4].copy_from_slice(&[0x00, 0x03, 0x04, 0x00]);
        assert_eq!(
            decode_plaintext(&P, CONTACT, &plaintext),
            Err(ProtocolError::BadPadding)
        );
        let mut plaintext = vec![0_u8; 1024];
        plaintext[..4].copy_from_slice(&[0x00, 0x03, 0xff, 0xff]);
        assert_eq!(
            decode_plaintext(&P, CONTACT, &plaintext),
            Err(ProtocolError::BadPadding)
        );
    }

    #[test]
    fn plaintext_length_must_be_legal() {
        for len in [0, 1, 3, 4, 1023, 1025, 64_512 + 1024] {
            let plaintext = vec![0_u8; len];
            assert_eq!(
                decode_plaintext(&P, CONTACT, &plaintext),
                Err(ProtocolError::FrameLengthOutOfRange),
                "{len}"
            );
        }
        // Two blocks are too many before confirmation.
        let two_blocks = encode_plaintext(&P, MessageType::ContactRequest, &[0; 1100]).unwrap();
        assert_eq!(
            decode_plaintext(&P, UNKNOWN, &two_blocks),
            Err(ProtocolError::FrameLengthOutOfRange)
        );
    }

    #[test]
    fn unknown_type_codes_are_rejected() {
        let mut plaintext = vec![0_u8; 1024];
        plaintext[..4].copy_from_slice(&[0x7f, 0xff, 0x00, 0x00]);
        assert_eq!(
            decode_plaintext(&P, CONTACT, &plaintext),
            Err(ProtocolError::UnknownMessageType)
        );
        plaintext[..2].copy_from_slice(&[0x00, 0x00]);
        assert_eq!(
            decode_plaintext(&P, CONTACT, &plaintext),
            Err(ProtocolError::UnknownMessageType)
        );
    }

    #[test]
    fn the_state_gate_runs_before_the_body_is_returned() {
        let chat = encode_plaintext(&P, MessageType::ChatMessage, &[0; 20]).unwrap();
        assert_eq!(
            decode_plaintext(&P, UNKNOWN, &chat),
            Err(ProtocolError::MessageNotPermitted)
        );
        assert_eq!(
            decode_plaintext(&P, SessionState::IdentityAuth, &chat),
            Err(ProtocolError::MessageNotPermitted)
        );
        assert!(decode_plaintext(&P, CONTACT, &chat).is_ok());

        let proof = encode_plaintext(&P, MessageType::AuthProof, &[0; 104]).unwrap();
        assert!(decode_plaintext(&P, SessionState::IdentityAuth, &proof).is_ok());
        assert_eq!(
            decode_plaintext(&P, CONTACT, &proof),
            Err(ProtocolError::MessageNotPermitted)
        );
    }

    #[test]
    fn outer_frame_round_trips() {
        let payload = vec![0x5A_u8; 1040];
        let frame = encode_outer(&P, &payload).unwrap();
        assert_eq!(frame.len(), 1042);
        assert_eq!(&frame[..2], &[0x04, 0x10]);

        let mut decoder = OuterDecoder::new(P);
        let (used, decoded) = decoder.feed(&frame, CONTACT).unwrap();
        assert_eq!(used, frame.len());
        assert_eq!(decoded, Some(payload));
        assert_eq!(decoder.buffered(), 0);
    }

    #[test]
    fn outer_encoder_refuses_illegal_payload_lengths() {
        for len in [0, 16, 1039, 1041, 64_529] {
            assert_eq!(
                encode_outer(&P, &vec![0; len]),
                Err(ProtocolError::FrameLengthOutOfRange),
                "{len}"
            );
        }
    }

    #[test]
    fn decoder_handles_any_split_of_the_stream() {
        let first = encode_outer(&P, &vec![1_u8; 1040]).unwrap();
        let second = encode_outer(&P, &vec![2_u8; 2064]).unwrap();
        let mut stream = first.clone();
        stream.extend_from_slice(&second);

        for piece in [1, 2, 3, 7, 1040, 1041, 1042, 1043, 5000] {
            let mut decoder = OuterDecoder::new(P);
            let mut frames = Vec::new();
            for chunk in stream.chunks(piece) {
                let mut rest = chunk;
                while !rest.is_empty() {
                    let (used, frame) = decoder.feed(rest, CONTACT).unwrap();
                    assert!(used > 0);
                    rest = &rest[used..];
                    if let Some(frame) = frame {
                        frames.push(frame);
                    }
                    assert!(decoder.buffered() <= 2064);
                }
            }
            assert_eq!(frames, vec![vec![1_u8; 1040], vec![2_u8; 2064]], "{piece}");
        }
    }

    #[test]
    fn decoder_rejects_a_bad_length_before_consuming_payload() {
        // Declares the largest possible length. Nothing is buffered for it.
        let mut decoder = OuterDecoder::new(P);
        assert_eq!(
            decoder.feed(&[0xff, 0xff, 1, 2, 3], CONTACT),
            Err(ProtocolError::FrameLengthOutOfRange)
        );
        assert_eq!(decoder.buffered(), 0);
        // The decoder stays failed.
        assert!(decoder.feed(&[0x04, 0x10], CONTACT).is_err());
    }

    #[test]
    fn decoder_applies_the_limit_of_the_state() {
        let two_blocks = encode_outer(&P, &vec![0_u8; 2064]).unwrap();
        let mut decoder = OuterDecoder::new(P);
        assert_eq!(
            decoder.feed(&two_blocks, UNKNOWN),
            Err(ProtocolError::FrameLengthOutOfRange)
        );

        let mut decoder = OuterDecoder::new(P);
        assert_eq!(
            decoder.feed(&two_blocks, SessionState::CryptoHandshake),
            Err(ProtocolError::FrameLengthOutOfRange)
        );
    }

    #[test]
    fn decoder_never_buffers_more_than_one_frame() {
        // An endless stream of maximum frames, fed in large pieces.
        let frame = encode_outer(&P, &vec![7_u8; MAX_FRAME_CIPHERTEXT_LEN]).unwrap();
        let mut stream = Vec::new();
        for _ in 0..4 {
            stream.extend_from_slice(&frame);
        }
        let mut decoder = OuterDecoder::new(P);
        let mut rest = &stream[..];
        let mut frames = 0;
        while !rest.is_empty() {
            let (used, frame) = decoder.feed(rest, CONTACT).unwrap();
            rest = &rest[used..];
            assert!(decoder.buffered() <= MAX_FRAME_CIPHERTEXT_LEN);
            if frame.is_some() {
                frames += 1;
            }
        }
        assert_eq!(frames, 4);
    }

    #[test]
    fn a_stream_that_never_completes_a_frame_holds_at_most_one_frame() {
        let mut decoder = OuterDecoder::new(P);
        let (used, frame) = decoder.feed(&[0x04, 0x10], CONTACT).unwrap();
        assert_eq!((used, frame), (2, None));
        let (used, frame) = decoder.feed(&vec![0_u8; 1039], CONTACT).unwrap();
        assert_eq!((used, frame), (1039, None));
        assert_eq!(decoder.buffered(), 1039);
    }
}

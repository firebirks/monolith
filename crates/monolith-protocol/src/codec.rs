//! Bounded readers and writers for the fixed-layout encoding.
//!
//! The encoding is defined in `docs/PROTOCOL.md` section 2: unsigned
//! integers are big-endian, fixed-size fields are raw bytes, and a
//! variable-size field is a 16-bit length followed by that many bytes, with a
//! minimum and a maximum that the caller must state.
//!
//! [`Reader`] never reads past its input and never allocates. [`Writer`]
//! refuses a field that exceeds its maximum. A structure is decoded by
//! reading its fields in order and then calling [`Reader::finish`], which
//! rejects trailing bytes; that is what makes every structure have exactly
//! one encoding.
//!
//! Both hold message content. Their `Debug` output states sizes only.

use core::fmt;

use zeroize::Zeroize;

use crate::ProtocolError;

/// Reads fields from a byte slice, front to back.
pub struct Reader<'a> {
    rest: &'a [u8],
}

impl fmt::Debug for Reader<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Reader")
            .field("remaining", &self.rest.len())
            .finish()
    }
}

impl<'a> Reader<'a> {
    /// Starts reading at the beginning of `input`.
    pub const fn new(input: &'a [u8]) -> Self {
        Self { rest: input }
    }

    /// Returns the number of bytes not yet read.
    pub const fn remaining(&self) -> usize {
        self.rest.len()
    }

    /// Returns the bytes not yet read, without consuming them.
    pub const fn peek(&self) -> &'a [u8] {
        self.rest
    }

    /// Reads exactly `len` bytes.
    pub fn take(&mut self, len: usize) -> Result<&'a [u8], ProtocolError> {
        let (head, tail) = self
            .rest
            .split_at_checked(len)
            .ok_or(ProtocolError::BadMessageLength)?;
        self.rest = tail;
        Ok(head)
    }

    /// Reads a fixed-size array.
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], ProtocolError> {
        let bytes = self.take(N)?;
        <[u8; N]>::try_from(bytes).map_err(|_| ProtocolError::BadMessageLength)
    }

    /// Reads one byte.
    pub fn u8(&mut self) -> Result<u8, ProtocolError> {
        self.array::<1>().map(u8::from_be_bytes)
    }

    /// Reads a big-endian 16-bit integer.
    pub fn u16(&mut self) -> Result<u16, ProtocolError> {
        self.array().map(u16::from_be_bytes)
    }

    /// Reads a big-endian 64-bit integer.
    pub fn u64(&mut self) -> Result<u64, ProtocolError> {
        self.array().map(u64::from_be_bytes)
    }

    /// Reads a presence byte: 0x00 or 0x01. Any other value is rejected.
    pub fn presence(&mut self) -> Result<bool, ProtocolError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(ProtocolError::InvalidValue),
        }
    }

    /// Reads a variable-size field whose length must lie in `min..=max`.
    ///
    /// The declared length is checked against `max` before any byte of the
    /// field is looked at.
    pub fn bytes(&mut self, min: usize, max: usize) -> Result<&'a [u8], ProtocolError> {
        let len = usize::from(self.u16()?);
        if len > max {
            return Err(ProtocolError::FieldTooLong);
        }
        if len < min {
            return Err(ProtocolError::InvalidValue);
        }
        self.take(len)
    }

    /// Ends the read. Fails if any input is left over.
    pub fn finish(self) -> Result<(), ProtocolError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(ProtocolError::BadMessageLength)
        }
    }
}

/// Appends fields to a byte vector.
///
/// What is written may be secret: a key in a vault record, the text of a
/// message. When the buffer has to grow, the bytes move to a new buffer
/// and the old one is erased before it is freed, and a writer that is
/// dropped without [`Self::into_bytes`] erases what it holds, so that no
/// copy stays behind in freed memory from here. Copies the compiler makes
/// are not reached (`docs/CRYPTOGRAPHY.md` section 8).
#[derive(Default)]
pub struct Writer {
    out: Vec<u8>,
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.out.as_mut_slice().zeroize();
    }
}

impl fmt::Debug for Writer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Writer")
            .field("len", &self.out.len())
            .finish()
    }
}

impl Writer {
    /// Starts with an empty buffer.
    pub const fn new() -> Self {
        Self { out: Vec::new() }
    }

    /// Starts with an empty buffer that has room for `capacity` bytes.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            out: Vec::with_capacity(capacity),
        }
    }

    /// Returns the number of bytes written so far.
    pub fn len(&self) -> usize {
        self.out.len()
    }

    /// Returns true if nothing has been written.
    pub fn is_empty(&self) -> bool {
        self.out.is_empty()
    }

    /// Makes room for `additional` more bytes, in a new buffer if needed;
    /// the old one is erased.
    fn reserve(&mut self, additional: usize) {
        let needed = self.out.len().saturating_add(additional);
        if needed <= self.out.capacity() {
            return;
        }
        let capacity = needed.max(self.out.capacity().saturating_mul(2)).max(64);
        let mut grown = Vec::with_capacity(capacity);
        grown.extend_from_slice(&self.out);
        let mut old = core::mem::replace(&mut self.out, grown);
        old.as_mut_slice().zeroize();
    }

    /// Writes raw bytes, with no length prefix.
    pub fn raw(&mut self, bytes: &[u8]) {
        self.reserve(bytes.len());
        self.out.extend_from_slice(bytes);
    }

    /// Writes one byte.
    pub fn u8(&mut self, value: u8) {
        self.reserve(1);
        self.out.push(value);
    }

    /// Writes a big-endian 16-bit integer.
    pub fn u16(&mut self, value: u16) {
        self.raw(&value.to_be_bytes());
    }

    /// Writes a big-endian 64-bit integer.
    pub fn u64(&mut self, value: u64) {
        self.raw(&value.to_be_bytes());
    }

    /// Writes a presence byte.
    pub fn presence(&mut self, present: bool) {
        self.u8(u8::from(present));
    }

    /// Writes a variable-size field: a 16-bit length, then the bytes.
    ///
    /// Fails if the field is longer than `max` or than a 16-bit length can
    /// express. Nothing is written in that case.
    pub fn bytes(&mut self, bytes: &[u8], max: usize) -> Result<(), ProtocolError> {
        if bytes.len() > max {
            return Err(ProtocolError::FieldTooLong);
        }
        let len = u16::try_from(bytes.len()).map_err(|_| ProtocolError::FieldTooLong)?;
        self.u16(len);
        self.raw(bytes);
        Ok(())
    }

    /// Returns the bytes written.
    pub fn into_bytes(mut self) -> Vec<u8> {
        core::mem::take(&mut self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integers_are_big_endian() {
        let mut writer = Writer::new();
        writer.u8(0x01);
        writer.u16(0x0203);
        writer.u64(0x0405_0607_0809_0a0b);
        let bytes = writer.into_bytes();
        assert_eq!(bytes, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]);

        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.u8(), Ok(0x01));
        assert_eq!(reader.u16(), Ok(0x0203));
        assert_eq!(reader.u64(), Ok(0x0405_0607_0809_0a0b));
        assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    fn reading_past_the_end_fails_without_consuming() {
        let mut reader = Reader::new(&[1, 2, 3]);
        assert_eq!(reader.u64(), Err(ProtocolError::BadMessageLength));
        assert_eq!(reader.remaining(), 3);
        assert_eq!(reader.take(4), Err(ProtocolError::BadMessageLength));
        assert_eq!(reader.take(3), Ok(&[1_u8, 2, 3][..]));
        assert_eq!(reader.u8(), Err(ProtocolError::BadMessageLength));
    }

    #[test]
    fn debug_output_shows_sizes_only() {
        let reader = Reader::new(&[0xAB, 0xCD, 0xEF]);
        assert_eq!(format!("{reader:?}"), "Reader { remaining: 3 }");
        let mut writer = Writer::new();
        writer.raw(&[0xAB, 0xCD]);
        assert_eq!(format!("{writer:?}"), "Writer { len: 2 }");
    }

    #[test]
    fn a_growing_writer_keeps_every_byte_in_order() {
        // Growth moves the bytes to a new buffer, more than once.
        let mut writer = Writer::new();
        let mut expected = Vec::new();
        for byte in 0..=u8::MAX {
            writer.u8(byte);
            writer.raw(&[byte; 7]);
            expected.push(byte);
            expected.extend_from_slice(&[byte; 7]);
        }
        assert_eq!(writer.len(), expected.len());
        assert_eq!(writer.into_bytes(), expected);
        let mut writer = Writer::with_capacity(2);
        writer.u16(0x0102);
        writer.u64(3);
        assert_eq!(writer.into_bytes(), [1, 2, 0, 0, 0, 0, 0, 0, 0, 3]);
    }

    #[test]
    fn finish_rejects_trailing_bytes() {
        let mut reader = Reader::new(&[1, 2]);
        assert_eq!(reader.u8(), Ok(1));
        assert_eq!(reader.finish(), Err(ProtocolError::BadMessageLength));
    }

    #[test]
    fn presence_byte_accepts_only_zero_and_one() {
        assert_eq!(Reader::new(&[0]).presence(), Ok(false));
        assert_eq!(Reader::new(&[1]).presence(), Ok(true));
        for value in 2..=255_u8 {
            assert_eq!(
                Reader::new(&[value]).presence(),
                Err(ProtocolError::InvalidValue)
            );
        }
    }

    #[test]
    fn variable_field_round_trips() {
        let mut writer = Writer::new();
        writer.bytes(b"abc", 3).unwrap();
        writer.bytes(b"", 0).unwrap();
        let bytes = writer.into_bytes();
        assert_eq!(bytes, [0, 3, b'a', b'b', b'c', 0, 0]);

        let mut reader = Reader::new(&bytes);
        assert_eq!(reader.bytes(0, 3), Ok(&b"abc"[..]));
        assert_eq!(reader.bytes(0, 0), Ok(&b""[..]));
        assert_eq!(reader.finish(), Ok(()));
    }

    #[test]
    fn variable_field_enforces_the_maximum_before_reading() {
        // Declares 4 bytes with a maximum of 3. The field is rejected for
        // its declared length even though the bytes are not there at all.
        let mut reader = Reader::new(&[0, 4]);
        assert_eq!(reader.bytes(0, 3), Err(ProtocolError::FieldTooLong));

        let mut reader = Reader::new(&[0xff, 0xff]);
        assert_eq!(reader.bytes(0, 16), Err(ProtocolError::FieldTooLong));
    }

    #[test]
    fn variable_field_enforces_the_minimum() {
        let mut reader = Reader::new(&[0, 0]);
        assert_eq!(reader.bytes(1, 3), Err(ProtocolError::InvalidValue));
    }

    #[test]
    fn variable_field_rejects_truncation() {
        let mut reader = Reader::new(&[0, 3, b'a', b'b']);
        assert_eq!(reader.bytes(0, 3), Err(ProtocolError::BadMessageLength));
    }

    #[test]
    fn writer_refuses_an_oversized_field_and_writes_nothing() {
        let mut writer = Writer::new();
        assert_eq!(writer.bytes(b"abcd", 3), Err(ProtocolError::FieldTooLong));
        assert!(writer.is_empty());

        let big = vec![0_u8; 70_000];
        assert_eq!(
            writer.bytes(&big, 100_000),
            Err(ProtocolError::FieldTooLong)
        );
        assert_eq!(writer.len(), 0);
    }
}

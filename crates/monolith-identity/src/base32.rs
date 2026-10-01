//! Base32 as RFC 4648 defines it, without padding.
//!
//! Used for fingerprints and for the text form of contact cards. The encoder
//! produces upper case, which is the canonical form. The decoder accepts
//! either case and nothing else: no padding, no whitespace, no characters
//! outside the alphabet, and no input whose unused trailing bits are not
//! zero. Every byte string therefore has exactly one canonical encoding, and
//! every accepted text decodes to exactly one byte string.

use crate::IdentityError;

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Bytes per full group.
const GROUP_BYTES: usize = 5;

/// Symbols per full group.
const GROUP_SYMBOLS: usize = 8;

/// Returns the number of symbols that encode `len` bytes.
pub const fn encoded_len(len: usize) -> Option<usize> {
    // ceil(len * 8 / 5)
    let Some(bits) = len.checked_mul(8) else {
        return None;
    };
    let Some(rounded) = bits.checked_add(4) else {
        return None;
    };
    Some(rounded / 5)
}

/// Encodes bytes as upper-case base32 without padding.
pub fn encode(input: &[u8]) -> String {
    let mut out = String::new();
    for chunk in input.chunks(GROUP_BYTES) {
        let mut b = [0_u8; GROUP_BYTES];
        for (dst, src) in b.iter_mut().zip(chunk) {
            *dst = *src;
        }
        let symbols = [
            b[0] >> 3,
            ((b[0] & 0x07) << 2) | (b[1] >> 6),
            (b[1] >> 1) & 0x1f,
            ((b[1] & 0x01) << 4) | (b[2] >> 4),
            ((b[2] & 0x0f) << 1) | (b[3] >> 7),
            (b[3] >> 2) & 0x1f,
            ((b[3] & 0x03) << 3) | (b[4] >> 5),
            b[4] & 0x1f,
        ];
        let used = match chunk.len() {
            1 => 2,
            2 => 4,
            3 => 5,
            4 => 7,
            _ => GROUP_SYMBOLS,
        };
        for symbol in symbols.iter().take(used) {
            if let Some(letter) = ALPHABET.get(usize::from(*symbol)) {
                out.push(char::from(*letter));
            }
        }
    }
    out
}

/// Decodes base32 text into at most `max_len` bytes.
///
/// Fails on any character outside the alphabet, on a length that no byte
/// string encodes to, on unused trailing bits that are not zero, and on
/// input that would decode to more than `max_len` bytes. The length check
/// happens before anything is allocated.
pub fn decode(input: &str, max_len: usize) -> Result<Vec<u8>, IdentityError> {
    let text = input.as_bytes();
    let longest = encoded_len(max_len).ok_or(IdentityError::InvalidBase32)?;
    if text.len() > longest {
        return Err(IdentityError::InvalidBase32);
    }

    let mut out = Vec::with_capacity(max_len.min(text.len()));
    for chunk in text.chunks(GROUP_SYMBOLS) {
        let mut s = [0_u8; GROUP_SYMBOLS];
        for (dst, src) in s.iter_mut().zip(chunk) {
            *dst = symbol_value(*src)?;
        }
        let bytes = [
            (s[0] << 3) | (s[1] >> 2),
            ((s[1] & 0x03) << 6) | (s[2] << 1) | (s[3] >> 4),
            ((s[3] & 0x0f) << 4) | (s[4] >> 1),
            ((s[4] & 0x01) << 7) | (s[5] << 2) | (s[6] >> 3),
            ((s[6] & 0x07) << 5) | s[7],
        ];
        // A final group shorter than eight symbols carries fewer bytes, and
        // the bits of its last symbol that belong to no byte must be zero.
        let (used, leftover) = match chunk.len() {
            2 => (1, s[1] & 0x03),
            4 => (2, s[3] & 0x0f),
            5 => (3, s[4] & 0x01),
            7 => (4, s[6] & 0x07),
            8 => (GROUP_BYTES, 0),
            _ => return Err(IdentityError::InvalidBase32),
        };
        if leftover != 0 {
            return Err(IdentityError::InvalidBase32);
        }
        out.extend(bytes.iter().take(used));
    }
    if out.len() > max_len {
        return Err(IdentityError::InvalidBase32);
    }
    Ok(out)
}

fn symbol_value(symbol: u8) -> Result<u8, IdentityError> {
    match symbol {
        b'A'..=b'Z' => Ok(symbol.wrapping_sub(b'A')),
        b'a'..=b'z' => Ok(symbol.wrapping_sub(b'a')),
        b'2'..=b'7' => Ok(symbol.wrapping_sub(b'2').wrapping_add(26)),
        _ => Err(IdentityError::InvalidBase32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test vectors of RFC 4648 section 10, with the padding removed.
    const RFC_VECTORS: [(&str, &str); 7] = [
        ("", ""),
        ("f", "MY"),
        ("fo", "MZXQ"),
        ("foo", "MZXW6"),
        ("foob", "MZXW6YQ"),
        ("fooba", "MZXW6YTB"),
        ("foobar", "MZXW6YTBOI"),
    ];

    #[test]
    fn encodes_the_rfc_vectors() {
        for (plain, encoded) in RFC_VECTORS {
            assert_eq!(encode(plain.as_bytes()), encoded);
        }
    }

    #[test]
    fn decodes_the_rfc_vectors() {
        for (plain, encoded) in RFC_VECTORS {
            assert_eq!(decode(encoded, 16).unwrap(), plain.as_bytes());
        }
    }

    #[test]
    fn decodes_lower_case() {
        assert_eq!(decode("mzxw6ytboi", 16).unwrap(), b"foobar");
        assert_eq!(decode("MzXw6yTbOi", 16).unwrap(), b"foobar");
    }

    #[test]
    fn rejects_characters_outside_the_alphabet() {
        for bad in [
            "MY======", "M Y", "MY\n", "M0", "M1", "M8", "M9", "M=", "MZ-Q", "\u{e9}Y",
        ] {
            assert_eq!(
                decode(bad, 16),
                Err(IdentityError::InvalidBase32),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn rejects_lengths_that_encode_nothing() {
        for bad in ["M", "MZX", "MZXW6Y", "MZXW6YTBO", "MZXW6YTBOIA"] {
            assert_eq!(
                decode(bad, 16),
                Err(IdentityError::InvalidBase32),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn rejects_nonzero_trailing_bits() {
        // "MY" is 'f'. "MZ" has the same first byte and a set bit left over.
        assert_eq!(decode("MZ", 16), Err(IdentityError::InvalidBase32));
        assert_eq!(decode("MZXR", 16), Err(IdentityError::InvalidBase32));
        assert_eq!(decode("MZXW7", 16), Err(IdentityError::InvalidBase32));
        assert_eq!(decode("MZXW6YR", 16), Err(IdentityError::InvalidBase32));
    }

    #[test]
    fn rejects_output_longer_than_the_limit() {
        assert!(decode("MZXW6YTBOI", 6).is_ok());
        assert_eq!(decode("MZXW6YTBOI", 5), Err(IdentityError::InvalidBase32));
        assert_eq!(decode("MY", 0), Err(IdentityError::InvalidBase32));
        assert_eq!(decode("", 0).unwrap(), b"");
    }

    #[test]
    fn encoded_len_matches_the_encoder() {
        for len in 0..64 {
            let data = vec![0xA5_u8; len];
            assert_eq!(encode(&data).len(), encoded_len(len).unwrap());
        }
        assert_eq!(encoded_len(usize::MAX), None);
    }

    #[test]
    fn every_byte_value_round_trips() {
        let data: Vec<u8> = (0..=255).collect();
        for len in 0..data.len() {
            let slice = &data[..len];
            assert_eq!(decode(&encode(slice), len).unwrap(), slice);
        }
    }
}

//! Secrets of the Tor adapter: the onion service key in Tor's format and the
//! stream isolation token.

use core::fmt;
use std::sync::Arc;

use monolith_identity::redact::REDACTED;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::TorError;

/// Length of an onion service secret key in Tor's `ED25519-V3` format.
pub const ONION_SECRET_LEN: usize = 64;

/// Length of that key in base64, with padding.
pub const ONION_SECRET_BASE64_LEN: usize = 88;

/// Length of an isolation token.
const ISOLATION_TOKEN_LEN: usize = 16;

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The secret key of an Onion Service, in the form Tor exports and imports
/// it: the 32-byte ed25519 secret scalar and the 32-byte PRF secret.
///
/// It is not an Ed25519 seed, not a Monolith identity key and not derived
/// from any Monolith key; Monolith never computes with it. It comes from
/// Tor (`250-PrivateKey=ED25519-V3:...`) and goes back to Tor unchanged.
///
/// Not `Copy`, not `Clone`. The bytes are overwritten when the value is
/// dropped, best effort: copies the compiler made and the memory of the
/// control connection's buffers are not reached. `Debug` shows nothing.
pub struct OnionServiceSecret(Zeroizing<[u8; ONION_SECRET_LEN]>);

impl OnionServiceSecret {
    /// Wraps the 64 key bytes.
    pub fn from_bytes(bytes: &[u8; ONION_SECRET_LEN]) -> Self {
        Self(Zeroizing::new(*bytes))
    }

    /// Returns the key bytes. Callers must not log them.
    pub fn expose(&self) -> &[u8; ONION_SECRET_LEN] {
        &self.0
    }

    /// Decodes the base64 form Tor uses: exactly 88 characters of the
    /// standard alphabet, ending in `==`, whose unused bits are zero, so that
    /// every key has one accepted text.
    pub(crate) fn from_base64(text: &[u8]) -> Result<Self, TorError> {
        if text.len() != ONION_SECRET_BASE64_LEN || !text.ends_with(b"==") {
            return Err(TorError::InvalidTorResponse);
        }
        let value = |character: &u8| -> Result<u8, TorError> {
            BASE64_ALPHABET
                .iter()
                .position(|candidate| candidate == character)
                .and_then(|position| u8::try_from(position).ok())
                .ok_or(TorError::InvalidTorResponse)
        };
        // 22 groups of four characters. The last is `XY==` and carries one
        // byte; the low four bits of `Y` are unused and must be zero.
        let mut bytes = Zeroizing::new(Vec::with_capacity(ONION_SECRET_LEN.saturating_add(2)));
        let mut groups = text.chunks_exact(4).peekable();
        while let Some(group) = groups.next() {
            let last = groups.peek().is_none();
            let [a, b, c, d] = group else {
                return Err(TorError::InvalidTorResponse);
            };
            let (a, b) = (value(a)?, value(b)?);
            bytes.push(a.wrapping_shl(2) | b.wrapping_shr(4));
            if last {
                if b & 0x0f != 0 {
                    return Err(TorError::InvalidTorResponse);
                }
            } else {
                let (c, d) = (value(c)?, value(d)?);
                bytes.push((b & 0x0f).wrapping_shl(4) | c.wrapping_shr(2));
                bytes.push((c & 0x03).wrapping_shl(6) | d);
            }
        }
        let mut out = Zeroizing::new([0_u8; ONION_SECRET_LEN]);
        if bytes.len() != ONION_SECRET_LEN {
            return Err(TorError::InvalidTorResponse);
        }
        out.copy_from_slice(&bytes);
        Ok(Self(out))
    }

    /// Encodes the key in the base64 form Tor expects.
    pub(crate) fn to_base64(&self) -> Zeroizing<Vec<u8>> {
        let mut out = Zeroizing::new(Vec::with_capacity(ONION_SECRET_BASE64_LEN));
        for chunk in self.0.chunks(3) {
            let mut group = [0_u8; 3];
            for (slot, byte) in group.iter_mut().zip(chunk) {
                *slot = *byte;
            }
            let [a, b, c] = group;
            let indices = [
                a.wrapping_shr(2),
                (a & 0x03).wrapping_shl(4) | b.wrapping_shr(4),
                (b & 0x0f).wrapping_shl(2) | c.wrapping_shr(6),
                c & 0x3f,
            ];
            let used = chunk.len().saturating_add(1);
            for (position, index) in indices.iter().enumerate() {
                if position < used {
                    out.push(alphabet(*index));
                } else {
                    out.push(b'=');
                }
            }
            group.zeroize();
        }
        out
    }
}

impl fmt::Debug for OnionServiceSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OnionServiceSecret({REDACTED})")
    }
}

impl ZeroizeOnDrop for OnionServiceSecret {}

/// The bytes behind an [`IsolationGroup`].
struct IsolationToken([u8; ISOLATION_TOKEN_LEN]);

impl Drop for IsolationToken {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// A Tor stream isolation context.
///
/// Streams dialed with the same group may share a Tor circuit; streams
/// dialed with different groups do not. The core keeps one group per
/// contact (`docs/TOR_INTEGRATION.md` section 3.2).
///
/// The token behind it is 16 bytes from the operating system's CSPRNG. It
/// names nothing: no identity, onion address, name or contact number goes
/// into it. It is runtime state: never stored, never logged, and a new one
/// is made after a restart. There is no accessor for the bytes outside
/// this crate. Cloning shares the same token.
#[derive(Clone)]
pub struct IsolationGroup(Arc<IsolationToken>);

impl IsolationGroup {
    /// Makes a group with a new random token.
    pub fn generate() -> Result<Self, TorError> {
        let mut bytes = [0_u8; ISOLATION_TOKEN_LEN];
        getrandom::fill(&mut bytes).map_err(|_| TorError::Randomness)?;
        let group = Self(Arc::new(IsolationToken(bytes)));
        bytes.zeroize();
        Ok(group)
    }

    /// The SOCKS5 password that carries the token: lower-case hex.
    pub(crate) fn password(&self) -> Zeroizing<Vec<u8>> {
        let mut out = Zeroizing::new(Vec::with_capacity(ISOLATION_TOKEN_LEN * 2));
        out.extend_from_slice(&hex(&self.0.0));
        out
    }

    /// Returns true if both values are the same group.
    pub fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for IsolationGroup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IsolationGroup({REDACTED})")
    }
}

/// The base64 character for a six-bit value.
fn alphabet(index: u8) -> u8 {
    BASE64_ALPHABET
        .get(usize::from(index))
        .copied()
        .unwrap_or(b'A')
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// The hex digit for a four-bit value.
fn hex_digit(nibble: u8) -> u8 {
    HEX.get(usize::from(nibble)).copied().unwrap_or(b'0')
}

/// Lower-case hex of bytes.
pub(crate) fn hex(bytes: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut out = Zeroizing::new(Vec::with_capacity(bytes.len().saturating_mul(2)));
    for byte in bytes {
        out.push(hex_digit(byte.wrapping_shr(4)));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

/// Decodes exactly `N` bytes of hex, either case.
pub(crate) fn unhex<const N: usize>(text: &[u8]) -> Result<[u8; N], TorError> {
    if text.len() != N.saturating_mul(2) {
        return Err(TorError::InvalidTorResponse);
    }
    let digit = |character: u8| -> Result<u8, TorError> {
        match character {
            b'0'..=b'9' => Ok(character.wrapping_sub(b'0')),
            b'a'..=b'f' => Ok(character.wrapping_sub(b'a').wrapping_add(10)),
            b'A'..=b'F' => Ok(character.wrapping_sub(b'A').wrapping_add(10)),
            _ => Err(TorError::InvalidTorResponse),
        }
    };
    let mut out = [0_u8; N];
    for (slot, pair) in out.iter_mut().zip(text.chunks_exact(2)) {
        let [high, low] = pair else {
            return Err(TorError::InvalidTorResponse);
        };
        *slot = digit(*high)?.wrapping_shl(4) | digit(*low)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key blob and its base64 form, computed with Python's base64
    /// module, independently of this crate: bytes 0, 1, ..., 63.
    const COUNTING_BASE64: &[u8] =
        b"AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8gISIjJCUmJygpKissLS4vMDEyMzQ1Njc4OTo7PD0+Pw==";

    fn counting() -> [u8; 64] {
        core::array::from_fn(|index| u8::try_from(index).unwrap())
    }

    #[test]
    fn the_key_round_trips_through_base64_as_tor_writes_it() {
        let secret = OnionServiceSecret::from_base64(COUNTING_BASE64).unwrap();
        assert_eq!(secret.expose(), &counting());
        assert_eq!(secret.to_base64().as_slice(), COUNTING_BASE64);
        for fill in [0_u8, 0xff, 0x5a] {
            let secret = OnionServiceSecret::from_bytes(&[fill; 64]);
            let text = secret.to_base64();
            assert_eq!(text.len(), ONION_SECRET_BASE64_LEN);
            assert_eq!(
                OnionServiceSecret::from_base64(&text).unwrap().expose(),
                &[fill; 64]
            );
        }
    }

    #[test]
    fn base64_that_is_not_exactly_tor_s_form_is_refused() {
        let good = COUNTING_BASE64.to_vec();
        // Lengths, padding, alphabet, and non-zero unused bits.
        assert!(OnionServiceSecret::from_base64(&good[..87]).is_err());
        assert!(OnionServiceSecret::from_base64(&good[..86]).is_err());
        let mut long = good.clone();
        long.push(b'=');
        assert!(OnionServiceSecret::from_base64(&long).is_err());
        assert!(OnionServiceSecret::from_base64(b"").is_err());
        for (position, bad) in [
            (87, b'A'),
            (86, b'A'),
            (0, b'-'),
            (0, b'_'),
            (40, b' '),
            (3, b'='),
        ] {
            let mut changed = good.clone();
            changed[position] = bad;
            assert!(
                OnionServiceSecret::from_base64(&changed).is_err(),
                "{position} {bad}"
            );
        }
        // The last character before the padding carries four unused bits.
        // `Pw` is canonical; `Px` sets one of them.
        let mut changed = good.clone();
        changed[85] = b'x';
        assert!(OnionServiceSecret::from_base64(&changed).is_err());
    }

    #[test]
    fn secrets_print_nothing() {
        let secret = OnionServiceSecret::from_bytes(&[0xAA; 64]);
        assert_eq!(format!("{secret:?}"), "OnionServiceSecret([redacted])");
        let group = IsolationGroup::generate().unwrap();
        assert_eq!(format!("{group:?}"), "IsolationGroup([redacted])");
    }

    #[test]
    fn isolation_groups_are_random_and_distinct() {
        let first = IsolationGroup::generate().unwrap();
        let second = IsolationGroup::generate().unwrap();
        assert!(!first.same_as(&second));
        assert_ne!(first.password().as_slice(), second.password().as_slice());
        let again = first.clone();
        assert!(again.same_as(&first));
        assert_eq!(again.password().as_slice(), first.password().as_slice());
        let password = first.password();
        assert_eq!(password.len(), 32);
        assert!(
            password
                .iter()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
    }

    #[test]
    fn hex_round_trips_and_refuses_bad_input() {
        let bytes = counting();
        let text = hex(&bytes[..32]);
        assert_eq!(unhex::<32>(&text).unwrap(), bytes[..32]);
        assert_eq!(
            unhex::<32>(&text.to_ascii_uppercase()).unwrap(),
            bytes[..32]
        );
        assert!(unhex::<32>(&text[..63]).is_err());
        let mut bad = text.to_vec();
        bad[5] = b'g';
        assert!(unhex::<32>(&bad).is_err());
    }
}

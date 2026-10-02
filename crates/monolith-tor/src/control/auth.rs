//! Control port authentication: `PROTOCOLINFO`, SAFECOOKIE and the cookie
//! file.
//!
//! SAFECOOKIE (control specification, `AUTHCHALLENGE`):
//!
//! ```text
//! ServerHash = HMAC-SHA256("Tor safe cookie authentication server-to-controller hash",
//!                          Cookie | ClientNonce | ServerNonce)
//! ClientHash = HMAC-SHA256("Tor safe cookie authentication controller-to-server hash",
//!                          Cookie | ClientNonce | ServerNonce)
//! ```
//!
//! Monolith checks the server hash before it sends the client hash, so it
//! proves knowledge of the cookie only to something that knows the cookie
//! itself. The cookie file is named by `PROTOCOLINFO`; it is read only on a
//! control connection to the endpoint the user configured, which is a
//! loopback address or a local socket (`TOR_CONTROL_SURFACE.md`).

use std::io::Read;
use std::path::{Path, PathBuf};

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use super::reply::Reply;
use crate::secret::unhex;
use crate::{TorError, TorVersion};

/// Length of the SAFECOOKIE nonces.
pub(crate) const NONCE_LEN: usize = 32;

/// Length of a control cookie. The specification forbids using a file of
/// any other length.
pub(crate) const COOKIE_LEN: usize = 32;

const SERVER_TO_CONTROLLER: &[u8] = b"Tor safe cookie authentication server-to-controller hash";
const CONTROLLER_TO_SERVER: &[u8] = b"Tor safe cookie authentication controller-to-server hash";

/// What `PROTOCOLINFO` said.
pub(crate) struct ProtocolInfo {
    /// SAFECOOKIE is offered.
    pub(crate) safecookie: bool,
    /// The cookie file, if one was named.
    pub(crate) cookie_file: Option<PathBuf>,
    /// The Tor version.
    pub(crate) version: TorVersion,
}

/// Reads a `PROTOCOLINFO 1` reply. The first line must be
/// `PROTOCOLINFO 1`, the last `OK`; exactly one `AUTH` and one `VERSION`
/// line must be present; other lines are ignored, as the specification
/// requires.
pub(crate) fn parse_protocolinfo(reply: &Reply) -> Result<ProtocolInfo, TorError> {
    if reply.code() != 250 {
        return Err(TorError::CommandRefused);
    }
    let lines = reply.lines();
    let (first, rest) = lines.split_first().ok_or(TorError::InvalidTorResponse)?;
    let (last, middle) = rest.split_last().ok_or(TorError::InvalidTorResponse)?;
    if first.text() != b"PROTOCOLINFO 1" || last.text() != b"OK" {
        return Err(TorError::InvalidTorResponse);
    }
    let mut auth = None;
    let mut version = None;
    for line in middle {
        if let Some(arguments) = line.text().strip_prefix(b"AUTH ") {
            if auth.replace(parse_auth_line(arguments)?).is_some() {
                return Err(TorError::InvalidTorResponse);
            }
        } else if let Some(arguments) = line.text().strip_prefix(b"VERSION ") {
            if version.replace(parse_version_line(arguments)?).is_some() {
                return Err(TorError::InvalidTorResponse);
            }
        }
    }
    let (safecookie, cookie_file) = auth.ok_or(TorError::InvalidTorResponse)?;
    Ok(ProtocolInfo {
        safecookie,
        cookie_file,
        version: version.ok_or(TorError::InvalidTorResponse)?,
    })
}

/// `METHODS=A,B[ COOKIEFILE="..."]`
fn parse_auth_line(arguments: &[u8]) -> Result<(bool, Option<PathBuf>), TorError> {
    let methods = arguments
        .strip_prefix(b"METHODS=")
        .ok_or(TorError::InvalidTorResponse)?;
    let end = methods
        .iter()
        .position(|byte| *byte == b' ')
        .unwrap_or(methods.len());
    let (list, rest) = methods.split_at_checked(end).ok_or(TorError::InvalidTorResponse)?;
    let mut safecookie = false;
    for method in list.split(|byte| *byte == b',') {
        if method.is_empty() || !method.iter().all(u8::is_ascii_uppercase) {
            return Err(TorError::InvalidTorResponse);
        }
        if method == b"SAFECOOKIE" {
            safecookie = true;
        }
    }
    let cookie_file = match rest {
        [] => None,
        _ => {
            let quoted = rest
                .strip_prefix(b" COOKIEFILE=")
                .ok_or(TorError::InvalidTorResponse)?;
            let (path, after) = decode_quoted(quoted)?;
            if !after.is_empty() || path.is_empty() {
                return Err(TorError::InvalidTorResponse);
            }
            Some(path_from_bytes(path)?)
        }
    };
    Ok((safecookie, cookie_file))
}

/// A path from the bytes Tor sent. On Unix any bytes are a path.
fn path_from_bytes(bytes: Vec<u8>) -> Result<PathBuf, TorError> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Ok(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes)
            .map(PathBuf::from)
            .map_err(|_| TorError::InvalidTorResponse)
    }
}

/// `Tor="<version>"[ more arguments]`
fn parse_version_line(arguments: &[u8]) -> Result<TorVersion, TorError> {
    let quoted = arguments
        .strip_prefix(b"Tor=")
        .ok_or(TorError::InvalidTorResponse)?;
    let (text, after) = decode_quoted(quoted)?;
    if !(after.is_empty() || after.starts_with(b" ")) {
        return Err(TorError::InvalidTorResponse);
    }
    TorVersion::parse(&text).ok_or(TorError::InvalidTorResponse)
}

/// Decodes a quoted string at the start of `input`, with the C escapes Tor
/// produces (`\\`, `\"`, `\'`, `\n`, `\t`, `\r` and up to three octal
/// digits), and returns it with the input after the closing quote. A
/// backslash before any other character stands for that character, as the
/// specification's note on Tor's escaping recommends.
pub(crate) fn decode_quoted(input: &[u8]) -> Result<(Vec<u8>, &[u8]), TorError> {
    let body = input.strip_prefix(b"\"").ok_or(TorError::InvalidTorResponse)?;
    let mut out = Vec::with_capacity(body.len());
    let mut index = 0_usize;
    while let Some(byte) = body.get(index) {
        index = index.saturating_add(1);
        match byte {
            b'"' => {
                let rest = body.get(index..).ok_or(TorError::InvalidTorResponse)?;
                return Ok((out, rest));
            }
            b'\\' => {
                let escaped = *body.get(index).ok_or(TorError::InvalidTorResponse)?;
                index = index.saturating_add(1);
                match escaped {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'0'..=b'7' => {
                        let mut value = u16::from(escaped.wrapping_sub(b'0'));
                        for _ in 0..2 {
                            match body.get(index) {
                                Some(digit @ b'0'..=b'7') => {
                                    value = value
                                        .wrapping_shl(3)
                                        .wrapping_add(u16::from(digit.wrapping_sub(b'0')));
                                    index = index.saturating_add(1);
                                }
                                _ => break,
                            }
                        }
                        out.push(u8::try_from(value).map_err(|_| TorError::InvalidTorResponse)?);
                    }
                    other => out.push(other),
                }
            }
            other => out.push(*other),
        }
    }
    // No closing quote.
    Err(TorError::InvalidTorResponse)
}

/// An HMAC-SHA256 value that is erased when dropped.
pub(crate) type Hash = Zeroizing<[u8; 32]>;

/// The SAFECOOKIE server hash and client hash.
pub(crate) fn safecookie_hashes(
    cookie: &[u8; COOKIE_LEN],
    client_nonce: &[u8; NONCE_LEN],
    server_nonce: &[u8; NONCE_LEN],
) -> Result<(Hash, Hash), TorError> {
    let hash = |key: &[u8]| -> Result<Hash, TorError> {
        let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| TorError::ControlAuthentication)?;
        mac.update(cookie);
        mac.update(client_nonce);
        mac.update(server_nonce);
        let mut out = Zeroizing::new([0_u8; 32]);
        out.copy_from_slice(&mac.finalize().into_bytes());
        Ok(out)
    };
    Ok((hash(SERVER_TO_CONTROLLER)?, hash(CONTROLLER_TO_SERVER)?))
}

/// Checks the server hash against the expected one, in constant time.
pub(crate) fn server_hash_matches(expected: &[u8; 32], received: &[u8; 32]) -> bool {
    expected.ct_eq(received).into()
}

/// Reads `250 AUTHCHALLENGE SERVERHASH=<64 hex> SERVERNONCE=<64 hex>`.
pub(crate) fn parse_authchallenge(reply: &Reply) -> Result<([u8; 32], [u8; NONCE_LEN]), TorError> {
    if reply.code() != 250 {
        return Err(TorError::ControlAuthentication);
    }
    let [line] = reply.lines() else {
        return Err(TorError::InvalidTorResponse);
    };
    let rest = line
        .text()
        .strip_prefix(b"AUTHCHALLENGE SERVERHASH=")
        .ok_or(TorError::InvalidTorResponse)?;
    let (hash, rest) = rest.split_at_checked(64).ok_or(TorError::InvalidTorResponse)?;
    let nonce = rest
        .strip_prefix(b" SERVERNONCE=")
        .ok_or(TorError::InvalidTorResponse)?;
    Ok((unhex::<32>(hash)?, unhex::<NONCE_LEN>(nonce)?))
}

/// Reads the cookie file: an absolute path naming a regular file of
/// exactly 32 bytes. Anything else is refused without being used.
pub(crate) fn read_cookie(path: &Path) -> Result<Zeroizing<[u8; COOKIE_LEN]>, TorError> {
    if !path.is_absolute() {
        return Err(TorError::ControlAuthentication);
    }
    let file = std::fs::File::open(path).map_err(|_| TorError::ControlAuthentication)?;
    let metadata = file.metadata().map_err(|_| TorError::ControlAuthentication)?;
    if !metadata.is_file() || metadata.len() != 32 {
        return Err(TorError::ControlAuthentication);
    }
    // At most one byte more than a cookie, to see that the file ends.
    let mut content = Zeroizing::new(Vec::with_capacity(COOKIE_LEN.saturating_add(1)));
    file.take(33)
        .read_to_end(&mut content)
        .map_err(|_| TorError::ControlAuthentication)?;
    let mut cookie = Zeroizing::new([0_u8; COOKIE_LEN]);
    if content.len() != COOKIE_LEN {
        return Err(TorError::ControlAuthentication);
    }
    cookie.copy_from_slice(&content);
    Ok(cookie)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::reply::ReplyParser;

    fn reply(bytes: &[u8]) -> Reply {
        let (used, reply) = ReplyParser::new().feed(bytes).unwrap();
        assert_eq!(used, bytes.len());
        reply.unwrap()
    }

    /// Computed with Python's hmac and hashlib modules, independently of
    /// this crate, for cookie bytes 0 to 31, client nonce 0x11 repeated and
    /// server nonce 0x22 repeated. The specification has no vectors.
    const SERVER_HASH: &str = "2aecbf4d284befddbbacc3817b41959eda061ac7e156db0c734cb28aa5e903ed";
    const CLIENT_HASH: &str = "eea571786a3fd103b6aa1ffa46249c4c850c509d0c84c550ed9e41477511d2ba";

    #[test]
    fn safecookie_matches_the_independent_computation() {
        let cookie: [u8; 32] = core::array::from_fn(|index| u8::try_from(index).unwrap());
        let (server, client) = safecookie_hashes(&cookie, &[0x11; 32], &[0x22; 32]).unwrap();
        assert_eq!(server.as_slice(), unhex::<32>(SERVER_HASH.as_bytes()).unwrap());
        assert_eq!(client.as_slice(), unhex::<32>(CLIENT_HASH.as_bytes()).unwrap());
        // Every input enters both hashes.
        let (other, _) = safecookie_hashes(&cookie, &[0x12; 32], &[0x22; 32]).unwrap();
        assert_ne!(other.as_slice(), server.as_slice());
        let (other, _) = safecookie_hashes(&cookie, &[0x11; 32], &[0x23; 32]).unwrap();
        assert_ne!(other.as_slice(), server.as_slice());
        let (other, _) = safecookie_hashes(&[0; 32], &[0x11; 32], &[0x22; 32]).unwrap();
        assert_ne!(other.as_slice(), server.as_slice());
        assert!(server_hash_matches(&server, &server));
        assert!(!server_hash_matches(&server, &client));
    }

    #[test]
    fn protocolinfo_of_a_system_tor_is_read() {
        let info = parse_protocolinfo(&reply(
            b"250-PROTOCOLINFO 1\r\n\
              250-AUTH METHODS=COOKIE,SAFECOOKIE COOKIEFILE=\"/run/tor/control.authcookie\"\r\n\
              250-VERSION Tor=\"0.4.9.13\"\r\n\
              250 OK\r\n",
        ))
        .unwrap();
        assert!(info.safecookie);
        assert_eq!(info.cookie_file, Some(PathBuf::from("/run/tor/control.authcookie")));
        assert_eq!(info.version, TorVersion::new(0, 4, 9, 13));

        // A filter that offers no authentication, lines in another order,
        // and an unknown line.
        let info = parse_protocolinfo(&reply(
            b"250-PROTOCOLINFO 1\r\n\
              250-VERSION Tor=\"0.4.9.13\" Extra=1\r\n\
              250-FUTURE something\r\n\
              250-AUTH METHODS=NULL\r\n\
              250 OK\r\n",
        ))
        .unwrap();
        assert!(!info.safecookie);
        assert_eq!(info.cookie_file, None);
    }

    #[test]
    fn malformed_protocolinfo_is_refused() {
        for bad in [
            &b"250-PROTOCOLINFO 2\r\n250-AUTH METHODS=NULL\r\n250-VERSION Tor=\"0.4.9.13\"\r\n250 OK\r\n"[..],
            b"250-PROTOCOLINFO 1\r\n250-VERSION Tor=\"0.4.9.13\"\r\n250 OK\r\n",
            b"250-PROTOCOLINFO 1\r\n250-AUTH METHODS=NULL\r\n250 OK\r\n",
            b"250-PROTOCOLINFO 1\r\n250-AUTH METHODS=NULL\r\n250-AUTH METHODS=NULL\r\n250-VERSION Tor=\"0.4.9.13\"\r\n250 OK\r\n",
            b"250-PROTOCOLINFO 1\r\n250-AUTH METHODS=\r\n250-VERSION Tor=\"0.4.9.13\"\r\n250 OK\r\n",
            b"250-PROTOCOLINFO 1\r\n250-AUTH METHODS=safecookie\r\n250-VERSION Tor=\"0.4.9.13\"\r\n250 OK\r\n",
            b"250-PROTOCOLINFO 1\r\n250-AUTH METHODS=SAFECOOKIE COOKIEFILE=/x\r\n250-VERSION Tor=\"0.4.9.13\"\r\n250 OK\r\n",
            b"250-PROTOCOLINFO 1\r\n250-AUTH METHODS=SAFECOOKIE COOKIEFILE=\"/x\r\n250-VERSION Tor=\"0.4.9.13\"\r\n250 OK\r\n",
            b"250-PROTOCOLINFO 1\r\n250-AUTH METHODS=SAFECOOKIE COOKIEFILE=\"\"\r\n250-VERSION Tor=\"0.4.9.13\"\r\n250 OK\r\n",
            b"250-PROTOCOLINFO 1\r\n250-AUTH METHODS=NULL\r\n250-VERSION Tor=\"zero\"\r\n250 OK\r\n",
            b"250-PROTOCOLINFO 1\r\n250-AUTH METHODS=NULL\r\n250-VERSION Tor=\"0.4.9.13\"\r\n250 NOT OK\r\n",
            b"250 PROTOCOLINFO 1\r\n",
        ] {
            assert!(parse_protocolinfo(&reply(bad)).is_err(), "{}", String::from_utf8_lossy(bad));
        }
        assert_eq!(
            parse_protocolinfo(&reply(b"514 Authentication required.\r\n")).err(),
            Some(TorError::CommandRefused)
        );
    }

    #[test]
    fn quoted_strings_decode_with_tor_s_escapes() {
        let decoded = |input: &[u8]| decode_quoted(input).map(|(text, rest)| (text, rest.to_vec()));
        assert_eq!(decoded(b"\"abc\" rest"), Ok((b"abc".to_vec(), b" rest".to_vec())));
        assert_eq!(
            decoded(b"\"a\\\\b\\\"c\\n\\t\\r\\'\""),
            Ok((b"a\\b\"c\n\t\r'".to_vec(), Vec::new()))
        );
        // Octal, as Tor writes bytes outside printable ASCII.
        assert_eq!(decoded(b"\"\\303\\251\\0\\7\""), Ok((vec![0xC3, 0xA9, 0, 7], Vec::new())));
        assert_eq!(decoded(b"\"\\q\""), Ok((b"q".to_vec(), Vec::new())));
        for bad in [&b"abc"[..], b"\"abc", b"\"abc\\", b"\"\\777\"", b""] {
            assert!(decode_quoted(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn authchallenge_replies_are_read_exactly() {
        let good = format!(
            "250 AUTHCHALLENGE SERVERHASH={} SERVERNONCE={}\r\n",
            "AB".repeat(32),
            "cd".repeat(32)
        );
        let (hash, nonce) = parse_authchallenge(&reply(good.as_bytes())).unwrap();
        assert_eq!(hash, [0xAB; 32]);
        assert_eq!(nonce, [0xCD; 32]);
        for bad in [
            format!("250 AUTHCHALLENGE SERVERHASH={} SERVERNONCE={}\r\n", "AB".repeat(31), "cd".repeat(32)),
            format!("250 AUTHCHALLENGE SERVERHASH={} SERVERNONCE={}\r\n", "AB".repeat(32), "cd".repeat(33)),
            format!("250 AUTHCHALLENGE SERVERNONCE={} SERVERHASH={}\r\n", "cd".repeat(32), "AB".repeat(32)),
            format!("250 AUTHCHALLENGE SERVERHASH={}XX SERVERNONCE={}\r\n", "AB".repeat(31), "cd".repeat(32)),
            format!("250-AUTHCHALLENGE SERVERHASH={} SERVERNONCE={}\r\n250 OK\r\n", "AB".repeat(32), "cd".repeat(32)),
        ] {
            assert!(parse_authchallenge(&reply(bad.as_bytes())).is_err(), "{bad}");
        }
        assert_eq!(
            parse_authchallenge(&reply(b"515 Cookie authentication is disabled\r\n")).err(),
            Some(TorError::ControlAuthentication)
        );
    }

    #[test]
    fn only_a_regular_file_of_exactly_32_bytes_is_a_cookie() {
        let dir = std::env::temp_dir().join(format!("monolith-cookie-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("good");
        std::fs::write(&good, [7_u8; 32]).unwrap();
        assert_eq!(*read_cookie(&good).unwrap(), [7_u8; 32]);
        for (name, len) in [("short", 31), ("long", 33), ("empty", 0)] {
            let path = dir.join(name);
            std::fs::write(&path, vec![7_u8; len]).unwrap();
            assert_eq!(read_cookie(&path).err(), Some(TorError::ControlAuthentication), "{name}");
        }
        assert!(read_cookie(&dir).is_err());
        assert!(read_cookie(&dir.join("missing")).is_err());
        assert!(read_cookie(Path::new("relative/cookie")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

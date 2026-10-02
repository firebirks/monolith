//! A SOCKS5 client for one purpose: a CONNECT to port 29170 of an onion
//! hostname, through Tor, with isolation credentials.
//!
//! One authentication method (username and password, RFC 1929), one
//! command (CONNECT), one address type (domain name, with the `.onion`
//! hostname built from a validated key). There is no other address type,
//! no other port and no way to ask for anything else, so nothing can reach
//! a resolver: Tor receives the name and does the onion service lookup
//! itself (`docs/TOR_INTEGRATION.md` section 3).
//!
//! The encoders and the reply decoder are plain functions over bytes; the
//! driver at the end runs them over a stream with the timeouts of
//! `RESOURCE_LIMITS.md`. Replies are read exactly: the CONNECT reply is
//! read up to its last byte and no further, because the bytes after it
//! belong to the peer.

use monolith_identity::ServiceId;
use monolith_protocol::limits::{
    CONNECT_TIMEOUT, MAX_SOCKS_REPLY_LEN, ONION_VIRTUAL_PORT, SOCKS_NEGOTIATION_TIMEOUT,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroizing;

use crate::{IsolationGroup, TorError};

const VERSION: u8 = 0x05;
const METHOD_USERNAME_PASSWORD: u8 = 0x02;
const AUTH_VERSION: u8 = 0x01;
const COMMAND_CONNECT: u8 = 0x01;
const ADDRESS_IPV4: u8 = 0x01;
const ADDRESS_DOMAIN: u8 = 0x03;
const ADDRESS_IPV6: u8 = 0x04;

/// The username of Tor's structured isolation format: the password is the
/// isolation parameter (`<torS0X>0`, Tor 0.4.9.1 and later).
const USERNAME: &[u8] = b"<torS0X>0";

/// The greeting: version 5, one method, username and password.
pub(crate) const GREETING: [u8; 3] = [VERSION, 0x01, METHOD_USERNAME_PASSWORD];

/// The authentication request that carries the isolation token.
pub(crate) fn auth_request(isolation: &IsolationGroup) -> Zeroizing<Vec<u8>> {
    let password = isolation.password();
    let mut request = Zeroizing::new(Vec::with_capacity(64));
    request.push(AUTH_VERSION);
    request.push(u8::try_from(USERNAME.len()).unwrap_or(u8::MAX));
    request.extend_from_slice(USERNAME);
    request.push(u8::try_from(password.len()).unwrap_or(u8::MAX));
    request.extend_from_slice(&password);
    request
}

/// The CONNECT request for port `ONION_VIRTUAL_PORT` of `target`, with the
/// `.onion` hostname as a domain name.
pub(crate) fn connect_request(target: &ServiceId) -> Vec<u8> {
    let hostname = target.hostname();
    let mut request = Vec::with_capacity(hostname.len().saturating_add(7));
    request.extend_from_slice(&[VERSION, COMMAND_CONNECT, 0x00, ADDRESS_DOMAIN]);
    request.push(u8::try_from(hostname.len()).unwrap_or(u8::MAX));
    request.extend_from_slice(hostname.as_bytes());
    request.extend_from_slice(&ONION_VIRTUAL_PORT.to_be_bytes());
    request
}

/// Checks the proxy's choice of method. Only username and password is
/// accepted: without it the isolation credentials would not be in effect.
pub(crate) fn check_method_reply(reply: [u8; 2]) -> Result<(), TorError> {
    match reply {
        [VERSION, METHOD_USERNAME_PASSWORD] => Ok(()),
        // No acceptable method, or a method Monolith did not offer.
        [VERSION, _] => Err(TorError::SocksAuthentication),
        _ => Err(TorError::SocksProtocol),
    }
}

/// Checks the answer to the authentication request.
pub(crate) fn check_auth_reply(reply: [u8; 2]) -> Result<(), TorError> {
    match reply {
        [AUTH_VERSION, 0x00] => Ok(()),
        [AUTH_VERSION, _] => Err(TorError::SocksAuthentication),
        _ => Err(TorError::SocksProtocol),
    }
}

/// The outcome a CONNECT reply reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectReply {
    /// The stream is open.
    Succeeded,
    /// The request failed with this reply code.
    Failed(u8),
}

/// Decodes a CONNECT reply from the start of `input`.
///
/// Returns `Ok(None)` while the reply is incomplete, and the outcome with
/// the length of the reply once it is complete. Bytes after the reply are
/// not looked at. The version must be 5 and the reserved byte 0; the bound
/// address may be IPv4, IPv6 or a domain name (Tor sends a zero IPv4 or
/// IPv6 address) and is skipped. No reply is longer than
/// `MAX_SOCKS_REPLY_LEN`.
pub fn decode_connect_reply(input: &[u8]) -> Result<Option<(ConnectReply, usize)>, TorError> {
    let Some(header) = input.first_chunk::<4>() else {
        return check_prefix(input).map(|()| None);
    };
    let [version, code, reserved, address_type] = *header;
    if version != VERSION || reserved != 0x00 {
        return Err(TorError::SocksProtocol);
    }
    let address_len = match address_type {
        ADDRESS_IPV4 => 4,
        ADDRESS_IPV6 => 16,
        ADDRESS_DOMAIN => match input.get(4) {
            Some(0) => return Err(TorError::SocksProtocol),
            Some(len) => usize::from(*len).saturating_add(1),
            None => return Ok(None),
        },
        _ => return Err(TorError::SocksProtocol),
    };
    let total = address_len.saturating_add(6);
    if total > MAX_SOCKS_REPLY_LEN {
        return Err(TorError::SocksProtocol);
    }
    if input.len() < total {
        return Ok(None);
    }
    let reply = if code == 0x00 {
        ConnectReply::Succeeded
    } else {
        ConnectReply::Failed(code)
    };
    Ok(Some((reply, total)))
}

/// Checks the first bytes of a reply before the header is complete, so
/// that a wrong version is refused at once.
fn check_prefix(input: &[u8]) -> Result<(), TorError> {
    match input {
        [] | [VERSION] | [VERSION, _] => Ok(()),
        [VERSION, _, 0x00] => Ok(()),
        _ => Err(TorError::SocksProtocol),
    }
}

/// Maps a failed CONNECT to an error. Nothing here leads to another
/// attempt by another route.
pub(crate) const fn failure(code: u8) -> TorError {
    match code {
        // General failure, not allowed by the rule set, command or address
        // type not supported.
        0x01 | 0x02 | 0x07 | 0x08 => TorError::SocksRefused,
        // Network or host unreachable, refused, TTL expired, and Tor's
        // onion service codes: descriptor not found or invalid,
        // introduction or rendezvous failed, client authorization missing
        // or wrong, bad address, introduction timed out.
        0x03..=0x06 | 0xF0..=0xF7 => TorError::OnionUnreachable,
        _ => TorError::SocksProtocol,
    }
}

/// Runs the SOCKS5 exchange on `stream`, which is connected to the proxy.
/// On success the stream carries the onion service connection.
pub(crate) async fn negotiate<S>(
    stream: &mut S,
    target: &ServiceId,
    isolation: &IsolationGroup,
) -> Result<(), TorError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::time::timeout(SOCKS_NEGOTIATION_TIMEOUT, async {
        stream.write_all(&GREETING).await.map_err(|_| TorError::SocksUnavailable)?;
        check_method_reply(read_two(stream).await?)?;
        let auth = auth_request(isolation);
        stream.write_all(&auth).await.map_err(|_| TorError::SocksUnavailable)?;
        check_auth_reply(read_two(stream).await?)
    })
    .await
    .map_err(|_| TorError::TimedOut)??;

    tokio::time::timeout(CONNECT_TIMEOUT, async {
        stream
            .write_all(&connect_request(target))
            .await
            .map_err(|_| TorError::SocksUnavailable)?;
        let mut reply = [0_u8; MAX_SOCKS_REPLY_LEN];
        let mut filled = 0_usize;
        loop {
            // One byte at a time, so that nothing after the reply is read.
            let slot = reply
                .get_mut(filled..=filled)
                .ok_or(TorError::SocksProtocol)?;
            stream
                .read_exact(slot)
                .await
                .map_err(|_| TorError::SocksProtocol)?;
            filled = filled.saturating_add(1);
            let decoded = decode_connect_reply(reply.get(..filled).unwrap_or_default())?;
            match decoded {
                None => {}
                Some((ConnectReply::Succeeded, _)) => return Ok(()),
                Some((ConnectReply::Failed(code), _)) => return Err(failure(code)),
            }
        }
    })
    .await
    .map_err(|_| TorError::TimedOut)?
}

async fn read_two<S: AsyncRead + Unpin>(stream: &mut S) -> Result<[u8; 2], TorError> {
    let mut reply = [0_u8; 2];
    stream
        .read_exact(&mut reply)
        .await
        .map_err(|_| TorError::SocksProtocol)?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use monolith_identity::{IdentitySecretKey, OnionServiceKey};
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

    fn target() -> ServiceId {
        let key = IdentitySecretKey::from_seed(&[9; 32]).public_key();
        ServiceId::from_key(&OnionServiceKey::from_bytes(key.as_bytes()).unwrap())
    }

    #[test]
    fn requests_are_byte_for_byte_those_of_the_specification() {
        assert_eq!(GREETING, [0x05, 0x01, 0x02]);

        let group = IsolationGroup::generate().unwrap();
        let auth = auth_request(&group);
        assert_eq!(auth[0], 0x01);
        assert_eq!(&auth[1..11], b"\x09<torS0X>0");
        assert_eq!(auth[11], 32);
        assert_eq!(&auth[12..], group.password().as_slice());
        assert_eq!(auth.len(), 44);

        let id = target();
        let request = connect_request(&id);
        assert_eq!(&request[..5], &[0x05, 0x01, 0x00, 0x03, 62]);
        // The literal onion hostname, as Tor must receive it.
        assert_eq!(&request[5..67], id.hostname().as_bytes());
        assert!(request[5..67].ends_with(b".onion"));
        assert_eq!(&request[67..], &[0x71, 0xF2]);
        assert_eq!(u16::from_be_bytes([0x71, 0xF2]), 29170);
    }

    #[test]
    fn the_isolation_token_names_nothing_about_the_target() {
        // The credentials are the same for every target and differ for
        // every group: they come from the group alone.
        let group = IsolationGroup::generate().unwrap();
        let first = auth_request(&group);
        assert_eq!(auth_request(&group).as_slice(), first.as_slice());
        let other = IsolationGroup::generate().unwrap();
        assert_ne!(auth_request(&other).as_slice(), first.as_slice());
        let id = target();
        assert!(!first.windows(8).any(|w| id.as_str().as_bytes().starts_with(w)));
    }

    #[test]
    fn method_and_auth_replies_are_checked() {
        assert_eq!(check_method_reply([5, 2]), Ok(()));
        for refused in [[5, 0], [5, 1], [5, 0xff]] {
            assert_eq!(check_method_reply(refused), Err(TorError::SocksAuthentication));
        }
        for bad in [[4, 2], [0, 0], [0x48, 0x54]] {
            assert_eq!(check_method_reply(bad), Err(TorError::SocksProtocol));
        }
        assert_eq!(check_auth_reply([1, 0]), Ok(()));
        assert_eq!(check_auth_reply([1, 1]), Err(TorError::SocksAuthentication));
        assert_eq!(check_auth_reply([5, 0]), Err(TorError::SocksProtocol));
    }

    #[test]
    fn connect_replies_decode_with_their_exact_length() {
        let v4 = [5, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        assert_eq!(decode_connect_reply(&v4), Ok(Some((ConnectReply::Succeeded, 10))));
        let mut v6 = vec![5, 0, 0, 4];
        v6.extend_from_slice(&[0; 18]);
        assert_eq!(decode_connect_reply(&v6), Ok(Some((ConnectReply::Succeeded, 22))));
        let mut domain = vec![5, 0, 0, 3, 3, b'a', b'b', b'c', 0, 0];
        assert_eq!(decode_connect_reply(&domain), Ok(Some((ConnectReply::Succeeded, 10))));
        // Bytes after the reply are not part of it.
        domain.extend_from_slice(b"peer data");
        assert_eq!(decode_connect_reply(&domain), Ok(Some((ConnectReply::Succeeded, 10))));
        // A failure code.
        let failed = [5, 0xF2, 0, 1, 0, 0, 0, 0, 0, 0];
        assert_eq!(decode_connect_reply(&failed), Ok(Some((ConnectReply::Failed(0xF2), 10))));
        // Every proper prefix is incomplete, not an error.
        for len in 0..v6.len() {
            assert_eq!(decode_connect_reply(&v6[..len]), Ok(None), "{len}");
        }
        // The longest possible reply.
        let mut longest = vec![5, 0, 0, 3, 255];
        longest.extend_from_slice(&[b'x'; 255]);
        longest.extend_from_slice(&[0, 0]);
        assert_eq!(longest.len(), MAX_SOCKS_REPLY_LEN);
        assert_eq!(
            decode_connect_reply(&longest),
            Ok(Some((ConnectReply::Succeeded, MAX_SOCKS_REPLY_LEN)))
        );
    }

    #[test]
    fn malformed_connect_replies_are_refused() {
        for bad in [
            &[4_u8][..],
            &[5, 0, 1],
            &[5, 0, 0, 2, 0, 0],
            &[5, 0, 0, 0],
            &[5, 0, 0, 5],
            &[5, 0, 0, 3, 0, 0, 0],
            &[0x48, 0x54, 0x54, 0x50],
        ] {
            assert_eq!(decode_connect_reply(bad), Err(TorError::SocksProtocol), "{bad:?}");
        }
    }

    #[test]
    fn failure_codes_map_to_errors_without_a_fallback() {
        assert_eq!(failure(0x01), TorError::SocksRefused);
        assert_eq!(failure(0x02), TorError::SocksRefused);
        for code in [0x03, 0x04, 0x05, 0x06, 0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7] {
            assert_eq!(failure(code), TorError::OnionUnreachable, "{code:#x}");
        }
        assert_eq!(failure(0x07), TorError::SocksRefused);
        assert_eq!(failure(0x09), TorError::SocksProtocol);
        assert_eq!(failure(0xF8), TorError::SocksProtocol);
    }

    /// A scripted proxy on the other end of an in-memory stream: it checks
    /// what the client sends and answers with `answers`, in pieces of
    /// `piece` bytes.
    async fn scripted(
        answers: Vec<u8>,
        piece: usize,
        trailing: &'static [u8],
    ) -> (Result<(), TorError>, Vec<u8>, Vec<u8>) {
        let (mut client, mut proxy) = duplex(4096);
        let id = target();
        let group = IsolationGroup::generate().unwrap();
        let proxy_task = tokio::spawn(async move {
            let mut received = vec![0_u8; 3 + 44 + 69];
            let mut got = 0;
            // Read whatever the client sends before each answer chunk.
            for chunk in answers.chunks(piece.max(1)) {
                let mut buffer = [0_u8; 256];
                if let Ok(Ok(n)) = tokio::time::timeout(
                    core::time::Duration::from_millis(50),
                    proxy.read(&mut buffer),
                )
                .await
                {
                    received[got..got + n].copy_from_slice(&buffer[..n]);
                    got += n;
                }
                let _ = proxy.write_all(chunk).await;
            }
            let _ = proxy.write_all(trailing).await;
            let mut rest = [0_u8; 256];
            while let Ok(Ok(n)) =
                tokio::time::timeout(core::time::Duration::from_millis(50), proxy.read(&mut rest))
                    .await
            {
                if n == 0 {
                    break;
                }
                received[got..got + n].copy_from_slice(&rest[..n]);
                got += n;
            }
            received.truncate(got);
            received
        });
        let result = negotiate(&mut client, &id, &group).await;
        let mut after = Vec::new();
        if result.is_ok() {
            let mut buffer = [0_u8; 64];
            if let Ok(Ok(n)) =
                tokio::time::timeout(core::time::Duration::from_millis(100), client.read(&mut buffer))
                    .await
            {
                after.extend_from_slice(&buffer[..n]);
            }
        }
        drop(client);
        (result, proxy_task.await.unwrap(), after)
    }

    fn success_answers() -> Vec<u8> {
        let mut answers = vec![5, 2, 1, 0];
        answers.extend_from_slice(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        answers
    }

    #[test]
    fn a_successful_exchange_leaves_the_peer_bytes_on_the_stream() {
        crate::testing::run(a_successful_exchange());
    }

    async fn a_successful_exchange() {
        for piece in [1, 2, 3, 14] {
            let (result, sent, after) = scripted(success_answers(), piece, b"peer").await;
            assert_eq!(result, Ok(()), "{piece}");
            // Greeting, authentication and the CONNECT with the hostname.
            assert_eq!(&sent[..3], &GREETING);
            assert!(sent.windows(6).any(|w| w == b".onion"));
            // Nothing after the reply was consumed.
            assert_eq!(after, b"peer", "{piece}");
        }
    }

    #[test]
    fn every_failure_of_the_proxy_is_an_error() {
        crate::testing::run(every_failure());
    }

    async fn every_failure() {
        let cases: Vec<(Vec<u8>, TorError)> = vec![
            (vec![5, 0], TorError::SocksAuthentication),
            (vec![5, 0xff], TorError::SocksAuthentication),
            (vec![4, 2], TorError::SocksProtocol),
            (vec![5, 2, 1, 1], TorError::SocksAuthentication),
            (vec![5, 2, 1, 0, 5, 0x05, 0, 1, 0, 0, 0, 0, 0, 0], TorError::OnionUnreachable),
            (vec![5, 2, 1, 0, 5, 0xF0, 0, 1, 0, 0, 0, 0, 0, 0], TorError::OnionUnreachable),
            (vec![5, 2, 1, 0, 5, 0x01, 0, 1, 0, 0, 0, 0, 0, 0], TorError::SocksRefused),
            (vec![5, 2, 1, 0, 5, 0, 1, 1, 0, 0, 0, 0, 0, 0], TorError::SocksProtocol),
            (vec![5, 2, 1, 0, 5, 0, 0, 9], TorError::SocksProtocol),
            (vec![5, 2, 1, 0, 6, 0, 0, 1], TorError::SocksProtocol),
            // The proxy closes in the middle of a reply.
            (vec![5, 2, 1, 0, 5, 0, 0, 1, 0, 0], TorError::SocksProtocol),
            (vec![5], TorError::SocksProtocol),
        ];
        for (answers, expected) in cases {
            let (result, _, _) = scripted(answers.clone(), 64, b"").await;
            assert_eq!(result, Err(expected), "{answers:?}");
        }
    }

    #[test]
    fn a_stalled_proxy_times_out() {
        crate::testing::run_paused(a_stalled_proxy());
    }

    async fn a_stalled_proxy() {
        // Silent after the greeting.
        let (mut client, _proxy) = duplex(4096);
        let group = IsolationGroup::generate().unwrap();
        let result = negotiate(&mut client, &target(), &group).await;
        assert_eq!(result, Err(TorError::TimedOut));

        // Silent after the CONNECT: the longer timeout applies.
        let (mut client, mut proxy) = duplex(4096);
        let started = tokio::time::Instant::now();
        let answer = tokio::spawn(async move {
            let mut buffer = [0_u8; 128];
            let _ = proxy.read(&mut buffer).await;
            let _ = proxy.write_all(&[5, 2]).await;
            let _ = proxy.read(&mut buffer).await;
            let _ = proxy.write_all(&[1, 0]).await;
            let _ = proxy.read(&mut buffer).await;
            // Never answers the CONNECT; keep the stream open.
            core::future::pending::<()>().await;
        });
        let result = negotiate(&mut client, &target(), &group).await;
        assert_eq!(result, Err(TorError::TimedOut));
        assert!(started.elapsed() >= CONNECT_TIMEOUT);
        answer.abort();
    }
}

//! A control connection to Tor, with the commands of
//! `docs/TOR_CONTROL_SURFACE.md` and nothing else.
//!
//! The connection offers typed operations only. There is no function that
//! sends a command, a GETINFO key or an option given as a string, inside
//! the crate or outside it.

pub(crate) mod auth;
pub(crate) mod command;
mod reply;

use core::net::SocketAddrV4;

use monolith_identity::{OnionServiceKey, ServiceId};
use monolith_protocol::limits::{CONTROL_COMMAND_TIMEOUT, CONTROL_CONNECT_TIMEOUT};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zeroize::Zeroizing;

pub(crate) use command::OnionKey;
pub use reply::{Reply, ReplyParser};

use self::auth::{
    NONCE_LEN, parse_authchallenge, parse_protocolinfo, read_cookie, safecookie_hashes,
    server_hash_matches,
};
use self::command::Command;
use crate::stream::{TorStream, connect};
use crate::{Bootstrap, ControlAuth, Endpoint, OnionServiceSecret, TorError, TorVersion};

/// An authenticated control connection.
pub(crate) struct ControlConnection {
    stream: TorStream,
    parser: ReplyParser,
    version: TorVersion,
}

impl ControlConnection {
    /// Connects to the configured endpoint and authenticates, within
    /// `CONTROL_CONNECT_TIMEOUT`.
    pub(crate) async fn open(endpoint: &Endpoint, auth: &ControlAuth) -> Result<Self, TorError> {
        tokio::time::timeout(CONTROL_CONNECT_TIMEOUT, Self::open_inner(endpoint, auth))
            .await
            .map_err(|_| TorError::TimedOut)?
    }

    async fn open_inner(endpoint: &Endpoint, auth: &ControlAuth) -> Result<Self, TorError> {
        let stream = connect(endpoint)
            .await
            .map_err(|_| TorError::ControlUnavailable)?;
        let mut connection = Self {
            stream,
            parser: ReplyParser::new(),
            version: TorVersion::new(0, 0, 0, 0),
        };
        let info = parse_protocolinfo(&connection.command(&Command::ProtocolInfo).await?)?;
        connection.version = info.version;
        match auth {
            ControlAuth::SafeCookie { cookie_file } => {
                if !info.safecookie {
                    return Err(TorError::ControlAuthenticationUnavailable);
                }
                // The endpoint must name the configured cookie file. Any
                // other file could be one the endpoint wrote itself, which
                // would let it pass SAFECOOKIE; nothing is read then.
                let named = info
                    .cookie_file
                    .ok_or(TorError::ControlAuthenticationUnavailable)?;
                if named != *cookie_file {
                    return Err(TorError::ControlAuthentication);
                }
                let cookie = read_cookie(cookie_file)?;
                let mut client_nonce = Zeroizing::new([0_u8; NONCE_LEN]);
                getrandom::fill(client_nonce.as_mut_slice()).map_err(|_| TorError::Randomness)?;
                let challenge = connection
                    .command(&Command::AuthChallenge(&client_nonce))
                    .await?;
                let (server_hash, server_nonce) = parse_authchallenge(&challenge)?;
                let (expected, client_hash) =
                    safecookie_hashes(&cookie, &client_nonce, &server_nonce)?;
                // The server proves it knows the cookie before Monolith does.
                if !server_hash_matches(&expected, &server_hash) {
                    return Err(TorError::ControlAuthentication);
                }
                let reply = connection
                    .command(&Command::AuthenticateSafeCookie(&client_hash))
                    .await?;
                expect_ok(&reply).map_err(|_| TorError::ControlAuthentication)?;
            }
            ControlAuth::TrustedFilter => {
                let reply = connection
                    .command(&Command::AuthenticateTrustedFilter)
                    .await?;
                expect_ok(&reply).map_err(|_| TorError::ControlAuthentication)?;
            }
        }
        Ok(connection)
    }

    /// The Tor version `PROTOCOLINFO` reported.
    pub(crate) const fn version(&self) -> TorVersion {
        self.version
    }

    /// Sends one command and reads its reply, within
    /// `CONTROL_COMMAND_TIMEOUT`.
    async fn command(&mut self, command: &Command<'_>) -> Result<Reply, TorError> {
        let line = command.encode()?;
        tokio::time::timeout(CONTROL_COMMAND_TIMEOUT, async {
            self.stream
                .write_all(&line)
                .await
                .map_err(|_| TorError::ControlLost)?;
            self.read_reply().await
        })
        .await
        .map_err(|_| TorError::TimedOut)?
    }

    async fn read_reply(&mut self) -> Result<Reply, TorError> {
        let mut chunk = Zeroizing::new([0_u8; 512]);
        loop {
            let read = self
                .stream
                .read(chunk.as_mut_slice())
                .await
                .map_err(|_| TorError::ControlLost)?;
            if read == 0 {
                return Err(TorError::ControlLost);
            }
            let input = chunk.get(..read).ok_or(TorError::InvalidTorResponse)?;
            let (used, reply) = self.parser.feed(input)?;
            if let Some(reply) = reply {
                // Monolith sends one command at a time and subscribes to no
                // event: nothing may follow the end of a reply.
                if used != read {
                    return Err(TorError::InvalidTorResponse);
                }
                return Ok(reply);
            }
        }
    }

    /// `GETINFO status/circuit-established`.
    pub(crate) async fn circuit_established(&mut self) -> Result<bool, TorError> {
        let reply = self.command(&Command::CircuitEstablished).await?;
        parse_circuit_established(&reply)
    }

    /// `GETINFO status/bootstrap-phase`, reduced to the progress number and
    /// whether Tor is done. A refusal makes it unknown.
    pub(crate) async fn bootstrap(&mut self) -> Result<Bootstrap, TorError> {
        let reply = self.command(&Command::BootstrapPhase).await?;
        parse_bootstrap_reply(&reply)
    }

    /// `ADD_ONION`, with `target` as the address Tor connects to.
    pub(crate) async fn add_onion(
        &mut self,
        key: OnionKey<'_>,
        target: SocketAddrV4,
    ) -> Result<(OnionServiceKey, Option<OnionServiceSecret>), TorError> {
        let new = matches!(key, OnionKey::New);
        let reply = self.command(&Command::AddOnion { key, target }).await?;
        parse_add_onion(&reply, new)
    }

    /// `DEL_ONION`.
    pub(crate) async fn del_onion(&mut self, service: &ServiceId) -> Result<(), TorError> {
        let reply = self.command(&Command::DelOnion(service)).await?;
        expect_ok(&reply).map_err(|_| TorError::OnionRemovalFailed)
    }

    /// Returns false if the connection is known to be gone. Tor sends
    /// nothing unasked, so readable data or the end of the stream both mean
    /// the connection can no longer be trusted.
    pub(crate) fn is_alive(&self) -> bool {
        let mut probe = [0_u8; 1];
        matches!(self.stream.try_read(&mut probe), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    }

    /// Waits until the connection is gone.
    pub(crate) async fn closed(&self) {
        loop {
            if self.stream.readable().await.is_err() || !self.is_alive() {
                return;
            }
        }
    }
}

/// Reads the reply to `GETINFO status/circuit-established`: `0` or `1`.
pub(crate) fn parse_circuit_established(reply: &Reply) -> Result<bool, TorError> {
    if reply.code() != 250 {
        return Err(TorError::CommandRefused);
    }
    match getinfo_value(reply, b"status/circuit-established")? {
        b"1" => Ok(true),
        b"0" => Ok(false),
        _ => Err(TorError::InvalidTorResponse),
    }
}

/// Reads the reply to `GETINFO status/bootstrap-phase`. A refusal makes
/// the progress unknown.
pub(crate) fn parse_bootstrap_reply(reply: &Reply) -> Result<Bootstrap, TorError> {
    if reply.code() != 250 {
        return Ok(Bootstrap::Unknown);
    }
    Ok(parse_bootstrap(getinfo_value(
        reply,
        b"status/bootstrap-phase",
    )?))
}

/// Checks a reply that must be exactly `250 OK`.
fn expect_ok(reply: &Reply) -> Result<(), TorError> {
    match reply.lines() {
        [line] if line.code() == 250 && line.text() == b"OK" => Ok(()),
        _ if reply.code() == 250 => Err(TorError::InvalidTorResponse),
        _ => Err(TorError::CommandRefused),
    }
}

/// The value of a one-key GETINFO reply: `250-<key>=<value>`, `250 OK`.
fn getinfo_value<'a>(reply: &'a Reply, key: &[u8]) -> Result<&'a [u8], TorError> {
    let [line, last] = reply.lines() else {
        return Err(TorError::InvalidTorResponse);
    };
    if last.text() != b"OK" {
        return Err(TorError::InvalidTorResponse);
    }
    line.text()
        .strip_prefix(key)
        .and_then(|rest| rest.strip_prefix(b"="))
        .ok_or(TorError::InvalidTorResponse)
}

/// Reads `NOTICE BOOTSTRAP PROGRESS=<n> TAG=<tag> SUMMARY="..."` (or the
/// `WARN` form) and keeps only the number and whether the tag is `done`.
/// Parsing stops at the first token with a quote, which is `SUMMARY`;
/// nothing after it, including the warning form's `REASON`, `HOSTID` and
/// `HOSTADDR`, is looked at, and nothing at all is kept.
pub(crate) fn parse_bootstrap(value: &[u8]) -> Bootstrap {
    let mut tokens = value.split(|byte| *byte == b' ');
    let severity = tokens.next();
    if !matches!(severity, Some(b"NOTICE" | b"WARN")) || tokens.next() != Some(b"BOOTSTRAP") {
        return Bootstrap::Unknown;
    }
    let mut progress = None;
    let mut done = false;
    for token in tokens {
        if token.contains(&b'"') {
            break;
        }
        if let Some(number) = token.strip_prefix(b"PROGRESS=") {
            progress = parse_percent(number);
        } else if token == b"TAG=done" {
            done = true;
        }
    }
    match (done, progress) {
        (true, _) => Bootstrap::Done,
        (false, Some(percent)) => Bootstrap::InProgress(percent),
        (false, None) => Bootstrap::Unknown,
    }
}

fn parse_percent(text: &[u8]) -> Option<u8> {
    if text.is_empty() || text.len() > 3 || !text.iter().all(u8::is_ascii_digit) {
        return None;
    }
    core::str::from_utf8(text)
        .ok()?
        .parse::<u8>()
        .ok()
        .filter(|percent| *percent <= 100)
}

/// Reads an `ADD_ONION` reply: `250-ServiceID=<id>`, then
/// `250-PrivateKey=ED25519-V3:<key>` exactly when a new key was asked for,
/// then `250 OK`. The ServiceID is decoded as an onion address.
pub(crate) fn parse_add_onion(
    reply: &Reply,
    new_key: bool,
) -> Result<(OnionServiceKey, Option<OnionServiceSecret>), TorError> {
    match reply.code() {
        250 => {}
        512 if reply.lines().first().is_some_and(|line| {
            line.text()
                .starts_with(b"Tor is in non-anonymous hidden service mode")
        }) =>
        {
            return Err(TorError::NonAnonymousTorMode);
        }
        510 => return Err(TorError::CommandRefused),
        _ => return Err(TorError::OnionPublicationFailed),
    }
    let (id_line, secret_line) = match (reply.lines(), new_key) {
        ([id, key, last], true) if last.text() == b"OK" => (id, Some(key)),
        ([id, last], false) if last.text() == b"OK" => (id, None),
        _ => return Err(TorError::InvalidTorResponse),
    };
    let id = id_line
        .text()
        .strip_prefix(b"ServiceID=")
        .ok_or(TorError::InvalidTorResponse)?;
    let key = ServiceId::parse(id).map_err(|_| TorError::InvalidTorResponse)?;
    let secret = match secret_line {
        None => None,
        Some(line) => {
            let blob = line
                .text()
                .strip_prefix(b"PrivateKey=ED25519-V3:")
                .ok_or(TorError::InvalidTorResponse)?;
            Some(OnionServiceSecret::from_base64(blob)?)
        }
    };
    Ok((key, secret))
}

#[cfg(test)]
mod tests {
    use super::*;
    use monolith_identity::IdentitySecretKey;

    fn reply(bytes: &[u8]) -> Reply {
        let (used, reply) = ReplyParser::new().feed(bytes).unwrap();
        assert_eq!(used, bytes.len());
        reply.unwrap()
    }

    fn service_id() -> ServiceId {
        let key = IdentitySecretKey::from_seed(&[5; 32]).public_key();
        ServiceId::from_key(&OnionServiceKey::from_bytes(key.as_bytes()).unwrap())
    }

    #[test]
    fn bootstrap_lines_keep_the_number_and_done_only() {
        assert_eq!(
            parse_bootstrap(b"NOTICE BOOTSTRAP PROGRESS=100 TAG=done SUMMARY=\"Done\""),
            Bootstrap::Done
        );
        assert_eq!(
            parse_bootstrap(b"NOTICE BOOTSTRAP PROGRESS=45 TAG=loading_descriptors SUMMARY=\"Loading relay descriptors\""),
            Bootstrap::InProgress(45)
        );
        // The warning form: the host fields after SUMMARY are not read.
        assert_eq!(
            parse_bootstrap(b"WARN BOOTSTRAP PROGRESS=10 TAG=conn_done SUMMARY=\"Connected\" WARNING=\"x\" REASON=TIMEOUT COUNT=1 RECOMMENDATION=warn HOSTID=\"0123\" HOSTADDR=\"192.0.2.1:443\""),
            Bootstrap::InProgress(10)
        );
        // A tag named in the summary does not count.
        assert_eq!(
            parse_bootstrap(b"NOTICE BOOTSTRAP PROGRESS=50 SUMMARY=\"x TAG=done\""),
            Bootstrap::InProgress(50)
        );
        // 100 without the tag is not done.
        assert_eq!(
            parse_bootstrap(b"NOTICE BOOTSTRAP PROGRESS=100 TAG=something"),
            Bootstrap::InProgress(100)
        );
        for unknown in [
            &b""[..],
            b"NOTICE BOOTSTRAP",
            b"INFO BOOTSTRAP PROGRESS=5",
            b"NOTICE OTHER PROGRESS=5",
            b"NOTICE BOOTSTRAP PROGRESS=101",
            b"NOTICE BOOTSTRAP PROGRESS=-1",
            b"NOTICE BOOTSTRAP PROGRESS=0005",
            b"NOTICE BOOTSTRAP PROGRESS=",
        ] {
            assert_eq!(parse_bootstrap(unknown), Bootstrap::Unknown, "{unknown:?}");
        }
    }

    #[test]
    fn getinfo_replies_are_read_strictly() {
        let good = reply(b"250-status/circuit-established=1\r\n250 OK\r\n");
        assert_eq!(
            getinfo_value(&good, b"status/circuit-established"),
            Ok(&b"1"[..])
        );
        // Another key, an extra line, no OK.
        assert!(getinfo_value(&good, b"status/bootstrap-phase").is_err());
        let extra = reply(b"250-status/circuit-established=1\r\n250-x=y\r\n250 OK\r\n");
        assert!(getinfo_value(&extra, b"status/circuit-established").is_err());
        let no_ok = reply(b"250-status/circuit-established=1\r\n250 DONE\r\n");
        assert!(getinfo_value(&no_ok, b"status/circuit-established").is_err());
        let prefix = reply(b"250-status/circuit-established-other=1\r\n250 OK\r\n");
        assert!(getinfo_value(&prefix, b"status/circuit-established").is_err());
    }

    #[test]
    fn add_onion_replies_are_read_strictly() {
        let id = service_id();
        let key = format!("{}==", "A".repeat(86));
        let new = format!(
            "250-ServiceID={}\r\n250-PrivateKey=ED25519-V3:{key}\r\n250 OK\r\n",
            id.as_str()
        );
        let (parsed, secret) = parse_add_onion(&reply(new.as_bytes()), true).unwrap();
        assert_eq!(ServiceId::from_key(&parsed), id);
        assert_eq!(secret.unwrap().expose(), &[0; 64]);

        let existing = format!("250-ServiceID={}\r\n250 OK\r\n", id.as_str());
        let (parsed, secret) = parse_add_onion(&reply(existing.as_bytes()), false).unwrap();
        assert_eq!(ServiceId::from_key(&parsed), id);
        assert!(secret.is_none());

        // A key that was not asked for, and one that is missing.
        assert_eq!(
            parse_add_onion(&reply(new.as_bytes()), false).err(),
            Some(TorError::InvalidTorResponse)
        );
        assert_eq!(
            parse_add_onion(&reply(existing.as_bytes()), true).err(),
            Some(TorError::InvalidTorResponse)
        );
        // A malformed ServiceID, a malformed key, a key of another type.
        let mut bad_id = id.as_str().as_bytes().to_vec();
        bad_id[0] = if bad_id[0] == b'a' { b'b' } else { b'a' };
        let bad = format!(
            "250-ServiceID={}\r\n250 OK\r\n",
            String::from_utf8(bad_id).unwrap()
        );
        assert_eq!(
            parse_add_onion(&reply(bad.as_bytes()), false).err(),
            Some(TorError::InvalidTorResponse)
        );
        for blob in [
            format!("{}=", "A".repeat(87)),
            format!("{}==", "A".repeat(85)),
            "!".repeat(88),
        ] {
            let bad = format!(
                "250-ServiceID={}\r\n250-PrivateKey=ED25519-V3:{blob}\r\n250 OK\r\n",
                id.as_str()
            );
            assert_eq!(
                parse_add_onion(&reply(bad.as_bytes()), true).err(),
                Some(TorError::InvalidTorResponse)
            );
        }
        let rsa = format!(
            "250-ServiceID={}\r\n250-PrivateKey=RSA1024:{key}\r\n250 OK\r\n",
            id.as_str()
        );
        assert!(parse_add_onion(&reply(rsa.as_bytes()), true).is_err());
        // Lines Tor would send only for client authorization.
        let auth = format!(
            "250-ServiceID={}\r\n250-ClientAuthV3=abc\r\n250 OK\r\n",
            id.as_str()
        );
        assert!(parse_add_onion(&reply(auth.as_bytes()), false).is_err());
    }

    #[test]
    fn add_onion_errors_are_told_apart() {
        assert_eq!(
            parse_add_onion(
                &reply(b"512 Tor is in non-anonymous hidden service mode\r\n"),
                true
            )
            .err(),
            Some(TorError::NonAnonymousTorMode)
        );
        assert_eq!(
            parse_add_onion(&reply(b"512 Invalid MaxStreams\r\n"), true).err(),
            Some(TorError::OnionPublicationFailed)
        );
        assert_eq!(
            parse_add_onion(&reply(b"510 Command filtered\r\n"), true).err(),
            Some(TorError::CommandRefused)
        );
        assert_eq!(
            parse_add_onion(&reply(b"551 Failed to add Onion Service\r\n"), true).err(),
            Some(TorError::OnionPublicationFailed)
        );
    }

    #[test]
    fn ok_replies_are_exact() {
        assert_eq!(expect_ok(&reply(b"250 OK\r\n")), Ok(()));
        assert_eq!(
            expect_ok(&reply(b"250 Fine\r\n")),
            Err(TorError::InvalidTorResponse)
        );
        assert_eq!(
            expect_ok(&reply(b"250-a\r\n250 OK\r\n")),
            Err(TorError::InvalidTorResponse)
        );
        assert_eq!(
            expect_ok(&reply(b"552 Unknown Onion Service id\r\n")),
            Err(TorError::CommandRefused)
        );
    }
}

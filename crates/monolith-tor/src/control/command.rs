//! The commands Monolith sends to the Tor control port: a closed set.
//!
//! Each variant is one line of `docs/TOR_CONTROL_SURFACE.md`, and its
//! encoding is fixed except for the typed values it carries. There is no
//! variant that takes a string, so no command, key, flag or option can be
//! added at run time, and nothing a peer sends can reach a command line.

use core::net::SocketAddrV4;

use monolith_identity::ServiceId;
use monolith_protocol::limits::{ONION_MAX_STREAMS, ONION_VIRTUAL_PORT};
use zeroize::Zeroizing;

use super::auth::NONCE_LEN;
use crate::secret::hex;
use crate::{OnionServiceSecret, TorError};

/// The key part of an `ADD_ONION`.
pub(crate) enum OnionKey<'a> {
    /// `NEW:ED25519-V3`: Tor makes a key and returns it.
    New,
    /// `ED25519-V3:<key>`: a key Tor returned earlier.
    Existing(&'a OnionServiceSecret),
}

/// A control command.
pub(crate) enum Command<'a> {
    /// `PROTOCOLINFO 1`
    ProtocolInfo,
    /// `AUTHCHALLENGE SAFECOOKIE <client nonce>`
    AuthChallenge(&'a [u8; NONCE_LEN]),
    /// `AUTHENTICATE <client hash>`, for SAFECOOKIE.
    AuthenticateSafeCookie(&'a [u8; 32]),
    /// `AUTHENTICATE`, for a configured trusted filter only.
    AuthenticateTrustedFilter,
    /// `GETINFO status/circuit-established`
    CircuitEstablished,
    /// `GETINFO status/bootstrap-phase`
    BootstrapPhase,
    /// `ADD_ONION` in one of its two forms, with the listener as target.
    AddOnion {
        key: OnionKey<'a>,
        target: SocketAddrV4,
    },
    /// `DEL_ONION <service id>`
    DelOnion(&'a ServiceId),
}

impl Command<'_> {
    /// The command line, with CRLF. The buffer is erased when dropped,
    /// because `ADD_ONION` with an existing key carries the key.
    ///
    /// Fails only for an `ADD_ONION` target that is not a loopback address.
    pub(crate) fn encode(&self) -> Result<Zeroizing<Vec<u8>>, TorError> {
        let mut line = Zeroizing::new(Vec::with_capacity(256));
        match self {
            Self::ProtocolInfo => line.extend_from_slice(b"PROTOCOLINFO 1"),
            Self::AuthChallenge(nonce) => {
                line.extend_from_slice(b"AUTHCHALLENGE SAFECOOKIE ");
                line.extend_from_slice(&hex(&nonce[..]));
            }
            Self::AuthenticateSafeCookie(hash) => {
                line.extend_from_slice(b"AUTHENTICATE ");
                line.extend_from_slice(&hex(&hash[..]));
            }
            Self::AuthenticateTrustedFilter => line.extend_from_slice(b"AUTHENTICATE"),
            Self::CircuitEstablished => {
                line.extend_from_slice(b"GETINFO status/circuit-established");
            }
            Self::BootstrapPhase => line.extend_from_slice(b"GETINFO status/bootstrap-phase"),
            Self::AddOnion { key, target } => {
                if !target.ip().is_loopback() || target.port() == 0 {
                    return Err(TorError::Listener);
                }
                line.extend_from_slice(b"ADD_ONION ");
                match key {
                    OnionKey::New => line.extend_from_slice(b"NEW:ED25519-V3"),
                    OnionKey::Existing(secret) => {
                        line.extend_from_slice(b"ED25519-V3:");
                        line.extend_from_slice(&secret.to_base64());
                    }
                }
                line.extend_from_slice(
                    format!(
                        " Flags=MaxStreamsCloseCircuit MaxStreams={ONION_MAX_STREAMS} \
                         PoWDefensesEnabled=1 Port={ONION_VIRTUAL_PORT},{}:{}",
                        target.ip(),
                        target.port()
                    )
                    .as_bytes(),
                );
            }
            Self::DelOnion(service) => {
                line.extend_from_slice(b"DEL_ONION ");
                line.extend_from_slice(service.as_str().as_bytes());
            }
        }
        line.extend_from_slice(b"\r\n");
        Ok(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::net::Ipv4Addr;
    use monolith_identity::{IdentitySecretKey, OnionServiceKey};

    fn text(command: &Command<'_>) -> String {
        String::from_utf8(command.encode().unwrap().to_vec()).unwrap()
    }

    fn service() -> ServiceId {
        let key = IdentitySecretKey::from_seed(&[3; 32]).public_key();
        ServiceId::from_key(&OnionServiceKey::from_bytes(key.as_bytes()).unwrap())
    }

    #[test]
    fn every_command_is_byte_for_byte_the_documented_one() {
        let target = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 41234);
        assert_eq!(text(&Command::ProtocolInfo), "PROTOCOLINFO 1\r\n");
        assert_eq!(
            text(&Command::AuthChallenge(&[0xAB; 32])),
            format!("AUTHCHALLENGE SAFECOOKIE {}\r\n", "ab".repeat(32))
        );
        assert_eq!(
            text(&Command::AuthenticateSafeCookie(&[0x01; 32])),
            format!("AUTHENTICATE {}\r\n", "01".repeat(32))
        );
        assert_eq!(text(&Command::AuthenticateTrustedFilter), "AUTHENTICATE\r\n");
        assert_eq!(
            text(&Command::CircuitEstablished),
            "GETINFO status/circuit-established\r\n"
        );
        assert_eq!(
            text(&Command::BootstrapPhase),
            "GETINFO status/bootstrap-phase\r\n"
        );
        assert_eq!(
            text(&Command::AddOnion {
                key: OnionKey::New,
                target
            }),
            "ADD_ONION NEW:ED25519-V3 Flags=MaxStreamsCloseCircuit MaxStreams=8 \
             PoWDefensesEnabled=1 Port=29170,127.0.0.1:41234\r\n"
        );
        let secret = OnionServiceSecret::from_bytes(&[0; 64]);
        assert_eq!(
            text(&Command::AddOnion {
                key: OnionKey::Existing(&secret),
                target
            }),
            format!(
                "ADD_ONION ED25519-V3:{}== Flags=MaxStreamsCloseCircuit MaxStreams=8 \
                 PoWDefensesEnabled=1 Port=29170,127.0.0.1:41234\r\n",
                "A".repeat(86)
            )
        );
        let id = service();
        assert_eq!(
            text(&Command::DelOnion(&id)),
            format!("DEL_ONION {}\r\n", id.as_str())
        );
    }

    #[test]
    fn add_onion_never_carries_a_forbidden_flag_or_option() {
        let target = SocketAddrV4::new(Ipv4Addr::LOCALHOST, 41234);
        let secret = OnionServiceSecret::from_bytes(&[7; 64]);
        for key in [OnionKey::New, OnionKey::Existing(&secret)] {
            let line = text(&Command::AddOnion { key, target });
            for forbidden in [
                "Detach",
                "DiscardPK",
                "NonAnonymous",
                "V3Auth",
                "BasicAuth",
                "ClientAuth",
                "PoWQueue",
                "RSA1024",
                "BEST",
                "unix:",
            ] {
                assert!(!line.contains(forbidden), "{forbidden}");
            }
            // One line, with exactly one CRLF at its end.
            assert_eq!(line.matches("\r\n").count(), 1);
            assert!(line.ends_with("\r\n"));
            assert!(!line[..line.len() - 2].contains(['\r', '\n']));
        }
    }

    #[test]
    fn add_onion_refuses_a_target_that_is_not_loopback() {
        for target in [
            SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 41234),
            SocketAddrV4::new(Ipv4Addr::new(10, 152, 152, 11), 29170),
            SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0),
        ] {
            assert!(
                Command::AddOnion {
                    key: OnionKey::New,
                    target
                }
                .encode()
                .is_err(),
                "{target}"
            );
        }
    }
}

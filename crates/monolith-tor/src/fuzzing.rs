//! Entry points for the fuzz targets in `fuzz/`.
//!
//! Compiled only into the tests of this crate and into builds made with
//! `--cfg fuzzing`. They expose the parsers of the control protocol and of
//! SOCKS replies, and the encoding of the one command that carries a value
//! from outside the process, so that the fuzz targets can check them
//! against rules written independently.

use core::net::{Ipv4Addr, SocketAddrV4};

use monolith_identity::OnionServiceKey;

use crate::control::{OnionKey, auth, command::Command};
pub use crate::control::{Reply, ReplyParser};
pub use crate::socks::{ConnectReply, decode_connect_reply};
use crate::{Bootstrap, OnionServiceSecret, TorError, TorVersion};

/// What each interpretation of a reply made of it.
#[derive(Debug)]
pub struct Interpretations {
    /// `PROTOCOLINFO`: whether SAFECOOKIE is offered, the cookie file as
    /// bytes, the version.
    pub protocolinfo: Result<(bool, Option<Vec<u8>>, TorVersion), TorError>,
    /// `AUTHCHALLENGE`: the server hash and nonce.
    pub authchallenge: Result<([u8; 32], [u8; 32]), TorError>,
    /// `GETINFO status/circuit-established`.
    pub circuit_established: Result<bool, TorError>,
    /// `GETINFO status/bootstrap-phase`.
    pub bootstrap: Result<Bootstrap, TorError>,
    /// `ADD_ONION` with a new key: the service and whether a key came.
    pub add_onion_new: Result<(OnionServiceKey, bool), TorError>,
    /// `ADD_ONION` with an existing key.
    pub add_onion_existing: Result<(OnionServiceKey, bool), TorError>,
}

/// Reads one reply as the answer to every command Monolith sends.
pub fn interpret(reply: &Reply) -> Interpretations {
    let add_onion = |new| {
        crate::control::parse_add_onion(reply, new).map(|(key, secret)| (key, secret.is_some()))
    };
    Interpretations {
        protocolinfo: auth::parse_protocolinfo(reply).map(|info| {
            (
                info.safecookie,
                info.cookie_file
                    .map(|path| path.as_os_str().as_encoded_bytes().to_vec()),
                info.version,
            )
        }),
        authchallenge: auth::parse_authchallenge(reply),
        circuit_established: crate::control::parse_circuit_established(reply),
        bootstrap: crate::control::parse_bootstrap_reply(reply),
        add_onion_new: add_onion(true),
        add_onion_existing: add_onion(false),
    }
}

/// The `ADD_ONION` line for an existing key and a listener port, or `None`
/// if the port is not one a listener can have.
pub fn add_onion_line(key: &[u8; 64], port: u16) -> Option<Vec<u8>> {
    let secret = OnionServiceSecret::from_bytes(key);
    Command::AddOnion {
        key: OnionKey::Existing(&secret),
        target: SocketAddrV4::new(Ipv4Addr::LOCALHOST, port),
    }
    .encode()
    .ok()
    .map(|line| line.to_vec())
}

//! Where the system Tor is, and how to authenticate to it.
//!
//! Both endpoints come from explicit configuration. Nothing here discovers
//! an endpoint, scans for one or falls back to another. A TCP endpoint must
//! be a loopback address, so that a mistake in the configuration cannot send
//! SOCKS or control traffic across a real network; the filtered Gateway
//! endpoints of Whonix belong to a platform adapter of a later phase.

use core::fmt;
use core::net::SocketAddr;
use std::path::PathBuf;

use crate::TorError;

/// A local Tor endpoint: a loopback TCP address or a Unix socket.
///
/// The only ways to make one are the constructors below, which check the
/// rule; the inside is private, so that no other code can build an
/// endpoint that is not local. The connection code checks the rule again.
#[derive(Clone, PartialEq, Eq)]
pub struct Endpoint(pub(crate) EndpointKind);

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum EndpointKind {
    Tcp(SocketAddr),
    #[cfg(unix)]
    Unix(PathBuf),
}

/// The rule for a TCP endpoint: a loopback address and a real port.
pub(crate) fn is_local(address: &SocketAddr) -> bool {
    address.ip().is_loopback() && address.port() != 0
}

impl Endpoint {
    /// A TCP endpoint. Fails with [`TorError::Configuration`] unless the
    /// address is a loopback address.
    pub fn tcp(address: SocketAddr) -> Result<Self, TorError> {
        if !is_local(&address) {
            return Err(TorError::Configuration);
        }
        Ok(Self(EndpointKind::Tcp(address)))
    }

    /// A Unix socket endpoint. Fails with [`TorError::Configuration`]
    /// unless the path is absolute.
    #[cfg(unix)]
    pub fn unix(path: PathBuf) -> Result<Self, TorError> {
        if !path.is_absolute() {
            return Err(TorError::Configuration);
        }
        Ok(Self(EndpointKind::Unix(path)))
    }

    /// Parses `127.0.0.1:9050`, `[::1]:9050` or `unix:/run/tor/control`,
    /// with the rules of [`Self::tcp`] and [`Self::unix`]. A host name is
    /// refused: nothing is resolved.
    pub fn parse(text: &str) -> Result<Self, TorError> {
        #[cfg(unix)]
        if let Some(path) = text.strip_prefix("unix:") {
            return Self::unix(PathBuf::from(path));
        }
        let address: SocketAddr = text.parse().map_err(|_| TorError::Configuration)?;
        Self::tcp(address)
    }
}

impl fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A local endpoint is not secret; it is configuration.
        match &self.0 {
            EndpointKind::Tcp(address) => write!(f, "Tcp({address})"),
            #[cfg(unix)]
            EndpointKind::Unix(path) => write!(f, "Unix({})", path.display()),
        }
    }
}

/// How the control connection is authenticated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlAuth {
    /// SAFECOOKIE, with the cookie file of the Tor the user runs, such as
    /// `/run/tor/control.authcookie` of a Debian system Tor.
    ///
    /// The path is configuration, not something the control endpoint says.
    /// `PROTOCOLINFO` must name this same file, or the endpoint is refused
    /// before any challenge: SAFECOOKIE proves only that the server knows
    /// the file it names, and a process that took the endpoint while Tor
    /// was down could name a file it wrote itself. The file must be one
    /// that only Tor can write.
    SafeCookie {
        /// The absolute path of the cookie file.
        cookie_file: PathBuf,
    },
    /// No authentication: the endpoint is a filter that answers
    /// `AUTHENTICATE` itself and restricts the commands, such as
    /// onion-grater. Safe only when the platform provides the access
    /// control around it, as the platform adapters of later phases do;
    /// anything that can listen on the endpoint is otherwise taken for
    /// Tor. Chosen only by configuration, never because a server offers
    /// `NULL`.
    TrustedFilter,
}

/// The configuration of a [`crate::SystemTorBackend`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemTorConfig {
    /// The SOCKS endpoint, for outbound streams.
    pub socks: Endpoint,
    /// The control endpoint, for status and Onion Services.
    pub control: Endpoint,
    /// How to authenticate on the control endpoint.
    pub auth: ControlAuth,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_tcp_and_absolute_socket_paths_are_endpoints() {
        assert!(Endpoint::parse("127.0.0.1:9050").is_ok());
        assert!(Endpoint::parse("127.3.4.5:9051").is_ok());
        assert!(Endpoint::parse("[::1]:9050").is_ok());
        for refused in [
            "0.0.0.0:9050",
            "10.152.152.10:9050",
            "192.168.1.1:9050",
            "[::]:9050",
            "[2001:db8::1]:9050",
            "127.0.0.1:0",
            "localhost:9050",
            "example.org:9050",
            "127.0.0.1",
            "",
        ] {
            assert_eq!(
                Endpoint::parse(refused),
                Err(TorError::Configuration),
                "{refused}"
            );
        }
        #[cfg(unix)]
        {
            assert!(Endpoint::parse("unix:/run/tor/control").is_ok());
            assert_eq!(
                Endpoint::parse("unix:run/tor/control"),
                Err(TorError::Configuration)
            );
            assert_eq!(Endpoint::parse("unix:"), Err(TorError::Configuration));
        }
    }
}

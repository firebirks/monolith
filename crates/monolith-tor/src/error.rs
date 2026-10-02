//! Error type of this crate.

use core::fmt;

/// Why a Tor operation failed.
///
/// The variants carry no addresses, credentials, keys or reply text, so
/// that an error can be logged or shown as it is. None of them leads to
/// another way of reaching a peer: every failure of Tor is a failure of
/// the operation (invariant S1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TorError {
    /// An endpoint in the configuration is not allowed: not loopback, not
    /// an absolute socket path, or otherwise unusable.
    Configuration,
    /// The SOCKS endpoint could not be reached.
    SocksUnavailable,
    /// The SOCKS proxy answered with something that is not a SOCKS5 reply
    /// Monolith accepts.
    SocksProtocol,
    /// The SOCKS proxy refused the isolation credentials.
    SocksAuthentication,
    /// The SOCKS proxy refused the request by its rules, or failed.
    SocksRefused,
    /// Tor could not reach the Onion Service: no descriptor, introduction
    /// or rendezvous failed, the host is unreachable or refused.
    OnionUnreachable,
    /// The control endpoint could not be reached.
    ControlUnavailable,
    /// Control authentication failed or was refused.
    ControlAuthentication,
    /// The control endpoint does not offer the authentication that the
    /// configuration requires.
    ControlAuthenticationUnavailable,
    /// Tor sent a control reply that is malformed, too large or not of the
    /// shape expected for the command.
    InvalidTorResponse,
    /// Tor refused a command, or a filter in front of it did.
    CommandRefused,
    /// The Tor version is below the feature baseline.
    UnsupportedTorVersion,
    /// Tor is configured for non-anonymous single onion services. Monolith
    /// does not publish in that mode.
    NonAnonymousTorMode,
    /// `ADD_ONION` failed.
    OnionPublicationFailed,
    /// The ServiceID Tor returned is not the one of the key that was given.
    OnionKeyMismatch,
    /// `DEL_ONION` failed. The service ends with its control connection
    /// anyway.
    OnionRemovalFailed,
    /// The control connection of a published service is gone, and with it
    /// the service.
    ControlLost,
    /// The operation did not finish in time.
    TimedOut,
    /// The random source of the operating system failed.
    Randomness,
    /// The local listener could not be bound or failed.
    Listener,
    /// The backend does not implement the operation.
    Unsupported,
}

impl fmt::Display for TorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Configuration => "invalid Tor endpoint configuration",
            Self::SocksUnavailable => "Tor SOCKS endpoint unavailable",
            Self::SocksProtocol => "malformed reply from the Tor SOCKS endpoint",
            Self::SocksAuthentication => "Tor SOCKS endpoint refused the isolation credentials",
            Self::SocksRefused => "Tor SOCKS endpoint refused the request",
            Self::OnionUnreachable => "onion service unreachable",
            Self::ControlUnavailable => "Tor control endpoint unavailable",
            Self::ControlAuthentication => "Tor control authentication failed",
            Self::ControlAuthenticationUnavailable => {
                "Tor control endpoint does not offer the configured authentication"
            }
            Self::InvalidTorResponse => "malformed reply from Tor",
            Self::CommandRefused => "Tor refused the command",
            Self::UnsupportedTorVersion => "Tor version below the supported baseline",
            Self::NonAnonymousTorMode => "Tor is in non-anonymous onion service mode",
            Self::OnionPublicationFailed => "onion service could not be published",
            Self::OnionKeyMismatch => "Tor published another onion service than requested",
            Self::OnionRemovalFailed => "onion service could not be removed",
            Self::ControlLost => "Tor control connection lost",
            Self::TimedOut => "timed out",
            Self::Randomness => "random source failed",
            Self::Listener => "local listener failed",
            Self::Unsupported => "operation not supported by this backend",
        })
    }
}

impl core::error::Error for TorError {}

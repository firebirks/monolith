//! Tor backend abstraction.
//!
//! Everything Monolith needs from Tor goes through [`TorBackend`]. The rest
//! of the workspace never opens a SOCKS or control connection itself and does
//! not know whether Tor is a local daemon, the Tor of a Whonix-Gateway behind
//! a control port filter, or something else. See `docs/TOR_INTEGRATION.md`
//! and `docs/adr/0004-tor-backend.md`.
//!
//! Phase 0 defines the interface only. `SystemTorBackend` and
//! `MockTorBackend` are Phase 3 work.

use core::fmt;
use core::future::Future;

use monolith_identity::OnionServiceKey;
use monolith_identity::redact::REDACTED;

/// What a backend can do for Monolith.
///
/// Implementations must fail closed. If Tor is unreachable, every method
/// returns an error; none of them may fall back to a direct connection or to
/// the system resolver (invariants S1 to S3).
pub trait TorBackend: Send + Sync {
    /// A bidirectional byte stream to a peer, carried by Tor.
    type Stream: Send;

    /// A published Onion Service that yields inbound streams.
    type Service: OnionService<Stream = Self::Stream>;

    /// Reports whether Tor is usable.
    fn status(&self) -> impl Future<Output = Result<TorStatus, TorError>> + Send;

    /// Opens a stream to an Onion Service.
    ///
    /// The backend derives the `.onion` name from `target` and hands it to
    /// Tor as a hostname. Streams opened with different isolation tokens must
    /// not share a circuit where the platform allows that to be requested.
    fn connect_onion(
        &self,
        target: &OnionServiceKey,
        virtual_port: u16,
        isolation: IsolationToken,
    ) -> impl Future<Output = Result<Self::Stream, TorError>> + Send;

    /// Publishes an Onion Service and returns a handle that accepts inbound
    /// streams. The service exists for as long as the handle does.
    fn publish_onion_service(
        &self,
        request: PublishRequest,
    ) -> impl Future<Output = Result<Self::Service, TorError>> + Send;

    /// Removes a published Onion Service.
    fn unpublish_onion_service(
        &self,
        service: Self::Service,
    ) -> impl Future<Output = Result<(), TorError>> + Send;
}

/// A published Onion Service.
pub trait OnionService: Send {
    /// The stream type yielded by [`OnionService::accept`].
    type Stream: Send;

    /// The key that names this service.
    fn key(&self) -> OnionServiceKey;

    /// The secret key of a service created with [`KeySource::Generate`], to
    /// be kept by the caller. `None` for a service created from a stored key
    /// and after the first call.
    fn take_generated_secret(&mut self) -> Option<OnionServiceSecret>;

    /// Waits for the next inbound stream.
    fn accept(&mut self) -> impl Future<Output = Result<Self::Stream, TorError>> + Send;
}

/// Coarse Tor status, as far as Monolith needs to know it.
///
/// Monolith asks Tor one question, whether it has established a circuit. It
/// does not read bootstrap details, which can name guards or bridges and are
/// rewritten by control port filters anyway.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TorStatus {
    /// Tor is running but has not established a circuit yet.
    NotReady,
    /// Tor has established a circuit.
    Ready,
}

/// Parameters of an Onion Service to publish.
#[derive(Debug)]
pub struct PublishRequest {
    /// Where the service key comes from.
    pub key: KeySource,
    /// Port peers connect to on the `.onion` name.
    pub virtual_port: u16,
    /// Whether to ask Tor for its proof-of-work defense.
    pub proof_of_work: ProofOfWork,
}

/// Where the key of a published Onion Service comes from.
///
/// There is no variant in which Tor keeps the key to itself. A service ends
/// when its control connection does, so the caller always holds the key and
/// can publish the same service again. Whether the key is then written to
/// the vault or only kept in memory is the caller's decision.
#[derive(Debug)]
pub enum KeySource {
    /// Tor generates a key and returns it once.
    Generate,
    /// The caller supplies a key that Tor returned earlier.
    Stored(OnionServiceSecret),
}

/// Whether to request Tor's Onion Service proof-of-work defense.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProofOfWork {
    /// Do not request it.
    Off,
    /// Request it if the Tor in use supports it; publish without it if not.
    IfSupported,
}

/// Secret key of an Onion Service, in the form Tor exports and imports it.
///
/// The bytes are opaque to Monolith. They are stored encrypted and handed
/// back to Tor unchanged.
pub struct OnionServiceSecret(Box<[u8]>);

impl OnionServiceSecret {
    /// Wraps secret key bytes.
    pub fn new(bytes: Box<[u8]>) -> Self {
        Self(bytes)
    }

    /// Returns the secret key bytes. Callers must not log them.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for OnionServiceSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "OnionServiceSecret({REDACTED})")
    }
}

/// Opaque value that separates the Tor circuits of unrelated streams.
///
/// The core assigns one random token per contact. The backend maps it to
/// whatever isolation mechanism the platform offers.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct IsolationToken([u8; 16]);

impl IsolationToken {
    /// Wraps token bytes. They must come from a CSPRNG.
    pub const fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the token bytes. Callers must not log them.
    pub const fn expose(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Debug for IsolationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IsolationToken({REDACTED})")
    }
}

/// Why a Tor operation failed.
///
/// The variants carry no addresses, credentials or reply text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TorError {
    /// The SOCKS or control endpoint could not be reached.
    Unavailable,
    /// Tor has not finished bootstrapping.
    NotBootstrapped,
    /// Control port authentication failed or was refused.
    AuthenticationFailed,
    /// The control port, or a filter in front of it, refused a command.
    CommandRefused,
    /// Tor sent a reply that could not be parsed within the limits.
    MalformedReply,
    /// The Onion Service could not be reached.
    Unreachable,
    /// The operation did not finish in time.
    TimedOut,
    /// The operation was cancelled.
    Cancelled,
    /// The backend does not implement the operation.
    Unsupported,
}

impl fmt::Display for TorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "Tor is unavailable",
            Self::NotBootstrapped => "Tor is not bootstrapped",
            Self::AuthenticationFailed => "Tor control authentication failed",
            Self::CommandRefused => "Tor control command refused",
            Self::MalformedReply => "malformed reply from Tor",
            Self::Unreachable => "onion service unreachable",
            Self::TimedOut => "timed out",
            Self::Cancelled => "cancelled",
            Self::Unsupported => "operation not supported by this backend",
        })
    }
}

impl core::error::Error for TorError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_types_do_not_print_their_contents() {
        let secret = OnionServiceSecret::new(vec![0xAA; 64].into_boxed_slice());
        assert_eq!(format!("{secret:?}"), "OnionServiceSecret([redacted])");

        let token = IsolationToken::from_bytes([0xBB; 16]);
        assert_eq!(format!("{token:?}"), "IsolationToken([redacted])");
    }

    #[test]
    fn publish_request_debug_does_not_print_a_stored_key() {
        let request = PublishRequest {
            key: KeySource::Stored(OnionServiceSecret::new(vec![0xAA; 64].into_boxed_slice())),
            virtual_port: 1,
            proof_of_work: ProofOfWork::IfSupported,
        };
        let printed = format!("{request:?}");
        assert!(printed.contains("[redacted]"));
        assert!(!printed.contains("170"));
    }
}

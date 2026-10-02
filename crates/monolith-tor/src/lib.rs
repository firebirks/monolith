//! Tor backend.
//!
//! Everything Monolith needs from Tor goes through [`TorBackend`]: a status
//! query, a stream to port 29170 of an Onion Service, and an Onion Service of
//! its own. The rest of the workspace never opens a SOCKS or control
//! connection itself, and does not see SOCKS, the control protocol,
//! SAFECOOKIE, `ADD_ONION` or Tor's key format. See
//! `docs/TOR_INTEGRATION.md`, `docs/TOR_CONTROL_SURFACE.md` and
//! `docs/adr/0004-tor-backend.md`.
//!
//! Implementations fail closed. If Tor cannot be reached, every operation
//! fails; none of them falls back to a direct connection or to a resolver
//! (invariants S1 to S3). A target is an [`OnionServiceKey`], never a host
//! name or an address, so there is nothing that could be resolved locally.
//!
//! This crate never starts, configures, restarts or stops a Tor process,
//! and has no code that could.

// Tests build their own inputs and do arithmetic and indexing on them.
#![cfg_attr(test, allow(clippy::arithmetic_side_effects))]

use core::future::Future;

use monolith_identity::OnionServiceKey;
use tokio::io::{AsyncRead, AsyncWrite};

mod config;
mod error;
mod secret;
mod socks;
mod status;
#[cfg(test)]
mod testing;

pub use config::{ControlAuth, Endpoint, SystemTorConfig};
pub use error::TorError;
pub use secret::{IsolationGroup, ONION_SECRET_LEN, OnionServiceSecret};
pub use status::{
    Bootstrap, ControlStatus, FEATURE_BASELINE, RECOMMENDED, Readiness, SocksStatus, TorStatus,
    TorVersion,
};

/// What a Tor backend does for Monolith.
pub trait TorBackend: Send + Sync {
    /// A bidirectional byte stream to or from a peer, carried by Tor.
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;

    /// A published Onion Service.
    type Service: OnionService<Stream = Self::Stream>;

    /// Reports the state of Tor. A failure is part of the status, not an
    /// error.
    fn status(&self) -> impl Future<Output = TorStatus> + Send;

    /// Makes a new isolation context. Streams dialed with different groups
    /// do not share a circuit.
    fn isolation_group(&self) -> Result<IsolationGroup, TorError> {
        IsolationGroup::generate()
    }

    /// Opens a stream to port `ONION_VIRTUAL_PORT` of the Onion Service
    /// named by `target`, through Tor, in the isolation context `isolation`.
    fn connect_onion(
        &self,
        target: &OnionServiceKey,
        isolation: &IsolationGroup,
    ) -> impl Future<Output = Result<Self::Stream, TorError>> + Send;

    /// Publishes an Onion Service whose port `ONION_VIRTUAL_PORT` leads to a
    /// listener the backend owns. The service exists until the returned
    /// handle is closed or dropped, or until Tor loses it.
    fn publish_onion(
        &self,
        key: KeySource,
    ) -> impl Future<Output = Result<Self::Service, TorError>> + Send;
}

/// A published Onion Service.
///
/// The handle is the publication: closing or dropping it ends the service.
/// It never reports a service as published after the backend has seen
/// that Tor no longer holds it.
pub trait OnionService: Send {
    /// The stream type of inbound connections.
    type Stream: Send;

    /// The key that names this service.
    fn service_key(&self) -> &OnionServiceKey;

    /// The secret key of a service published with [`KeySource::Generate`],
    /// once. `None` for a service published from an existing key, and after
    /// the first call.
    fn take_generated_secret(&mut self) -> Option<OnionServiceSecret>;

    /// Returns false once the service is known to be gone: closed, or its
    /// control connection lost.
    fn is_published(&mut self) -> bool;

    /// Waits for the next inbound stream. Fails with
    /// [`TorError::ControlLost`] when Tor lost the service; it then never
    /// succeeds again.
    fn accept(&mut self) -> impl Future<Output = Result<Self::Stream, TorError>> + Send;

    /// Removes the service: `DEL_ONION` with a short deadline, then the
    /// control connection and the listener are closed. The service is gone
    /// afterwards even if this returns an error.
    fn close(self) -> impl Future<Output = Result<(), TorError>> + Send;
}

/// Where the key of a service to publish comes from.
#[derive(Debug)]
pub enum KeySource {
    /// Tor makes a new key and returns it once, through
    /// [`OnionService::take_generated_secret`]. The caller decides whether
    /// it is kept in memory only (an ephemeral endpoint) or stored.
    Generate,
    /// A key Tor returned earlier. `expected` is the service it names, as
    /// the caller knows it from its own contact card; the backend checks
    /// that Tor published exactly that service.
    Existing {
        /// The key, in Tor's format.
        secret: OnionServiceSecret,
        /// The service the key names.
        expected: OnionServiceKey,
    },
}

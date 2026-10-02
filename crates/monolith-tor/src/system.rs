//! The backend for an existing C tor: its SOCKS endpoint for outbound
//! streams, its control endpoint for status and Onion Services.

use core::fmt;
use core::future::Future;
use core::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use core::task::Poll;

use monolith_identity::{OnionServiceKey, ServiceId};
use monolith_protocol::limits::{SHUTDOWN_TIMEOUT, SOCKS_NEGOTIATION_TIMEOUT};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::control::{ControlConnection, OnionKey};
use crate::socks;
use crate::stream::{TorStream, connect};
use crate::{
    ControlStatus, FEATURE_BASELINE, IsolationGroup, KeySource, OnionService,
    OnionServiceSecret, SocksStatus, SystemTorConfig, TorBackend, TorError, TorStatus,
};

/// A [`TorBackend`] that uses a Tor the system runs.
///
/// It never starts, configures or stops that Tor. Its endpoints come from
/// the configuration and are local by construction.
#[derive(Debug)]
pub struct SystemTorBackend {
    config: SystemTorConfig,
}

impl SystemTorBackend {
    /// A backend for the Tor described by `config`. Nothing is opened until
    /// an operation needs it.
    pub const fn new(config: SystemTorConfig) -> Self {
        Self { config }
    }

    async fn control_status(&self) -> ControlStatus {
        let mut control = match ControlConnection::open(&self.config.control, self.config.auth).await {
            Ok(control) => control,
            Err(TorError::ControlUnavailable | TorError::TimedOut | TorError::ControlLost) => {
                return ControlStatus::Unavailable;
            }
            Err(TorError::ControlAuthenticationUnavailable) => {
                return ControlStatus::AuthenticationUnavailable;
            }
            Err(TorError::ControlAuthentication) => return ControlStatus::AuthenticationFailed,
            Err(_) => return ControlStatus::InvalidResponse,
        };
        let version = control.version();
        if version < FEATURE_BASELINE {
            return ControlStatus::UnsupportedVersion(version);
        }
        let Ok(circuit_established) = control.circuit_established().await else {
            return ControlStatus::InvalidResponse;
        };
        let Ok(bootstrap) = control.bootstrap().await else {
            return ControlStatus::InvalidResponse;
        };
        ControlStatus::Reachable {
            version,
            circuit_established,
            bootstrap,
        }
    }

    async fn socks_status(&self) -> SocksStatus {
        let attempt = tokio::time::timeout(SOCKS_NEGOTIATION_TIMEOUT, async {
            let mut stream = connect(&self.config.socks)
                .await
                .map_err(|_| SocksStatus::Unavailable)?;
            stream
                .write_all(&socks::GREETING)
                .await
                .map_err(|_| SocksStatus::Unavailable)?;
            let mut reply = [0_u8; 2];
            stream
                .read_exact(&mut reply)
                .await
                .map_err(|_| SocksStatus::InvalidResponse)?;
            socks::check_method_reply(reply).map_err(|_| SocksStatus::InvalidResponse)
        })
        .await;
        match attempt {
            Ok(Ok(())) => SocksStatus::Reachable,
            Ok(Err(status)) => status,
            Err(_) => SocksStatus::InvalidResponse,
        }
    }
}

impl TorBackend for SystemTorBackend {
    type Stream = TorStream;
    type Service = PublishedOnionService;

    async fn status(&self) -> TorStatus {
        TorStatus {
            control: self.control_status().await,
            socks: self.socks_status().await,
        }
    }

    async fn connect_onion(
        &self,
        target: &OnionServiceKey,
        isolation: &IsolationGroup,
    ) -> Result<TorStream, TorError> {
        let service = ServiceId::from_key(target);
        let mut stream = tokio::time::timeout(SOCKS_NEGOTIATION_TIMEOUT, connect(&self.config.socks))
            .await
            .map_err(|_| TorError::TimedOut)?
            .map_err(|_| TorError::SocksUnavailable)?;
        socks::negotiate(&mut stream, &service, isolation).await?;
        Ok(stream)
    }

    async fn publish_onion(&self, key: KeySource) -> Result<PublishedOnionService, TorError> {
        let mut control = ControlConnection::open(&self.config.control, self.config.auth).await?;
        if control.version() < FEATURE_BASELINE {
            return Err(TorError::UnsupportedTorVersion);
        }
        // Loopback only, with a port the operating system chooses. The
        // Onion Service is the only intended way in; the listener is
        // reachable by local processes, which is why every stream has to
        // pass the handshake.
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| TorError::Listener)?;
        let SocketAddr::V4(target) = listener.local_addr().map_err(|_| TorError::Listener)? else {
            return Err(TorError::Listener);
        };
        let (service_key, secret) = match &key {
            KeySource::Generate => control.add_onion(OnionKey::New, target).await?,
            KeySource::Existing { secret, expected } => {
                let published = control.add_onion(OnionKey::Existing(secret), target).await?;
                // The ServiceID must name the key the caller gave. On a
                // mismatch the control connection is dropped below, which
                // removes whatever Tor published.
                if published.0 != *expected {
                    return Err(TorError::OnionKeyMismatch);
                }
                published
            }
        };
        Ok(PublishedOnionService {
            key: service_key,
            id: ServiceId::from_key(&service_key),
            secret,
            listener,
            control: Some(control),
        })
    }
}

/// An Onion Service published on a system Tor.
///
/// It owns the control connection that created the service, so the
/// service lives exactly as long as that connection: closing or dropping
/// the handle ends it, and so does a Tor that drops the connection. It also
/// owns the listener the service points at.
pub struct PublishedOnionService {
    key: OnionServiceKey,
    id: ServiceId,
    secret: Option<OnionServiceSecret>,
    listener: TcpListener,
    /// `None` once the service is known to be gone.
    control: Option<ControlConnection>,
}

impl PublishedOnionService {
    /// Forgets the control connection when it is gone.
    fn check_control(&mut self) -> bool {
        if self.control.as_ref().is_some_and(ControlConnection::is_alive) {
            true
        } else {
            self.control = None;
            false
        }
    }
}

impl OnionService for PublishedOnionService {
    type Stream = TorStream;

    fn service_key(&self) -> &OnionServiceKey {
        &self.key
    }

    fn take_generated_secret(&mut self) -> Option<OnionServiceSecret> {
        self.secret.take()
    }

    fn is_published(&mut self) -> bool {
        self.check_control()
    }

    async fn accept(&mut self) -> Result<TorStream, TorError> {
        if !self.check_control() {
            return Err(TorError::ControlLost);
        }
        let Some(control) = self.control.as_ref() else {
            return Err(TorError::ControlLost);
        };
        // Whichever comes first: a stream, or the end of the control
        // connection and with it the service.
        let accepted = {
            let mut accept = core::pin::pin!(self.listener.accept());
            let mut closed = core::pin::pin!(control.closed());
            core::future::poll_fn(|cx| {
                if let Poll::Ready(result) = accept.as_mut().poll(cx) {
                    return Poll::Ready(Some(result));
                }
                if closed.as_mut().poll(cx).is_ready() {
                    return Poll::Ready(None);
                }
                Poll::Pending
            })
            .await
        };
        match accepted {
            Some(Ok((stream, _))) => Ok(TorStream::Tcp(stream)),
            Some(Err(_)) => Err(TorError::Listener),
            None => {
                self.control = None;
                Err(TorError::ControlLost)
            }
        }
    }

    async fn close(mut self) -> Result<(), TorError> {
        let Some(mut control) = self.control.take() else {
            // Tor already lost the service.
            return Ok(());
        };
        let removed = tokio::time::timeout(SHUTDOWN_TIMEOUT, control.del_onion(&self.id))
            .await
            .map_err(|_| TorError::TimedOut)
            .and_then(|result| result);
        // Closing the connection removes the service if DEL_ONION did not.
        drop(control);
        removed
    }
}

impl fmt::Debug for PublishedOnionService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // No address, no key.
        f.debug_struct("PublishedOnionService")
            .field("published", &self.control.is_some())
            .finish_non_exhaustive()
    }
}

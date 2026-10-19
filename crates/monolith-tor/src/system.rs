//! The backend for an existing C tor: its SOCKS endpoint for outbound
//! streams, its control endpoint for status and Onion Services.

use core::fmt;
use core::future::Future;
use core::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use core::task::Poll;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use monolith_identity::{OnionServiceKey, ServiceId};
use monolith_protocol::limits::{SHUTDOWN_TIMEOUT, SOCKS_NEGOTIATION_TIMEOUT};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::control::{ControlConnection, OnionKey};
use crate::socks;
use crate::stream::{TorStream, connect};
use crate::{
    ControlAuth, ControlStatus, FEATURE_BASELINE, IsolationGroup, KeySource, OnionService,
    OnionServiceSecret, SOCKS_PROOF_LIFETIME, SocksProof, SocksStatus, SystemTorConfig, TorBackend,
    TorError, TorStatus,
};

/// A [`TorBackend`] that uses a Tor the system runs.
///
/// It never starts, configures or stops that Tor. Its endpoints come from
/// the configuration and are local by construction.
#[derive(Debug)]
pub struct SystemTorBackend {
    config: SystemTorConfig,
    /// The last proof of [`SocksProof::ControlListeners`]. Locked for as
    /// long as a proof takes, so that dials at the same time wait for one
    /// proof instead of each making their own. The configuration of a
    /// backend never changes, so a proof is never taken for another.
    proof: Mutex<Option<Proof>>,
    /// Proofs attempted so far. A dial that waited while one was attempted
    /// takes its outcome instead of attempting another at once.
    attempts: AtomicU64,
    /// Failures of Tor seen so far: a failed dial, a status query that
    /// found either endpoint failing, a lost Onion Service. Shared with
    /// every published service. A proof counts only while none came after
    /// it began.
    failures: Arc<AtomicU64>,
}

/// Tor listed the SOCKS endpoint at `at`, in a proof that began when
/// `failures` failures had been seen.
#[derive(Clone, Copy, Debug)]
struct Proof {
    at: Instant,
    failures: u64,
}

impl SystemTorBackend {
    /// A backend for the Tor described by `config`. Nothing is opened until
    /// an operation needs it.
    pub fn new(config: SystemTorConfig) -> Self {
        Self {
            config,
            proof: Mutex::new(None),
            attempts: AtomicU64::new(0),
            failures: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Under [`SocksProof::ControlListeners`], fails unless a proof that
    /// the SOCKS endpoint is Tor's counts, and makes one if none does.
    /// Nothing is sent to the SOCKS endpoint here.
    async fn prove_socks(&self) -> Result<(), TorError> {
        if self.config.socks_proof == SocksProof::Off {
            return Ok(());
        }
        let attempts = self.attempts.load(Ordering::SeqCst);
        let mut proof = self.proof.lock().await;
        if proof.is_some_and(|last| self.counts(last)) {
            return Ok(());
        }
        // A proof was attempted while this call waited for the lock, and
        // failed or no longer counts: that outcome stands.
        if self.attempts.load(Ordering::SeqCst) != attempts {
            return Err(TorError::SocksUnavailable);
        }
        let failures = self.failures.load(Ordering::SeqCst);
        let listed = self.socks_listed().await;
        self.attempts.fetch_add(1, Ordering::SeqCst);
        *proof = listed.then(|| Proof {
            at: Instant::now(),
            failures,
        });
        if listed {
            Ok(())
        } else {
            Err(TorError::SocksUnavailable)
        }
    }

    /// Whether `proof` still lets dials through.
    fn counts(&self, proof: Proof) -> bool {
        proof.failures == self.failures.load(Ordering::SeqCst)
            && proof.at.elapsed() < SOCKS_PROOF_LIFETIME
    }

    /// Ends every proof made so far: Tor failed after it.
    fn forget_proof(&self) {
        self.failures.fetch_add(1, Ordering::SeqCst);
    }

    /// Whether the Tor of the control endpoint, authenticated with
    /// SAFECOOKIE, lists the SOCKS endpoint among its SOCKS listeners. A
    /// filter that answers authentication itself proves nothing, so
    /// without SAFECOOKIE nothing is asked.
    async fn socks_listed(&self) -> bool {
        if !matches!(self.config.auth, ControlAuth::SafeCookie { .. }) {
            return false;
        }
        let Ok(mut control) =
            ControlConnection::open(&self.config.control, &self.config.auth).await
        else {
            return false;
        };
        control
            .lists_socks_listener(&self.config.socks)
            .await
            .unwrap_or(false)
    }

    /// The SOCKS exchange of one dial, once the proof let it through.
    async fn dial(
        &self,
        service: ServiceId,
        isolation: &IsolationGroup,
    ) -> Result<TorStream, TorError> {
        let mut stream =
            tokio::time::timeout(SOCKS_NEGOTIATION_TIMEOUT, connect(&self.config.socks))
                .await
                .map_err(|_| TorError::TimedOut)?
                .map_err(|_| TorError::SocksUnavailable)?;
        socks::negotiate(&mut stream, &service, isolation).await?;
        Ok(stream)
    }

    async fn control_status(&self) -> ControlStatus {
        let mut control =
            match ControlConnection::open(&self.config.control, &self.config.auth).await {
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

    /// What the SOCKS endpoint says to a greeting. `control_answered`
    /// tells whether the control endpoint answered the status query.
    async fn socks_status(&self, control_answered: bool) -> SocksStatus {
        // With the listener check, a SOCKS endpoint that Tor has not proved
        // its own is not even greeted: Tor did not list it, or could not be
        // asked without its control endpoint.
        if self.prove_socks().await.is_err() {
            return if control_answered {
                SocksStatus::Unavailable
            } else {
                SocksStatus::Unproven
            };
        }
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
        let status = match attempt {
            Ok(Ok(())) => SocksStatus::Reachable,
            Ok(Err(status)) => status,
            Err(_) => SocksStatus::InvalidResponse,
        };
        if status != SocksStatus::Reachable {
            self.forget_proof();
        }
        status
    }
}

impl TorBackend for SystemTorBackend {
    type Stream = TorStream;
    type Service = PublishedOnionService;

    async fn status(&self) -> TorStatus {
        let control = self.control_status().await;
        // A control endpoint that fails ends the proof of the SOCKS
        // endpoint, which is then made again or refused below.
        let answered = matches!(control, ControlStatus::Reachable { .. });
        if !answered {
            self.forget_proof();
        }
        TorStatus {
            control,
            socks: self.socks_status(answered).await,
        }
    }

    async fn connect_onion(
        &self,
        target: &OnionServiceKey,
        isolation: &IsolationGroup,
    ) -> Result<TorStream, TorError> {
        self.prove_socks().await?;
        self.dial(ServiceId::from_key(target), isolation)
            .await
            .inspect_err(|_| self.forget_proof())
    }

    async fn publish_onion(&self, key: KeySource) -> Result<PublishedOnionService, TorError> {
        let mut control = ControlConnection::open(&self.config.control, &self.config.auth).await?;
        if control.version() < FEATURE_BASELINE {
            return Err(TorError::UnsupportedTorVersion);
        }
        // Loopback only, with a port the operating system chooses. The
        // Onion Service is the only intended way in; the listener is
        // reachable by local processes, which is why every stream has to
        // pass the handshake.
        // The one listener: on 127.0.0.1, from a SocketAddr.
        #[allow(clippy::disallowed_methods)]
        let bound = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).await;
        let listener = bound.map_err(|_| TorError::Listener)?;
        let SocketAddr::V4(target) = listener.local_addr().map_err(|_| TorError::Listener)? else {
            return Err(TorError::Listener);
        };
        let (service_key, secret) = match &key {
            KeySource::Generate => control.add_onion(OnionKey::New, target).await?,
            KeySource::Existing { secret, expected } => {
                let published = control
                    .add_onion(OnionKey::Existing(secret), target)
                    .await?;
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
            failures: Arc::clone(&self.failures),
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
    /// The failures of Tor its backend counts.
    failures: Arc<AtomicU64>,
}

impl PublishedOnionService {
    /// Forgets the control connection when it is gone.
    fn check_control(&mut self) -> bool {
        if self
            .control
            .as_ref()
            .is_some_and(ControlConnection::is_alive)
        {
            true
        } else {
            self.lose_control();
            false
        }
    }

    /// Forgets the control connection. Losing it is a failure of Tor, and
    /// ends every proof that the SOCKS endpoint is Tor's.
    fn lose_control(&mut self) {
        if self.control.take().is_some() {
            self.failures.fetch_add(1, Ordering::SeqCst);
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
                self.lose_control();
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

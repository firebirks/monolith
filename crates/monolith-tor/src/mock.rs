//! An in-memory Tor for tests of the layers above the backend.
//!
//! [`MockNetwork`] holds the published services of every
//! [`MockTorBackend`] made from it; a dial to a published key yields one end
//! of an in-memory stream and delivers the other to the service's `accept`.
//! It implements the [`TorBackend`] contract, not Tor: there is no SOCKS, no
//! control protocol and no parser, so it shares no code with
//! [`crate::SystemTorBackend`]. Failures are switched on by the test.

use core::fmt;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use monolith_identity::{IdentitySecretKey, OnionServiceKey};
use monolith_protocol::limits::MAX_INBOUND_HANDSHAKES;
use tokio::io::DuplexStream;
use tokio::sync::mpsc;

use crate::{
    Bootstrap, ControlStatus, IsolationGroup, KeySource, OnionService, OnionServiceSecret,
    RECOMMENDED, SocksStatus, TorBackend, TorError, TorStatus,
};

/// Capacity of each direction of an in-memory stream.
const STREAM_CAPACITY: usize = 64 * 1024;

struct State {
    control_available: bool,
    socks_available: bool,
    circuit_established: bool,
    bootstrap: Bootstrap,
    publication_error: Option<TorError>,
    services: HashMap<OnionServiceKey, Entry>,
    /// Services whose next accept fails as a listener would.
    listener_errors: HashMap<OnionServiceKey, usize>,
    next_id: u64,
    dials: usize,
}

/// A published service: the channel to its `accept`, and which publication
/// it is.
struct Entry {
    id: u64,
    sender: mpsc::Sender<DuplexStream>,
}

/// The shared state of a set of mock backends.
#[derive(Clone)]
pub struct MockNetwork {
    state: Arc<Mutex<State>>,
}

impl Default for MockNetwork {
    fn default() -> Self {
        Self::new()
    }
}

impl MockNetwork {
    /// A network on which Tor is ready.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                control_available: true,
                socks_available: true,
                circuit_established: true,
                bootstrap: Bootstrap::Done,
                publication_error: None,
                services: HashMap::new(),
                listener_errors: HashMap::new(),
                next_id: 0,
                dials: 0,
            })),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A backend on this network.
    pub fn backend(&self) -> MockTorBackend {
        MockTorBackend {
            network: self.clone(),
        }
    }

    /// Makes the control endpoint reachable or not. When it becomes
    /// unreachable every published service is lost.
    pub fn set_control_available(&self, available: bool) {
        let mut state = self.lock();
        state.control_available = available;
        if !available {
            state.services.clear();
        }
    }

    /// Makes the SOCKS endpoint reachable or not.
    pub fn set_socks_available(&self, available: bool) {
        self.lock().socks_available = available;
    }

    /// Sets what a status query reports about readiness.
    pub fn set_readiness(&self, circuit_established: bool, bootstrap: Bootstrap) {
        let mut state = self.lock();
        state.circuit_established = circuit_established;
        state.bootstrap = bootstrap;
    }

    /// Makes every following publication fail with `error`, or none.
    pub fn fail_publications(&self, error: Option<TorError>) {
        self.lock().publication_error = error;
    }

    /// Drops the control connection of the service `key`, as Tor would on
    /// a restart: the service is gone.
    pub fn lose_service(&self, key: &OnionServiceKey) {
        self.lock().services.remove(key);
    }

    /// Makes the next `count` accepts of the service `key` fail as a
    /// listener with too many open files would.
    pub fn fail_accepts(&self, key: &OnionServiceKey, count: usize) {
        self.lock().listener_errors.insert(*key, count);
    }

    /// Returns true if a service with this key is published.
    pub fn is_published(&self, key: &OnionServiceKey) -> bool {
        self.lock()
            .services
            .get(key)
            .is_some_and(|entry| !entry.sender.is_closed())
    }

    /// How many dials reached the network so far.
    pub fn dials(&self) -> usize {
        self.lock().dials
    }
}

impl fmt::Debug for MockNetwork {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MockNetwork")
    }
}

/// A [`TorBackend`] on a [`MockNetwork`].
#[derive(Debug)]
pub struct MockTorBackend {
    network: MockNetwork,
}

impl TorBackend for MockTorBackend {
    type Stream = DuplexStream;
    type Service = MockOnionService;

    async fn status(&self) -> TorStatus {
        let state = self.network.lock();
        TorStatus {
            control: if state.control_available {
                ControlStatus::Reachable {
                    version: RECOMMENDED,
                    circuit_established: state.circuit_established,
                    bootstrap: state.bootstrap,
                }
            } else {
                ControlStatus::Unavailable
            },
            socks: if state.socks_available {
                SocksStatus::Reachable
            } else {
                SocksStatus::Unavailable
            },
        }
    }

    async fn connect_onion(
        &self,
        target: &OnionServiceKey,
        _isolation: &IsolationGroup,
    ) -> Result<DuplexStream, TorError> {
        let mut state = self.network.lock();
        if !state.socks_available {
            return Err(TorError::SocksUnavailable);
        }
        state.dials = state.dials.saturating_add(1);
        if !state.circuit_established {
            return Err(TorError::OnionUnreachable);
        }
        let entry = state
            .services
            .get(target)
            .ok_or(TorError::OnionUnreachable)?;
        let (client, server) = tokio::io::duplex(STREAM_CAPACITY);
        // A service that does not accept fast enough refuses new streams,
        // as a full listener backlog would.
        entry
            .sender
            .try_send(server)
            .map_err(|_| TorError::OnionUnreachable)?;
        Ok(client)
    }

    async fn publish_onion(&self, key: KeySource) -> Result<MockOnionService, TorError> {
        let mut state = self.network.lock();
        if !state.control_available {
            return Err(TorError::ControlUnavailable);
        }
        if let Some(error) = state.publication_error {
            return Err(error);
        }
        let (service_key, secret) = match key {
            KeySource::Generate => {
                let mut seed = [0_u8; 32];
                getrandom::fill(&mut seed).map_err(|_| TorError::Randomness)?;
                let public = IdentitySecretKey::from_seed(&seed).public_key();
                let service_key = OnionServiceKey::from_bytes(public.as_bytes())
                    .map_err(|_| TorError::OnionPublicationFailed)?;
                let mut blob = [0_u8; 64];
                getrandom::fill(&mut blob).map_err(|_| TorError::Randomness)?;
                (service_key, Some(OnionServiceSecret::from_bytes(&blob)))
            }
            KeySource::Existing { expected, .. } => (expected, None),
        };
        if state.services.contains_key(&service_key) {
            return Err(TorError::OnionPublicationFailed);
        }
        let (sender, receiver) = mpsc::channel(MAX_INBOUND_HANDSHAKES);
        let id = state.next_id;
        state.next_id = state.next_id.saturating_add(1);
        state.services.insert(service_key, Entry { id, sender });
        Ok(MockOnionService {
            key: service_key,
            id,
            secret,
            receiver,
            network: self.network.clone(),
        })
    }
}

/// A service published on a [`MockNetwork`].
pub struct MockOnionService {
    key: OnionServiceKey,
    id: u64,
    secret: Option<OnionServiceSecret>,
    receiver: mpsc::Receiver<DuplexStream>,
    network: MockNetwork,
}

impl OnionService for MockOnionService {
    type Stream = DuplexStream;

    fn service_key(&self) -> &OnionServiceKey {
        &self.key
    }

    fn take_generated_secret(&mut self) -> Option<OnionServiceSecret> {
        self.secret.take()
    }

    fn is_published(&mut self) -> bool {
        let state = self.network.lock();
        state
            .services
            .get(&self.key)
            .is_some_and(|entry| entry.id == self.id)
    }

    async fn accept(&mut self) -> Result<DuplexStream, TorError> {
        {
            let mut state = self.network.lock();
            if let Some(left) = state.listener_errors.get_mut(&self.key) {
                if *left > 0 {
                    *left = left.saturating_sub(1);
                    return Err(TorError::Listener);
                }
            }
        }
        self.receiver.recv().await.ok_or(TorError::ControlLost)
    }

    async fn close(self) -> Result<(), TorError> {
        // Drop removes this publication.
        Ok(())
    }
}

impl Drop for MockOnionService {
    fn drop(&mut self) {
        // Like a closed control connection: the service ends.
        let mut state = self.network.lock();
        if state
            .services
            .get(&self.key)
            .is_some_and(|entry| entry.id == self.id)
        {
            state.services.remove(&self.key);
        }
    }
}

impl fmt::Debug for MockOnionService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MockOnionService")
    }
}

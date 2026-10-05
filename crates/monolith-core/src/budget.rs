//! Bounds on what the network can make Monolith hold at once.
//!
//! Budgets come in the three levels of `RESOURCE_LIMITS.md` section 5.1.
//! Per local identity, so that load on one identity is not visible through
//! another: inbound streams that have not authenticated yet
//! (`MAX_INBOUND_HANDSHAKES`), authenticated peers that are not contacts
//! (`MAX_UNKNOWN_SESSIONS`, with the eviction of `crate::strangers`), and
//! sessions with contacts (`MAX_CONTACT_SESSIONS`); they live in the
//! context of the identity. Process-wide, as ceilings that bind only when
//! the process as a whole is under pressure: inbound handshakes of all
//! identities together (`MAX_PROCESS_INBOUND_HANDSHAKES`), contact sessions
//! of all identities together (`MAX_PROCESS_CONTACT_SESSIONS`), and
//! outbound dials, each of which includes a SOCKS negotiation
//! (`MAX_CONCURRENT_DIALS`); they are [`Budgets`]. Every limit is a fixed
//! number of permits; a stream that finds none is closed at once, and the
//! accept loop pauses for `ACCEPT_BACKOFF` so that a flood does not keep
//! it spinning. Every task the accept loop starts is owned by it and ends
//! with it.

use core::future::Future;
use core::task::Poll;
use std::sync::Arc;

use monolith_protocol::limits::{
    ACCEPT_BACKOFF, MAX_CONCURRENT_DIALS, MAX_PROCESS_CONTACT_SESSIONS,
    MAX_PROCESS_INBOUND_HANDSHAKES,
};
use monolith_tor::{OnionService, TorError};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio::task::JoinSet;

use crate::identity::LocalIdentity;

/// The process-wide budgets. One value per process, passed by reference;
/// the budgets of each identity are in its context.
#[derive(Clone, Debug)]
pub struct Budgets {
    handshakes: Arc<Semaphore>,
    contact_sessions: Arc<Semaphore>,
    dials: Arc<Semaphore>,
}

impl Default for Budgets {
    fn default() -> Self {
        Self::new()
    }
}

/// The two permits an inbound handshake holds: its identity's and the
/// process's.
#[derive(Debug)]
pub struct HandshakePermit {
    _identity: OwnedSemaphorePermit,
    _process: OwnedSemaphorePermit,
}

/// The two permits a contact session holds.
#[derive(Debug)]
pub struct ContactPermit {
    _identity: OwnedSemaphorePermit,
    _process: OwnedSemaphorePermit,
}

impl Budgets {
    /// Budgets with the limits of `RESOURCE_LIMITS.md`.
    pub fn new() -> Self {
        Self {
            handshakes: Arc::new(Semaphore::new(MAX_PROCESS_INBOUND_HANDSHAKES)),
            contact_sessions: Arc::new(Semaphore::new(MAX_PROCESS_CONTACT_SESSIONS)),
            dials: Arc::new(Semaphore::new(MAX_CONCURRENT_DIALS)),
        }
    }

    /// A slot for an inbound handshake at `identity`, if both its budget
    /// and the process's have one.
    pub fn inbound_handshake(&self, identity: &LocalIdentity) -> Option<HandshakePermit> {
        let identity = identity.handshakes.clone().try_acquire_owned().ok()?;
        let process = self.handshakes.clone().try_acquire_owned().ok()?;
        Some(HandshakePermit {
            _identity: identity,
            _process: process,
        })
    }

    /// A slot for a contact session of `identity`, if both budgets have
    /// one.
    pub fn contact_session(&self, identity: &LocalIdentity) -> Option<ContactPermit> {
        let identity = identity.contact_sessions.clone().try_acquire_owned().ok()?;
        let process = self.contact_sessions.clone().try_acquire_owned().ok()?;
        Some(ContactPermit {
            _identity: identity,
            _process: process,
        })
    }

    /// A slot for an outbound dial. Waits for one; dials are made by the
    /// local scheduler, not by peers.
    pub async fn dial(&self) -> Option<OwnedSemaphorePermit> {
        self.dials.clone().acquire_owned().await.ok()
    }

    /// Free inbound handshake slots of the process.
    pub fn free_inbound_handshakes(&self) -> usize {
        self.handshakes.available_permits()
    }

    /// Free contact session slots of the process.
    pub fn free_contact_sessions(&self) -> usize {
        self.contact_sessions.available_permits()
    }
}

/// How an accept loop ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServeEnd {
    /// Shutdown was requested.
    Shutdown,
    /// The service is gone: Tor lost it, or its listener failed.
    Service(TorError),
}

/// Accepts streams from `service`, the Onion Service of `identity`, until
/// `shutdown` turns true or the service is gone, and runs `handler` on each
/// in a task owned by this loop, with an inbound handshake permit of that
/// identity and of the process. A stream that finds no permit is closed
/// without a task. When the loop ends, every task it started is aborted
/// and awaited, so none outlives it.
pub async fn serve<V, F, Fut>(
    service: &mut V,
    budgets: &Budgets,
    identity: &LocalIdentity,
    mut shutdown: watch::Receiver<bool>,
    mut handler: F,
) -> ServeEnd
where
    V: OnionService,
    F: FnMut(V::Stream, HandshakePermit) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut tasks = JoinSet::new();
    let end = loop {
        if *shutdown.borrow() {
            break ServeEnd::Shutdown;
        }
        // Finished handlers are collected here, so the set holds only the
        // ones that run; there are at most as many as permits.
        while tasks.try_join_next().is_some() {}

        let next = {
            let mut accept = core::pin::pin!(service.accept());
            let mut changed = core::pin::pin!(shutdown.changed());
            core::future::poll_fn(|cx| {
                if let Poll::Ready(result) = accept.as_mut().poll(cx) {
                    return Poll::Ready(Some(result.map_err(Some)));
                }
                if let Poll::Ready(result) = changed.as_mut().poll(cx) {
                    return Poll::Ready(result.is_err().then_some(Err(None)));
                }
                Poll::Pending
            })
            .await
        };
        match next {
            // The shutdown value changed; it is read at the top.
            None => continue,
            // The shutdown sender is gone: nobody can stop this loop any
            // more, so it stops now.
            Some(Err(None)) => break ServeEnd::Shutdown,
            // A listener error such as too many open files passes; the
            // service is still published, so the loop waits and goes on.
            Some(Err(Some(TorError::Listener))) => {
                tokio::time::sleep(ACCEPT_BACKOFF).await;
            }
            Some(Err(Some(error))) => break ServeEnd::Service(error),
            Some(Ok(stream)) => match budgets.inbound_handshake(identity) {
                Some(permit) => {
                    tasks.spawn(handler(stream, permit));
                }
                None => {
                    drop(stream);
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                }
            },
        }
    };
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    end
}

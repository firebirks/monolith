//! Bounds on what the network can make Monolith hold at once.
//!
//! Inbound streams that have not authenticated yet are limited to
//! `MAX_INBOUND_HANDSHAKES`, authenticated peers that are not contacts to
//! `MAX_UNKNOWN_SESSIONS`, and outbound dials, each of which includes a
//! SOCKS negotiation, to `MAX_CONCURRENT_DIALS`. Every limit is a semaphore
//! with a fixed number of permits; a stream that finds no permit is closed
//! at once, and the accept loop pauses for `ACCEPT_BACKOFF` so that a flood
//! does not keep it spinning. Every task the accept loop starts is owned by
//! it and ends with it.
//!
//! The policies of `RESOURCE_LIMITS.md` section 5 that need contacts and
//! rates (closing the oldest handshake, the rate buckets, the contact
//! session budget) come with the application core of Phase 4.

use core::future::Future;
use core::task::Poll;
use std::sync::Arc;

use monolith_protocol::limits::{
    ACCEPT_BACKOFF, MAX_CONCURRENT_DIALS, MAX_INBOUND_HANDSHAKES, MAX_UNKNOWN_SESSIONS,
};
use monolith_tor::{OnionService, TorError};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tokio::task::JoinSet;

/// The semaphores of one Monolith instance.
#[derive(Clone, Debug)]
pub struct Budgets {
    handshakes: Arc<Semaphore>,
    unknown: Arc<Semaphore>,
    dials: Arc<Semaphore>,
}

impl Default for Budgets {
    fn default() -> Self {
        Self::new()
    }
}

impl Budgets {
    /// Budgets with the limits of `RESOURCE_LIMITS.md`.
    pub fn new() -> Self {
        Self {
            handshakes: Arc::new(Semaphore::new(MAX_INBOUND_HANDSHAKES)),
            unknown: Arc::new(Semaphore::new(MAX_UNKNOWN_SESSIONS)),
            dials: Arc::new(Semaphore::new(MAX_CONCURRENT_DIALS)),
        }
    }

    /// A slot for an inbound handshake, if one is free.
    pub fn inbound_handshake(&self) -> Option<OwnedSemaphorePermit> {
        self.handshakes.clone().try_acquire_owned().ok()
    }

    /// A slot for an authenticated peer that is not a contact, if one is
    /// free.
    pub fn unknown_session(&self) -> Option<OwnedSemaphorePermit> {
        self.unknown.clone().try_acquire_owned().ok()
    }

    /// A slot for an outbound dial. Waits for one; dials are made by the
    /// local scheduler, not by peers.
    pub async fn dial(&self) -> Option<OwnedSemaphorePermit> {
        self.dials.clone().acquire_owned().await.ok()
    }

    /// Free inbound handshake slots.
    pub fn free_inbound_handshakes(&self) -> usize {
        self.handshakes.available_permits()
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

/// Accepts streams from `service` until `shutdown` turns true or the
/// service is gone, and runs `handler` on each in a task owned by this
/// loop, with an inbound handshake permit. A stream that finds no permit
/// is closed without a task. When the loop ends, every task it started is
/// aborted and awaited, so none outlives it.
pub async fn serve<V, F, Fut>(
    service: &mut V,
    budgets: &Budgets,
    mut shutdown: watch::Receiver<bool>,
    mut handler: F,
) -> ServeEnd
where
    V: OnionService,
    F: FnMut(V::Stream, OwnedSemaphorePermit) -> Fut,
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
            Some(Ok(stream)) => match budgets.inbound_handshake() {
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

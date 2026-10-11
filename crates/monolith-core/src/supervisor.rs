//! The supervisor of the Onion Service of one local identity.
//!
//! Tor removes a service when the control connection that published it
//! closes: a lost control connection, a restart of Tor, a filter that
//! restarted. The backend reports that and never claims the service is
//! still there (`TOR_INTEGRATION.md` section 4). Publishing it again is
//! this module's job:
//!
//! - the service is published from the key the identity holds, so it
//!   comes back under the same name; Tor's ServiceID is checked against
//!   the expected key by the backend;
//! - while it is published, its accept loop runs ([`crate::budget::serve`]);
//! - when the loop ends because the service is gone, the publication is
//!   reported unavailable and published again after a delay;
//! - delays follow the reconnect schedule of `RESOURCE_LIMITS.md` section
//!   7: 10 seconds, doubled after each failure up to 30 minutes, each
//!   drawn between half and one and a half times its value, and reset
//!   once a publication stayed up for a minute. There is no loop without
//!   a delay;
//! - shutdown ends the loop at once, wherever it is, and removes the
//!   service, and so does the deletion of the identity: a publication
//!   under way is dropped, which removes whatever Tor published with its
//!   control connection, and a service that Tor returns for an identity
//!   deleted meanwhile is removed at once, without being reported
//!   available. A deletion that comes in the instant the service is
//!   reported available ends the accept loop at its first look, which
//!   removes the service; it may have been reported available for that
//!   instant.
//!
//! The supervisor reaches Tor only through the backend, which fails
//! closed: it never falls back to a direct connection, a resolver or
//! anything outside Tor (S1 to S3). One supervisor runs per identity, with
//! that identity's key and accept loop; the supervisors of two identities
//! share nothing but the backend and the process budgets.

use core::future::Future;
use core::time::Duration;

use monolith_protocol::limits::{
    RECONNECT_BACKOFF_FACTOR, RECONNECT_DELAY_INITIAL, RECONNECT_DELAY_MAX,
    RECONNECT_JITTER_PERCENT, RECONNECT_RESET_AFTER,
};
use monolith_tor::{KeySource, OnionService, TorBackend, TorError};
use tokio::sync::watch;

use crate::budget::{Budgets, HandshakePermit, ServeEnd, serve};
use crate::identity::LocalIdentity;

/// Where the publication of an identity stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Publication {
    /// The service is being published.
    Publishing,
    /// Tor holds the service and its accept loop runs.
    Available,
    /// The service is not published: the last attempt failed, or Tor lost
    /// it. Another attempt follows after a delay.
    Unavailable(TorError),
    /// The identity holds no key Tor could publish again. Nothing more is
    /// tried.
    NoKey,
    /// The supervisor ended at shutdown.
    Stopped,
}

/// The delays between attempts to publish.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Backoff {
    next: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

impl Backoff {
    /// A schedule that starts at `RECONNECT_DELAY_INITIAL`.
    pub const fn new() -> Self {
        Self {
            next: RECONNECT_DELAY_INITIAL,
        }
    }

    /// The delay before the next attempt, with `jitter` in `0..=100` taken
    /// from a random source: the delay times `(100 - J + 2 J * jitter /
    /// 100) / 100`, where J is `RECONNECT_JITTER_PERCENT`. The delay after
    /// it doubles, up to `RECONNECT_DELAY_MAX`.
    pub fn delay(&mut self, jitter: u32) -> Duration {
        let base = self.next;
        self.next = base
            .checked_mul(RECONNECT_BACKOFF_FACTOR)
            .unwrap_or(RECONNECT_DELAY_MAX)
            .min(RECONNECT_DELAY_MAX);
        let jitter = jitter.min(100);
        let percent = 100_u32
            .saturating_sub(RECONNECT_JITTER_PERCENT)
            .saturating_add(
                RECONNECT_JITTER_PERCENT
                    .saturating_mul(2)
                    .saturating_mul(jitter)
                    / 100,
            );
        base.checked_mul(percent)
            .and_then(|scaled| scaled.checked_div(100))
            .unwrap_or(RECONNECT_DELAY_MAX)
    }

    /// Starts over: a publication stayed up long enough.
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

/// A number in `0..=100` from the random source of the operating system,
/// for the jitter of a delay. A failing source gives the middle, which is
/// the delay without jitter: the jitter only spreads attempts, and a
/// failure here must not make them faster.
fn jitter() -> u32 {
    let mut byte = [0_u8; 1];
    match getrandom::fill(&mut byte) {
        Ok(()) => u32::from(u8::from_be_bytes(byte)) % 101,
        Err(_) => 50,
    }
}

/// How a wait that `shutdown` can cut short ended.
enum Waited<T> {
    /// The work finished.
    Done(T),
    /// The shutdown value changed. It is read again by the caller.
    Changed,
    /// The shutdown sender is gone: nobody can stop the supervisor any
    /// more, so it stops now.
    Gone,
}

/// Runs `work` unless `shutdown` changes or `identity` is deleted first.
/// A deletion ends it as a change of `shutdown` does: the caller reads both
/// again. `work` is dropped then, unfinished.
async fn unless_stopped<T>(
    work: impl Future<Output = T>,
    shutdown: &mut watch::Receiver<bool>,
    identity: &LocalIdentity,
) -> Waited<T> {
    let mut deletion = identity.closing();
    let mut work = core::pin::pin!(work);
    let mut changed = core::pin::pin!(shutdown.changed());
    let mut deleted = core::pin::pin!(deletion.wait_for(|deleted| *deleted));
    core::future::poll_fn(|cx| {
        if deleted.as_mut().poll(cx).is_ready() {
            return core::task::Poll::Ready(Waited::Changed);
        }
        if let core::task::Poll::Ready(value) = work.as_mut().poll(cx) {
            return core::task::Poll::Ready(Waited::Done(value));
        }
        if let core::task::Poll::Ready(result) = changed.as_mut().poll(cx) {
            return core::task::Poll::Ready(if result.is_ok() {
                Waited::Changed
            } else {
                Waited::Gone
            });
        }
        core::task::Poll::Pending
    })
    .await
}

/// Sleeps for `delay` unless shutdown is requested or the identity is
/// deleted first. Returns true if either happened, or if the shutdown
/// sender is gone.
async fn sleep_or_stop(
    delay: Duration,
    shutdown: &mut watch::Receiver<bool>,
    identity: &LocalIdentity,
) -> bool {
    let Some(deadline) = tokio::time::Instant::now().checked_add(delay) else {
        return true;
    };
    let mut deletion = identity.closing();
    loop {
        if *shutdown.borrow_and_update() || identity.is_closed() {
            return true;
        }
        let sleep = async {
            let mut deleted = core::pin::pin!(deletion.wait_for(|deleted| *deleted));
            let mut sleep = core::pin::pin!(tokio::time::sleep_until(deadline));
            core::future::poll_fn(|cx| {
                if deleted.as_mut().poll(cx).is_ready() {
                    return core::task::Poll::Ready(false);
                }
                sleep.as_mut().poll(cx).map(|()| true)
            })
            .await
        };
        match unless_stopped(sleep, shutdown, identity).await {
            Waited::Done(slept) => {
                return !slept || *shutdown.borrow() || identity.is_closed();
            }
            Waited::Changed => {}
            Waited::Gone => return true,
        }
    }
}

/// Keeps the Onion Service of `identity` published through `backend`
/// until `shutdown` turns true, and runs `handler` on every inbound stream
/// with its handshake permit, as [`serve`] does. Reports every change in
/// `state`. Returns when shut down, or at once if the identity holds no key
/// to publish.
pub async fn supervise<B, F, Fut>(
    backend: &B,
    budgets: &Budgets,
    identity: &LocalIdentity,
    mut shutdown: watch::Receiver<bool>,
    state: &watch::Sender<Publication>,
    mut handler: F,
) -> Publication
where
    B: TorBackend,
    F: FnMut(B::Stream, HandshakePermit) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut backoff = Backoff::new();
    loop {
        // A deleted identity is not published again.
        if *shutdown.borrow_and_update() || identity.is_closed() {
            break;
        }
        let Some(secret) = identity.onion_secret() else {
            // Deleted since the look above: its key is gone with it.
            if identity.is_closed() {
                break;
            }
            state.send_replace(Publication::NoKey);
            return Publication::NoKey;
        };
        state.send_replace(Publication::Publishing);
        let key = KeySource::Existing {
            secret,
            expected: identity.endpoint(),
        };
        // A deletion while Tor publishes drops the publication: its
        // control connection closes, which removes whatever Tor published.
        let published =
            match unless_stopped(backend.publish_onion(key), &mut shutdown, identity).await {
                Waited::Done(published) => published,
                // The values are read at the top.
                Waited::Changed => continue,
                Waited::Gone => break,
            };
        let failure = match published {
            // Deleted as Tor answered: the service is removed, and never
            // reported available.
            Ok(service) if identity.is_closed() => {
                let _ = service.close().await;
                break;
            }
            Ok(mut service) => {
                state.send_replace(Publication::Available);
                let started = tokio::time::Instant::now();
                let end = serve(
                    &mut service,
                    budgets,
                    identity,
                    shutdown.clone(),
                    &mut handler,
                )
                .await;
                if started.elapsed() >= RECONNECT_RESET_AFTER {
                    backoff.reset();
                }
                match end {
                    ServeEnd::Shutdown => {
                        let _ = service.close().await;
                        break;
                    }
                    ServeEnd::Service(error) => error,
                }
            }
            Err(error) => error,
        };
        state.send_replace(Publication::Unavailable(failure));
        if sleep_or_stop(backoff.delay(jitter()), &mut shutdown, identity).await {
            break;
        }
    }
    state.send_replace(Publication::Stopped);
    Publication::Stopped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_delays_follow_the_reconnect_schedule() {
        let mut backoff = Backoff::new();
        // Without jitter (the middle of the range): 10, 20, 40, ... s.
        let delays: Vec<u64> = (0..10).map(|_| backoff.delay(50).as_secs()).collect();
        assert_eq!(
            delays,
            vec![10, 20, 40, 80, 160, 320, 640, 1280, 1800, 1800]
        );
        // The jitter spans half to one and a half times the delay.
        let mut low = Backoff::new();
        assert_eq!(low.delay(0), Duration::from_secs(5));
        let mut high = Backoff::new();
        assert_eq!(high.delay(100), Duration::from_secs(15));
        // Out of range jitter is clamped.
        let mut clamped = Backoff::new();
        assert_eq!(clamped.delay(1000), Duration::from_secs(15));
        backoff.reset();
        assert_eq!(backoff.delay(50), RECONNECT_DELAY_INITIAL);
        // Never below half the initial delay: no hot loop.
        let mut any = Backoff::new();
        for _ in 0..40 {
            assert!(any.delay(0) >= RECONNECT_DELAY_INITIAL / 2);
        }
    }
}

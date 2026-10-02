//! Runtimes for tests, without the test macros of tokio.

use core::future::Future;

/// Runs a future on a single-threaded runtime with I/O and time.
pub(crate) fn run<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

/// Runs a future with the clock paused: time advances only when every task
/// waits for a timer.
pub(crate) fn run_paused<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .start_paused(true)
        .build()
        .unwrap()
        .block_on(future)
}

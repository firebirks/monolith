//! When a change of the contact state is durable, and the rule that
//! nothing is used before it is.
//!
//! Every change of durable state is made in memory first, under the lock
//! of what it changes, and stamped with the next generation of the
//! installation. The generation that is on disk follows. A change that
//! takes something away (a retired key, a blocked contact) acts at once,
//! in memory; a change that grants something (message 3, a contact
//! session, the success of a user command) is used only once its
//! generation is durable ([`Durability::wait`]). A crash in between loses
//! the change and everything that depended on it, and nothing that
//! depended on it was used, so a restart finds a state that a valid
//! sequence of transitions produced (`docs/STORAGE.md` section 8, S31).
//!
//! In ephemeral mode a generation is durable as soon as it exists. With a
//! vault, a waiter that finds no write under way starts one: a job on the
//! blocking pool takes the vault, snapshots the whole installation, which
//! holds every change up to the latest generation, writes it, puts the
//! vault back and publishes the outcome. Later waiters find their
//! generation covered. A job that finds changes applied after its snapshot
//! writes again before it publishes its outcome: on and on while nobody
//! waits, and a bounded number of times while a waiter is left, which
//! then starts the next write if changes are left. So a change is written
//! also when its waiter was cancelled or nothing waits for it, and nothing
//! is held once the outcome is out.
//! The job does all of that whatever becomes of the
//! waiter that started it, so a waiter that is cancelled (a deadline, an
//! aborted task) loses neither the vault nor its lock, and no other waiter
//! misses the outcome. A failed write marks the installation failed: every
//! later wait for a generation that is not durable fails, and the store
//! refuses everything until the process starts again. What an earlier
//! write made durable stays durable, and its wait says so.

use core::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use tokio::sync::watch;

use monolith_storage::StorageError;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The generations of the durable state of one installation.
pub(crate) struct Durability {
    /// The latest generation applied in memory.
    applied: AtomicU64,
    /// The latest generation on disk.
    durable: watch::Sender<u64>,
    /// A write failed. Nothing is durable any more.
    failed: AtomicBool,
    /// Why, if a write said so.
    error: Mutex<Option<StorageError>>,
}

impl fmt::Debug for Durability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Durability")
            .field("applied", &self.applied.load(Ordering::SeqCst))
            .field("durable", &*self.durable.borrow())
            .field("failed", &self.failed.load(Ordering::SeqCst))
            .finish()
    }
}

impl Durability {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            applied: AtomicU64::new(0),
            durable: watch::Sender::new(0),
            failed: AtomicBool::new(false),
            error: Mutex::new(None),
        })
    }

    /// Stamps a change that was just applied in memory, under the lock of
    /// what it changed. Returns its generation.
    pub(crate) fn bump(&self) -> u64 {
        self.applied
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1)
    }

    /// The latest generation applied in memory: what a decision that read
    /// the state just now may depend on.
    pub(crate) fn applied(&self) -> u64 {
        self.applied.load(Ordering::SeqCst)
    }

    pub(crate) fn durable(&self) -> u64 {
        *self.durable.borrow()
    }

    pub(crate) fn is_failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }

    /// Records that `generation` is on disk, and wakes every waiter, also
    /// when the value does not grow.
    pub(crate) fn mark_durable(&self, generation: u64) {
        self.durable.send_modify(|durable| {
            *durable = (*durable).max(generation);
        });
    }

    /// Records that a write failed, with its reason if there is one, and
    /// wakes every waiter, which then sees the failure.
    pub(crate) fn mark_failed(&self, error: Option<StorageError>) {
        self.fail_quietly(error);
        self.wake();
    }

    /// Records that a write failed, and wakes nobody yet.
    fn fail_quietly(&self, error: Option<StorageError>) {
        if let Some(error) = error {
            lock(&self.error).get_or_insert(error);
        }
        self.failed.store(true, Ordering::SeqCst);
    }

    /// Wakes every waiter.
    fn wake(&self) {
        self.durable.send_modify(|_| {});
    }

    /// What a wait reports once the installation failed.
    fn failure(&self) -> CommitError {
        lock(&self.error).map_or(CommitError::Failed, CommitError::Storage)
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.durable.subscribe()
    }

    /// Returns true while a wait is under way.
    fn waiting(&self) -> bool {
        self.durable.receiver_count() > 0
    }
}

/// Why a change could not be made durable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommitError {
    /// A write of the vault failed, now or earlier. The installation
    /// refuses every change until the process starts again.
    Storage(StorageError),
    /// The installation failed earlier.
    Failed,
}

impl fmt::Display for CommitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Storage(error) => write!(f, "vault write failed: {error}"),
            Self::Failed => f.write_str("the vault failed earlier; restart needed"),
        }
    }
}

impl core::error::Error for CommitError {}

/// What a vault write needs: the encoded state of the installation as of
/// a generation.
/// A job may call it more than once, for changes applied while it wrote.
pub(crate) type Snapshot =
    Box<dyn FnMut() -> Result<(u64, zeroize::Zeroizing<Vec<u8>>), StorageError> + Send>;

/// How many times one job writes, at most, while a waiter is left: once,
/// and again while changes were applied after its last snapshot. The bound
/// keeps waiters from waiting for ever when changes keep coming; a waiter
/// left then starts the next write itself. A job that has no waiter left
/// writes on until nothing is left to write.
const WRITES_PER_JOB: usize = 4;

/// What a write of the installation needs from it.
pub(crate) struct Write {
    /// The state to write.
    pub(crate) snapshot: Snapshot,
    /// What the installation does when the write fails: it withdraws every
    /// session (fail closed). The job runs it itself, with no lock held,
    /// whether or not a waiter is left to see the failure. It holds the
    /// installation weakly, as the job holds the vault's slot: once the job
    /// has published a durable outcome it keeps nothing of the installation
    /// alive, so closing the installation lets go of the vault and the lock
    /// of its directory at once.
    pub(crate) failed: Box<dyn FnOnce() + Send>,
}

/// Writes the vault. One per installation with a vault.
pub(crate) trait Writer: Send {
    /// Replaces the vault with `plaintext`. Runs on the blocking pool.
    fn write(&mut self, plaintext: &[u8]) -> Result<(), StorageError>;
}

impl<D: monolith_storage::dir::VaultDir + Send> Writer for monolith_storage::vault::Vault<D> {
    fn write(&mut self, plaintext: &[u8]) -> Result<(), StorageError> {
        monolith_storage::vault::Vault::write(self, plaintext)
    }
}

/// The vault of an installation, and whether a write has it.
pub(crate) struct Slot {
    /// The vault; taken by the job that writes and put back by it.
    vault: Option<Box<dyn Writer>>,
    /// A job is writing.
    writing: bool,
}

/// Where the durable state goes.
pub(crate) enum Store {
    /// Nothing is written: every generation is durable at once.
    Ephemeral,
    /// The vault. The lock is never held across an `await`.
    Vault(Arc<Mutex<Slot>>),
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ephemeral => "Ephemeral",
            Self::Vault(_) => "Vault",
        })
    }
}

/// One write of the vault, on the blocking pool. It ends the same way
/// however it ends, even by unwinding or by being dropped unstarted at
/// shutdown: the vault goes back into its slot first, and then the outcome
/// is published, so a waiter that the outcome wakes finds the vault free.
/// The slot is held weakly: if the store is gone when the write ends, the
/// vault, and the lock of its directory, are dropped here.
struct Job {
    slot: Weak<Mutex<Slot>>,
    durability: Arc<Durability>,
    vault: Option<Box<dyn Writer>>,
    outcome: Option<Result<u64, StorageError>>,
    /// The generation the last write that succeeded covers: published as
    /// durable even if a later write of the job failed.
    written: Option<u64>,
    failed: Option<Box<dyn FnOnce() + Send>>,
}

impl Job {
    /// Writes the state, and writes it again while changes were applied
    /// after the snapshot it wrote (up to `WRITES_PER_JOB` times while a
    /// waiter is left): a change
    /// whose waiter was cancelled during the write, or that nothing waits
    /// for, is written before the outcome is out. Everything happens before
    /// the outcome is published, so once it is, the job holds nothing.
    fn run(mut self, mut take: Snapshot) {
        let mut outcome = Err(StorageError::Internal);
        let mut writes = 0_usize;
        while let Some(vault) = self.vault.as_mut() {
            outcome =
                take().and_then(|(covered, plaintext)| vault.write(&plaintext).map(|()| covered));
            writes = writes.saturating_add(1);
            match outcome {
                Ok(covered) => {
                    self.written = Some(covered);
                    let behind = self.durability.applied() > covered;
                    // Past the bound only for changes nobody waits for: a
                    // waiter starts the next write itself once this one
                    // is out.
                    let waited = self.durability.waiting();
                    if !behind || (writes >= WRITES_PER_JOB && waited) {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        drop(take);
        self.outcome = Some(outcome);
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        let outcome = self.outcome.take();
        let failed = !matches!(outcome, Some(Ok(_)));
        if failed {
            // An error, or a write that did not finish. The installation is
            // failed before the vault is back, so that no wait starts
            // another write in between. What an earlier write of the job
            // made durable is durable all the same.
            if let Some(written) = self.written {
                self.durability.durable.send_if_modified(|durable| {
                    *durable = (*durable).max(written);
                    false
                });
            }
            self.durability
                .fail_quietly(outcome.as_ref().and_then(|result| result.err()));
        }
        let vault = self.vault.take();
        if let Some(slot) = self.slot.upgrade() {
            let mut slot = lock(&slot);
            slot.vault = vault;
            slot.writing = false;
        }
        match outcome {
            Some(Ok(covered)) => {
                // Nothing of the installation is held once the outcome is
                // out: the waiter it wakes may close the installation and
                // open the vault again at once.
                drop(self.failed.take());
                self.durability.mark_durable(covered);
            }
            _ => {
                // Every session is withdrawn, and the installation let go
                // of, before any waiter learns the outcome: none starts a
                // session on an installation that failed, or finds it
                // still held.
                if let Some(failed) = self.failed.take() {
                    failed();
                }
                self.durability.wake();
            }
        }
    }
}

impl Store {
    /// The store of a persistent installation.
    pub(crate) fn vault(vault: Box<dyn Writer>) -> Self {
        Self::Vault(Arc::new(Mutex::new(Slot {
            vault: Some(vault),
            writing: false,
        })))
    }

    /// Starts a write unless one is under way, on the blocking pool of
    /// `runtime`: `write` gives the snapshot, which encodes the state of
    /// the installation and the generation it covers and is called there
    /// with no lock of the contact state held, what to do if the write
    /// fails, and what to do if the state moved on meanwhile.
    pub(crate) fn start<F>(
        &self,
        durability: &Arc<Durability>,
        runtime: &tokio::runtime::Handle,
        write: F,
    ) -> Result<(), CommitError>
    where
        F: FnOnce() -> Write,
    {
        let Self::Vault(slot) = self else {
            return Ok(());
        };
        let job = {
            let mut held = lock(slot);
            if durability.is_failed() {
                return Err(durability.failure());
            }
            if held.writing {
                // The write under way covers what was applied before its
                // snapshot, and starts another for the rest.
                None
            } else if let Some(vault) = held.vault.take() {
                held.writing = true;
                Some(Job {
                    slot: Arc::downgrade(slot),
                    durability: durability.clone(),
                    vault: Some(vault),
                    outcome: None,
                    written: None,
                    failed: None,
                })
            } else {
                // Without a write under way the vault is in its slot.
                drop(held);
                durability.mark_failed(None);
                return Err(durability.failure());
            }
        };
        if let Some(mut job) = job {
            let Write { snapshot, failed } = write();
            job.failed = Some(failed);
            // Not awaited: the job finishes on its own, and its outcome
            // comes through `durable`.
            drop(runtime.spawn_blocking(move || job.run(snapshot)));
        }
        Ok(())
    }

    /// Waits until `generation` is durable. With a vault, a wait that
    /// finds no write under way starts one ([`Self::start`]).
    pub(crate) async fn wait<F>(
        &self,
        durability: &Arc<Durability>,
        generation: u64,
        write: F,
    ) -> Result<(), CommitError>
    where
        F: Fn() -> Write,
    {
        if matches!(self, Self::Ephemeral) {
            durability.mark_durable(durability.applied());
            return if durability.is_failed() {
                Err(durability.failure())
            } else {
                Ok(())
            };
        }
        let runtime = tokio::runtime::Handle::current();
        let mut durable = durability.subscribe();
        loop {
            // Seen before the state is looked at: an outcome published
            // after this point ends the wait below at once.
            durable.borrow_and_update();
            // What is durable is durable, also once a later write failed.
            if durability.durable() >= generation {
                // A job that stopped for its waiters may have left changes
                // that nobody else waits for: they are written next.
                if durability.applied() > durability.durable() {
                    let _ = self.start(durability, &runtime, &write);
                }
                return Ok(());
            }
            if durability.is_failed() {
                return Err(durability.failure());
            }
            self.start(durability, &runtime, &write)?;
            if durable.changed().await.is_err() {
                return Err(CommitError::Failed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::arithmetic_side_effects)]

    use core::time::Duration;
    use std::sync::Condvar;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc::Sender;

    use zeroize::Zeroizing;

    use super::*;

    /// A vault that counts its writes, fails them on request, and can be
    /// held in the middle of a write.
    struct TestWriter {
        writes: Arc<AtomicUsize>,
        gate: Arc<(Mutex<bool>, Condvar)>,
        fail: bool,
    }

    impl Writer for TestWriter {
        fn write(&mut self, _plaintext: &[u8]) -> Result<(), StorageError> {
            let (open, opened) = &*self.gate;
            let mut open = lock(open);
            while !*open {
                open = opened
                    .wait(open)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            if self.fail {
                return Err(StorageError::Io);
            }
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn gate(open: bool) -> Arc<(Mutex<bool>, Condvar)> {
        Arc::new((Mutex::new(open), Condvar::new()))
    }

    fn open_gate(gate: &(Mutex<bool>, Condvar)) {
        *lock(&gate.0) = true;
        gate.1.notify_all();
    }

    /// A write of nothing, covering what is applied when it is taken.
    fn snapshot(durability: &Arc<Durability>) -> impl Fn() -> Write + use<> {
        let durability = durability.clone();
        move || {
            let durability = durability.clone();
            Write {
                snapshot: Box::new(move || Ok((durability.applied(), Zeroizing::new(vec![0_u8])))),
                failed: Box::new(|| {}),
            }
        }
    }

    fn vault_of(store: &Store) -> bool {
        match store {
            Store::Vault(slot) => lock(slot).vault.is_some(),
            Store::Ephemeral => false,
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    #[test]
    fn a_cancelled_wait_loses_neither_the_vault_nor_the_outcome() {
        let durability = Durability::new();
        let writes = Arc::new(AtomicUsize::new(0));
        let held = gate(false);
        let store = Store::vault(Box::new(TestWriter {
            writes: writes.clone(),
            gate: held.clone(),
            fail: false,
        }));
        runtime().block_on(async {
            let first = durability.bump();
            // The wait starts the write and is cancelled while it runs.
            let waited = tokio::time::timeout(
                Duration::from_millis(50),
                store.wait(&durability, first, snapshot(&durability)),
            )
            .await;
            assert!(waited.is_err());
            assert!(!vault_of(&store));
            open_gate(&held);
            // The write ends on its own: another wait finds its outcome,
            // and the vault is back in its slot.
            store
                .wait(&durability, first, snapshot(&durability))
                .await
                .unwrap();
            assert!(!durability.is_failed());
            assert!(durability.durable() >= first);
            assert_eq!(writes.load(Ordering::SeqCst), 1);
            assert!(vault_of(&store));
            // And it writes again.
            let second = durability.bump();
            store
                .wait(&durability, second, snapshot(&durability))
                .await
                .unwrap();
            assert_eq!(writes.load(Ordering::SeqCst), 2);
        });
    }

    /// A write of nothing, as [`snapshot`], that reports each generation it
    /// covers to `taken`.
    fn reported(durability: &Arc<Durability>, taken: &Sender<u64>) -> impl Fn() -> Write + use<> {
        let durability = durability.clone();
        let taken = taken.clone();
        move || {
            let durability = durability.clone();
            let taken = taken.clone();
            Write {
                snapshot: Box::new(move || {
                    let covered = durability.applied();
                    let _ = taken.send(covered);
                    Ok((covered, Zeroizing::new(vec![0_u8])))
                }),
                failed: Box::new(|| {}),
            }
        }
    }

    #[test]
    fn a_change_is_written_though_its_waiter_was_cancelled_during_a_write() {
        // A write is under way with a snapshot that does not hold the
        // second change; the wait for that change is cancelled, and
        // nothing else waits. The job writes the second change too before
        // its outcome is out, and holds nothing once it is: the vault is
        // back in its slot when the wait for the first change returns.
        let durability = Durability::new();
        let writes = Arc::new(AtomicUsize::new(0));
        let held = gate(false);
        let store = Store::vault(Box::new(TestWriter {
            writes: writes.clone(),
            gate: held.clone(),
            fail: false,
        }));
        let (taken, snapshots) = std::sync::mpsc::channel();
        let write = reported(&durability, &taken);
        runtime().block_on(async {
            let first = durability.bump();
            let waited = tokio::time::timeout(
                Duration::from_millis(10),
                store.wait(&durability, first, &write),
            )
            .await;
            assert!(waited.is_err());
            assert_eq!(snapshots.recv().unwrap(), first);
            let second = durability.bump();
            let waited = tokio::time::timeout(
                Duration::from_millis(10),
                store.wait(&durability, second, &write),
            )
            .await;
            assert!(waited.is_err());
            open_gate(&held);
            store.wait(&durability, first, &write).await.unwrap();
            assert!(
                durability.durable() >= second,
                "the second change was not written"
            );
            assert_eq!(writes.load(Ordering::SeqCst), 2);
            assert!(vault_of(&store));
            assert!(!durability.is_failed());
        });
    }

    /// A vault whose writes follow a script: each write calls `during`
    /// with its number, from 1, and fails if `during` says so.
    struct Scripted<F: FnMut(usize) -> bool + Send> {
        writes: usize,
        during: F,
    }

    impl<F: FnMut(usize) -> bool + Send> Writer for Scripted<F> {
        fn write(&mut self, _plaintext: &[u8]) -> Result<(), StorageError> {
            self.writes += 1;
            if (self.during)(self.writes) {
                Ok(())
            } else {
                Err(StorageError::Io)
            }
        }
    }

    #[test]
    fn changes_that_keep_coming_with_nobody_waiting_are_all_written() {
        // A change is applied during each of the first writes, more of
        // them than a job writes for its waiters, and the only wait is
        // cancelled at once. Every change is written all the same.
        let durability = Durability::new();
        let changing = durability.clone();
        let held = gate(false);
        let opened = held.clone();
        let store = Store::vault(Box::new(Scripted {
            writes: 0,
            during: move |write| {
                if write == 1 {
                    let (open, wake) = &*opened;
                    let mut open = lock(open);
                    while !*open {
                        open = wake
                            .wait(open)
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                    }
                }
                if write <= WRITES_PER_JOB + 2 {
                    changing.bump();
                }
                true
            },
        }));
        runtime().block_on(async {
            let first = durability.bump();
            let waited = tokio::time::timeout(
                Duration::from_millis(10),
                store.wait(&durability, first, snapshot(&durability)),
            )
            .await;
            assert!(waited.is_err());
            open_gate(&held);
            let written = tokio::time::timeout(Duration::from_secs(10), async {
                while durability.durable() < durability.applied() {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await;
            assert!(written.is_ok(), "a change was left unwritten");
            assert!(vault_of(&store));
        });
    }

    #[test]
    fn a_write_that_failed_after_one_that_succeeded_keeps_its_success() {
        // The first write of a job succeeds; a change comes during it, and
        // the second write, for that change, fails. What the first wrote is
        // durable, and its wait says so; the installation has failed.
        let durability = Durability::new();
        let changing = durability.clone();
        let store = Store::vault(Box::new(Scripted {
            writes: 0,
            during: move |write| {
                if write == 1 {
                    changing.bump();
                }
                write == 1
            },
        }));
        runtime().block_on(async {
            let first = durability.bump();
            assert_eq!(
                store.wait(&durability, first, snapshot(&durability)).await,
                Ok(())
            );
            assert!(durability.is_failed());
            assert!(durability.durable() >= first);
            let later = durability.applied();
            assert!(
                store
                    .wait(&durability, later, snapshot(&durability))
                    .await
                    .is_err()
            );
        });
    }

    #[test]
    fn changes_left_when_a_job_stops_for_its_waiter_are_written_next() {
        // A change comes during each of the first writes, more than a job
        // writes while its waiter is left. The waiter returns once its own
        // change is durable; the rest is written without another wait.
        let durability = Durability::new();
        let changing = durability.clone();
        let store = Store::vault(Box::new(Scripted {
            writes: 0,
            during: move |write| {
                if write <= WRITES_PER_JOB + 2 {
                    changing.bump();
                }
                true
            },
        }));
        runtime().block_on(async {
            let first = durability.bump();
            store
                .wait(&durability, first, snapshot(&durability))
                .await
                .unwrap();
            let written = tokio::time::timeout(Duration::from_secs(10), async {
                while durability.durable() < durability.applied() {
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await;
            assert!(written.is_ok(), "changes were left unwritten");
        });
    }

    #[test]
    fn nothing_is_written_once_a_write_failed_and_sessions_go_first() {
        // The second write of a job fails. When its waiter learns that its
        // own change is durable, the installation has already withdrawn
        // its sessions, and no wait starts another write.
        let durability = Durability::new();
        let changing = durability.clone();
        let writes = Arc::new(AtomicUsize::new(0));
        let counted = writes.clone();
        let store = Store::vault(Box::new(Scripted {
            writes: 0,
            during: move |write| {
                counted.fetch_add(1, Ordering::SeqCst);
                if write == 1 {
                    changing.bump();
                }
                write == 1
            },
        }));
        let withdrawn = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let write = {
            let durability = durability.clone();
            let withdrawn = withdrawn.clone();
            move || {
                let durability = durability.clone();
                let withdrawn = withdrawn.clone();
                Write {
                    snapshot: Box::new(move || {
                        Ok((durability.applied(), Zeroizing::new(vec![0_u8])))
                    }),
                    // Slow, so that a waiter woken before it is done
                    // would see it unfinished.
                    failed: Box::new(move || {
                        std::thread::sleep(Duration::from_millis(100));
                        withdrawn.store(true, Ordering::SeqCst);
                    }),
                }
            }
        };
        runtime().block_on(async {
            let first = durability.bump();
            assert_eq!(store.wait(&durability, first, &write).await, Ok(()));
            assert!(withdrawn.load(Ordering::SeqCst));
            assert!(durability.is_failed());
            let later = durability.bump();
            assert!(store.wait(&durability, later, &write).await.is_err());
            assert_eq!(
                store.start(&durability, &tokio::runtime::Handle::current(), &write),
                Err(CommitError::Storage(StorageError::Io))
            );
            // Time for a write that should not have started to run.
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert_eq!(writes.load(Ordering::SeqCst), 2);
        });
    }

    #[test]
    fn a_failed_write_keeps_the_vault_and_fails_every_wait() {
        let durability = Durability::new();
        let store = Store::vault(Box::new(TestWriter {
            writes: Arc::new(AtomicUsize::new(0)),
            gate: gate(true),
            fail: true,
        }));
        runtime().block_on(async {
            let generation = durability.bump();
            assert_eq!(
                store
                    .wait(&durability, generation, snapshot(&durability))
                    .await,
                Err(CommitError::Storage(StorageError::Io))
            );
            assert!(durability.is_failed());
            // The vault, and with it the lock of its directory, is kept.
            assert!(vault_of(&store));
            let later = durability.bump();
            assert_eq!(
                store.wait(&durability, later, snapshot(&durability)).await,
                Err(CommitError::Storage(StorageError::Io))
            );
        });
    }

    #[test]
    fn many_waiters_on_many_threads_all_finish() {
        // Every wait ends, whichever task writes and however the outcomes
        // and the waits interleave.
        let durability = Durability::new();
        let writes = Arc::new(AtomicUsize::new(0));
        let store = Arc::new(Store::vault(Box::new(TestWriter {
            writes: writes.clone(),
            gate: gate(true),
            fail: false,
        })));
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let mut tasks = tokio::task::JoinSet::new();
            for _ in 0..8 {
                let store = store.clone();
                let durability = durability.clone();
                tasks.spawn(async move {
                    for _ in 0..100 {
                        let generation = durability.bump();
                        store
                            .wait(&durability, generation, snapshot(&durability))
                            .await
                            .unwrap();
                        assert!(durability.durable() >= generation);
                    }
                });
            }
            let finished = tokio::time::timeout(Duration::from_secs(60), async {
                while let Some(task) = tasks.join_next().await {
                    task.unwrap();
                }
            })
            .await;
            assert!(finished.is_ok());
        });
        assert!(writes.load(Ordering::SeqCst) >= 1);
        assert!(!durability.is_failed());
    }
}

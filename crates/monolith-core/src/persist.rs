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
//! generation covered. The job does all of that whatever becomes of the
//! waiter that started it, so a waiter that is cancelled (a deadline, an
//! aborted task) loses neither the vault nor its lock, and no other waiter
//! misses the outcome. A failed write marks the installation failed: every
//! later wait fails, and the store refuses everything until the process
//! starts again.

use core::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

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
        if let Some(error) = error {
            lock(&self.error).get_or_insert(error);
        }
        self.failed.store(true, Ordering::SeqCst);
        self.durable.send_modify(|_| {});
    }

    /// What a wait reports once the installation failed.
    fn failure(&self) -> CommitError {
        lock(&self.error).map_or(CommitError::Failed, CommitError::Storage)
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<u64> {
        self.durable.subscribe()
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
pub(crate) type Snapshot =
    Box<dyn FnOnce() -> Result<(u64, zeroize::Zeroizing<Vec<u8>>), StorageError> + Send>;

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
struct Job {
    slot: Arc<Mutex<Slot>>,
    durability: Arc<Durability>,
    vault: Option<Box<dyn Writer>>,
    outcome: Option<Result<u64, StorageError>>,
}

impl Job {
    fn run(mut self, take: Snapshot) {
        let outcome = match self.vault.as_mut() {
            Some(vault) => {
                take().and_then(|(covered, plaintext)| vault.write(&plaintext).map(|()| covered))
            }
            None => Err(StorageError::Internal),
        };
        self.outcome = Some(outcome);
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        {
            let mut slot = lock(&self.slot);
            slot.vault = self.vault.take();
            slot.writing = false;
        }
        match self.outcome.take() {
            Some(Ok(covered)) => self.durability.mark_durable(covered),
            Some(Err(error)) => self.durability.mark_failed(Some(error)),
            // It did not finish.
            None => self.durability.mark_failed(None),
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

    /// Waits until `generation` is durable. With a vault, a wait that
    /// finds no write under way starts one: `snapshot` encodes the state
    /// of the installation and the generation it covers, and is called on
    /// the blocking pool with no lock of the contact state held.
    pub(crate) async fn wait<F>(
        &self,
        durability: &Arc<Durability>,
        generation: u64,
        snapshot: F,
    ) -> Result<(), CommitError>
    where
        F: Fn() -> Snapshot,
    {
        let Self::Vault(slot) = self else {
            durability.mark_durable(durability.applied());
            return if durability.is_failed() {
                Err(durability.failure())
            } else {
                Ok(())
            };
        };
        let mut durable = durability.subscribe();
        loop {
            // Seen before the state is looked at: an outcome published
            // after this point ends the wait below at once.
            durable.borrow_and_update();
            if durability.is_failed() {
                return Err(durability.failure());
            }
            if durability.durable() >= generation {
                return Ok(());
            }
            let job = {
                let mut held = lock(slot);
                if held.writing {
                    // The write under way may cover this generation; its
                    // outcome wakes this wait.
                    None
                } else if let Some(vault) = held.vault.take() {
                    held.writing = true;
                    Some(Job {
                        slot: slot.clone(),
                        durability: durability.clone(),
                        vault: Some(vault),
                        outcome: None,
                    })
                } else {
                    // Without a write under way the vault is in its slot.
                    drop(held);
                    durability.mark_failed(None);
                    return Err(durability.failure());
                }
            };
            if let Some(job) = job {
                let take = snapshot();
                // Not awaited: the job finishes on its own, and its
                // outcome comes through `durable`.
                drop(tokio::task::spawn_blocking(move || job.run(take)));
            }
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

    /// A snapshot of nothing, covering what is applied when it is taken.
    fn snapshot(durability: &Arc<Durability>) -> impl Fn() -> Snapshot + use<> {
        let durability = durability.clone();
        move || {
            let durability = durability.clone();
            Box::new(move || Ok((durability.applied(), Zeroizing::new(vec![0_u8]))))
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

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
//! vault, the task that waits writes: the first waiter takes the writer,
//! snapshots the whole installation, which holds every change up to the
//! latest generation, and writes it on the blocking pool; later waiters
//! find their generation covered. A failed write marks the installation
//! failed: every later wait fails, and the store refuses everything until
//! the process starts again.

use core::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tokio::sync::{Mutex as AsyncMutex, watch};

use monolith_storage::StorageError;

/// The generations of the durable state of one installation.
pub(crate) struct Durability {
    /// The latest generation applied in memory.
    applied: AtomicU64,
    /// The latest generation on disk.
    durable: watch::Sender<u64>,
    /// A write failed. Nothing is durable any more.
    failed: AtomicBool,
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

    pub(crate) fn mark_durable(&self, generation: u64) {
        self.durable.send_if_modified(|durable| {
            if generation > *durable {
                *durable = generation;
                true
            } else {
                false
            }
        });
    }

    pub(crate) fn mark_failed(&self) {
        self.failed.store(true, Ordering::SeqCst);
        // Wakes every waiter, which then sees the failure.
        self.durable.send_modify(|_| {});
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

/// Where the durable state goes.
pub(crate) enum Store {
    /// Nothing is written: every generation is durable at once.
    Ephemeral,
    /// The vault, taken by the task that writes.
    Vault(AsyncMutex<Option<Box<dyn Writer>>>),
}

impl fmt::Debug for Store {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ephemeral => "Ephemeral",
            Self::Vault(_) => "Vault",
        })
    }
}

impl Store {
    /// Waits until `generation` is durable. With a vault the caller may do
    /// the write itself: `snapshot` encodes the state of the installation
    /// and the generation it covers, and is called with no lock of the
    /// contact state held.
    pub(crate) async fn wait<F>(
        &self,
        durability: &Durability,
        generation: u64,
        snapshot: F,
    ) -> Result<(), CommitError>
    where
        F: Fn() -> Snapshot,
    {
        let Self::Vault(writer) = self else {
            durability.mark_durable(durability.applied());
            return if durability.is_failed() {
                Err(CommitError::Failed)
            } else {
                Ok(())
            };
        };
        let mut durable = durability.subscribe();
        loop {
            if durability.is_failed() {
                return Err(CommitError::Failed);
            }
            if durability.durable() >= generation {
                return Ok(());
            }
            let Ok(mut guard) = writer.try_lock() else {
                // Another task writes; its write may cover this generation.
                if durable.changed().await.is_err() {
                    return Err(CommitError::Failed);
                }
                continue;
            };
            if durability.durable() >= generation {
                return Ok(());
            }
            let Some(mut vault) = guard.take() else {
                return Err(CommitError::Failed);
            };
            let take = snapshot();
            let outcome = tokio::task::spawn_blocking(move || {
                let result = take()
                    .and_then(|(covered, plaintext)| vault.write(&plaintext).map(|()| covered));
                (vault, result)
            })
            .await;
            match outcome {
                Ok((vault, Ok(covered))) => {
                    *guard = Some(vault);
                    durability.mark_durable(covered);
                }
                Ok((vault, Err(error))) => {
                    *guard = Some(vault);
                    durability.mark_failed();
                    return Err(CommitError::Storage(error));
                }
                Err(_) => {
                    durability.mark_failed();
                    return Err(CommitError::Failed);
                }
            }
        }
    }
}

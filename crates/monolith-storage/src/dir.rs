//! The directory that holds a vault, and the file operations a vault
//! needs from it.
//!
//! Every write of a vault is a short, fixed sequence of operations
//! (`docs/STORAGE.md` section 3.5): create `vault.new`, write it, flush
//! it, rename or link it, flush the directory. [`VaultDir`] is that set of
//! operations and nothing more, so that the same vault code runs on the
//! real disk ([`DiskDir`]) and on a directory in memory that can be
//! stopped after any step, as a crash would stop it ([`MemoryDir`]).

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use crate::StorageError;

/// The name of the vault file.
pub const VAULT: &str = "vault";

/// The name of a vault file while it is written.
pub const VAULT_NEW: &str = "vault.new";

/// The name of the single-instance lock file.
pub const LOCK: &str = "lock";

/// The name of the message store file (`docs/adr/0008-message-store.md`).
pub const MESSAGES: &str = "messages";

/// The name of a message store file while it is written.
pub const MESSAGES_NEW: &str = "messages.new";

/// The name of the transfer store file (`docs/adr/0014-transfer-staging.md`).
pub const TRANSFERS: &str = "transfers";

/// The name of a transfer store file while it is written.
pub const TRANSFERS_NEW: &str = "transfers.new";

/// A file of a vault directory held open to be read and written at
/// offsets: the staged data of a transfer (`docs/adr/0014`).
pub trait BlobFile: Send {
    /// The length of the file in bytes.
    fn length(&self) -> Result<u64, StorageError>;

    /// Fills `buf` with the bytes of the file from `offset`. Fails if the
    /// file ends before `buf` is full.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), StorageError>;

    /// Writes `bytes` at `offset`. Fails with [`StorageError::Full`] when
    /// the space is missing and with [`StorageError::Busy`] when another
    /// process holds the file.
    fn write_at(&mut self, offset: u64, bytes: &[u8]) -> Result<(), StorageError>;

    /// Flushes what was written to stable storage.
    fn sync(&mut self) -> Result<(), StorageError>;
}

/// The file operations of a vault directory.
///
/// Names are file names inside the directory, never paths. An
/// implementation never follows a symbolic link and creates files that
/// only their owner can read and write.
pub trait VaultDir {
    /// Reads the whole file `name`. Returns `None` if there is no such
    /// file, and fails with [`StorageError::TooLarge`] for a file longer
    /// than `max` bytes, before reading it.
    fn read(&self, name: &str, max: u64) -> Result<Option<Vec<u8>>, StorageError>;

    /// Creates the file `name`, which must not exist, writes `bytes` into
    /// it and flushes it to stable storage.
    fn write_new(&self, name: &str, bytes: &[u8]) -> Result<(), StorageError>;

    /// Renames `from` to `to`, replacing `to` if it exists.
    fn rename(&self, from: &str, to: &str) -> Result<(), StorageError>;

    /// Gives the file `from` the second name `to`. Fails if `to` exists.
    fn link(&self, from: &str, to: &str) -> Result<(), StorageError>;

    /// Removes the name `name`.
    fn remove(&self, name: &str) -> Result<(), StorageError>;

    /// Flushes the names in the directory to stable storage.
    fn sync(&self) -> Result<(), StorageError>;

    /// Creates the file `name`, which must not exist, and reserves `len`
    /// bytes for it, leaving at least `margin` bytes of the file system
    /// free where that can be known (`docs/adr/0014`). The file then has
    /// the length `len`. Fails with [`StorageError::Full`] if the space is
    /// missing; nothing is left behind then.
    fn create_reserved(
        &self,
        name: &str,
        len: u64,
        margin: u64,
    ) -> Result<Box<dyn BlobFile>, StorageError>;

    /// Opens the file `name` to read and write it at offsets. Returns
    /// `None` if there is no such file.
    fn open_blob(&self, name: &str) -> Result<Option<Box<dyn BlobFile>>, StorageError>;

    /// The names of the files in the directory, in no particular order.
    fn list(&self) -> Result<Vec<String>, StorageError>;
}

impl<T: VaultDir + ?Sized> VaultDir for Box<T> {
    fn read(&self, name: &str, max: u64) -> Result<Option<Vec<u8>>, StorageError> {
        (**self).read(name, max)
    }

    fn write_new(&self, name: &str, bytes: &[u8]) -> Result<(), StorageError> {
        (**self).write_new(name, bytes)
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), StorageError> {
        (**self).rename(from, to)
    }

    fn link(&self, from: &str, to: &str) -> Result<(), StorageError> {
        (**self).link(from, to)
    }

    fn remove(&self, name: &str) -> Result<(), StorageError> {
        (**self).remove(name)
    }

    fn sync(&self) -> Result<(), StorageError> {
        (**self).sync()
    }

    fn create_reserved(
        &self,
        name: &str,
        len: u64,
        margin: u64,
    ) -> Result<Box<dyn BlobFile>, StorageError> {
        (**self).create_reserved(name, len, margin)
    }

    fn open_blob(&self, name: &str) -> Result<Option<Box<dyn BlobFile>>, StorageError> {
        (**self).open_blob(name)
    }

    fn list(&self) -> Result<Vec<String>, StorageError> {
        (**self).list()
    }
}

/// One directory shared by the vault and the message store, which write
/// files of their own names in it under its one lock.
impl<T: VaultDir + ?Sized> VaultDir for Arc<T> {
    fn read(&self, name: &str, max: u64) -> Result<Option<Vec<u8>>, StorageError> {
        (**self).read(name, max)
    }

    fn write_new(&self, name: &str, bytes: &[u8]) -> Result<(), StorageError> {
        (**self).write_new(name, bytes)
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), StorageError> {
        (**self).rename(from, to)
    }

    fn link(&self, from: &str, to: &str) -> Result<(), StorageError> {
        (**self).link(from, to)
    }

    fn remove(&self, name: &str) -> Result<(), StorageError> {
        (**self).remove(name)
    }

    fn sync(&self) -> Result<(), StorageError> {
        (**self).sync()
    }

    fn create_reserved(
        &self,
        name: &str,
        len: u64,
        margin: u64,
    ) -> Result<Box<dyn BlobFile>, StorageError> {
        (**self).create_reserved(name, len, margin)
    }

    fn open_blob(&self, name: &str) -> Result<Option<Box<dyn BlobFile>>, StorageError> {
        (**self).open_blob(name)
    }

    fn list(&self) -> Result<Vec<String>, StorageError> {
        (**self).list()
    }
}

#[cfg(unix)]
pub use crate::disk_unix::DiskDir;
#[cfg(windows)]
pub use crate::disk_windows::DiskDir;

/// What survives a crash of a [`MemoryDir`].
///
/// A real file system may keep more than what was flushed, or less of a
/// file that was being written. The four combinations cover what a vault
/// has to survive. For a file written at offsets (a [`BlobFile`]) torn
/// contents go further: of the writes since its last flush, the first,
/// third and so on survive torn (the first half of their bytes, the old
/// bytes after it) and the others whole, so that a later write can survive
/// an earlier one (`docs/adr/0014-transfer-staging.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CrashOutcome {
    /// Names that were not flushed with [`VaultDir::sync`] survive as they
    /// were last changed, instead of reverting to the last flush.
    pub keep_unsynced_names: bool,
    /// Contents that were not flushed survive torn: the first half of
    /// what was written, instead of nothing.
    pub keep_torn_contents: bool,
}

impl CrashOutcome {
    /// Every outcome.
    pub const ALL: [Self; 4] = [
        Self {
            keep_unsynced_names: false,
            keep_torn_contents: false,
        },
        Self {
            keep_unsynced_names: true,
            keep_torn_contents: false,
        },
        Self {
            keep_unsynced_names: false,
            keep_torn_contents: true,
        },
        Self {
            keep_unsynced_names: true,
            keep_torn_contents: true,
        },
    ];
}

/// A file of a [`MemoryDir`]: what is written and what was flushed.
#[derive(Clone, Debug, Default)]
struct Node {
    content: Vec<u8>,
    flushed: Option<Vec<u8>>,
    /// Written at offsets, as a [`BlobFile`]: the writes since the last
    /// flush, in order, decide what a crash leaves.
    blob: bool,
    pending: Vec<(usize, Vec<u8>)>,
}

/// Writes `bytes` into `content` at `offset`, extending it with zero bytes
/// if it is shorter.
fn write_into(content: &mut Vec<u8>, offset: usize, bytes: &[u8]) -> Result<(), StorageError> {
    let end = offset
        .checked_add(bytes.len())
        .ok_or(StorageError::TooLarge)?;
    if content.len() < end {
        content.resize(end, 0);
    }
    content
        .get_mut(offset..end)
        .ok_or(StorageError::Internal)?
        .copy_from_slice(bytes);
    Ok(())
}

#[derive(Clone, Debug, Default)]
struct MemoryState {
    nodes: Vec<Node>,
    /// Names as they are now.
    names: HashMap<String, usize>,
    /// Names as of the last flush of the directory.
    flushed_names: HashMap<String, usize>,
    /// Operations that changed something, counted from the start.
    steps: u64,
    /// The step that fails; from then on every operation fails.
    crash_at: Option<u64>,
    crashed: bool,
    /// The free space the directory pretends to have, for reservations.
    free: Option<u64>,
    /// The error the next write of a blob fails with, once.
    fail_next_write: Option<StorageError>,
}

impl MemoryState {
    /// Counts one step and fails it if the crash is due.
    fn step(&mut self) -> Result<(), StorageError> {
        if self.crashed {
            return Err(StorageError::Io);
        }
        self.steps = self.steps.saturating_add(1);
        if self.crash_at == Some(self.steps) {
            self.crashed = true;
            return Err(StorageError::Io);
        }
        Ok(())
    }

    fn node(&mut self, name: &str) -> Result<&mut Node, StorageError> {
        let index = *self.names.get(name).ok_or(StorageError::Io)?;
        self.nodes.get_mut(index).ok_or(StorageError::Io)
    }
}

/// A vault directory in memory that models what a crash leaves behind.
///
/// Each operation that changes something counts as one or more steps:
/// creating a file, writing its first half, its second half, flushing it,
/// a rename, a link, a removal and a flush of the directory.
/// [`Self::crash_at`] makes a given step fail, and every operation after
/// it, as if the process had stopped there. [`Self::restart`] then gives
/// the directory a new process finds, as [`CrashOutcome`] says.
/// [`Self::hold_writes`] stops the next file write before its first step
/// until [`Self::release_writes`], so that a test can look at what happens
/// while a write is under way. Clones share the same directory.
#[derive(Clone, Debug, Default)]
pub struct MemoryDir {
    state: Arc<Mutex<MemoryState>>,
    hold: Arc<Hold>,
}

/// Writes held back by a test.
#[derive(Debug, Default)]
struct Hold {
    state: Mutex<HoldState>,
    changed: Condvar,
}

#[derive(Debug, Default)]
struct HoldState {
    holding: bool,
    waiting: bool,
}

impl Hold {
    fn state(&self) -> MutexGuard<'_, HoldState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Waits while writes are held.
    fn pass(&self) {
        let mut state = self.state();
        while state.holding {
            state.waiting = true;
            self.changed.notify_all();
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
        state.waiting = false;
    }
}

impl MemoryDir {
    /// An empty directory.
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> MutexGuard<'_, MemoryState> {
        // A test that panicked while holding the lock left a consistent
        // state behind: every change is made in one statement.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The number of steps taken so far.
    pub fn steps(&self) -> u64 {
        self.state().steps
    }

    /// Makes step `step`, counted from the start, fail, and every
    /// operation after it.
    pub fn crash_at(&self, step: u64) {
        self.state().crash_at = Some(step);
    }

    /// Returns true once the crash happened.
    pub fn crashed(&self) -> bool {
        self.state().crashed
    }

    /// Holds the next file write before its first step, until
    /// [`Self::release_writes`].
    pub fn hold_writes(&self) {
        self.hold.state().holding = true;
    }

    /// Returns true while a write waits at the hold.
    pub fn write_held(&self) -> bool {
        self.hold.state().waiting
    }

    /// Lets writes go on.
    pub fn release_writes(&self) {
        self.hold.state().holding = false;
        self.hold.changed.notify_all();
    }

    /// The directory as a new process finds it after a crash at this
    /// point. Names and contents that were flushed survive; the rest as
    /// `outcome` says.
    pub fn restart(&self, outcome: CrashOutcome) -> Self {
        let state = self.state();
        let names = if outcome.keep_unsynced_names {
            state.names.clone()
        } else {
            state.flushed_names.clone()
        };
        let nodes = state
            .nodes
            .iter()
            .map(|node| {
                let content = if node.blob {
                    let mut content = node.flushed.clone().unwrap_or_default();
                    if outcome.keep_torn_contents {
                        for (index, (offset, bytes)) in node.pending.iter().enumerate() {
                            let kept = if index % 2 == 0 {
                                bytes.get(..bytes.len() / 2).unwrap_or_default()
                            } else {
                                bytes.as_slice()
                            };
                            // Within the bounds the write had, so it fits.
                            let _ = write_into(&mut content, *offset, kept);
                        }
                    }
                    content
                } else {
                    match (&node.flushed, outcome.keep_torn_contents) {
                        (Some(flushed), _) => flushed.clone(),
                        (None, true) => node
                            .content
                            .get(..node.content.len() / 2)
                            .unwrap_or_default()
                            .to_vec(),
                        (None, false) => Vec::new(),
                    }
                };
                Node {
                    flushed: Some(content.clone()),
                    content,
                    blob: node.blob,
                    pending: Vec::new(),
                }
            })
            .collect();
        Self {
            state: Arc::new(Mutex::new(MemoryState {
                nodes,
                flushed_names: names.clone(),
                names,
                steps: 0,
                crash_at: None,
                crashed: false,
                free: state.free,
                fail_next_write: None,
            })),
            hold: Arc::default(),
        }
    }

    /// The names in the directory.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.state().names.keys().cloned().collect();
        names.sort();
        names
    }

    /// Replaces the content of an existing file, as someone with access to
    /// the directory could. For tests of damaged vaults.
    pub fn overwrite(&self, name: &str, bytes: &[u8]) -> Result<(), StorageError> {
        let mut state = self.state();
        let node = state.node(name)?;
        node.content = bytes.to_vec();
        node.flushed = Some(bytes.to_vec());
        node.pending.clear();
        Ok(())
    }

    /// The content of the file `name` as it is now, flushed or not. For
    /// tests.
    pub fn content(&self, name: &str) -> Result<Vec<u8>, StorageError> {
        let mut state = self.state();
        Ok(state.node(name)?.content.clone())
    }

    /// Pretends that the file system has `free` bytes free, so that a
    /// reservation that leaves less than its margin fails with
    /// [`StorageError::Full`]. `None`, the default, has no limit.
    pub fn set_free(&self, free: Option<u64>) {
        self.state().free = free;
    }

    /// Makes the next write of a blob fail with `error`, once, without a
    /// crash: a full disk, or a file another process holds.
    pub fn fail_next_write(&self, error: StorageError) {
        self.state().fail_next_write = Some(error);
    }
}

/// A [`BlobFile`] of a [`MemoryDir`].
#[derive(Debug)]
struct MemoryBlob {
    state: Arc<Mutex<MemoryState>>,
    node: usize,
}

impl MemoryBlob {
    fn state(&self) -> MutexGuard<'_, MemoryState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl BlobFile for MemoryBlob {
    fn length(&self) -> Result<u64, StorageError> {
        let state = self.state();
        if state.crashed {
            return Err(StorageError::Io);
        }
        let node = state.nodes.get(self.node).ok_or(StorageError::Io)?;
        u64::try_from(node.content.len()).map_err(|_| StorageError::TooLarge)
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), StorageError> {
        let state = self.state();
        if state.crashed {
            return Err(StorageError::Io);
        }
        let node = state.nodes.get(self.node).ok_or(StorageError::Io)?;
        let start = usize::try_from(offset).map_err(|_| StorageError::Io)?;
        let end = start.checked_add(buf.len()).ok_or(StorageError::Io)?;
        buf.copy_from_slice(node.content.get(start..end).ok_or(StorageError::Io)?);
        Ok(())
    }

    fn write_at(&mut self, offset: u64, bytes: &[u8]) -> Result<(), StorageError> {
        let mut state = self.state();
        if let Some(error) = state.fail_next_write.take() {
            return Err(error);
        }
        state.step()?;
        let offset = usize::try_from(offset).map_err(|_| StorageError::TooLarge)?;
        let node = state.nodes.get_mut(self.node).ok_or(StorageError::Io)?;
        write_into(&mut node.content, offset, bytes)?;
        node.pending.push((offset, bytes.to_vec()));
        Ok(())
    }

    fn sync(&mut self) -> Result<(), StorageError> {
        let mut state = self.state();
        state.step()?;
        let node = state.nodes.get_mut(self.node).ok_or(StorageError::Io)?;
        node.flushed = Some(node.content.clone());
        node.pending.clear();
        Ok(())
    }
}

impl VaultDir for MemoryDir {
    fn read(&self, name: &str, max: u64) -> Result<Option<Vec<u8>>, StorageError> {
        let mut state = self.state();
        if state.crashed {
            return Err(StorageError::Io);
        }
        if !state.names.contains_key(name) {
            return Ok(None);
        }
        let content = state.node(name)?.content.clone();
        if u64::try_from(content.len()).map_err(|_| StorageError::TooLarge)? > max {
            return Err(StorageError::TooLarge);
        }
        Ok(Some(content))
    }

    fn write_new(&self, name: &str, bytes: &[u8]) -> Result<(), StorageError> {
        self.hold.pass();
        let mut state = self.state();
        if state.crashed {
            return Err(StorageError::Io);
        }
        if state.names.contains_key(name) {
            return Err(StorageError::Exists);
        }
        state.step()?;
        let index = state.nodes.len();
        state.nodes.push(Node::default());
        state.names.insert(name.to_owned(), index);
        let half = bytes.len() / 2;
        for part in [bytes.get(..half), bytes.get(half..)] {
            state.step()?;
            state
                .node(name)?
                .content
                .extend_from_slice(part.unwrap_or_default());
        }
        state.step()?;
        let node = state.node(name)?;
        node.flushed = Some(node.content.clone());
        Ok(())
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), StorageError> {
        let mut state = self.state();
        state.step()?;
        let index = state.names.remove(from).ok_or(StorageError::Io)?;
        state.names.insert(to.to_owned(), index);
        Ok(())
    }

    fn link(&self, from: &str, to: &str) -> Result<(), StorageError> {
        let mut state = self.state();
        if !state.crashed && state.names.contains_key(to) {
            return Err(StorageError::Exists);
        }
        state.step()?;
        let index = *state.names.get(from).ok_or(StorageError::Io)?;
        state.names.insert(to.to_owned(), index);
        Ok(())
    }

    fn remove(&self, name: &str) -> Result<(), StorageError> {
        let mut state = self.state();
        state.step()?;
        state.names.remove(name).ok_or(StorageError::Io)?;
        Ok(())
    }

    fn sync(&self) -> Result<(), StorageError> {
        let mut state = self.state();
        state.step()?;
        state.flushed_names = state.names.clone();
        Ok(())
    }

    /// Two steps: the name, then the reservation. Neither is durable
    /// before the file and the directory are flushed.
    fn create_reserved(
        &self,
        name: &str,
        len: u64,
        margin: u64,
    ) -> Result<Box<dyn BlobFile>, StorageError> {
        self.hold.pass();
        let mut state = self.state();
        if state.crashed {
            return Err(StorageError::Io);
        }
        if state.names.contains_key(name) {
            return Err(StorageError::Exists);
        }
        if let Some(free) = state.free {
            if free < len.saturating_add(margin) {
                return Err(StorageError::Full);
            }
        }
        let size = usize::try_from(len).map_err(|_| StorageError::TooLarge)?;
        state.step()?;
        let node = state.nodes.len();
        state.nodes.push(Node {
            blob: true,
            ..Node::default()
        });
        state.names.insert(name.to_owned(), node);
        state.step()?;
        let reserved = state.nodes.get_mut(node).ok_or(StorageError::Io)?;
        reserved.content = vec![0; size];
        if let Some(free) = state.free.as_mut() {
            *free = free.saturating_sub(len);
        }
        Ok(Box::new(MemoryBlob {
            state: Arc::clone(&self.state),
            node,
        }))
    }

    fn open_blob(&self, name: &str) -> Result<Option<Box<dyn BlobFile>>, StorageError> {
        let mut state = self.state();
        if state.crashed {
            return Err(StorageError::Io);
        }
        let Some(&node) = state.names.get(name) else {
            return Ok(None);
        };
        state.nodes.get_mut(node).ok_or(StorageError::Io)?.blob = true;
        Ok(Some(Box::new(MemoryBlob {
            state: Arc::clone(&self.state),
            node,
        })))
    }

    fn list(&self) -> Result<Vec<String>, StorageError> {
        let state = self.state();
        if state.crashed {
            return Err(StorageError::Io);
        }
        Ok(state.names.keys().cloned().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_crash_keeps_what_was_flushed() {
        let dir = MemoryDir::new();
        dir.write_new(VAULT_NEW, b"0123").unwrap();
        dir.link(VAULT_NEW, VAULT).unwrap();
        dir.remove(VAULT_NEW).unwrap();
        dir.sync().unwrap();
        // A write that is stopped at the flush of the new file: both halves
        // are written, nothing is flushed.
        dir.crash_at(dir.steps() + 4);
        assert_eq!(dir.write_new(VAULT_NEW, b"abcd"), Err(StorageError::Io));
        assert!(dir.crashed());
        assert_eq!(dir.read(VAULT, 4), Err(StorageError::Io));
        for outcome in CrashOutcome::ALL {
            let after = dir.restart(outcome);
            assert_eq!(after.read(VAULT, 4).unwrap(), Some(b"0123".to_vec()));
            let new = after.read(VAULT_NEW, 4).unwrap();
            match (outcome.keep_unsynced_names, outcome.keep_torn_contents) {
                (false, _) => assert_eq!(new, None),
                (true, false) => assert_eq!(new, Some(Vec::new())),
                (true, true) => assert_eq!(new, Some(b"ab".to_vec())),
            }
        }
    }

    const BLOB: &str = "transfer-00000000000000000000000000000001";

    #[test]
    fn a_blob_keeps_what_was_flushed_and_unflushed_writes_survive_torn_or_whole() {
        let dir = MemoryDir::new();
        let mut blob = dir.create_reserved(BLOB, 16, 0).unwrap();
        assert_eq!(blob.length().unwrap(), 16);
        blob.write_at(0, b"head").unwrap();
        blob.sync().unwrap();
        dir.sync().unwrap();
        // Two writes not flushed: the first survives torn, the second whole.
        blob.write_at(4, b"abcd").unwrap();
        blob.write_at(8, b"wxyz").unwrap();
        let mut buf = [0; 12];
        blob.read_at(0, &mut buf).unwrap();
        assert_eq!(&buf, b"headabcdwxyz");
        for outcome in CrashOutcome::ALL {
            let after = dir.restart(outcome);
            let content = after.content(BLOB).unwrap();
            assert_eq!(content.len(), 16);
            let expected: &[u8] = if outcome.keep_torn_contents {
                b"headab\0\0wxyz\0\0\0\0"
            } else {
                b"head\0\0\0\0\0\0\0\0\0\0\0\0"
            };
            assert_eq!(content, expected, "{outcome:?}");
        }
    }

    #[test]
    fn a_blob_neither_flushed_nor_named_durably_is_gone_after_a_crash() {
        let dir = MemoryDir::new();
        let mut blob = dir.create_reserved(BLOB, 8, 0).unwrap();
        blob.write_at(0, b"12345678").unwrap();
        let after = dir.restart(CrashOutcome::ALL[0]);
        assert_eq!(after.open_blob(BLOB).unwrap().map(|_| ()), None);
        // The name survives, the reservation did not.
        let after = dir.restart(CrashOutcome::ALL[1]);
        assert_eq!(after.content(BLOB).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn a_reservation_without_room_for_the_margin_fails_and_leaves_nothing() {
        let dir = MemoryDir::new();
        dir.set_free(Some(100));
        assert!(matches!(
            dir.create_reserved(BLOB, 60, 41),
            Err(StorageError::Full)
        ));
        assert!(dir.list().unwrap().is_empty());
        assert!(dir.create_reserved(BLOB, 60, 40).is_ok());
        assert_eq!(dir.list().unwrap(), vec![BLOB.to_owned()]);
        assert!(matches!(
            dir.create_reserved(BLOB, 1, 0),
            Err(StorageError::Exists)
        ));
    }

    #[test]
    fn a_write_can_fail_for_a_full_disk_or_a_held_file_without_a_crash() {
        let dir = MemoryDir::new();
        let mut blob = dir.create_reserved(BLOB, 4, 0).unwrap();
        dir.fail_next_write(StorageError::Busy);
        assert_eq!(blob.write_at(0, b"ab"), Err(StorageError::Busy));
        blob.write_at(0, b"ab").unwrap();
        dir.fail_next_write(StorageError::Full);
        assert_eq!(blob.write_at(2, b"cd"), Err(StorageError::Full));
        assert!(!dir.crashed());
        let mut opened = dir.open_blob(BLOB).unwrap().unwrap();
        let mut buf = [0; 2];
        opened.read_at(0, &mut buf).unwrap();
        assert_eq!(&buf, b"ab");
        assert_eq!(opened.read_at(3, &mut buf), Err(StorageError::Io));
    }
}

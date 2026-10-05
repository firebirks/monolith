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
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use rustix::fs::{AtFlags, FileType, FlockOperation, Mode, OFlags};

use crate::StorageError;

/// The name of the vault file.
pub const VAULT: &str = "vault";

/// The name of a vault file while it is written.
pub const VAULT_NEW: &str = "vault.new";

/// The name of the single-instance lock file.
pub const LOCK: &str = "lock";

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
}

fn io(_: rustix::io::Errno) -> StorageError {
    StorageError::Io
}

fn std_io(_: std::io::Error) -> StorageError {
    StorageError::Io
}

/// A vault directory on disk, held for the life of the value.
///
/// Opening it checks `docs/STORAGE.md` section 3.7: the directory is not a
/// symbolic link, belongs to the current user and is accessible to nobody
/// else; and it takes the single-instance lock, so that a second process
/// on the same directory fails with [`StorageError::Locked`]. The lock is
/// released when the value is dropped, and the operating system releases
/// it when the process ends, so a crash leaves no stale lock.
#[derive(Debug)]
pub struct DiskDir {
    dir: OwnedFd,
    /// Holds the lock while it is open.
    _lock: OwnedFd,
}

/// The mode of a file: owner read and write.
const FILE_MODE: u32 = 0o600;

/// The mode of the directory: owner read, write and search.
const DIR_MODE: u32 = 0o700;

/// Bits that let anyone but the owner in.
const OTHERS: u32 = 0o077;

impl DiskDir {
    /// Opens the vault directory at `path`, creating it with mode 0700 if
    /// it does not exist and `create` is true.
    ///
    /// Fails with [`StorageError::NotFound`] if it does not exist and
    /// `create` is false, with [`StorageError::Permissions`] if it is a
    /// symbolic link, is not a directory, belongs to another user or is
    /// accessible to others, and with [`StorageError::Locked`] if another
    /// process holds it.
    pub fn open(path: &Path, create: bool) -> Result<Self, StorageError> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        let dir = match rustix::fs::open(path, flags, Mode::empty()) {
            Ok(dir) => dir,
            Err(rustix::io::Errno::NOENT) if create => {
                rustix::fs::mkdir(path, Mode::from_raw_mode(DIR_MODE)).map_err(io)?;
                rustix::fs::open(path, flags, Mode::empty()).map_err(io)?
            }
            Err(rustix::io::Errno::NOENT) => return Err(StorageError::NotFound),
            // A symbolic link, or something else that is not a directory.
            Err(rustix::io::Errno::LOOP | rustix::io::Errno::NOTDIR) => {
                return Err(StorageError::Permissions);
            }
            Err(_) => return Err(StorageError::Io),
        };
        check_owned(&dir, FileType::Directory)?;
        let lock = rustix::fs::openat(
            &dir,
            LOCK,
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(FILE_MODE),
        )
        .map_err(|error| match error {
            rustix::io::Errno::LOOP => StorageError::Permissions,
            _ => StorageError::Io,
        })?;
        check_owned(&lock, FileType::RegularFile)?;
        match rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => {}
            Err(rustix::io::Errno::WOULDBLOCK) => return Err(StorageError::Locked),
            Err(_) => return Err(StorageError::Io),
        }
        Ok(Self { dir, _lock: lock })
    }

    fn open_file(&self, name: &str) -> Result<Option<File>, StorageError> {
        match rustix::fs::openat(
            &self.dir,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => {
                check_owned(&fd, FileType::RegularFile)?;
                Ok(Some(File::from(fd)))
            }
            Err(rustix::io::Errno::NOENT) => Ok(None),
            Err(rustix::io::Errno::LOOP) => Err(StorageError::Permissions),
            Err(_) => Err(StorageError::Io),
        }
    }
}

/// Checks that `fd` is of the type `kind`, belongs to the current user and
/// is accessible to nobody else.
fn check_owned<Fd: AsFd>(fd: Fd, kind: FileType) -> Result<(), StorageError> {
    let stat = rustix::fs::fstat(fd).map_err(io)?;
    if FileType::from_raw_mode(stat.st_mode) != kind
        || stat.st_uid != rustix::process::getuid().as_raw()
        || stat.st_mode & OTHERS != 0
    {
        return Err(StorageError::Permissions);
    }
    Ok(())
}

impl VaultDir for DiskDir {
    fn read(&self, name: &str, max: u64) -> Result<Option<Vec<u8>>, StorageError> {
        let Some(mut file) = self.open_file(name)? else {
            return Ok(None);
        };
        let len = file.metadata().map_err(std_io)?.len();
        if len > max {
            return Err(StorageError::TooLarge);
        }
        let mut bytes =
            Vec::with_capacity(usize::try_from(len).map_err(|_| StorageError::TooLarge)?);
        // One byte more than allowed is read, so that a file that grew
        // after the length was taken is still refused.
        let limit = max.saturating_add(1);
        Read::by_ref(&mut file)
            .take(limit)
            .read_to_end(&mut bytes)
            .map_err(std_io)?;
        if u64::try_from(bytes.len()).map_err(|_| StorageError::TooLarge)? > max {
            return Err(StorageError::TooLarge);
        }
        Ok(Some(bytes))
    }

    fn write_new(&self, name: &str, bytes: &[u8]) -> Result<(), StorageError> {
        let fd = rustix::fs::openat(
            &self.dir,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(FILE_MODE),
        )
        .map_err(|error| match error {
            rustix::io::Errno::EXIST => StorageError::Exists,
            _ => StorageError::Io,
        })?;
        let mut file = File::from(fd);
        file.write_all(bytes).map_err(std_io)?;
        file.sync_all().map_err(std_io)
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), StorageError> {
        rustix::fs::renameat(&self.dir, from, &self.dir, to).map_err(io)
    }

    fn link(&self, from: &str, to: &str) -> Result<(), StorageError> {
        rustix::fs::linkat(&self.dir, from, &self.dir, to, AtFlags::empty()).map_err(|error| {
            match error {
                rustix::io::Errno::EXIST => StorageError::Exists,
                _ => StorageError::Io,
            }
        })
    }

    fn remove(&self, name: &str) -> Result<(), StorageError> {
        rustix::fs::unlinkat(&self.dir, name, AtFlags::empty()).map_err(io)
    }

    fn sync(&self) -> Result<(), StorageError> {
        rustix::fs::fsync(&self.dir).map_err(io)
    }
}

/// What survives a crash of a [`MemoryDir`].
///
/// A real file system may keep more than what was flushed, or less of a
/// file that was being written. The four combinations cover what a vault
/// has to survive.
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
/// the directory a new process finds, as [`CrashOutcome`] says. Clones
/// share the same directory.
#[derive(Clone, Debug, Default)]
pub struct MemoryDir(Arc<Mutex<MemoryState>>);

impl MemoryDir {
    /// An empty directory.
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> MutexGuard<'_, MemoryState> {
        // A test that panicked while holding the lock left a consistent
        // state behind: every change is made in one statement.
        self.0
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
                let content = match (&node.flushed, outcome.keep_torn_contents) {
                    (Some(flushed), _) => flushed.clone(),
                    (None, true) => node
                        .content
                        .get(..node.content.len() / 2)
                        .unwrap_or_default()
                        .to_vec(),
                    (None, false) => Vec::new(),
                };
                Node {
                    flushed: Some(content.clone()),
                    content,
                }
            })
            .collect();
        Self(Arc::new(Mutex::new(MemoryState {
            nodes,
            flushed_names: names.clone(),
            names,
            steps: 0,
            crash_at: None,
            crashed: false,
        })))
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    fn temp(name: &str) -> std::path::PathBuf {
        let base =
            std::env::temp_dir().join(format!("monolith-storage-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        base
    }

    #[test]
    fn a_directory_is_created_private_and_locked_once() {
        let path = temp("create");
        assert_eq!(
            DiskDir::open(&path, false).err(),
            Some(StorageError::NotFound)
        );
        let dir = DiskDir::open(&path, true).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777 & !0o700, 0);
        // A second holder is refused while the first holds it.
        assert_eq!(
            DiskDir::open(&path, false).err(),
            Some(StorageError::Locked)
        );
        drop(dir);
        let dir = DiskDir::open(&path, false).unwrap();
        dir.write_new(VAULT_NEW, b"abc").unwrap();
        let mode = std::fs::metadata(path.join(VAULT_NEW))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(dir.write_new(VAULT_NEW, b"x"), Err(StorageError::Exists));
        dir.link(VAULT_NEW, VAULT).unwrap();
        assert_eq!(dir.link(VAULT_NEW, VAULT), Err(StorageError::Exists));
        dir.remove(VAULT_NEW).unwrap();
        dir.sync().unwrap();
        assert_eq!(dir.read(VAULT, 3).unwrap(), Some(b"abc".to_vec()));
        assert_eq!(dir.read(VAULT, 2), Err(StorageError::TooLarge));
        assert_eq!(dir.read(VAULT_NEW, 3).unwrap(), None);
        std::fs::remove_dir_all(&path).unwrap();
    }

    #[test]
    fn a_directory_others_can_reach_or_a_link_is_refused() {
        let path = temp("permissions");
        std::fs::DirBuilder::new()
            .mode(0o755)
            .create(&path)
            .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            DiskDir::open(&path, false).err(),
            Some(StorageError::Permissions)
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = temp("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert_eq!(
            DiskDir::open(&link, false).err(),
            Some(StorageError::Permissions)
        );
        // A vault file that others can read is refused, and so is a link.
        let dir = DiskDir::open(&path, false).unwrap();
        dir.write_new(VAULT, b"abc").unwrap();
        std::fs::set_permissions(path.join(VAULT), std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(dir.read(VAULT, 10), Err(StorageError::Permissions));
        std::os::unix::fs::symlink(path.join(VAULT), path.join(VAULT_NEW)).unwrap();
        assert_eq!(dir.read(VAULT_NEW, 10), Err(StorageError::Permissions));
        std::fs::remove_file(&link).unwrap();
        std::fs::remove_dir_all(&path).unwrap();
    }

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
}

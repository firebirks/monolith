//! The vault directory on Windows (`docs/STORAGE.md` section 3.8).
//!
//! The same operations as on Unix, with what the standard library offers
//! on Windows and no code of Monolith's own outside it:
//!
//! - the directory is held open for the life of the value, without the
//!   share mode that would let it be renamed or deleted (but see the
//!   limits below);
//! - nothing that is a reparse point (a symbolic link or a junction) is
//!   followed: every file is opened as itself and refused if it is one;
//! - the single-instance lock is a file opened with no sharing at all,
//!   which a second process cannot open, and which Windows releases when
//!   the process ends;
//! - a file is created only if it does not exist, and flushed with
//!   `FlushFileBuffers`; the names of the directory are flushed with
//!   `FlushFileBuffers` on the handle of the directory;
//! - a rename is that of the standard library, which replaces its target:
//!   `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` and without
//!   `MOVEFILE_WRITE_THROUGH`, and only if that is refused with
//!   `ERROR_ACCESS_DENIED`, `SetFileInformationByHandle` with
//!   `FileRenameInfoEx` and POSIX semantics (as in the source of Rust
//!   1.95). One that another process blocks for the moment, holding the
//!   file without the sharing that lets it be renamed, as a scanner may,
//!   is tried again for at most [`RENAME_WAITS`] together, and so is the
//!   removal of a file (since Phase 7, P7-10 and P7-23); a second name is
//!   a hard link.
//!
//! What Unix checks and this does not: who owns the directory and who
//! else may read it. Windows keeps that in access control lists, which
//! the standard library cannot read. The default data directory is inside
//! the user's profile, whose list gives access to the user, SYSTEM and
//! the administrators only; a directory elsewhere has the list its parent
//! gives it.
//!
//! Two limits besides. Microsoft does not document that `FlushFileBuffers`
//! on a handle of a directory makes a rename in it durable, as `fsync` of
//! a directory does on Unix; it is the nearest call Windows has. And only
//! the last component of the directory is held open: every operation
//! names its file by the path of the directory joined with the file name,
//! which Windows resolves again each time.

use core::time::Duration;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::StorageError;
use crate::dir::{LOCK, VaultDir};

/// Opens a directory (needed to open one at all).
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
/// Opens a reparse point as itself instead of what it points to.
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
/// The attribute of a reparse point.
const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
/// The attribute of a directory.
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x0000_0010;
/// Others may read and write a handle, but not delete or rename its file.
const FILE_SHARE_READ_WRITE: u32 = 0x0000_0001 | 0x0000_0002;
/// `ERROR_SHARING_VIOLATION`: another process holds the file.
const ERROR_SHARING_VIOLATION: i32 = 32;
/// `ERROR_ACCESS_DENIED`: what a rename gets when another process holds
/// its target.
const ERROR_ACCESS_DENIED: i32 = 5;
/// The pauses before a rename is tried again, while another process holds
/// its file: about 1.6 s in all, then the rename fails.
const RENAME_WAITS: [Duration; 7] = [
    Duration::from_millis(10),
    Duration::from_millis(20),
    Duration::from_millis(50),
    Duration::from_millis(100),
    Duration::from_millis(200),
    Duration::from_millis(400),
    Duration::from_millis(800),
];

fn std_io(_: std::io::Error) -> StorageError {
    StorageError::Io
}

/// Runs `operation`, a rename or a removal, and runs it again after each
/// of [`RENAME_WAITS`] while another process holds the file without the
/// sharing that allows it, for the moment, as a scanner may, or for good:
/// a few pauses tell which.
fn retried(mut operation: impl FnMut() -> std::io::Result<()>) -> Result<(), StorageError> {
    let mut waits = RENAME_WAITS.iter();
    loop {
        match operation() {
            Ok(()) => return Ok(()),
            Err(error)
                if matches!(
                    error.raw_os_error(),
                    Some(ERROR_SHARING_VIOLATION | ERROR_ACCESS_DENIED)
                ) =>
            {
                let wait = waits.next().ok_or(StorageError::Io)?;
                std::thread::sleep(*wait);
            }
            Err(_) => return Err(StorageError::Io),
        }
    }
}

/// Checks that the open `file` is of the expected kind and is not a
/// reparse point.
fn check_kind(file: &File, directory: bool) -> Result<(), StorageError> {
    let attributes = file.metadata().map_err(std_io)?.file_attributes();
    let is_directory = attributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 || is_directory != directory {
        return Err(StorageError::Permissions);
    }
    Ok(())
}

/// A vault directory on disk, held for the life of the value. See the
/// module documentation.
#[derive(Debug)]
pub struct DiskDir {
    path: PathBuf,
    /// Holds the directory, and flushes its names.
    dir: File,
    /// Holds the lock while it is open.
    _lock: File,
}

fn open_directory(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

impl DiskDir {
    /// Opens the vault directory at `path`, creating it if it does not
    /// exist and `create` is true.
    ///
    /// Fails with [`StorageError::NotFound`] if it does not exist and
    /// `create` is false, with [`StorageError::Permissions`] if it is a
    /// reparse point or not a directory, and with [`StorageError::Locked`]
    /// if another process holds it.
    pub fn open(path: &Path, create: bool) -> Result<Self, StorageError> {
        let dir = match open_directory(path) {
            Ok(dir) => dir,
            Err(error) if error.kind() == ErrorKind::NotFound && create => {
                std::fs::create_dir(path).map_err(std_io)?;
                open_directory(path).map_err(std_io)?
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(StorageError::NotFound);
            }
            Err(_) => return Err(StorageError::Io),
        };
        check_kind(&dir, true)?;
        if create {
            // The name of the directory is durable only once its parent is
            // flushed, as on Unix.
            let parent = match path.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => parent,
                _ => Path::new("."),
            };
            open_directory(parent)
                .and_then(|parent| parent.sync_all())
                .map_err(std_io)?;
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .share_mode(0)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path.join(LOCK))
            .map_err(|error| {
                if error.raw_os_error() == Some(ERROR_SHARING_VIOLATION) {
                    StorageError::Locked
                } else {
                    StorageError::Io
                }
            })?;
        check_kind(&lock, false)?;
        Ok(Self {
            path: path.to_owned(),
            dir,
            _lock: lock,
        })
    }

    /// The path of the file `name`, which is a file name of the vault and
    /// never a path.
    fn file(&self, name: &str) -> Result<PathBuf, StorageError> {
        if name.is_empty() || name.contains(['/', '\\', ':']) || name == "." || name == ".." {
            return Err(StorageError::Io);
        }
        Ok(self.path.join(name))
    }

    fn open_file(&self, name: &str) -> Result<Option<File>, StorageError> {
        let path = self.file(name)?;
        match OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(&path)
        {
            Ok(file) => {
                check_kind(&file, false)?;
                Ok(Some(file))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            // A directory cannot be opened as a file: it is not one.
            Err(_) if std::fs::symlink_metadata(&path).is_ok_and(|meta| !meta.is_file()) => {
                Err(StorageError::Permissions)
            }
            Err(_) => Err(StorageError::Io),
        }
    }
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
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(self.file(name)?)
            .map_err(|error| match error.kind() {
                ErrorKind::AlreadyExists => StorageError::Exists,
                _ => StorageError::Io,
            })?;
        file.write_all(bytes).map_err(std_io)?;
        file.sync_all().map_err(std_io)
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), StorageError> {
        let (from, to) = (self.file(from)?, self.file(to)?);
        retried(|| std::fs::rename(&from, &to))
    }

    fn link(&self, from: &str, to: &str) -> Result<(), StorageError> {
        std::fs::hard_link(self.file(from)?, self.file(to)?).map_err(|error| match error.kind() {
            ErrorKind::AlreadyExists => StorageError::Exists,
            _ => StorageError::Io,
        })
    }

    fn remove(&self, name: &str) -> Result<(), StorageError> {
        let path = self.file(name)?;
        retried(|| std::fs::remove_file(&path))
    }

    fn sync(&self) -> Result<(), StorageError> {
        self.dir.sync_all().map_err(std_io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dir::{VAULT, VAULT_NEW};

    fn temp(name: &str) -> PathBuf {
        let base =
            std::env::temp_dir().join(format!("monolith-storage-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        base
    }

    #[test]
    fn a_directory_is_created_and_locked_once() {
        let path = temp("create");
        assert_eq!(
            DiskDir::open(&path, false).err(),
            Some(StorageError::NotFound)
        );
        let dir = DiskDir::open(&path, true).unwrap();
        // A second holder is refused while the first holds it.
        assert_eq!(
            DiskDir::open(&path, false).err(),
            Some(StorageError::Locked)
        );
        drop(dir);
        let dir = DiskDir::open(&path, false).unwrap();
        dir.write_new(VAULT_NEW, b"abc").unwrap();
        assert_eq!(dir.write_new(VAULT_NEW, b"x"), Err(StorageError::Exists));
        dir.link(VAULT_NEW, VAULT).unwrap();
        assert_eq!(dir.link(VAULT_NEW, VAULT), Err(StorageError::Exists));
        dir.remove(VAULT_NEW).unwrap();
        dir.sync().unwrap();
        assert_eq!(dir.read(VAULT, 3).unwrap(), Some(b"abc".to_vec()));
        assert_eq!(dir.read(VAULT, 2), Err(StorageError::TooLarge));
        assert_eq!(dir.read(VAULT_NEW, 3).unwrap(), None);
        // A rename replaces its target.
        dir.write_new(VAULT_NEW, b"defg").unwrap();
        dir.rename(VAULT_NEW, VAULT).unwrap();
        dir.sync().unwrap();
        assert_eq!(dir.read(VAULT, 4).unwrap(), Some(b"defg".to_vec()));
        assert_eq!(dir.read(VAULT_NEW, 4).unwrap(), None);
        drop(dir);
        std::fs::remove_dir_all(&path).unwrap();
    }

    /// Opens `path` as a scanner may: for reading, without the sharing
    /// that lets the file be renamed or deleted.
    fn held(path: &Path) -> File {
        OpenOptions::new()
            .read(true)
            .share_mode(0x0000_0001)
            .open(path)
            .unwrap()
    }

    #[test]
    fn a_rename_waits_a_little_for_a_file_another_process_holds() {
        let path = temp("rename-held");
        let dir = DiskDir::open(&path, true).unwrap();
        for name in [VAULT_NEW, VAULT] {
            dir.write_new(VAULT_NEW, b"new").unwrap();
            if !path.join(VAULT).exists() {
                std::fs::write(path.join(VAULT), b"old").unwrap();
            }
            // The source or the target, held for a moment.
            let file = held(&path.join(name));
            let releasing = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(150));
                drop(file);
            });
            assert_eq!(dir.rename(VAULT_NEW, VAULT), Ok(()), "{name}");
            releasing.join().unwrap();
            assert_eq!(dir.read(VAULT, 3).unwrap(), Some(b"new".to_vec()));
        }
        drop(dir);
        std::fs::remove_dir_all(&path).unwrap();
    }

    #[test]
    fn a_removal_waits_a_little_for_a_file_another_process_holds() {
        // What a write that used up its pauses left, still held by a
        // scanner when the next write removes it.
        let path = temp("remove-held");
        let dir = DiskDir::open(&path, true).unwrap();
        dir.write_new(VAULT_NEW, b"new").unwrap();
        let file = held(&path.join(VAULT_NEW));
        let releasing = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(file);
        });
        assert_eq!(dir.remove(VAULT_NEW), Ok(()));
        releasing.join().unwrap();
        assert_eq!(dir.read(VAULT_NEW, 3).unwrap(), None);
        drop(dir);
        std::fs::remove_dir_all(&path).unwrap();
    }

    #[test]
    fn a_rename_of_a_file_held_for_good_fails_in_bounded_time() {
        let path = temp("rename-held-long");
        let dir = DiskDir::open(&path, true).unwrap();
        dir.write_new(VAULT_NEW, b"new").unwrap();
        let file = held(&path.join(VAULT_NEW));
        let start = std::time::Instant::now();
        assert_eq!(dir.rename(VAULT_NEW, VAULT), Err(StorageError::Io));
        let waited = start.elapsed();
        let all: Duration = RENAME_WAITS.iter().sum();
        assert!(waited >= all, "{waited:?}");
        assert!(waited < all * 3, "{waited:?}");
        // Nothing changed.
        drop(file);
        assert_eq!(dir.read(VAULT, 3).unwrap(), None);
        assert_eq!(dir.read(VAULT_NEW, 3).unwrap(), Some(b"new".to_vec()));
        drop(dir);
        std::fs::remove_dir_all(&path).unwrap();
    }

    #[test]
    fn a_name_is_never_a_path() {
        let path = temp("names");
        let dir = DiskDir::open(&path, true).unwrap();
        for name in ["..\\vault", "a/b", "C:vault", "", ".", ".."] {
            assert_eq!(dir.write_new(name, b"x"), Err(StorageError::Io));
            assert_eq!(dir.read(name, 1), Err(StorageError::Io));
        }
        drop(dir);
        std::fs::remove_dir_all(&path).unwrap();
    }

    #[test]
    fn a_file_where_the_directory_should_be_is_refused() {
        let path = temp("file");
        std::fs::write(&path, b"not a directory").unwrap();
        assert_eq!(
            DiskDir::open(&path, false).err(),
            Some(StorageError::Permissions)
        );
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn a_directory_where_a_file_should_be_is_refused() {
        let path = temp("dir-for-file");
        let dir = DiskDir::open(&path, true).unwrap();
        std::fs::create_dir(path.join(VAULT)).unwrap();
        assert_eq!(dir.read(VAULT, 10), Err(StorageError::Permissions));
        drop(dir);
        std::fs::remove_dir_all(&path).unwrap();
    }
}

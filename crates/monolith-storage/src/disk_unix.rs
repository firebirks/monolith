//! The vault directory on Unix (`docs/STORAGE.md` section 3.7): every
//! operation goes through the file descriptor of the directory, so that no
//! path is looked up again after the checks.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::FileExt;
use std::path::Path;

use rustix::fs::{AtFlags, FileType, FlockOperation, Mode, OFlags};
use rustix::io::Errno;

use crate::StorageError;
use crate::dir::{BlobFile, LOCK, VaultDir};

fn io(_: rustix::io::Errno) -> StorageError {
    StorageError::Io
}

fn std_io(_: std::io::Error) -> StorageError {
    StorageError::Io
}

/// An error of a write: a full file system or quota is
/// [`StorageError::Full`], anything else an I/O failure.
pub(crate) fn write_error(error: &std::io::Error) -> StorageError {
    let full = [Errno::NOSPC, Errno::DQUOT].map(Errno::raw_os_error);
    if error
        .raw_os_error()
        .is_some_and(|code| full.contains(&code))
    {
        StorageError::Full
    } else {
        StorageError::Io
    }
}

/// A file of the directory held open to be read and written at offsets.
#[derive(Debug)]
struct DiskBlob(File);

impl BlobFile for DiskBlob {
    fn length(&self) -> Result<u64, StorageError> {
        Ok(self.0.metadata().map_err(std_io)?.len())
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), StorageError> {
        self.0.read_exact_at(buf, offset).map_err(std_io)
    }

    fn write_at(&mut self, offset: u64, bytes: &[u8]) -> Result<(), StorageError> {
        self.0
            .write_all_at(bytes, offset)
            .map_err(|error| write_error(&error))
    }

    fn sync(&mut self) -> Result<(), StorageError> {
        self.0.sync_all().map_err(|error| write_error(&error))
    }
}

/// Gives the open file `fd` the length `len`, allocated where the file
/// system allows it, so that writing into it later needs no new space
/// (`docs/adr/0014`). Where allocation is refused or does not exist, the
/// file only gets the length; the free space was checked before.
fn reserve(fd: &OwnedFd, len: u64) -> Result<(), StorageError> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    match rustix::fs::fallocate(fd, rustix::fs::FallocateFlags::empty(), 0, len) {
        Ok(()) => return Ok(()),
        Err(Errno::NOSPC | Errno::DQUOT) => return Err(StorageError::Full),
        Err(Errno::OPNOTSUPP | Errno::NOSYS) => {}
        Err(_) => return Err(StorageError::Io),
    }
    match rustix::fs::ftruncate(fd, len) {
        Ok(()) => Ok(()),
        Err(Errno::NOSPC | Errno::DQUOT | Errno::FBIG) => Err(StorageError::Full),
        Err(_) => Err(StorageError::Io),
    }
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
        if create {
            // The name of the directory is durable only once its parent is
            // flushed, and a vault created in it is reported durable. Also
            // when the directory was there already: an earlier attempt may
            // have made it and failed before the flush.
            let parent = match path.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => parent,
                _ => Path::new("."),
            };
            let parent = rustix::fs::open(
                parent,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(io)?;
            rustix::fs::fsync(&parent).map_err(io)?;
        }
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

    fn create_reserved(
        &self,
        name: &str,
        len: u64,
        margin: u64,
    ) -> Result<Box<dyn BlobFile>, StorageError> {
        // The blocks available to this user, not to the superuser.
        let stat = rustix::fs::fstatvfs(&self.dir).map_err(io)?;
        let free = stat.f_bavail.saturating_mul(stat.f_frsize);
        if free < len.saturating_add(margin) {
            return Err(StorageError::Full);
        }
        let fd = rustix::fs::openat(
            &self.dir,
            name,
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(FILE_MODE),
        )
        .map_err(|error| match error {
            Errno::EXIST => StorageError::Exists,
            _ => StorageError::Io,
        })?;
        if let Err(error) = reserve(&fd, len) {
            drop(fd);
            let _ = rustix::fs::unlinkat(&self.dir, name, AtFlags::empty());
            return Err(error);
        }
        Ok(Box::new(DiskBlob(File::from(fd))))
    }

    fn open_blob(&self, name: &str) -> Result<Option<Box<dyn BlobFile>>, StorageError> {
        match rustix::fs::openat(
            &self.dir,
            name,
            OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => {
                check_owned(&fd, FileType::RegularFile)?;
                Ok(Some(Box::new(DiskBlob(File::from(fd)))))
            }
            Err(Errno::NOENT) => Ok(None),
            Err(Errno::LOOP) => Err(StorageError::Permissions),
            Err(_) => Err(StorageError::Io),
        }
    }

    fn list(&self) -> Result<Vec<String>, StorageError> {
        let mut names = Vec::new();
        for entry in rustix::fs::Dir::read_from(&self.dir).map_err(io)? {
            let entry = entry.map_err(io)?;
            let name = entry.file_name().to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            // A name that is not UTF-8 is no name of Monolith's.
            if let Ok(name) = core::str::from_utf8(name) {
                names.push(name.to_owned());
            }
        }
        Ok(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dir::{VAULT, VAULT_NEW};
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    fn temp(name: &str) -> std::path::PathBuf {
        let base =
            std::env::temp_dir().join(format!("monolith-storage-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        base
    }

    #[test]
    fn a_blob_is_reserved_written_and_read_at_offsets_and_listed() {
        let base = temp("blob");
        let dir = DiskDir::open(&base, true).unwrap();
        let name = "transfer-0123456789abcdef0123456789abcdef";
        let mut blob = dir.create_reserved(name, 100, 0).unwrap();
        assert_eq!(blob.length().unwrap(), 100);
        blob.write_at(10, b"chunk").unwrap();
        blob.sync().unwrap();
        dir.sync().unwrap();
        assert!(matches!(
            dir.create_reserved(name, 1, 0),
            Err(StorageError::Exists)
        ));
        drop(blob);
        let mut opened = dir.open_blob(name).unwrap().unwrap();
        let mut buf = [0xFF; 7];
        opened.read_at(9, &mut buf).unwrap();
        assert_eq!(&buf, b"\0chunk\0");
        assert_eq!(opened.read_at(98, &mut buf), Err(StorageError::Io));
        assert!(dir.open_blob("transfer-none").unwrap().is_none());
        let names = dir.list().unwrap();
        assert!(names.iter().any(|listed| listed == name));
        assert!(names.iter().any(|listed| listed == LOCK));
        drop(opened);
        dir.remove(name).unwrap();
        drop(dir);
        let _ = std::fs::remove_dir_all(&base);
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
}

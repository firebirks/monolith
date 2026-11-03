//! Saving a received file in a folder the user chose
//! (`docs/adr/0014-transfer-staging.md`, "Saving a received file").
//!
//! The front ends do not write the file themselves: the runtime saves it
//! here, so that every front end saves the same way. The place is a path
//! the user chose, a folder and a name in it; the name a peer gave is at
//! most a suggestion the front end showed. In the folder, in order:
//!
//! 1. whether it takes a hard link is found out with a probe file of no
//!    content, created only if no file has its name, linked and removed;
//!    on Linux a folder that refuses the link (`EPERM`, `EOPNOTSUPP`) is
//!    probed for a rename that keeps an existing name (`renameat2` with
//!    `RENAME_NOREPLACE`) instead. A folder that takes neither is refused
//!    before anything is written; on Windows no safe call renames without
//!    replacing, so there a folder without hard links is refused;
//! 2. a temporary file is created, only if no file has its name, named
//!    [`PARTIAL_PREFIX`] and 16 random hexadecimal digits: a visible name;
//! 3. what the source gives is written to it, up to the first error;
//! 4. once the source ended without an error, the file is synced;
//! 5. it gets the name chosen without replacing anything: a hard link,
//!    which fails if the name exists, and the temporary name removed; or
//!    the rename of step 1;
//! 6. the folder is synced (on Windows by flushing a handle of the
//!    folder, with the caveat of `STORAGE.md` section 3.8);
//! 7. only then is the file saved.
//!
//! A failure before step 5 removes the temporary file and leaves nothing.
//! A failure after it (the folder sync, or removing the temporary name)
//! leaves the whole file under its name, and the save is not confirmed.
//! Nothing is ever written under the name chosen directly, and nothing is
//! replaced. A crash leaves at most a file named `monolith-partial-...`.
//!
//! The place must be one of this computer ([`crate::local::here`] for the
//! folder, [`crate::local::named_here`] for the whole path): on Windows a
//! share, a link or a junction on the way, a name of a device or a stream
//! is refused before anything is opened.

use std::ffi::OsStr;
use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::path::Path;

use zeroize::Zeroizing;

use crate::StorageError;
use crate::local;

/// The start of the name of a temporary file, and of a probe file.
pub const PARTIAL_PREFIX: &str = "monolith-partial-";

/// How much is read from the source at a time.
const BUFFER: usize = 64 * 1024;

/// How a save that wrote the file ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Saved {
    /// The file is whole under its name, and the folder was synced.
    Confirmed,
    /// The file is whole under its name, but the folder could not be
    /// synced or the temporary name removed: a crash now could lose the
    /// name, and a file named `monolith-partial-...` may be left.
    Unconfirmed,
}

/// Why a save left no file under the name chosen. The folder holds what it
/// held before.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ExportError {
    /// Something has that name already. Nothing was replaced.
    Exists,
    /// The place is refused before anything is written there: not an
    /// absolute path, not a folder of this computer, a name that is not a
    /// plain file name, or a folder that takes neither a hard link nor, on
    /// Linux, a rename that keeps an existing name.
    Refused,
    /// The source gave an error before its end: what it read back is not
    /// what was received, or the save was cancelled.
    Source,
    /// The disk has no room left.
    Full,
    /// The disk failed, or the random source.
    Io,
}

impl core::fmt::Display for ExportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::Exists => "a file of that name exists",
            Self::Refused => "not a place a file can be saved",
            Self::Source => "the received file does not read back whole",
            Self::Full => "not enough disk space",
            Self::Io => "disk failure",
        })
    }
}

impl core::error::Error for ExportError {}

fn written(error: StorageError) -> ExportError {
    match error {
        StorageError::Full => ExportError::Full,
        _ => ExportError::Io,
    }
}

/// How the name chosen is given to the file: by a hard link, or by a
/// rename that keeps an existing name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Way {
    Link,
    #[cfg(any(target_os = "linux", target_os = "android"))]
    Rename,
}

/// A name for a temporary or probe file.
fn partial_name() -> Result<String, ExportError> {
    let mut bytes = [0_u8; 8];
    getrandom::fill(&mut bytes).map_err(|_| ExportError::Io)?;
    let mut name = String::from(PARTIAL_PREFIX);
    for byte in bytes {
        name.push_str(&format!("{byte:02x}"));
    }
    Ok(name)
}

/// The folder and the name of `destination`, once the place passed the
/// rules of this computer.
fn place(destination: &Path) -> Result<(&Path, &OsStr), ExportError> {
    let name = destination.file_name().ok_or(ExportError::Refused)?;
    let folder = destination
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .ok_or(ExportError::Refused)?;
    if !destination.is_absolute() || !local::named_here(destination) || !local::here(folder) {
        return Err(ExportError::Refused);
    }
    Ok((folder, name))
}

/// Writes what `source` gives into `file`, up to its end or its first
/// error, then syncs the file.
fn copy(source: &mut dyn Read, file: &mut File) -> Result<(), ExportError> {
    let mut buffer = Zeroizing::new(vec![0_u8; BUFFER]);
    loop {
        let read = match source.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return Err(ExportError::Source),
        };
        let chunk = buffer.get(..read).ok_or(ExportError::Source)?;
        file.write_all(chunk)
            .map_err(|error| written(platform::write_error(&error)))?;
    }
    file.sync_all()
        .map_err(|error| written(platform::write_error(&error)))
}

/// Checks `destination` as [`save_new`] does before it writes anything,
/// probe aside: the rules of this computer, a folder that opens, and no
/// file of that name. Writes nothing; a save may still fail.
pub fn check(destination: &Path) -> Result<(), ExportError> {
    let (folder, name) = place(destination)?;
    if platform::Folder::open(folder)?.has(name) {
        return Err(ExportError::Exists);
    }
    Ok(())
}

/// Saves what `source` gives as a new file at `destination`, the folder and
/// the name the user chose; see the module documentation. The source must
/// return an error, not its end, unless what it gave is whole and right.
pub fn save_new(destination: &Path, source: &mut dyn Read) -> Result<Saved, ExportError> {
    let (folder, name) = place(destination)?;
    let folder = platform::Folder::open(folder)?;
    // Not the check that counts (step 5 is), but a file that exists is
    // refused before the whole file is written for nothing.
    if folder.has(name) {
        return Err(ExportError::Exists);
    }
    let way = folder.probe()?;
    let partial = partial_name()?;
    let mut file = folder.create(&partial)?;
    let copied = copy(source, &mut file);
    drop(file);
    if let Err(error) = copied {
        let _ = folder.remove(&partial);
        return Err(error);
    }
    if let Err(error) = folder.give_name(&partial, name, way) {
        let _ = folder.remove(&partial);
        return Err(error);
    }
    let removed = match way {
        Way::Link => folder.remove(&partial).is_ok(),
        #[cfg(any(target_os = "linux", target_os = "android"))]
        Way::Rename => true,
    };
    let synced = folder.sync().is_ok();
    Ok(if removed && synced {
        Saved::Confirmed
    } else {
        Saved::Unconfirmed
    })
}

#[cfg(unix)]
mod platform {
    use std::ffi::OsStr;
    use std::fs::File;
    use std::os::fd::OwnedFd;
    use std::path::Path;

    use rustix::fs::{AtFlags, Mode, OFlags};
    use rustix::io::Errno;

    use super::{ExportError, Way, partial_name};

    pub(super) use crate::disk_unix::write_error;

    /// The mode of a saved file: the user's alone, as a saved card.
    const FILE_MODE: u32 = 0o600;

    /// The folder, held open: every step names its file relative to it,
    /// so that the steps happen in one folder whatever its path names
    /// meanwhile.
    pub(super) struct Folder(OwnedFd);

    impl Folder {
        pub(super) fn open(path: &Path) -> Result<Self, ExportError> {
            rustix::fs::open(
                path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map(Self)
            .map_err(|error| match error {
                Errno::NOENT | Errno::NOTDIR | Errno::ACCESS | Errno::LOOP => ExportError::Refused,
                _ => ExportError::Io,
            })
        }

        pub(super) fn has(&self, name: &OsStr) -> bool {
            rustix::fs::statat(&self.0, name, AtFlags::SYMLINK_NOFOLLOW).is_ok()
        }

        pub(super) fn create(&self, name: &str) -> Result<File, ExportError> {
            rustix::fs::openat(
                &self.0,
                name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::from_raw_mode(FILE_MODE),
            )
            .map(File::from)
            .map_err(|error| match error {
                Errno::NOSPC | Errno::DQUOT => ExportError::Full,
                Errno::ACCESS | Errno::PERM | Errno::ROFS => ExportError::Refused,
                _ => ExportError::Io,
            })
        }

        pub(super) fn remove(&self, name: &str) -> Result<(), ExportError> {
            rustix::fs::unlinkat(&self.0, name, AtFlags::empty()).map_err(|_| ExportError::Io)
        }

        pub(super) fn sync(&self) -> Result<(), ExportError> {
            rustix::fs::fsync(&self.0).map_err(|_| ExportError::Io)
        }

        /// Step 1: how the folder gives a name without replacing one.
        pub(super) fn probe(&self) -> Result<Way, ExportError> {
            let probe = partial_name()?;
            let other = partial_name()?;
            drop(self.create(&probe)?);
            let found = match rustix::fs::linkat(&self.0, &probe, &self.0, &other, AtFlags::empty())
            {
                Ok(()) => {
                    let _ = self.remove(&other);
                    Ok(Way::Link)
                }
                Err(Errno::PERM | Errno::OPNOTSUPP) => self.probe_rename(&probe, &other),
                Err(_) => Err(ExportError::Io),
            };
            let _ = self.remove(&probe);
            found
        }

        #[cfg(any(target_os = "linux", target_os = "android"))]
        fn probe_rename(&self, probe: &str, other: &str) -> Result<Way, ExportError> {
            use rustix::fs::RenameFlags;
            match rustix::fs::renameat_with(&self.0, probe, &self.0, other, RenameFlags::NOREPLACE)
            {
                Ok(()) => {
                    let _ = self.remove(other);
                    Ok(Way::Rename)
                }
                Err(Errno::INVAL | Errno::NOSYS) => Err(ExportError::Refused),
                Err(_) => Err(ExportError::Io),
            }
        }

        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        #[allow(clippy::unused_self)]
        fn probe_rename(&self, _: &str, _: &str) -> Result<Way, ExportError> {
            Err(ExportError::Refused)
        }

        /// Step 5: `partial` gets `name`, unless something has it.
        pub(super) fn give_name(
            &self,
            partial: &str,
            name: &OsStr,
            way: Way,
        ) -> Result<(), ExportError> {
            let given = match way {
                Way::Link => rustix::fs::linkat(&self.0, partial, &self.0, name, AtFlags::empty()),
                #[cfg(any(target_os = "linux", target_os = "android"))]
                Way::Rename => rustix::fs::renameat_with(
                    &self.0,
                    partial,
                    &self.0,
                    name,
                    rustix::fs::RenameFlags::NOREPLACE,
                ),
            };
            given.map_err(|error| match error {
                Errno::EXIST => ExportError::Exists,
                Errno::NOSPC | Errno::DQUOT => ExportError::Full,
                _ => ExportError::Io,
            })
        }
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::OsStr;
    use std::fs::{File, OpenOptions};
    use std::io::ErrorKind;
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::{Path, PathBuf};

    use super::{ExportError, Way, partial_name};

    pub(super) use crate::disk_windows::write_error;
    use crate::disk_windows::{open_directory, retried};

    /// Opens a reparse point as itself instead of what it points to.
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    /// The folder, by its path, which Windows resolves again for each
    /// step: the standard library cannot name a file relative to an open
    /// folder. `local::here` looked at every part of the way before.
    pub(super) struct Folder(PathBuf);

    impl Folder {
        pub(super) fn open(path: &Path) -> Result<Self, ExportError> {
            if !std::fs::symlink_metadata(path).is_ok_and(|looked| looked.is_dir()) {
                return Err(ExportError::Refused);
            }
            Ok(Self(path.to_owned()))
        }

        pub(super) fn has(&self, name: &OsStr) -> bool {
            std::fs::symlink_metadata(self.0.join(name)).is_ok()
        }

        pub(super) fn create(&self, name: &str) -> Result<File, ExportError> {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(self.0.join(name))
                .map_err(|error| match error.kind() {
                    ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => {
                        ExportError::Refused
                    }
                    _ if super::written(write_error(&error)) == ExportError::Full => {
                        ExportError::Full
                    }
                    _ => ExportError::Io,
                })
        }

        pub(super) fn remove(&self, name: &str) -> Result<(), ExportError> {
            let path = self.0.join(name);
            retried(|| std::fs::remove_file(&path)).map_err(|_| ExportError::Io)
        }

        pub(super) fn sync(&self) -> Result<(), ExportError> {
            open_directory(&self.0)
                .and_then(|folder| folder.sync_all())
                .map_err(|_| ExportError::Io)
        }

        /// Step 1: whether the folder takes a hard link.
        pub(super) fn probe(&self) -> Result<Way, ExportError> {
            let probe = partial_name()?;
            let other = partial_name()?;
            drop(self.create(&probe)?);
            let linked = std::fs::hard_link(self.0.join(&probe), self.0.join(&other));
            if linked.is_ok() {
                let _ = self.remove(&other);
            }
            let _ = self.remove(&probe);
            linked.map(|()| Way::Link).map_err(|_| ExportError::Refused)
        }

        /// Step 5: `partial` gets `name`, unless something has it.
        pub(super) fn give_name(
            &self,
            partial: &str,
            name: &OsStr,
            way: Way,
        ) -> Result<(), ExportError> {
            match way {
                Way::Link => {
                    std::fs::hard_link(self.0.join(partial), self.0.join(name)).map_err(|error| {
                        match error.kind() {
                            ErrorKind::AlreadyExists => ExportError::Exists,
                            _ => ExportError::Io,
                        }
                    })
                }
            }
        }
    }
}

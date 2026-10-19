//! What the screens ask of the disk, done on a thread of its own so that
//! no frame waits: a card saved as a new file, a card file read, the
//! settings file written. Done in the order asked.

use std::fs::{File, Metadata};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use core::time::Duration;

use monolith_ui::EffectResult;
use monolith_ui_model::{Failure, UiIdentityId};
use zeroize::Zeroizing;

use crate::Join;
use crate::bridge::Wake;
use crate::config::{self, Config};
use crate::local;
use crate::places;

/// The largest card file read (`docs/adr/0011-ui-security-boundary.md`).
pub const MAX_CARD_FILE: u64 = 64 * 1024;

/// How many jobs wait at most.
const QUEUE: usize = 16;

/// How long the window waits, when it closes, for the jobs asked for.
const CLOSE_WAIT: Duration = Duration::from_secs(3);

/// Something to do on the disk.
pub enum Job {
    /// Save a card as a new file in `dir`.
    SaveCard {
        /// The home folder: the card goes into its downloads folder, or
        /// into it if it has none (`places::save_dir`).
        home: Option<PathBuf>,
        /// The file name the screens chose.
        name: String,
        /// The card.
        contents: Zeroizing<String>,
    },
    /// Read a card file for the "Add contact" dialog that asked.
    ReadCard {
        /// The file.
        path: PathBuf,
        /// The identity of the dialog.
        identity: UiIdentityId,
        /// The dialog.
        dialog: u64,
    },
    /// Write the settings file.
    Config(PathBuf, Box<Config>),
}

/// The window's end of the disk thread.
pub struct Disk {
    jobs: Option<mpsc::SyncSender<Job>>,
    results: mpsc::Receiver<EffectResult>,
    thread: Join,
}

impl core::fmt::Debug for Disk {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Disk").finish_non_exhaustive()
    }
}

impl Disk {
    /// Starts the thread.
    pub fn start(wake: Wake) -> std::io::Result<Self> {
        Self::start_with(wake, CLOSE_WAIT, run)
    }

    /// Starts the thread, doing each job with `run`.
    fn start_with(
        wake: Wake,
        close_wait: Duration,
        run: fn(Job) -> Option<EffectResult>,
    ) -> std::io::Result<Self> {
        let (jobs, queue) = mpsc::sync_channel::<Job>(QUEUE);
        let (sender, results) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("monolith-disk".to_owned())
            .spawn(move || {
                while let Ok(job) = queue.recv() {
                    if let Some(result) = run(job) {
                        if sender.send(result).is_err() {
                            return;
                        }
                        wake();
                    }
                }
            })?;
        Ok(Self {
            jobs: Some(jobs),
            results,
            thread: Join::new(thread, close_wait),
        })
    }

    /// Takes the waiting out: dropped, the disk then lets the jobs asked
    /// for finish without waiting for them, and what this returns waits
    /// instead.
    pub fn take_join(&mut self) -> Join {
        self.thread.take()
    }

    /// Asks for `job`. If too many wait, it fails at once.
    pub fn ask(&self, job: Job) -> Option<EffectResult> {
        let Some(jobs) = &self.jobs else {
            return Some(EffectResult::Failed(Failure::File));
        };
        match jobs.try_send(job) {
            Ok(()) => None,
            Err(_) => Some(EffectResult::Failed(Failure::File)),
        }
    }

    /// The next result, if one is ready.
    pub fn try_result(&self) -> Option<EffectResult> {
        self.results.try_recv().ok()
    }
}

impl Drop for Disk {
    /// Lets the jobs asked for finish, so that the last settings are
    /// written, and waits for them a while at most, unless the waiting was
    /// taken out ([`Disk::take_join`]): a job that hangs does not hold the
    /// exit.
    fn drop(&mut self) {
        self.jobs = None;
        drop(self.thread.take());
    }
}

fn run(job: Job) -> Option<EffectResult> {
    match job {
        Job::SaveCard {
            home,
            name,
            contents,
        } => Some(match home.map(|home| places::save_dir(&home)) {
            Some(dir) => match save_card(&dir, &name, contents.as_bytes()) {
                Ok(path) => EffectResult::CardSaved {
                    path: path.display().to_string(),
                },
                Err(_) => EffectResult::Failed(Failure::File),
            },
            None => EffectResult::Failed(Failure::File),
        }),
        Job::ReadCard {
            path,
            identity,
            dialog,
        } => Some(match read_card(&path) {
            Ok(text) => EffectResult::CardRead {
                identity,
                dialog,
                text,
            },
            Err(failure) => EffectResult::Failed(failure),
        }),
        Job::Config(path, config) => match config::write(&path, &config) {
            Ok(()) => None,
            Err(_) => Some(EffectResult::Failed(Failure::File)),
        },
    }
}

/// A file name made of `name`: letters, digits, `.`, `-` and `_`, not
/// hidden, not empty, not long.
fn file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    let cleaned = cleaned.trim_start_matches(['.', '-']);
    if cleaned.is_empty() {
        "monolith-card.txt".to_owned()
    } else {
        cleaned.to_owned()
    }
}

/// Saves `contents` in `dir` as a new file named after `name`, readable
/// by the user alone. An existing file is never replaced: the name gets a
/// number instead. A folder that is not one of this computer
/// (`local::here`) is refused before anything is written.
pub fn save_card(dir: &Path, name: &str, contents: &[u8]) -> std::io::Result<PathBuf> {
    if !local::here(dir) {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    let name = file_name(name);
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem.to_owned(), format!(".{extension}")),
        _ => (name.clone(), String::new()),
    };
    for number in 1..=99_u32 {
        let candidate = if number == 1 {
            dir.join(&name)
        } else {
            dir.join(format!("{stem}-{number}{extension}"))
        };
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        match options.open(&candidate) {
            Ok(mut file) => {
                file.write_all(contents)?;
                file.sync_all()?;
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::ErrorKind::AlreadyExists.into())
}

/// Reads a card file: a regular file of at most [`MAX_CARD_FILE`] bytes of
/// UTF-8. What it says is checked like pasted text.
///
/// The path must name a file of this computer (`local::here`): on
/// Windows, a path that names a device, or that leads to a share, through a
/// link or a mapped drive, or to a file kept only in the cloud, is refused
/// before it is read, a device before anything is opened. The name is
/// looked at before anything is opened, and anything but a regular file is
/// refused unopened, a link included: opening a device can do something of
/// its own (rewind a tape, raise a line of a modem), and a pipe or a device
/// could block or never end. The file is then opened without following a
/// link, on Unix without waiting and on Windows without fetching it from
/// remote storage, and read only if it is still a regular file held on the
/// disk, the same one: on Unix the same device and inode, on Windows the
/// same size and creation and write times, which is what the standard
/// library tells of a file there. A name given to something else in
/// between is refused, unread.
pub fn read_card(path: &Path) -> Result<Zeroizing<String>, Failure> {
    read_card_with(path, open)
}

/// [`read_card`], opening the file with `open`.
fn read_card_with(
    path: &Path,
    open: impl FnOnce(&Path) -> std::io::Result<File>,
) -> Result<Zeroizing<String>, Failure> {
    if !local::here(path) {
        return Err(Failure::File);
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|_| Failure::File)?;
    if !metadata.is_file() {
        return Err(Failure::File);
    }
    let file = open(path).map_err(|_| Failure::File)?;
    let opened = file.metadata().map_err(|_| Failure::File)?;
    if !same_file(&metadata, &opened) {
        return Err(Failure::File);
    }
    if opened.len() > MAX_CARD_FILE {
        return Err(Failure::FileTooLarge);
    }
    // Room for one byte more than allowed, so that the card is not copied
    // around as it is read and a file that grew is still refused.
    let room = usize::try_from(MAX_CARD_FILE.saturating_add(1)).map_err(|_| Failure::File)?;
    let mut bytes = Zeroizing::new(Vec::with_capacity(room));
    file.take(MAX_CARD_FILE.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::File)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_CARD_FILE {
        return Err(Failure::FileTooLarge);
    }
    let text = core::str::from_utf8(&bytes).map_err(|_| Failure::InvalidCard)?;
    Ok(Zeroizing::new(text.to_owned()))
}

/// Opens `path` to read it. On Unix a link is not followed, and the file is
/// opened without waiting, so that a pipe without a writer does not block
/// the thread.
#[cfg(unix)]
fn open(path: &Path) -> std::io::Result<File> {
    use rustix::fs::{Mode, OFlags};
    let flags =
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::CLOEXEC;
    Ok(File::from(rustix::fs::open(path, flags, Mode::empty())?))
}

/// Opens `path` to read it. On Windows a reparse point is opened as itself,
/// not followed (`FILE_FLAG_OPEN_REPARSE_POINT`), and a file in remote
/// storage is not brought back to the disk (`FILE_FLAG_OPEN_NO_RECALL`);
/// others may read it meanwhile, but not write it.
#[cfg(windows)]
fn open(path: &Path) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_FLAG_OPEN_NO_RECALL: u32 = 0x0010_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    std::fs::OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_OPEN_NO_RECALL)
        .open(path)
}

#[cfg(not(any(unix, windows)))]
fn open(path: &Path) -> std::io::Result<File> {
    File::open(path)
}

/// Whether the file opened is a regular file and, on Unix, the one looked
/// at: the same device and inode.
#[cfg(unix)]
fn same_file(looked_at: &Metadata, opened: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    opened.is_file() && opened.dev() == looked_at.dev() && opened.ino() == looked_at.ino()
}

/// Whether the file opened is a regular file held on the disk, not a link,
/// and the one looked at, as far as the standard library tells: the same
/// size, creation time and last write time.
#[cfg(windows)]
fn same_file(looked_at: &Metadata, opened: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    opened.is_file()
        && !opened.file_type().is_symlink()
        && local::held_here(opened.file_attributes())
        && opened.file_size() == looked_at.file_size()
        && opened.creation_time() == looked_at.creation_time()
        && opened.last_write_time() == looked_at.last_write_time()
}

#[cfg(not(any(unix, windows)))]
fn same_file(_: &Metadata, opened: &Metadata) -> bool {
    opened.is_file()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::Arc;
    use std::time::Instant;

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("monolith-desktop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_saved_card_never_replaces_a_file() {
        let dir = scratch("save");
        let first = save_card(&dir, "monolith-card.txt", b"one").unwrap();
        let second = save_card(&dir, "monolith-card.txt", b"two").unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), b"one");
        assert_eq!(std::fs::read(&second).unwrap(), b"two");
        assert!(second.ends_with("monolith-card-2.txt"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&first).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_card_goes_to_the_downloads_folder_chosen_by_the_thread_of_the_files() {
        let home = scratch("home");
        let save = |home: &Path| {
            run(Job::SaveCard {
                home: Some(home.to_owned()),
                name: "monolith-card.txt".to_owned(),
                contents: Zeroizing::new("card".to_owned()),
            })
        };
        let saved = |result: Option<EffectResult>| match result {
            Some(EffectResult::CardSaved { path }) => PathBuf::from(path),
            other => panic!("{other:?}"),
        };
        // Without a downloads folder, into the home folder.
        assert_eq!(saved(save(&home)).parent(), Some(home.as_path()));
        let downloads = home.join("Downloads");
        std::fs::create_dir(&downloads).unwrap();
        assert_eq!(saved(save(&home)).parent(), Some(downloads.as_path()));
        assert!(matches!(
            run(Job::SaveCard {
                home: None,
                name: String::new(),
                contents: Zeroizing::new(String::new()),
            }),
            Some(EffectResult::Failed(Failure::File))
        ));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_name_cannot_leave_the_folder() {
        assert_eq!(file_name("../../etc/passwd"), "etc-passwd");
        assert_eq!(file_name("..\\x.txt"), "x.txt");
        assert_eq!(file_name(".hidden"), "hidden");
        assert_eq!(file_name(""), "monolith-card.txt");
        assert_eq!(file_name("/"), "monolith-card.txt");
    }

    #[test]
    fn a_card_file_is_read_only_if_it_is_small_text() {
        let dir = scratch("read");
        let card = dir.join("card.txt");
        std::fs::write(&card, "MONOLITH card").unwrap();
        assert_eq!(read_card(&card).unwrap().as_str(), "MONOLITH card");
        let large = dir.join("large.txt");
        std::fs::write(&large, vec![b'a'; 64 * 1024 + 1]).unwrap();
        assert_eq!(read_card(&large), Err(Failure::FileTooLarge));
        let binary = dir.join("binary");
        std::fs::write(&binary, [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(read_card(&binary), Err(Failure::InvalidCard));
        assert_eq!(read_card(&dir), Err(Failure::File));
        assert_eq!(read_card(&dir.join("missing")), Err(Failure::File));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_path_of_another_computer_or_of_a_device_is_never_opened() {
        let dir = scratch("paths");
        let card = dir.join("card.txt");
        std::fs::write(&card, "MONOLITH card").unwrap();
        assert_eq!(read_card(&card).unwrap().as_str(), "MONOLITH card");
        // The same file named another way is refused before it is opened,
        // and so is anything Windows would look for on the network.
        let verbatim = PathBuf::from(format!(r"\\?\{}", card.display()));
        let forward = PathBuf::from(format!("//?/{}", card.display()));
        assert_eq!(read_card(&verbatim), Err(Failure::File));
        assert_eq!(read_card(&forward), Err(Failure::File));
        for path in [
            r"\\monolith.invalid\share\card.txt",
            "//monolith.invalid/share/card.txt",
            r"\/monolith.invalid\share\card.txt",
            r"\\.\NUL",
            r"\\.\pipe\monolith",
            r"\??\UNC\monolith.invalid\share\card.txt",
            r"\monolith\card.txt",
            "card.txt",
            "C:card.txt",
        ] {
            assert_eq!(read_card(Path::new(path)), Err(Failure::File), "{path}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn closing_waits_a_while_only_for_a_job_that_hangs() {
        fn hang(_: Job) -> Option<EffectResult> {
            std::thread::sleep(Duration::from_secs(30));
            None
        }
        let disk = Disk::start_with(Arc::new(|| {}), Duration::from_millis(200), hang).unwrap();
        let job = Job::SaveCard {
            home: None,
            name: String::new(),
            contents: Zeroizing::new(String::new()),
        };
        assert!(disk.ask(job).is_none());
        let started = Instant::now();
        drop(disk);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_pipe_is_opened_without_waiting_and_refused() {
        use rustix::fs::{CWD, Mode};
        let dir = scratch("pipe");
        let fifo = dir.join("card.txt");
        rustix::fs::mkfifoat(CWD, &fifo, Mode::RUSR | Mode::WUSR).unwrap();
        // Nobody writes to it: an open that waited for a writer would
        // never return, and the thread would hold the exit.
        let (sender, receiver) = mpsc::channel();
        let path = fifo.clone();
        std::thread::spawn(move || {
            let opened = open(&path).is_ok();
            let read = read_card(&path);
            let _ = sender.send((opened, read));
        });
        let (opened, read) = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(opened);
        assert_eq!(read, Err(Failure::File));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_device_is_never_read_as_a_card() {
        // It is not a file: it is refused before a byte is read, never
        // read up to the limit.
        assert_eq!(read_card(Path::new("/dev/zero")), Err(Failure::File));
        assert_eq!(read_card(Path::new("/dev/null")), Err(Failure::File));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_pipe_is_never_opened_by_its_name_or_through_a_link() {
        use core::mem::MaybeUninit;
        use rustix::fs::inotify::{self, CreateFlags, ReadFlags, WatchFlags};
        use rustix::fs::{CWD, Mode};
        let dir = scratch("unopened");
        let fifo = dir.join("pipe");
        rustix::fs::mkfifoat(CWD, &fifo, Mode::RUSR | Mode::WUSR).unwrap();
        let link = dir.join("card.txt");
        std::os::unix::fs::symlink(&fifo, &link).unwrap();
        // Every opening of the pipe is seen.
        let watch = inotify::init(CreateFlags::NONBLOCK | CreateFlags::CLOEXEC).unwrap();
        inotify::add_watch(&watch, &fifo, WatchFlags::OPEN).unwrap();
        let mut buffer = [MaybeUninit::uninit(); 1024];
        let mut events = inotify::Reader::new(&watch, &mut buffer);
        assert_eq!(read_card(&fifo), Err(Failure::File));
        assert_eq!(read_card(&link), Err(Failure::File));
        assert_eq!(
            events.next().err(),
            Some(rustix::io::Errno::AGAIN),
            "the pipe was opened"
        );
        // As it is when it is opened.
        drop(open(&fifo).unwrap());
        assert!(events.next().unwrap().events().contains(ReadFlags::OPEN));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_is_never_followed() {
        let dir = scratch("links");
        let card = dir.join("card.txt");
        std::fs::write(&card, "MONOLITH card").unwrap();
        let to_card = dir.join("to-card.txt");
        std::os::unix::fs::symlink(&card, &to_card).unwrap();
        let to_device = dir.join("to-device.txt");
        std::os::unix::fs::symlink("/dev/zero", &to_device).unwrap();
        assert_eq!(read_card(&card).unwrap().as_str(), "MONOLITH card");
        assert_eq!(read_card(&to_card), Err(Failure::File));
        assert_eq!(read_card(&to_device), Err(Failure::File));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_name_given_to_something_else_once_looked_at_is_not_read() {
        /// Reads `card` once it is looked at; `swap` gives its name to
        /// something else before it is opened.
        fn read(
            card: &Path,
            other: &Path,
            swap: fn(&Path, &Path),
        ) -> Result<Zeroizing<String>, Failure> {
            let _ = std::fs::remove_file(card);
            let _ = std::fs::remove_file(other);
            std::fs::write(card, "MONOLITH card").unwrap();
            read_card_with(card, |path| {
                swap(path, other);
                open(path)
            })
        }
        let dir = scratch("swapped");
        let card = dir.join("card.txt");
        let other = dir.join("other.txt");
        assert_eq!(
            read(&card, &other, |_, _| {}).unwrap().as_str(),
            "MONOLITH card"
        );
        // Another file.
        let file = read(&card, &other, |card, other| {
            std::fs::write(other, "MONOLITH other").unwrap();
            std::fs::rename(other, card).unwrap();
        });
        assert_eq!(file, Err(Failure::File));
        // A link to the file looked at, under another name.
        let link = read(&card, &other, |card, other| {
            std::fs::rename(card, other).unwrap();
            std::os::unix::fs::symlink(other, card).unwrap();
        });
        assert_eq!(link, Err(Failure::File));
        // A pipe: an open that waited for a writer would never return.
        let (sender, receiver) = mpsc::channel();
        let (at, beside) = (card.clone(), other.clone());
        std::thread::spawn(move || {
            let pipe = read(&at, &beside, |card, _| {
                use rustix::fs::{CWD, Mode};
                std::fs::remove_file(card).unwrap();
                rustix::fs::mkfifoat(CWD, card, Mode::RUSR | Mode::WUSR).unwrap();
            });
            let _ = sender.send(pipe);
        });
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(10)).unwrap(),
            Err(Failure::File)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Makes `link` a junction to the folder `target`, as `mklink /J` does,
    /// which needs no privilege.
    #[cfg(windows)]
    fn junction(link: &Path, target: &Path) {
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(made.status.success(), "{made:?}");
    }

    #[cfg(windows)]
    #[test]
    fn a_card_behind_a_junction_is_never_read() {
        // A junction on the way could lead anywhere, a share included: the
        // file behind it is refused, and the same file by its own path is
        // read.
        let dir = scratch("junction");
        let real = dir.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("card.txt"), "MONOLITH card").unwrap();
        let link = dir.join("link");
        junction(&link, &real);
        assert_eq!(
            read_card(&real.join("card.txt")).unwrap().as_str(),
            "MONOLITH card"
        );
        assert_eq!(read_card(&link.join("card.txt")), Err(Failure::File));
        // Nor is a card saved into it.
        assert!(save_card(&link, "monolith-card.txt", b"card").is_err());
        assert!(!real.join("monolith-card.txt").exists());
        assert!(save_card(&real, "monolith-card.txt", b"card").is_ok());
        let _ = std::fs::remove_dir(&link);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_card_linked_to_is_never_read_when_links_can_be_made() {
        let dir = scratch("symlink");
        let card = dir.join("card.txt");
        std::fs::write(&card, "MONOLITH card").unwrap();
        let link = dir.join("link.txt");
        // Making a symbolic link needs a privilege or the developer mode;
        // without either there is nothing to read through one.
        if std::os::windows::fs::symlink_file(&card, &link).is_err() {
            let _ = std::fs::remove_dir_all(&dir);
            return;
        }
        assert_eq!(read_card(&link), Err(Failure::File));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_card_that_is_not_on_the_disk_is_never_read() {
        // Offline: Windows would fetch it from remote storage.
        let dir = scratch("offline");
        let card = dir.join("card.txt");
        std::fs::write(&card, "MONOLITH card").unwrap();
        let marked = std::process::Command::new("attrib")
            .arg("+O")
            .arg(&card)
            .output()
            .unwrap();
        assert!(marked.status.success(), "{marked:?}");
        assert_eq!(read_card(&card), Err(Failure::File));
        let _ = std::process::Command::new("attrib")
            .arg("-O")
            .arg(&card)
            .output();
        assert_eq!(read_card(&card).unwrap().as_str(), "MONOLITH card");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_name_of_a_device_is_refused_before_anything_is_opened() {
        let dir = scratch("devices");
        for name in ["COM3", "NUL.txt", "conout$", "LPT1.card", "aux .txt"] {
            assert_eq!(read_card(&dir.join(name)), Err(Failure::File), "{name}");
        }
        for path in [r"C:\COM3", r"C:\NUL"] {
            assert_eq!(read_card(Path::new(path)), Err(Failure::File), "{path}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_name_given_to_another_file_once_looked_at_is_not_read_on_windows() {
        let dir = scratch("swapped-windows");
        let card = dir.join("card.txt");
        let other = dir.join("other.txt");
        std::fs::write(&card, "MONOLITH card").unwrap();
        assert_eq!(
            read_card_with(&card, open).unwrap().as_str(),
            "MONOLITH card"
        );
        let swapped = read_card_with(&card, |path| {
            std::thread::sleep(Duration::from_millis(20));
            std::fs::write(&other, "MONOLITH other card").unwrap();
            std::fs::remove_file(path).unwrap();
            std::fs::rename(&other, path).unwrap();
            open(path)
        });
        assert_eq!(swapped, Err(Failure::File));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

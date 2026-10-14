//! What the screens ask of the disk, done on a thread of its own so that
//! no frame waits: a card saved as a new file, a card file read, the
//! settings file written. Done in the order asked.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;

use core::time::Duration;

use monolith_ui::EffectResult;
use monolith_ui_model::{Failure, UiIdentityId};
use zeroize::Zeroizing;

use crate::bridge::Wake;
use crate::config::{self, Config};

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
        /// The folder.
        dir: Option<PathBuf>,
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
    thread: Option<std::thread::JoinHandle<()>>,
    close_wait: Duration,
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
            thread: Some(thread),
            close_wait,
        })
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
    /// written, and waits for them a while at most: a job that hangs does
    /// not hold the exit.
    fn drop(&mut self) {
        self.jobs = None;
        let Some(thread) = self.thread.take() else {
            return;
        };
        let deadline = Instant::now().checked_add(self.close_wait);
        while !thread.is_finished() && deadline.is_some_and(|deadline| Instant::now() < deadline) {
            std::thread::sleep(Duration::from_millis(10));
        }
        if thread.is_finished() {
            let _ = thread.join();
        }
    }
}

fn run(job: Job) -> Option<EffectResult> {
    match job {
        Job::SaveCard {
            dir,
            name,
            contents,
        } => Some(match dir {
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
/// number instead.
pub fn save_card(dir: &Path, name: &str, contents: &[u8]) -> std::io::Result<PathBuf> {
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
pub fn read_card(path: &Path) -> Result<Zeroizing<String>, Failure> {
    if !local(path) {
        return Err(Failure::File);
    }
    // Opened first, then checked: what is checked is what is read, even
    // if the name is given to something else in between. A device or a
    // pipe could block or never end; only files are read.
    let file = open(path).map_err(|_| Failure::File)?;
    let metadata = file.metadata().map_err(|_| Failure::File)?;
    if !metadata.is_file() {
        return Err(Failure::File);
    }
    if metadata.len() > MAX_CARD_FILE {
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

/// Whether `path` names a file of this computer. On Windows that is an
/// absolute path on a drive (`C:\...`): a path that starts with two
/// separators (`\\server\share`, `//server/share`, `\\?\`, `\\.\`), an NT
/// path or a path relative to a current directory, which can itself be a
/// share, could make Windows reach another computer over SMB, outside
/// Tor, or open a device.
#[cfg(windows)]
fn local(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    let on_a_drive = matches!(
        path.components().next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))
    );
    on_a_drive && path.is_absolute()
}

#[cfg(not(windows))]
fn local(_: &Path) -> bool {
    true
}

/// Opens `path` to read it. On Unix it is opened without waiting, so that
/// a pipe without a writer does not block the thread.
#[cfg(unix)]
fn open(path: &Path) -> std::io::Result<std::fs::File> {
    use rustix::fs::{Mode, OFlags};
    let flags = OFlags::RDONLY | OFlags::NONBLOCK | OFlags::NOCTTY | OFlags::CLOEXEC;
    Ok(std::fs::File::from(rustix::fs::open(
        path,
        flags,
        Mode::empty(),
    )?))
}

#[cfg(not(unix))]
fn open(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::Arc;

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
            dir: None,
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
}

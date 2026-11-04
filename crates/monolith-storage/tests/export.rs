//! A received file saved where the user chose
//! (`docs/adr/0014-transfer-staging.md`, "Saving a received file"): whole
//! under its name or not at all, never over an existing file, never
//! outside the folder chosen, never in a place that is not one of this
//! computer.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::cast_possible_truncation
)]

use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use monolith_protocol::text::Filename;
use monolith_storage::export::{ExportError, PARTIAL_PREFIX, Saved, save_new};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("monolith-export-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // The path as Windows resolves it has no short names in it.
    std::fs::canonicalize(&dir)
        .map(|resolved| {
            let text = resolved.to_string_lossy().into_owned();
            text.strip_prefix(r"\\?\")
                .map_or(resolved.clone(), PathBuf::from)
        })
        .unwrap()
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 13 % 251) as u8).collect()
}

/// A source that gives its bytes and then an error in place of its end, as
/// a reader of a staged file that does not match its digest does.
struct Failing {
    data: Cursor<Vec<u8>>,
}

impl Read for Failing {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.data.read(buf)? {
            0 => Err(std::io::Error::other("not what was received")),
            read => Ok(read),
        }
    }
}

#[test]
fn a_file_is_saved_whole_under_its_name_and_nothing_else_is_left() {
    let dir = scratch("whole");
    let bytes = data(300_000);
    let saved = save_new(&dir.join("report.pdf"), &mut Cursor::new(bytes.clone())).unwrap();
    assert_eq!(saved, Saved::Confirmed);
    assert_eq!(std::fs::read(dir.join("report.pdf")).unwrap(), bytes);
    // No temporary name and no probe left.
    assert_eq!(names(&dir), vec!["report.pdf"]);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join("report.pdf"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // A file of no bytes too.
    let saved = save_new(&dir.join("empty"), &mut Cursor::new(Vec::new())).unwrap();
    assert_eq!(saved, Saved::Confirmed);
    assert_eq!(std::fs::read(dir.join("empty")).unwrap(), Vec::<u8>::new());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A source that must not be read.
#[cfg(windows)]
struct Unread;

#[cfg(windows)]
impl Read for Unread {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        panic!("the source was read");
    }
}

/// A name with a character Windows refuses is refused before anything is
/// read or written, not with a failure of the disk once the whole file is.
#[cfg(windows)]
#[test]
fn a_name_windows_refuses_is_refused_before_anything_is_written() {
    let dir = scratch("refused-name");
    for name in [
        "a*b.txt",
        "what?.pdf",
        "x<y",
        "x>y",
        "pipe|d",
        "quote\"d",
        "tab\u{9}bed",
    ] {
        assert_eq!(
            save_new(&dir.join(name), &mut Unread),
            Err(ExportError::Refused),
            "{name:?}"
        );
    }
    assert!(names(&dir).is_empty(), "{:?}", names(&dir));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_existing_name_is_never_replaced() {
    let dir = scratch("exists");
    std::fs::write(dir.join("notes.txt"), "mine").unwrap();
    std::fs::create_dir(dir.join("folder")).unwrap();
    for name in ["notes.txt", "folder"] {
        assert_eq!(
            save_new(&dir.join(name), &mut Cursor::new(data(10))),
            Err(ExportError::Exists),
            "{name}"
        );
    }
    assert_eq!(std::fs::read(dir.join("notes.txt")).unwrap(), b"mine");
    #[cfg(unix)]
    {
        // A link, even one that leads nowhere, is something with the name.
        std::os::unix::fs::symlink(dir.join("nowhere"), dir.join("link")).unwrap();
        assert_eq!(
            save_new(&dir.join("link"), &mut Cursor::new(data(10))),
            Err(ExportError::Exists)
        );
        assert!(!dir.join("nowhere").exists());
    }
    assert!(
        names(&dir)
            .iter()
            .all(|name| !name.starts_with(PARTIAL_PREFIX))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_name_taken_while_the_file_is_written_is_not_replaced() {
    let dir = scratch("taken");
    let destination = dir.join("photo.jpg");
    /// Gives its bytes; on the first read another program takes the name.
    struct Racing {
        data: Cursor<Vec<u8>>,
        destination: PathBuf,
        raced: bool,
    }
    impl Read for Racing {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if !self.raced {
                self.raced = true;
                std::fs::write(&self.destination, "theirs").unwrap();
            }
            self.data.read(buf)
        }
    }
    let mut source = Racing {
        data: Cursor::new(data(200_000)),
        destination: destination.clone(),
        raced: false,
    };
    assert_eq!(
        save_new(&destination, &mut source),
        Err(ExportError::Exists)
    );
    assert_eq!(std::fs::read(&destination).unwrap(), b"theirs");
    assert_eq!(names(&dir), vec!["photo.jpg"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_source_that_fails_midway_or_at_its_end_leaves_nothing() {
    let dir = scratch("failing");
    for len in [0, 100, 200_000] {
        let mut source = Failing {
            data: Cursor::new(data(len)),
        };
        assert_eq!(
            save_new(&dir.join("broken.bin"), &mut source),
            Err(ExportError::Source),
            "{len}"
        );
        assert!(names(&dir).is_empty(), "{len}: {:?}", names(&dir));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_place_that_is_not_a_folder_of_this_computer_is_refused_before_anything_is_written() {
    let dir = scratch("refused");
    std::fs::write(dir.join("file"), "a file").unwrap();
    let refused = [
        // Not absolute.
        PathBuf::from("relative.txt"),
        // A folder that does not exist, or is a file.
        dir.join("missing").join("x.txt"),
        dir.join("file").join("x.txt"),
        // No name.
        dir.join(".."),
    ];
    for path in &refused {
        assert_eq!(
            save_new(path, &mut Cursor::new(data(10))),
            Err(ExportError::Refused),
            "{path:?}"
        );
    }
    assert!(!dir.join("missing").exists());
    assert_eq!(names(&dir), vec!["file"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(windows)]
#[test]
fn a_share_a_device_a_stream_or_a_junction_is_refused_on_windows() {
    let dir = scratch("windows");
    let real = dir.join("real");
    std::fs::create_dir(&real).unwrap();
    let junction = dir.join("junction");
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(&real)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    let refused = [
        PathBuf::from(r"\\server\share\x.txt"),
        PathBuf::from(r"\\?\C:\x.txt"),
        dir.join("NUL.txt"),
        dir.join("com3"),
        dir.join("a.txt:stream"),
        dir.join("trailing."),
        dir.join("trailing "),
        junction.join("x.txt"),
    ];
    for path in &refused {
        assert_eq!(
            save_new(path, &mut Cursor::new(data(10))),
            Err(ExportError::Refused),
            "{path:?}"
        );
    }
    assert!(names(&real).is_empty());
    // The same folder by its own path takes the file.
    assert_eq!(
        save_new(&real.join("x.txt"), &mut Cursor::new(data(10))),
        Ok(Saved::Confirmed)
    );
    let _ = std::fs::remove_dir(&junction);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_name_a_peer_suggests_saves_in_the_folder_chosen_and_nowhere_else() {
    let dir = scratch("hostile");
    let hostile = [
        "CON",
        "nul.txt",
        "..",
        "...",
        " report.pdf ",
        ".hidden",
        "a:b",
        "a<b>c|d?e*f\"g",
        "photo\u{202e}gpj.exe",
        "COM1.tar.gz",
        "trailing. ",
    ];
    for (n, raw) in hostile.iter().enumerate() {
        let Ok(name) = Filename::new(raw) else {
            continue;
        };
        let suggested = name.save_name();
        assert!(!suggested.contains(['/', '\\']), "{raw:?}: {suggested:?}");
        let folder = dir.join(n.to_string());
        std::fs::create_dir(&folder).unwrap();
        let destination = folder.join(&suggested);
        assert_eq!(destination.parent(), Some(folder.as_path()), "{raw:?}");
        assert_eq!(
            save_new(&destination, &mut Cursor::new(data(10))),
            Ok(Saved::Confirmed),
            "{raw:?}: {suggested:?}"
        );
        assert_eq!(names(&folder), vec![suggested.clone()], "{raw:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

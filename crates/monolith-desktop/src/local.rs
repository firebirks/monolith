//! Whether a path names a file of this computer (`DESIGN_QUESTIONS.md`
//! P7-3).
//!
//! On Windows a path can do more than name a file. A UNC path, a drive
//! letter mapped to a share, or a link or a junction on the way can make
//! Windows reach another computer over SMB, outside Tor. A name Windows
//! reserves (`CON`, `NUL`, `COM3`, in any folder and with any extension)
//! opens a device. A file that a sync client keeps only in the cloud is
//! fetched when it is opened or read. What the client reads or writes by a
//! path the user gave (a card file, the folder a card is saved in, the
//! cookie of Tor, the data directory and the settings file) is checked
//! here first:
//!
//! - by its name alone, with no call to the system: an absolute path on a
//!   drive, every part of it a plain name, none a name Windows reserves,
//!   none with a colon (a stream of a file) or ending in a dot or a space
//!   (which Windows strips, so that two names name one file);
//! - then on the disk, from the root down: no folder on the way and not
//!   the file itself is a link or a junction, which would send the rest of
//!   the path elsewhere, and none is offline or kept in the cloud only;
//! - and then where it ends: the path Windows resolves for it is on a
//!   drive, not on a share (`\\?\UNC\`), which is what a drive mapped to a
//!   share resolves to.
//!
//! Each step looks at a part only once the parts above it passed, so that
//! nothing is looked at through a link. The last step opens the file, as
//! the standard library does to resolve a path; for a drive mapped to a
//! share, Windows does that on the share the user mapped, over the
//! connection the mapping made, and the file is then refused unread.
//! Without code that is not safe Rust, Monolith cannot ask Windows what a
//! drive is before it opens a file on it. Nor can it hold a folder while it
//! opens what is in it: a process of the user that replaces a folder on
//! the way between the checks and the open is not defended against; it can
//! read the user's files anyway.
//!
//! On Unix nothing here applies, and every path passes: a card file is
//! opened without following a link (`crate::files`), and a file on a
//! network file system mounted on the computer is read as any other.

use std::path::Path;

/// The names Windows reserves for devices, compared without regard to
/// case.
#[cfg(any(windows, test))]
const DEVICES: [&str; 8] = [
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "CLOCK$", "KEYBD$",
];

/// The attributes of a file Windows does not hold on the disk: offline
/// (`FILE_ATTRIBUTE_OFFLINE`), fetched when opened
/// (`FILE_ATTRIBUTE_RECALL_ON_OPEN`) or when read
/// (`FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`).
#[cfg(any(windows, test))]
const NOT_HERE: u32 = 0x0000_1000 | 0x0004_0000 | 0x0040_0000;

/// Whether `name`, a part of a path, is a name Windows reserves for a
/// device: one of [`DEVICES`], or `COM` or `LPT` with a digit (or a
/// superscript one, two or three), whatever extension follows it and
/// whatever spaces end it.
#[cfg(any(windows, test))]
pub(crate) fn names_a_device(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or("").trim_end_matches(' ');
    let upper = stem.to_ascii_uppercase();
    if DEVICES.contains(&upper.as_str()) {
        return true;
    }
    let mut chars = upper.chars();
    let three: String = chars.by_ref().take(3).collect();
    let rest: String = chars.collect();
    (three == "COM" || three == "LPT")
        && matches!(
            rest.as_str(),
            "0" | "1"
                | "2"
                | "3"
                | "4"
                | "5"
                | "6"
                | "7"
                | "8"
                | "9"
                | "\u{b9}"
                | "\u{b2}"
                | "\u{b3}"
        )
}

/// Whether the attributes `attributes` are those of a file Windows holds
/// on the disk.
#[cfg(any(windows, test))]
pub(crate) fn held_here(attributes: u32) -> bool {
    attributes & NOT_HERE == 0
}

/// Whether `canonical`, a path as Windows resolves it
/// (`std::fs::canonicalize`), is on a drive and not on a share.
#[cfg(any(windows, test))]
pub(crate) fn resolved_on_a_drive(canonical: &Path) -> bool {
    let text = canonical.as_os_str().to_string_lossy();
    let Some(rest) = text.strip_prefix(r"\\?\") else {
        return false;
    };
    let mut chars = rest.chars();
    matches!(
        (chars.next(), chars.next(), chars.next()),
        (Some(letter), Some(':'), Some('\\')) if letter.is_ascii_alphabetic()
    )
}

/// Whether `path` names a file or folder of this computer by its name
/// alone, with no call to the system. On Unix, every path.
pub(crate) fn named_here(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        let mut components = path.components();
        let on_a_drive = matches!(
            components.next(),
            Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_))
        );
        if !on_a_drive || !path.is_absolute() {
            return false;
        }
        components.all(|component| match component {
            Component::RootDir => true,
            Component::Normal(name) => name.to_str().is_some_and(|name| {
                !name.is_empty()
                    && !name.contains(':')
                    && !name.ends_with(['.', ' '])
                    && !names_a_device(name)
            }),
            Component::Prefix(_) | Component::CurDir | Component::ParentDir => false,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        true
    }
}

/// Whether the folders on the way to `path`, from the root down, and the
/// last part of `path` itself if `last` and it exists, are held on the disk
/// and none is a link or a junction. Each part is looked at only once
/// those above it passed. A part that does not exist ends the walk with
/// true: there is nothing further to follow.
#[cfg(windows)]
fn way_held_here(path: &Path, last: bool) -> bool {
    use std::os::windows::fs::MetadataExt;
    let parts: Vec<&Path> = path.ancestors().collect();
    let skip = usize::from(!last);
    for part in parts.iter().rev().take(parts.len().saturating_sub(skip)) {
        let looked = match std::fs::symlink_metadata(part) {
            Ok(looked) => looked,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
            Err(_) => return false,
        };
        if looked.file_type().is_symlink() || !held_here(looked.file_attributes()) {
            return false;
        }
    }
    true
}

/// Whether `path`, an existing file or folder, is one of this computer:
/// [`named_here`], every part of the way held on the disk and none a link,
/// and resolved on a drive. On Unix, every path.
pub(crate) fn here(path: &Path) -> bool {
    #[cfg(windows)]
    {
        named_here(path)
            && way_held_here(path, true)
            && std::fs::canonicalize(path).is_ok_and(|canonical| resolved_on_a_drive(&canonical))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        true
    }
}

/// Whether `path`, which may not exist yet, would be one of this computer:
/// [`named_here`], and the part of its way that exists is held on the disk,
/// none of it a link, and resolved on a drive. For the data directory and
/// the settings file, which the client may make. On Unix, every path.
pub(crate) fn would_be_here(path: &Path) -> bool {
    #[cfg(windows)]
    {
        if !named_here(path) || !way_held_here(path, true) {
            return false;
        }
        let Some(existing) = path.ancestors().find(|part| part.exists()) else {
            return false;
        };
        std::fs::canonicalize(existing).is_ok_and(|canonical| resolved_on_a_drive(&canonical))
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        true
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn the_names_windows_keeps_for_devices_are_known_in_every_form() {
        for name in [
            "CON",
            "con",
            "Nul.txt",
            "NUL.tar.gz",
            "aux ",
            "COM1",
            "com9.log",
            "LPT3",
            "COM\u{b9}",
            "lpt\u{b2}.txt",
            "CONOUT$",
            "conin$.txt",
            "CLOCK$",
            "COM0",
        ] {
            assert!(names_a_device(name), "{name:?}");
        }
        for name in [
            "card.txt",
            "console.txt",
            "COM10",
            "COMX",
            "LPT",
            "NULL",
            "connect",
            "monolith-card.txt",
            "",
        ] {
            assert!(!names_a_device(name), "{name:?}");
        }
    }

    #[test]
    fn a_file_is_held_here_unless_it_is_offline_or_in_the_cloud_only() {
        assert!(held_here(0x20)); // archive
        assert!(held_here(0x400)); // a reparse point of a sync client, on the disk
        assert!(!held_here(0x1000)); // offline
        assert!(!held_here(0x4_0000)); // fetched when opened
        assert!(!held_here(0x40_0000)); // fetched when read
        assert!(!held_here(0x40_0400));
    }

    #[test]
    fn a_path_resolved_on_a_share_is_not_on_a_drive() {
        for path in [
            r"\\?\C:\Users\someone\card.txt",
            r"\\?\d:\card.txt",
            r"\\?\Z:\",
        ] {
            assert!(resolved_on_a_drive(&PathBuf::from(path)), "{path}");
        }
        for path in [
            r"\\?\UNC\server\share\card.txt",
            r"\\server\share\card.txt",
            r"C:\card.txt",
            r"\\?\Volume{0c2f}\card.txt",
            r"\\?\GLOBALROOT\Device\Mup\server\share",
            "",
        ] {
            assert!(!resolved_on_a_drive(&PathBuf::from(path)), "{path}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn only_a_plain_path_on_a_drive_is_named_here() {
        for path in [
            r"C:\Users\someone\Downloads\card.txt",
            r"D:\card.txt",
            r"C:\",
        ] {
            assert!(named_here(Path::new(path)), "{path}");
        }
        for path in [
            r"\\server\share\card.txt",
            "//server/share/card.txt",
            r"\\?\C:\card.txt",
            r"\\.\COM3",
            r"\??\C:\card.txt",
            r"C:card.txt",
            "card.txt",
            r"\card.txt",
            r"C:\COM3",
            r"C:\x\NUL.txt",
            r"C:\x\conout$",
            r"C:\x\com1 .txt",
            r"C:\x\card.txt:stream",
            r"C:\x\card.txt.",
            r"C:\x\card.txt ",
            r"C:\x\..\card.txt",
        ] {
            assert!(!named_here(Path::new(path)), "{path}");
        }
    }
}

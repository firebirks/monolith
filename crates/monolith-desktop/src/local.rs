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
//! - then its drive: the path Windows resolves for the root of the drive
//!   is on a drive, not on a share (`\\?\UNC\`), which is what a drive
//!   mapped to a share resolves to;
//! - then on the disk, from the root down: no folder on the way and not
//!   the file itself is a link or a junction, which would send the rest of
//!   the path elsewhere, and none is offline or kept in the cloud only;
//! - and then where it ends: the path Windows resolves for it is on a
//!   drive too.
//!
//! Each step looks at a part only once the parts above it passed, so that
//! nothing is looked at through a link. To resolve the root of a drive the
//! standard library opens it; for a drive mapped to a share, Windows does
//! that once, on the share the user mapped, and nothing below the root is
//! looked at. If the mapping was disconnected, that open makes Windows
//! connect to the share again, outside Tor (since Phase 7, P7-23; before,
//! each folder on the way was looked at first). Without code that is not
//! safe Rust, Monolith cannot ask Windows what a drive is without opening
//! it. Nor can it hold a folder while it opens what is in it: a process of
//! the user that replaces a folder on the way between the checks and the
//! open is not defended against; it can read the user's files anyway.
//!
//! A drive that a driver backs with storage elsewhere but presents as a
//! disk (a cloud drive in streaming mode, a file system in user space, a
//! disk image or iSCSI volume) resolves on a drive and passes; so does a
//! folder a sync client keeps, whose files are read from the disk but
//! uploaded when they change. Nothing in safe Rust tells them apart.
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

/// What a look at one part of a path found, without following a link.
#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug)]
struct Looked {
    /// A link, a junction or a mount point.
    link: bool,
    /// Its attributes.
    attributes: u32,
}

/// The calls to the system the checks make, kept apart so that a test can
/// see which are made and in what order.
#[cfg(any(windows, test))]
trait System {
    /// Looks at `part` without following it.
    fn look(&mut self, part: &Path) -> std::io::Result<Looked>;
    /// The path Windows resolves for `path` (`std::fs::canonicalize`).
    fn resolve(&mut self, path: &Path) -> std::io::Result<std::path::PathBuf>;
}

/// The disk.
#[cfg(windows)]
struct Disk;

#[cfg(windows)]
impl System for Disk {
    fn look(&mut self, part: &Path) -> std::io::Result<Looked> {
        use std::os::windows::fs::MetadataExt;
        let looked = std::fs::symlink_metadata(part)?;
        Ok(Looked {
            link: looked.file_type().is_symlink(),
            attributes: looked.file_attributes(),
        })
    }

    fn resolve(&mut self, path: &Path) -> std::io::Result<std::path::PathBuf> {
        std::fs::canonicalize(path)
    }
}

/// The checks on the disk, for a path [`named_here`] passed. The root
/// first: a drive mapped to a share resolves to the share, and is refused
/// there, before anything on it is looked at. Then the parts from the root
/// down: none a link and none offline or in the cloud only, each looked at
/// only once those above it passed; a part that does not exist ends the
/// walk. Then the deepest part that exists resolves on a drive. With
/// `whole`, `path` itself has to exist.
#[cfg(any(windows, test))]
fn on_this_computer(path: &Path, whole: bool, system: &mut impl System) -> bool {
    let Some(root) = path.ancestors().last() else {
        return false;
    };
    if !system
        .resolve(root)
        .is_ok_and(|canonical| resolved_on_a_drive(&canonical))
    {
        return false;
    }
    let parts: Vec<&Path> = path.ancestors().collect();
    let mut deepest = None;
    for part in parts.into_iter().rev().skip(1) {
        match system.look(part) {
            Ok(looked) if !looked.link && held_here(looked.attributes) => deepest = Some(part),
            Ok(_) => return false,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return false,
        }
    }
    match deepest {
        Some(part) => {
            (!whole || part == path)
                && system
                    .resolve(part)
                    .is_ok_and(|canonical| resolved_on_a_drive(&canonical))
        }
        None => !whole || root == path,
    }
}

/// Whether `path`, an existing file or folder, is one of this computer:
/// [`named_here`], its drive resolved on a drive, every part of the way
/// held on the disk and none a link, and resolved on a drive. On Unix,
/// every path.
pub(crate) fn here(path: &Path) -> bool {
    #[cfg(windows)]
    {
        named_here(path) && on_this_computer(path, true, &mut Disk)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        true
    }
}

/// Whether `path`, which may not exist yet, would be one of this computer:
/// [`named_here`], its drive resolved on a drive, and the part of its way
/// that exists held on the disk, none of it a link, and resolved on a
/// drive. For the data directory and the settings file, which the client
/// may make. On Unix, every path.
pub(crate) fn would_be_here(path: &Path) -> bool {
    #[cfg(windows)]
    {
        named_here(path) && on_this_computer(path, false, &mut Disk)
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

    /// A disk that answers from a table and records every call.
    struct Fake {
        /// Parts that resolve on a share; every other one on a drive.
        on_a_share: Vec<PathBuf>,
        /// Parts that are links.
        links: Vec<PathBuf>,
        /// Parts that exist.
        existing: Vec<PathBuf>,
        calls: Vec<String>,
    }

    impl System for Fake {
        fn look(&mut self, part: &Path) -> std::io::Result<Looked> {
            self.calls.push(format!("look {}", part.display()));
            if !self.existing.iter().any(|known| known == part) {
                return Err(std::io::ErrorKind::NotFound.into());
            }
            Ok(Looked {
                link: self.links.iter().any(|link| link == part),
                attributes: 0x20,
            })
        }

        fn resolve(&mut self, path: &Path) -> std::io::Result<PathBuf> {
            self.calls.push(format!("resolve {}", path.display()));
            Ok(if self.on_a_share.iter().any(|share| share == path) {
                PathBuf::from(r"\\?\UNC\nas\share")
            } else {
                PathBuf::from(r"\\?\C:\")
            })
        }
    }

    /// The parts of a path from the root down, written for this platform.
    fn parts() -> [PathBuf; 4] {
        if cfg!(windows) {
            [r"Z:\", r"Z:\cards", r"Z:\cards\in", r"Z:\cards\in\card.txt"].map(PathBuf::from)
        } else {
            ["/", "/cards", "/cards/in", "/cards/in/card.txt"].map(PathBuf::from)
        }
    }

    fn fake() -> Fake {
        Fake {
            on_a_share: Vec::new(),
            links: Vec::new(),
            existing: parts().to_vec(),
            calls: Vec::new(),
        }
    }

    #[test]
    fn a_drive_on_a_share_is_refused_before_anything_on_it_is_looked_at() {
        let [root, _, _, file] = parts();
        let mut system = Fake {
            on_a_share: vec![root.clone()],
            ..fake()
        };
        assert!(!on_this_computer(&file, true, &mut system));
        assert!(!on_this_computer(&file, false, &mut system));
        let resolved = format!("resolve {}", root.display());
        assert_eq!(system.calls, vec![resolved.clone(), resolved]);
    }

    #[test]
    fn nothing_below_a_link_is_looked_at() {
        let [root, cards, inner, file] = parts();
        let mut system = Fake {
            links: vec![cards.clone()],
            ..fake()
        };
        assert!(!on_this_computer(&file, true, &mut system));
        assert_eq!(
            system.calls,
            vec![
                format!("resolve {}", root.display()),
                format!("look {}", cards.display()),
            ]
        );
        let _ = inner;
    }

    #[test]
    fn a_path_of_a_drive_is_looked_at_from_the_root_down_and_resolved_where_it_ends() {
        let [root, cards, inner, file] = parts();
        let mut system = fake();
        assert!(on_this_computer(&file, true, &mut system));
        assert_eq!(
            system.calls,
            vec![
                format!("resolve {}", root.display()),
                format!("look {}", cards.display()),
                format!("look {}", inner.display()),
                format!("look {}", file.display()),
                format!("resolve {}", file.display()),
            ]
        );
        // A file that does not exist yet would be here; it is not here.
        let mut system = Fake {
            existing: vec![root.clone(), cards.clone()],
            ..fake()
        };
        assert!(!on_this_computer(&file, true, &mut system));
        assert!(on_this_computer(&file, false, &mut system));
        assert_eq!(
            system.calls.last(),
            Some(&format!("resolve {}", cards.display()))
        );
        // A deeper part resolved on a share is refused there.
        let mut system = Fake {
            on_a_share: vec![file.clone()],
            ..fake()
        };
        assert!(!on_this_computer(&file, true, &mut system));
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

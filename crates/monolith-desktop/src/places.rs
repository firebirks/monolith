//! Where Monolith keeps its files, and the Tor it expects, on each
//! platform. Only absolute paths from the environment are taken.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use monolith_ui::{TorPreset, TorSettings};

fn absolute(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

fn home() -> Option<PathBuf> {
    #[cfg(windows)]
    return absolute("USERPROFILE");
    #[cfg(not(windows))]
    return absolute("HOME");
}

/// The Persistent Storage of Tails, where Tails mounts it in the home
/// folder (`docs/PLATFORM_TAILS.md` section 5).
const TAILS_PERSISTENT: &str = "/home/amnesia/Persistent";

/// Whether this system is Tails: its os-release says so, or the file Tails
/// keeps its version in exists.
pub fn is_tails() -> bool {
    cfg!(target_os = "linux")
        && (std::fs::read_to_string("/etc/os-release").is_ok_and(|text| os_release_is_tails(&text))
            || Path::new("/etc/amnesia/version").exists())
}

fn os_release_is_tails(text: &str) -> bool {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("ID="))
        .any(|id| id.trim_matches('"') == "tails")
}

/// The marker files of Whonix, of the Workstation and of Whonix in general
/// (`docs/PLATFORM_WHONIX.md` section 8).
const WHONIX_MARKERS: [&str; 2] = [
    "/usr/share/anon-ws-base-files/workstation",
    "/usr/share/whonix/marker",
];

/// Whether this system is Whonix: one of its marker files exists.
pub fn is_whonix() -> bool {
    cfg!(target_os = "linux")
        && WHONIX_MARKERS
            .iter()
            .any(|marker| Path::new(marker).exists())
}

/// Whether Tor is the platform's own, behind an onion-grater filter: on
/// Tails and on Whonix.
pub fn filtered_tor() -> bool {
    is_tails() || is_whonix()
}

/// A mount point as the mount table writes it: space, tab, newline and
/// backslash as octal escapes.
fn unescape(field: &str) -> String {
    field
        .replace(r"\040", " ")
        .replace(r"\011", "\t")
        .replace(r"\012", "\n")
        .replace(r"\134", "\\")
}

/// Whether the mount table `table` (the format of `/proc/self/mountinfo`)
/// has a file system mounted at `path`.
fn mount_table_has(table: &str, path: &Path) -> bool {
    let Some(path) = path.to_str() else {
        return false;
    };
    table
        .lines()
        .filter_map(|line| line.split(' ').nth(4))
        .any(|point| unescape(point) == path)
}

/// Whether the Persistent Storage of Tails is unlocked.
fn tails_persistent_unlocked() -> bool {
    std::fs::read_to_string("/proc/self/mountinfo")
        .is_ok_and(|table| mount_table_has(&table, Path::new(TAILS_PERSISTENT)))
}

/// Whether `path` lies inside the Persistent Storage of Tails while it is
/// unlocked: nothing else survives a shutdown of Tails. The path is judged
/// by the folders that exist on it, links followed.
pub fn in_tails_persistent(path: &Path) -> bool {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        || !tails_persistent_unlocked()
    {
        return false;
    }
    let Ok(persistent) = Path::new(TAILS_PERSISTENT).canonicalize() else {
        return false;
    };
    let existing = path.ancestors().find(|ancestor| ancestor.exists());
    existing
        .and_then(|ancestor| ancestor.canonicalize().ok())
        .is_some_and(|resolved| resolved.starts_with(&persistent))
}

/// The data directory: the vault and the message store. On Tails, only
/// in the Persistent Storage while it is unlocked; otherwise there is none,
/// and only a session that keeps nothing can start.
pub fn data_dir() -> Option<PathBuf> {
    if is_tails() {
        let dir = Path::new(TAILS_PERSISTENT).join("monolith");
        return in_tails_persistent(&dir).then_some(dir);
    }
    #[cfg(windows)]
    return absolute("LOCALAPPDATA").map(|base| base.join("Monolith").join("Profile"));
    #[cfg(not(windows))]
    return absolute("XDG_DATA_HOME")
        .or_else(|| home().map(|home| home.join(".local").join("share")))
        .map(|base| base.join("monolith"));
}

/// The settings file of the desktop client. On Tails, only in the
/// Persistent Storage while it is unlocked; otherwise there is none, and
/// the settings last as long as the session.
pub fn config_file() -> Option<PathBuf> {
    if is_tails() {
        let file = Path::new(TAILS_PERSISTENT).join("monolith-desktop.conf");
        return in_tails_persistent(&file).then_some(file);
    }
    #[cfg(windows)]
    let base = absolute("LOCALAPPDATA").map(|base| base.join("Monolith"));
    #[cfg(not(windows))]
    let base = absolute("XDG_CONFIG_HOME")
        .or_else(|| home().map(|home| home.join(".config")))
        .map(|base| base.join("monolith"));
    base.map(|base| base.join("desktop.conf"))
}

/// The home folder, from the environment alone.
pub fn home_dir() -> Option<PathBuf> {
    home()
}

/// Where a saved card goes: the downloads folder of `home`, or `home`.
/// It looks at the disk, so it runs on the thread of the files. A home
/// that is not named as a folder of this computer is not looked into, and
/// on Windows a downloads folder that is a link or a junction is not
/// followed (`crate::local`); the card is then saved in `home`, which
/// `files::save_card` checks.
pub fn save_dir(home: &Path) -> PathBuf {
    let downloads = home.join("Downloads");
    let look = |path: &Path| {
        if cfg!(windows) {
            std::fs::symlink_metadata(path)
        } else {
            std::fs::metadata(path)
        }
    };
    if crate::local::named_here(home) && look(&downloads).is_ok_and(|looked| looked.is_dir()) {
        downloads
    } else {
        home.to_owned()
    }
}

fn settings(socks: &str, control: &str, cookie_file: OsString) -> TorSettings {
    TorSettings {
        socks: socks.to_owned(),
        control: control.to_owned(),
        cookie_file: cookie_file.to_string_lossy().into_owned(),
    }
}

/// The folder Tor Browser's installer offers on Windows, the desktop of
/// the home folder `home`. OneDrive's backup of folders moves the desktop
/// into the OneDrive folder; that one is taken when Tor Browser is there.
#[cfg(any(windows, test))]
fn windows_tor_browser(home: &Path) -> PathBuf {
    let onedrive = home.join("OneDrive").join("Desktop").join("Tor Browser");
    if onedrive.is_dir() {
        onedrive
    } else {
        home.join("Desktop").join("Tor Browser")
    }
}

/// The Tor of Tor Browser, as it is installed by default.
fn tor_browser() -> TorSettings {
    // Tor Browser installs into a folder of the user's choice; on Windows
    // it offers the desktop, on Linux it is unpacked where the user wants.
    #[cfg(windows)]
    let folder = home().map(|home| windows_tor_browser(&home));
    #[cfg(not(windows))]
    let folder = home().map(|home| home.join("tor-browser"));
    let cookie = folder
        .map(|folder| {
            folder
                .join("Browser")
                .join("TorBrowser")
                .join("Data")
                .join("Tor")
                .join("control_auth_cookie")
                .into_os_string()
        })
        .unwrap_or_default();
    settings("127.0.0.1:9150", "127.0.0.1:9151", cookie)
}

/// The Tor installed as a system service (Debian's `tor` package).
#[cfg(not(windows))]
fn system_tor() -> TorSettings {
    settings(
        "127.0.0.1:9050",
        "unix:/run/tor/control",
        "/run/tor/control.authcookie".into(),
    )
}

/// The Tor of the Tor Expert Bundle, run as the user with its defaults
/// and `CookieAuthentication 1`: the cookie is in its data directory.
#[cfg(windows)]
fn expert_bundle() -> TorSettings {
    let cookie = absolute("APPDATA")
        .map(|base| {
            base.join("tor")
                .join("control_auth_cookie")
                .into_os_string()
        })
        .unwrap_or_default();
    settings("127.0.0.1:9050", "127.0.0.1:9051", cookie)
}

/// The settings the user can pick from, the default first.
pub fn tor_presets() -> Vec<TorPreset> {
    #[cfg(windows)]
    return vec![
        TorPreset {
            name: "Tor Browser".to_owned(),
            settings: tor_browser(),
        },
        TorPreset {
            name: "Tor Expert Bundle".to_owned(),
            settings: expert_bundle(),
        },
    ];
    #[cfg(not(windows))]
    return vec![
        TorPreset {
            name: "System Tor".to_owned(),
            settings: system_tor(),
        },
        TorPreset {
            name: "Tor Browser".to_owned(),
            settings: tor_browser(),
        },
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tails_is_known_by_its_os_release() {
        assert!(os_release_is_tails("NAME=\"Tails\"\nID=\"tails\"\n"));
        assert!(os_release_is_tails("ID=tails"));
        assert!(!os_release_is_tails("ID=debian\nID_LIKE=tails\n"));
        assert!(!os_release_is_tails(""));
    }

    #[test]
    fn a_mount_point_is_read_from_the_mount_table() {
        let table = concat!(
            "36 35 98:0 / /home/amnesia/Persistent rw - ext4 /dev/dm-0 rw\n",
            r"37 35 98:0 / /media/with\040space rw - ext4 /dev/sdb1 rw",
            "\n",
        );
        assert!(mount_table_has(
            table,
            Path::new("/home/amnesia/Persistent")
        ));
        assert!(mount_table_has(table, Path::new("/media/with space")));
        assert!(!mount_table_has(table, Path::new("/home/amnesia")));
        assert!(!mount_table_has(
            table,
            Path::new("/home/amnesia/Persistent/monolith")
        ));
    }

    #[test]
    fn tor_browser_is_looked_for_on_a_desktop_that_onedrive_moved() {
        let home =
            std::env::temp_dir().join(format!("monolith-places-test-{}", std::process::id()));
        let desktop = home.join("Desktop").join("Tor Browser");
        assert_eq!(windows_tor_browser(&home), desktop);
        // A OneDrive desktop without Tor Browser is not taken.
        let onedrive = home.join("OneDrive").join("Desktop");
        std::fs::create_dir_all(&onedrive).unwrap();
        assert_eq!(windows_tor_browser(&home), desktop);
        std::fs::create_dir_all(onedrive.join("Tor Browser")).unwrap();
        assert_eq!(windows_tor_browser(&home), onedrive.join("Tor Browser"));
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn a_path_that_climbs_out_is_never_in_the_persistent_storage() {
        assert!(!in_tails_persistent(Path::new(
            "/home/amnesia/Persistent/../amnesia/monolith"
        )));
        assert!(!in_tails_persistent(Path::new("relative/monolith")));
    }
}

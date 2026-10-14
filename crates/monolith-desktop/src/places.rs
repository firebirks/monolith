//! Where Monolith keeps its files, and the Tor it expects, on each
//! platform. Only absolute paths from the environment are taken.

use std::ffi::OsString;
use std::path::PathBuf;

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

/// The data directory: the vault and the message store.
pub fn data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    return absolute("LOCALAPPDATA").map(|base| base.join("Monolith").join("Profile"));
    #[cfg(not(windows))]
    return absolute("XDG_DATA_HOME")
        .or_else(|| home().map(|home| home.join(".local").join("share")))
        .map(|base| base.join("monolith"));
}

/// The settings file of the desktop client.
pub fn config_file() -> Option<PathBuf> {
    #[cfg(windows)]
    let base = absolute("LOCALAPPDATA").map(|base| base.join("Monolith"));
    #[cfg(not(windows))]
    let base = absolute("XDG_CONFIG_HOME")
        .or_else(|| home().map(|home| home.join(".config")))
        .map(|base| base.join("monolith"));
    base.map(|base| base.join("desktop.conf"))
}

/// Where a saved card goes: the downloads folder, or the home folder.
pub fn save_dir() -> Option<PathBuf> {
    let home = home()?;
    let downloads = home.join("Downloads");
    Some(if downloads.is_dir() { downloads } else { home })
}

fn settings(socks: &str, control: &str, cookie_file: OsString) -> TorSettings {
    TorSettings {
        socks: socks.to_owned(),
        control: control.to_owned(),
        cookie_file: cookie_file.to_string_lossy().into_owned(),
    }
}

/// The Tor of Tor Browser, as it is installed by default.
fn tor_browser() -> TorSettings {
    // Tor Browser installs into a folder of the user's choice; on Windows
    // it offers the desktop, on Linux it is unpacked where the user wants.
    #[cfg(windows)]
    let folder = home().map(|home| home.join("Desktop").join("Tor Browser"));
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

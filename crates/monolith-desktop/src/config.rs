//! The settings file (`docs/adr/0011-ui-security-boundary.md`): the size
//! of the window, the theme, reduced motion and the Tor endpoints, as
//! `key = value` lines. Nothing else is written to it. A file that cannot
//! be read, or a line that does not parse, leaves the defaults.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use monolith_runtime::{ControlAuth, Endpoint, SocksProof, SystemTorConfig};
use monolith_ui::{Preferences, ThemeChoice, TorSettings};

use crate::{local, places};

/// The largest settings file read.
const MAX_FILE: u64 = 64 * 1024;

/// The longest value kept.
const MAX_VALUE: usize = 4096;

/// The smallest and the largest window, in points.
const MIN_SIDE: f32 = 360.0;
const MAX_SIDE: f32 = 16_384.0;

/// The size of the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Window {
    /// Width in points.
    pub width: f32,
    /// Height in points.
    pub height: f32,
    /// Maximized.
    pub maximized: bool,
}

impl Default for Window {
    /// The size the design system gives a window at first start.
    fn default() -> Self {
        let size = monolith_ui_kit::theme::Theme::light().size.window_default;
        Self {
            width: size.x,
            height: size.y,
            maximized: false,
        }
    }
}

/// Everything in the settings file.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// The preferences.
    pub preferences: Preferences,
    /// The window.
    pub window: Window,
}

impl Config {
    /// The defaults, with `tor` as the Tor settings.
    pub fn defaults(tor: TorSettings) -> Self {
        Self {
            preferences: Preferences {
                theme: ThemeChoice::System,
                reduced_motion: false,
                tor,
            },
            window: Window::default(),
        }
    }
}

fn side(value: &str) -> Option<f32> {
    value
        .parse::<f32>()
        .ok()
        .filter(|side| side.is_finite())
        .map(|side| side.clamp(MIN_SIDE, MAX_SIDE))
}

fn flag(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// A value as it may be kept: one line, no control characters, not too
/// long.
fn clean(value: &str) -> Option<String> {
    (value.len() <= MAX_VALUE && !value.chars().any(char::is_control)).then(|| value.to_owned())
}

/// Reads the settings in `text` over `defaults`.
pub fn parse(text: &str, defaults: Config) -> Config {
    let mut config = defaults;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        let preferences = &mut config.preferences;
        match key.trim() {
            "window.width" => {
                config.window.width = side(value).unwrap_or(config.window.width);
            }
            "window.height" => {
                config.window.height = side(value).unwrap_or(config.window.height);
            }
            "window.maximized" => {
                config.window.maximized = flag(value).unwrap_or(config.window.maximized);
            }
            "theme" => {
                preferences.theme = match value {
                    "system" => ThemeChoice::System,
                    "light" => ThemeChoice::Light,
                    "dark" => ThemeChoice::Dark,
                    _ => preferences.theme,
                };
            }
            "reduced_motion" => {
                preferences.reduced_motion = flag(value).unwrap_or(preferences.reduced_motion);
            }
            "tor.socks" => {
                if let Some(value) = clean(value) {
                    preferences.tor.socks = value;
                }
            }
            "tor.control" => {
                if let Some(value) = clean(value) {
                    preferences.tor.control = value;
                }
            }
            "tor.cookie_file" => {
                if let Some(value) = clean(value) {
                    preferences.tor.cookie_file = value;
                }
            }
            _ => {}
        }
    }
    config
}

/// The settings as the file holds them.
pub fn render(config: &Config) -> String {
    let preferences = &config.preferences;
    let theme = match preferences.theme {
        ThemeChoice::System => "system",
        ThemeChoice::Light => "light",
        ThemeChoice::Dark => "dark",
    };
    let value = |text: &str| clean(text.trim()).unwrap_or_default();
    format!(
        "# Monolith desktop settings\n\
         window.width = {}\n\
         window.height = {}\n\
         window.maximized = {}\n\
         theme = {theme}\n\
         reduced_motion = {}\n\
         tor.socks = {}\n\
         tor.control = {}\n\
         tor.cookie_file = {}\n",
        config.window.width.round(),
        config.window.height.round(),
        config.window.maximized,
        preferences.reduced_motion,
        value(&preferences.tor.socks),
        value(&preferences.tor.control),
        value(&preferences.tor.cookie_file),
    )
}

/// Reads the settings file at `path` over `defaults`.
pub fn read(path: &Path, defaults: Config) -> Config {
    let Ok(file) = std::fs::File::open(path) else {
        return defaults;
    };
    let mut text = String::new();
    match file.take(MAX_FILE).read_to_string(&mut text) {
        Ok(_) => parse(&text, defaults),
        Err(_) => defaults,
    }
}

/// Writes the settings file at `path`, readable by the user alone: a new
/// file next to it, then renamed over it.
pub fn write(path: &Path, config: &Config) -> std::io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    create_private_dir(parent)?;
    let mut new = PathBuf::from(path);
    new.set_extension("conf.new");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&new)?;
    file.write_all(render(config).as_bytes())?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&new, path)
}

fn create_private_dir(path: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

/// The Tor configuration of `settings`, if they are complete: endpoints on
/// the loopback or a socket path, and the absolute path of the cookie of
/// SAFECOOKIE authentication, named as a file of this computer
/// (`crate::local`). No setting turns authentication off.
pub fn tor_config(settings: &TorSettings) -> Option<SystemTorConfig> {
    tor_config_on(settings, places::filtered_tor())
}

/// [`tor_config`] on a system whose Tor is the platform's own behind a
/// filter (`filtered`), or not. It does not look at the disk (the cookie
/// path is judged by its name alone), so the window's thread calls it.
pub(crate) fn tor_config_on(settings: &TorSettings, filtered: bool) -> Option<SystemTorConfig> {
    let socks = Endpoint::parse(settings.socks.trim()).ok()?;
    let control = Endpoint::parse(settings.control.trim()).ok()?;
    let cookie_file = PathBuf::from(settings.cookie_file.trim());
    if !cookie_file.is_absolute() || !local::named_here(&cookie_file) {
        return None;
    }
    Some(SystemTorConfig {
        socks,
        control,
        auth: ControlAuth::SafeCookie { cookie_file },
        socks_proof: socks_proof(filtered),
    })
}

/// Whether a dial waits until Tor proved the SOCKS endpoint its own
/// ([`SocksProof::ControlListeners`]), on a system whose Tor is the
/// platform's own behind a filter (`filtered`), or not.
///
/// On, unless `filtered`. The default Tor on Windows is Tor Browser's,
/// which is closed more often than not; then any local account can take
/// its SOCKS port and would receive the onion name of every dial and the
/// isolation credentials. A system Tor that is stopped is no different.
/// Off on Tails and Whonix: the control endpoint there is an onion-grater
/// filter, which answers authentication itself and may refuse the GETINFO
/// key, and the SOCKS port belongs to the platform's Tor, which holds it
/// for as long as the system runs.
pub fn socks_proof(filtered: bool) -> SocksProof {
    if filtered {
        SocksProof::Off
    } else {
        SocksProof::ControlListeners
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An absolute path on this platform.
    const COOKIE: &str = if cfg!(windows) {
        r"C:\Tor\control_auth_cookie"
    } else {
        "/run/tor/control.authcookie"
    };

    fn defaults() -> Config {
        Config::defaults(TorSettings {
            socks: "127.0.0.1:9050".to_owned(),
            control: "127.0.0.1:9051".to_owned(),
            cookie_file: COOKIE.to_owned(),
        })
    }

    #[test]
    fn what_is_written_is_read_back() {
        let mut config = defaults();
        config.window = Window {
            width: 900.0,
            height: 640.0,
            maximized: true,
        };
        config.preferences.theme = ThemeChoice::Dark;
        config.preferences.reduced_motion = true;
        config.preferences.tor.socks = "127.0.0.1:9150".to_owned();
        assert_eq!(parse(&render(&config), defaults()), config);
    }

    #[test]
    fn a_line_that_does_not_parse_leaves_the_default() {
        let config = parse(
            "theme = purple\nwindow.width = NaN\nwindow.height = -5\nreduced_motion = 1\n\
             tor.socks = a\u{7}b\nnonsense\nunknown = 3\n",
            defaults(),
        );
        let expected = Config {
            window: Window {
                height: MIN_SIDE,
                ..Window::default()
            },
            ..defaults()
        };
        assert_eq!(config, expected);
    }

    #[test]
    fn a_value_cannot_add_a_line() {
        let mut config = defaults();
        config.preferences.tor.cookie_file = "/tmp/a\ntheme = dark".to_owned();
        let text = render(&config);
        assert_eq!(text.lines().count(), 9);
        assert_eq!(
            parse(&text, defaults()).preferences.theme,
            ThemeChoice::System
        );
    }

    #[test]
    fn tor_settings_without_a_cookie_are_refused() {
        let mut settings = defaults().preferences.tor;
        assert!(tor_config(&settings).is_some());
        settings.cookie_file = String::new();
        assert!(tor_config(&settings).is_none());
        settings.cookie_file = "relative/cookie".to_owned();
        assert!(tor_config(&settings).is_none());
        // A cookie on another computer, or named as a device (P7-3).
        #[cfg(windows)]
        for cookie in [r"\\server\share\control_auth_cookie", r"C:\Tor\COM1"] {
            settings.cookie_file = cookie.to_owned();
            assert!(tor_config(&settings).is_none(), "{cookie}");
        }
        settings.cookie_file = COOKIE.to_owned();
        settings.socks = "192.168.1.1:9050".to_owned();
        assert!(tor_config(&settings).is_none());
    }

    #[test]
    fn the_socks_endpoint_is_proved_except_on_tails_and_whonix() {
        let settings = defaults().preferences.tor;
        assert_eq!(
            tor_config_on(&settings, false).map(|config| config.socks_proof),
            Some(SocksProof::ControlListeners)
        );
        assert_eq!(
            tor_config_on(&settings, true).map(|config| config.socks_proof),
            Some(SocksProof::Off)
        );
        assert_eq!(
            tor_config(&settings).map(|config| config.socks_proof),
            Some(socks_proof(places::filtered_tor()))
        );
    }
}

//! The Monolith desktop client (`docs/adr/0010-gui-toolkit-egui.md`,
//! `docs/adr/0011-ui-security-boundary.md`).
//!
//! This crate is the only GUI crate that depends on `monolith-runtime`. It
//! runs the runtime behind a bridge ([`bridge`]) that turns its plain values
//! into the handles of the UI model and back, opens the window, and does
//! what the screens ask of the disk: cards saved and read, and the settings
//! file. It holds no object of the runtime besides the [`Messenger`] the
//! bridge owns, and the screens see nothing of it.
//!
//! [`Messenger`]: monolith_runtime::Messenger

pub mod bridge;
pub mod calendar;
pub mod config;
mod convert;
pub mod files;
mod local;
pub mod places;
pub mod system_fonts;
pub mod window;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Instant;

use core::time::Duration;

use monolith_runtime::{ControlAuth, Endpoint, SystemTorBackend, SystemTorConfig};
use monolith_ui::{App, EffectResult, TorSettings};
use monolith_ui_model::{Failure, Model};

use crate::bridge::{Setup, Wake};
use crate::config::Config;
use crate::window::Window;

/// How long the runtime has, when the window closes, to close its sessions
/// and remove its Onion Services.
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(3);

/// How long the window waits for the runtime to stop.
const CLOSE_WAIT: Duration = Duration::from_secs(5);

const USAGE: &str = "usage: monolith-desktop [--data-dir DIR] [--config FILE]";

/// A thread of the client, waited for when this is dropped, a while at
/// most: a thread that hangs does not hold the exit.
#[derive(Debug)]
pub struct Join {
    thread: Option<JoinHandle<()>>,
    wait: Duration,
}

impl Join {
    fn new(thread: JoinHandle<()>, wait: Duration) -> Self {
        Self {
            thread: Some(thread),
            wait,
        }
    }

    /// Takes the thread out, leaving nothing to wait for.
    fn take(&mut self) -> Self {
        Self {
            thread: self.thread.take(),
            wait: self.wait,
        }
    }
}

impl Drop for Join {
    fn drop(&mut self) {
        let Some(thread) = self.thread.take() else {
            return;
        };
        let deadline = Instant::now().checked_add(self.wait);
        while !thread.is_finished() && deadline.is_some_and(|deadline| Instant::now() < deadline) {
            std::thread::sleep(Duration::from_millis(10));
        }
        if thread.is_finished() {
            let _ = thread.join();
        }
    }
}

/// What the command line says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// The data directory, instead of the default.
    pub data_dir: Option<PathBuf>,
    /// The settings file, instead of the default.
    pub config: Option<PathBuf>,
}

impl Options {
    /// Reads the arguments after the program name.
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            // A place on another computer would be reached outside Tor
            // (`local`, P7-3).
            let mut path = |name: &str| {
                args.next()
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute() && local::would_be_here(path))
                    .ok_or_else(|| {
                        format!(
                            "{name} needs an absolute path on a drive of this computer\n{USAGE}"
                        )
                    })
            };
            match arg.to_str() {
                Some("--data-dir") => options.data_dir = Some(path("--data-dir")?),
                Some("--config") => options.config = Some(path("--config")?),
                _ => return Err(USAGE.to_owned()),
            }
        }
        Ok(options)
    }
}

/// The Tor the runtime uses: the settings if they are complete, and the
/// default otherwise. Authentication is SAFECOOKIE in either case, even
/// with no cookie file to read: then Tor refuses Monolith, and the screens
/// say so. The SOCKS endpoint is proved Tor's before a dial unless the
/// system's Tor is the platform's own behind a filter (`filtered`), as
/// [`config::socks_proof`] explains.
fn backend_config(settings: &TorSettings, filtered: bool) -> Option<SystemTorConfig> {
    let cookie_file = PathBuf::from(settings.cookie_file.trim());
    if cookie_here(&cookie_file) {
        if let Some(config) = config::tor_config_on(settings, filtered) {
            return Some(config);
        }
    }
    // A cookie file that is not one of this computer is never read: the
    // path is left empty, and Tor refuses Monolith.
    Some(SystemTorConfig {
        socks: Endpoint::parse(settings.socks.trim()).ok()?,
        control: Endpoint::parse(settings.control.trim()).ok()?,
        auth: ControlAuth::SafeCookie {
            cookie_file: if cookie_here(&cookie_file) {
                cookie_file
            } else {
                PathBuf::new()
            },
        },
        socks_proof: config::socks_proof(filtered),
    })
}

/// Whether the cookie file `path` is one of this computer, or would be
/// once Tor writes it (`local::would_be_here`): looked at once, at the
/// start, since the Tor settings take effect then.
fn cookie_here(path: &Path) -> bool {
    path.is_absolute() && local::would_be_here(path)
}

/// Runs the client until its window closes.
pub fn run(options: Options) -> ExitCode {
    let presets = places::tor_presets();
    let default_tor = presets
        .first()
        .map(|preset| preset.settings.clone())
        .unwrap_or_else(|| TorSettings {
            socks: String::new(),
            control: String::new(),
            cookie_file: String::new(),
        });
    // On Tails a file is kept only in the Persistent Storage; a place named
    // on the command line elsewhere is refused, not replaced by another.
    let tails = places::is_tails();
    let kept = |path: PathBuf| (!tails || places::in_tails_persistent(&path)).then_some(path);
    let config_path = match options.config {
        Some(path) => kept(path),
        None => places::config_file(),
    };
    let defaults = Config::defaults(default_tor.clone());
    let mut config = match &config_path {
        Some(path) => config::read(path, defaults),
        None => defaults,
    };
    let tor_settings_refused = (config::tor_config(&config.preferences.tor).is_none()
        || !cookie_here(Path::new(config.preferences.tor.cookie_file.trim())))
        && config.preferences.tor != default_tor;
    if tor_settings_refused {
        config.preferences.tor = default_tor;
    }
    // The default endpoints always parse; nothing else is ever used.
    let Some(tor) = backend_config(&config.preferences.tor, places::filtered_tor()) else {
        return ExitCode::FAILURE;
    };
    let data_dir = match options.data_dir {
        Some(path) => kept(path),
        None => places::data_dir(),
    };

    let window = &config.window;
    let sizes = monolith_ui_kit::theme::Theme::light().size;
    let viewport = egui::ViewportBuilder::default()
        .with_title("Monolith")
        .with_app_id("monolith")
        .with_inner_size([window.width, window.height])
        .with_min_inner_size(sizes.window_min)
        .with_maximized(window.maximized)
        .with_icon(Arc::new(window::icon()));
    let native = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        centered: true,
        persist_window: false,
        ..eframe::NativeOptions::default()
    };
    // eframe drops the window's app before it destroys the window, which on
    // Wayland cannot hide itself: the runtime and the disk thread are
    // waited for once the window is gone, after `run_native` returns.
    let mut threads = None;
    let slot = &mut threads;
    let started = eframe::run_native(
        "Monolith",
        native,
        Box::new(move |creation| {
            let ctx = creation.egui_ctx.clone();
            // egui's own light and dark styles never show: the theme of the
            // design system is installed in the light slot, whatever the
            // system says, and follows the preferences itself.
            ctx.set_theme(egui::ThemePreference::Light);
            monolith_ui_kit::fonts::install(&ctx, &system_fonts::load());
            let wake: Wake = {
                let ctx = ctx.clone();
                Arc::new(move || ctx.request_repaint())
            };
            let setup = Setup {
                data_dir,
                backend: Box::new(move || SystemTorBackend::new(tor.clone())),
                shutdown_deadline: SHUTDOWN_DEADLINE,
                close_wait: CLOSE_WAIT,
            };
            let mut app = App::new(
                Model::new(),
                Box::new(calendar::SystemCalendar::new()),
                config.preferences.clone(),
            );
            app.set_tor_presets(presets);
            if tor_settings_refused {
                app.effect_done(EffectResult::Failed(Failure::TorSettings));
            }
            let (window, joins) = Window::start(app, setup, wake, config, config_path)?;
            *slot = Some(joins);
            Ok(Box::new(window))
        }),
    );
    drop(threads);
    match started {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use monolith_runtime::SocksProof;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn only_absolute_paths_are_taken() {
        let absolute = if cfg!(windows) { "C:\\data" } else { "/data" };
        assert_eq!(
            Options::parse(args(&["--data-dir", absolute])),
            Ok(Options {
                data_dir: Some(PathBuf::from(absolute)),
                config: None,
            })
        );
        assert!(Options::parse(args(&["--data-dir", "relative"])).is_err());
        assert!(Options::parse(args(&["--config"])).is_err());
        assert!(Options::parse(args(&["--open", "https://example.com"])).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn a_place_on_another_computer_is_not_taken() {
        // It would be reached over SMB, outside Tor (P7-3).
        for path in [
            r"\\server\share\monolith",
            "//server/share/monolith",
            r"\\?\UNC\server\share\monolith",
            r"C:\NUL\monolith",
        ] {
            for option in ["--data-dir", "--config"] {
                assert!(
                    Options::parse(args(&[option, path])).is_err(),
                    "{option} {path}"
                );
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_cookie_on_another_computer_is_never_read() {
        let settings = TorSettings {
            socks: "127.0.0.1:9150".to_owned(),
            control: "127.0.0.1:9151".to_owned(),
            cookie_file: r"\\server\share\control_auth_cookie".to_owned(),
        };
        let config = backend_config(&settings, false).unwrap();
        assert_eq!(
            config.auth,
            ControlAuth::SafeCookie {
                cookie_file: PathBuf::new()
            }
        );
    }

    #[test]
    fn the_runtime_proves_the_socks_endpoint_except_on_tails_and_whonix() {
        let complete = TorSettings {
            socks: "127.0.0.1:9150".to_owned(),
            control: "127.0.0.1:9151".to_owned(),
            cookie_file: if cfg!(windows) {
                r"C:\Tor\control_auth_cookie"
            } else {
                "/run/tor/control.authcookie"
            }
            .to_owned(),
        };
        // Without a cookie file the settings are not complete, and the
        // runtime still gets a configuration.
        let without_cookie = TorSettings {
            cookie_file: String::new(),
            ..complete.clone()
        };
        for settings in [&complete, &without_cookie] {
            assert_eq!(
                backend_config(settings, false).map(|config| config.socks_proof),
                Some(SocksProof::ControlListeners)
            );
            assert_eq!(
                backend_config(settings, true).map(|config| config.socks_proof),
                Some(SocksProof::Off)
            );
        }
    }
}

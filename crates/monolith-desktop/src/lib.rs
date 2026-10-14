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
pub mod places;
pub mod system_fonts;
pub mod window;

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use core::time::Duration;

use monolith_runtime::{ControlAuth, Endpoint, SystemTorBackend, SystemTorConfig};
use monolith_ui::{App, EffectResult, TorSettings};
use monolith_ui_model::{Failure, Model};

use crate::bridge::{Bridge, Setup, Wake};
use crate::config::Config;
use crate::files::Disk;
use crate::window::Window;

/// How long the runtime has, when the window closes, to close its sessions
/// and remove its Onion Services.
const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(3);

/// How long the window waits for the runtime to stop.
const CLOSE_WAIT: Duration = Duration::from_secs(5);

const USAGE: &str = "usage: monolith-desktop [--data-dir DIR] [--config FILE]";

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
            let mut path = |name: &str| {
                args.next()
                    .map(PathBuf::from)
                    .filter(|path| path.is_absolute())
                    .ok_or_else(|| format!("{name} needs an absolute path\n{USAGE}"))
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
/// say so.
fn backend_config(settings: &TorSettings) -> Option<SystemTorConfig> {
    if let Some(config) = config::tor_config(settings) {
        return Some(config);
    }
    Some(SystemTorConfig {
        socks: Endpoint::parse(settings.socks.trim()).ok()?,
        control: Endpoint::parse(settings.control.trim()).ok()?,
        auth: ControlAuth::SafeCookie {
            cookie_file: PathBuf::from(settings.cookie_file.trim()),
        },
    })
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
    let config_path = options.config.or_else(places::config_file);
    let defaults = Config::defaults(default_tor.clone());
    let mut config = match &config_path {
        Some(path) => config::read(path, defaults),
        None => defaults,
    };
    let tor_settings_refused = config::tor_config(&config.preferences.tor).is_none()
        && config.preferences.tor != default_tor;
    if tor_settings_refused {
        config.preferences.tor = default_tor;
    }
    // The default endpoints always parse; nothing else is ever used.
    let Some(tor) = backend_config(&config.preferences.tor) else {
        return ExitCode::FAILURE;
    };
    let data_dir = options.data_dir.or_else(places::data_dir);

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
            let bridge = Bridge::start(setup, Arc::clone(&wake))?;
            let disk = Disk::start(wake)?;
            let mut app = App::new(
                Model::new(),
                Box::new(calendar::SystemCalendar::new()),
                config.preferences.clone(),
            );
            app.set_tor_presets(presets);
            if tor_settings_refused {
                app.effect_done(EffectResult::Failed(Failure::TorSettings));
            }
            Ok(Box::new(Window::new(
                app,
                bridge,
                disk,
                config,
                config_path,
            )))
        }),
    );
    match started {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

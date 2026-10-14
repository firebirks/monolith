//! The window: each frame takes what the bridge and the disk said, draws
//! the screens, and hands on what they asked for.

use std::path::PathBuf;
#[cfg(feature = "screenshot")]
use std::sync::Arc;

use core::time::Duration;

use egui::{Color32, Context, IconData, OutputCommand};
use monolith_ui::{App, Effect, EffectResult, ThemeChoice};
use monolith_ui_kit::theme::{self, Mode, Theme};
use monolith_ui_model::Failure;

use crate::bridge::Bridge;
use crate::config::{self, Config};
use crate::files::{Disk, Job};
use crate::places;

/// The window of the client.
pub struct Window {
    app: App,
    bridge: Bridge,
    disk: Disk,
    config: Config,
    config_path: Option<PathBuf>,
    /// The theme installed: light or dark, and reduced motion.
    installed: Option<(Mode, bool)>,
    /// The colour behind the panels.
    clear: Color32,
    /// Where the window photographs itself to, and whether it asked.
    #[cfg(feature = "screenshot")]
    shot: Option<(PathBuf, bool)>,
}

impl core::fmt::Debug for Window {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Window").finish_non_exhaustive()
    }
}

impl Window {
    /// A window over `app`, with the bridge and the disk thread running.
    pub fn new(
        app: App,
        bridge: Bridge,
        disk: Disk,
        config: Config,
        config_path: Option<PathBuf>,
    ) -> Self {
        Self {
            app,
            bridge,
            disk,
            config,
            config_path,
            installed: None,
            clear: Color32::BLACK,
            #[cfg(feature = "screenshot")]
            shot: std::env::var_os("MONOLITH_SCREENSHOT").map(|path| (PathBuf::from(path), false)),
        }
    }

    /// Two seconds after the window opened, asks for a picture of it;
    /// once it is there, writes it and closes the window.
    #[cfg(feature = "screenshot")]
    fn shoot(&mut self, ctx: &Context) {
        let Some((path, asked)) = &mut self.shot else {
            return;
        };
        let taken = ctx.input(|input| {
            input.raw.events.iter().find_map(|event| match event {
                egui::Event::Screenshot { image, .. } => Some(Arc::clone(image)),
                _ => None,
            })
        });
        if let Some(image) = taken {
            let picture = monolith_ui_snapshot::Image {
                width: image.width(),
                height: image.height(),
                rgba: image
                    .pixels
                    .iter()
                    .flat_map(|pixel| pixel.to_srgba_unmultiplied())
                    .collect(),
            };
            let _ = picture.save_png(path);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !*asked && ctx.input(|input| input.time) > 2.0 {
            *asked = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint();
    }

    /// Installs the theme the preferences and the system ask for, if it
    /// is not the one installed.
    fn apply_theme(&mut self, ctx: &Context) {
        let preferences = self.app.preferences();
        let mode = match preferences.theme {
            ThemeChoice::Light => Mode::Light,
            ThemeChoice::Dark => Mode::Dark,
            ThemeChoice::System => match ctx.system_theme() {
                Some(egui::Theme::Dark) => Mode::Dark,
                _ => Mode::Light,
            },
        };
        let wanted = (mode, preferences.reduced_motion);
        if self.installed == Some(wanted) {
            return;
        }
        let theme = match mode {
            Mode::Light => Theme::light(),
            Mode::Dark => Theme::dark(),
        }
        .with_reduced_motion(preferences.reduced_motion);
        self.clear = theme.color.bg_app;
        theme::install(ctx, theme);
        self.installed = Some(wanted);
    }

    /// Keeps the size of the window for the settings file.
    fn track_window(&mut self, ctx: &Context) {
        let (inner, maximized) = ctx.input(|input| {
            let viewport = input.viewport();
            (viewport.inner_rect, viewport.maximized)
        });
        if let Some(maximized) = maximized {
            self.config.window.maximized = maximized;
        }
        // The size of a maximized window is not the one to come back to.
        if self.config.window.maximized {
            return;
        }
        if let Some(inner) = inner {
            self.config.window.width = inner.width();
            self.config.window.height = inner.height();
        }
    }

    /// Writes the settings file; at exit, nobody is told if it fails.
    fn save_config(&self) {
        if let Some(path) = &self.config_path {
            let _ = self
                .disk
                .ask(Job::Config(path.clone(), Box::new(self.config.clone())));
        }
    }

    fn effect(&mut self, effect: Effect) {
        let job = match effect {
            Effect::SaveCard { name, contents } => Job::SaveCard {
                dir: places::save_dir(),
                name,
                contents,
            },
            Effect::ReadCard { path } => Job::ReadCard(path),
            Effect::Preferences(preferences) => {
                if config::tor_config(&preferences.tor).is_none() {
                    self.app
                        .effect_done(EffectResult::Failed(Failure::TorSettings));
                    return;
                }
                self.config.preferences = preferences.clone();
                self.app.set_preferences(preferences);
                let Some(path) = &self.config_path else {
                    return;
                };
                Job::Config(path.clone(), Box::new(self.config.clone()))
            }
        };
        if let Some(failed) = self.disk.ask(job) {
            self.app.effect_done(failed);
        }
    }
}

impl eframe::App for Window {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        while let Some(update) = self.bridge.try_update() {
            self.app.model.apply(update);
        }
        while let Some(result) = self.disk.try_result() {
            self.app.effect_done(result);
        }
        self.apply_theme(ctx);
        self.track_window(ctx);

        monolith_ui::show(ctx, &mut self.app);

        for (command, what) in self.app.model.take_commands() {
            self.bridge.send(command, what);
        }
        self.bridge.flush();
        if self.bridge.backlogged() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        for effect in self.app.take_effects() {
            self.effect(effect);
        }
        strip_urls(ctx);
        #[cfg(feature = "screenshot")]
        self.shoot(ctx);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.clear.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // The runtime stops when the bridge is dropped, after this.
        self.app.model.shut_down();
        for (command, what) in self.app.model.take_commands() {
            self.bridge.send(command, what);
        }
        self.save_config();
    }
}

/// Removes every request to open a URL from this frame's output: eframe
/// would hand it to the system's browser (`docs/adr/0010-gui-toolkit-egui.md`).
pub fn strip_urls(ctx: &Context) {
    ctx.output_mut(|output| {
        output
            .commands
            .retain(|command| !matches!(command, OutputCommand::OpenUrl(_)));
        #[allow(deprecated)]
        {
            output.open_url = None;
        }
    });
}

/// A channel value, rounded and kept within 0 to 255.
fn byte(value: f32) -> u8 {
    // Clamped first, so the cast neither truncates nor loses a sign.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let byte = value.clamp(0.0, 255.0).round() as u8;
    byte
}

/// The icon of the window: a standing slab on the accent colour.
pub fn icon() -> IconData {
    icon_of_side(64)
}

/// The icon, `side` pixels square: for the window, and for the packages
/// that install an icon file.
pub fn icon_of_side(side: u32) -> IconData {
    // The shapes are given on a grid of 64 and drawn at `side`.
    let unit = side as f32 / 64.0;
    let palette = Theme::dark().color;
    let background = palette.accent;
    let slab = palette.text_on_accent;
    // The coverage of a rounded rectangle at a pixel centre, from its
    // signed distance, for edges without steps.
    let coverage = |x: f32, y: f32, centre: (f32, f32), half: (f32, f32), radius: f32| {
        let dx = ((x - centre.0).abs() - (half.0 - radius)).max(0.0);
        let dy = ((y - centre.1).abs() - (half.1 - radius)).max(0.0);
        let inside = ((x - centre.0).abs() - (half.0 - radius))
            .max((y - centre.1).abs() - (half.1 - radius))
            .min(0.0);
        let distance = (dx * dx + dy * dy).sqrt() + inside - radius;
        (0.5 - distance).clamp(0.0, 1.0)
    };
    let mut rgba = Vec::new();
    for row in 0..side {
        for column in 0..side {
            let (x, y) = (column as f32 + 0.5, row as f32 + 0.5);
            let tile = coverage(
                x,
                y,
                (32.0 * unit, 32.0 * unit),
                (30.0 * unit, 30.0 * unit),
                13.0 * unit,
            );
            let stone = coverage(
                x,
                y,
                (32.0 * unit, 33.0 * unit),
                (7.0 * unit, 19.0 * unit),
                1.5 * unit,
            );
            let mix = |under: u8, over: u8| {
                byte(f32::from(under) * (1.0 - stone) + f32::from(over) * stone)
            };
            rgba.extend([
                mix(background.r(), slab.r()),
                mix(background.g(), slab.g()),
                mix(background.b(), slab.b()),
                byte(tile * 255.0),
            ]);
        }
    }
    IconData {
        rgba,
        width: side,
        height: side,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_url_leaves_a_frame() {
        let ctx = Context::default();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            ctx.open_url(egui::OpenUrl::new_tab("https://example.com"));
            #[allow(deprecated)]
            ctx.output_mut(|output| {
                output.open_url = Some(egui::OpenUrl::same_tab("https://example.org"));
            });
            ctx.copy_text("kept".to_owned());
            strip_urls(ctx);
        });
        let commands = &output.platform_output.commands;
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, OutputCommand::OpenUrl(_))),
            "{commands:?}"
        );
        #[allow(deprecated)]
        let open = output.platform_output.open_url.is_none();
        assert!(open);
        assert!(commands.contains(&OutputCommand::CopyText("kept".to_owned())));
    }

    #[test]
    fn the_icon_is_square_and_opaque_in_the_middle() {
        let icon = icon();
        assert_eq!(icon.rgba.len(), 64 * 64 * 4);
        let middle = (32 * 64 + 32) * 4;
        assert_eq!(icon.rgba[middle + 3], 255);
        assert_eq!(icon.rgba[3], 0);
    }
}

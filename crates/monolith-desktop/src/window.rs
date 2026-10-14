//! The window: each frame takes what the bridge and the disk said, draws
//! the screens, and hands on what they asked for.

use std::path::PathBuf;
#[cfg(feature = "screenshot")]
use std::sync::Arc;

use core::time::Duration;

use egui::{Color32, Context, IconData, OutputCommand, UserAttentionType, ViewportCommand};
use monolith_ui::{App, Effect, EffectResult, ThemeChoice};
use monolith_ui_kit::theme::{self, Mode, Theme};
use monolith_ui_model::{Failure, Model};

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
    /// What waits, as the title says it.
    waiting: u64,
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
        // eframe clears the window before the first frame: with the
        // colour of the theme the preferences ask for, as far as it is
        // known before the system is asked (light while it is not).
        let clear = theme_of(mode(app.preferences().theme, None), false)
            .color
            .bg_app;
        Self {
            app,
            bridge,
            disk,
            config,
            config_path,
            installed: None,
            clear,
            waiting: 0,
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
            self.app.allow_close();
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
        let mode = mode(preferences.theme, ctx.system_theme());
        let wanted = (mode, preferences.reduced_motion);
        if self.installed == Some(wanted) {
            return;
        }
        let theme = theme_of(mode, preferences.reduced_motion);
        self.clear = theme.color.bg_app;
        theme::install(ctx, theme);
        self.installed = Some(wanted);
    }

    /// Says in the title what waits over all identities, and asks for the
    /// user's attention when more waits while the window is in the
    /// background.
    fn signal_waiting(&mut self, ctx: &Context) {
        let now = waiting(&self.app.model);
        let focused = ctx.input(|input| input.viewport().focused.unwrap_or(input.focused));
        let signal = signal(self.waiting, now, focused);
        if let Some(title) = signal.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title));
        }
        if signal.attention {
            ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(
                UserAttentionType::Informational,
            ));
        }
        self.waiting = now;
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
        if let Some(job) = self.config_job() {
            let _ = self.disk.ask(job);
        }
    }

    /// The writing of the settings file. None in a session that saves
    /// nothing: nothing is kept when it closes, its settings included.
    fn config_job(&self) -> Option<Job> {
        if self.app.model.ephemeral() {
            return None;
        }
        let path = self.config_path.as_ref()?;
        Some(Job::Config(path.clone(), Box::new(self.config.clone())))
    }

    fn effect(&mut self, effect: Effect) {
        let job = match effect {
            Effect::SaveCard { name, contents } => Job::SaveCard {
                dir: places::save_dir(),
                name,
                contents,
            },
            Effect::ReadCard {
                path,
                identity,
                dialog,
            } => Job::ReadCard {
                path,
                identity,
                dialog,
            },
            Effect::Preferences(preferences) => {
                if config::tor_config(&preferences.tor).is_none() {
                    self.app
                        .effect_done(EffectResult::Failed(Failure::TorSettings));
                    return;
                }
                self.config.preferences = preferences.clone();
                self.app.set_preferences(preferences);
                let Some(job) = self.config_job() else {
                    return;
                };
                job
            }
        };
        if let Some(failed) = self.disk.ask(job) {
            self.app.effect_done(failed);
        }
    }
}

impl eframe::App for Window {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        // A delay in what the bridge says counts from now.
        self.app.model.set_time(self.app.now());
        while let Some(update) = self.bridge.try_update() {
            self.app.model.apply(update);
        }
        while let Some(result) = self.disk.try_result() {
            self.app.effect_done(result);
        }
        close_or_ask(ctx, &mut self.app);
        self.apply_theme(ctx);
        self.track_window(ctx);

        monolith_ui::show(ctx, &mut self.app);
        self.signal_waiting(ctx);

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

/// What the window does when it is asked to close, by its close button,
/// Alt+F4, the system, or Close Monolith in the question: it closes if
/// that costs nothing the person has not agreed to; otherwise it stays,
/// back from the taskbar if it was minimized, and the screens ask.
pub fn close_or_ask(ctx: &Context, app: &mut App) {
    if !ctx.input(|input| input.viewport().close_requested()) {
        return;
    }
    if app.close_requested() {
        hide_when_closing(ctx);
        return;
    }
    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
    if ctx.input(|input| input.viewport().minimized == Some(true)) {
        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(ViewportCommand::Focus);
    }
}

/// Takes the window off the screen as soon as it closes. The runtime then
/// takes a few seconds to stop, while the window, no longer drawn, would
/// stay on screen frozen.
pub fn hide_when_closing(ctx: &Context) {
    if ctx.input(|input| input.viewport().close_requested()) {
        ctx.send_viewport_cmd(ViewportCommand::Visible(false));
    }
}

/// Light or dark: as the preferences say, or as the system says if they
/// follow it, and light while it says nothing.
fn mode(choice: ThemeChoice, system: Option<egui::Theme>) -> Mode {
    match choice {
        ThemeChoice::Light => Mode::Light,
        ThemeChoice::Dark => Mode::Dark,
        ThemeChoice::System => match system {
            Some(egui::Theme::Dark) => Mode::Dark,
            _ => Mode::Light,
        },
    }
}

fn theme_of(mode: Mode, reduced_motion: bool) -> Theme {
    match mode {
        Mode::Light => Theme::light(),
        Mode::Dark => Theme::dark(),
    }
    .with_reduced_motion(reduced_motion)
}

/// What waits for the user over all identities: messages not read and
/// requests not answered.
pub fn waiting(model: &Model) -> u64 {
    model
        .identities()
        .iter()
        .filter_map(|identity| model.slice(identity.id))
        .map(|slice| {
            let unread = slice
                .unread
                .values()
                .fold(0_u64, |sum, count| sum.saturating_add(u64::from(*count)));
            let requests = u64::try_from(slice.requests.len()).unwrap_or(u64::MAX);
            unread.saturating_add(requests)
        })
        .fold(0_u64, u64::saturating_add)
}

/// The title of the window when `waiting` things wait.
pub fn title(waiting: u64) -> String {
    if waiting == 0 {
        "Monolith".to_owned()
    } else {
        format!("Monolith ({waiting})")
    }
}

/// What the window does when what waits goes from one count to another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signal {
    /// A new title, if it changes.
    pub title: Option<String>,
    /// Ask for the user's attention.
    pub attention: bool,
}

/// What the window does when what waits goes from `before` to `now`:
/// the title changes with the count, and more waiting while the window
/// is in the background asks for the user's attention.
pub fn signal(before: u64, now: u64, focused: bool) -> Signal {
    Signal {
        title: (now != before).then(|| title(now)),
        attention: now > before && !focused,
    }
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
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use std::sync::Arc;

    use monolith_tor::MockNetwork;
    use monolith_ui::{Preferences, TorSettings};
    use monolith_ui_model::{Command, Model, Outcome, ProfileOnDisk, Update};

    use super::*;
    use crate::bridge::Setup;

    fn tor() -> TorSettings {
        TorSettings {
            socks: "127.0.0.1:9050".to_owned(),
            control: "127.0.0.1:9051".to_owned(),
            cookie_file: if cfg!(windows) {
                r"C:\Tor\control_auth_cookie"
            } else {
                "/run/tor/control.authcookie"
            }
            .to_owned(),
        }
    }

    fn preferences(theme: ThemeChoice) -> Preferences {
        Preferences {
            theme,
            reduced_motion: false,
            tor: tor(),
        }
    }

    /// A window over a model whose profile is open, keeping its settings
    /// at `config_path`.
    fn open_window(config_path: PathBuf, ephemeral: bool, theme: ThemeChoice) -> Window {
        let network = MockNetwork::new();
        let bridge = Bridge::start(
            Setup {
                data_dir: None,
                backend: Box::new(move || network.backend()),
                shutdown_deadline: Duration::from_secs(1),
                close_wait: Duration::from_secs(10),
            },
            Arc::new(|| {}),
        )
        .unwrap();
        let disk = Disk::start(Arc::new(|| {})).unwrap();
        let mut model = Model::new();
        model.apply(Update::Started {
            profile: ProfileOnDisk::Absent,
            location: String::new(),
        });
        model.start_without_saving();
        let commands = model.take_commands();
        assert!(matches!(commands[0].1, Command::StartWithoutSaving));
        model.apply(Update::Done {
            command: commands[0].0,
            outcome: Outcome::ProfileOpen {
                ephemeral,
                message_store_missing: false,
            },
        });
        let mut app = monolith_ui::App::for_tests(model, 0);
        app.set_preferences(preferences(theme));
        let config = Config::defaults(tor());
        Window::new(app, bridge, disk, config, Some(config_path))
    }

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("monolith-window-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_session_that_saves_nothing_writes_no_settings_file() {
        let dir = scratch("settings");
        // A profile kept on disk keeps its settings.
        let kept = dir.join("kept.conf");
        let mut window = open_window(kept.clone(), false, ThemeChoice::System);
        window.effect(Effect::Preferences(preferences(ThemeChoice::Dark)));
        eframe::App::on_exit(&mut window, None);
        drop(window);
        assert!(kept.exists());
        // A session that saves nothing applies them, and writes nothing,
        // neither when they change nor when the window closes.
        let path = dir.join("ephemeral.conf");
        let mut window = open_window(path.clone(), true, ThemeChoice::System);
        window.effect(Effect::Preferences(preferences(ThemeChoice::Dark)));
        assert_eq!(window.app.preferences().theme, ThemeChoice::Dark);
        eframe::App::on_exit(&mut window, None);
        drop(window);
        assert!(!path.exists());
        assert!(!dir.join("ephemeral.conf.new").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_title_counts_what_waits_over_all_identities() {
        use monolith_ui_model::{
            IdentitySnapshot, IdentityView, RequestMode, RequestView, RuntimeEvent, Service,
            UiContactId, UiIdentityId, UiMessageId, UiRequestId,
        };
        let identity = |n: u8| UiIdentityId::from_bytes([n; 16]);
        let mut model = Model::new();
        model.apply(Update::Started {
            profile: ProfileOnDisk::Absent,
            location: String::new(),
        });
        model.start_without_saving();
        assert_eq!(waiting(&model), 0);
        // Two identities: one request waits for the first, and three
        // messages came for the second, which is not shown.
        for _ in 0..3 {
            for (id, command) in model.take_commands() {
                let outcome = match command {
                    Command::StartWithoutSaving => Outcome::ProfileOpen {
                        ephemeral: true,
                        message_store_missing: false,
                    },
                    Command::LoadIdentities => Outcome::Identities(
                        (1..=2)
                            .map(|n| IdentityView {
                                id: identity(n),
                                name: None,
                                fingerprint: String::new(),
                                full_fingerprint: String::new(),
                            })
                            .collect(),
                    ),
                    Command::LoadIdentity(asked) => Outcome::Identity(Box::new(IdentitySnapshot {
                        identity: asked,
                        contacts: Vec::new(),
                        conversations: Vec::new(),
                        requests: (asked == identity(1))
                            .then(|| RequestView {
                                id: UiRequestId::new(asked, 1),
                                fingerprint: String::new(),
                                full_fingerprint: String::new(),
                                display_name: String::new(),
                                introduction: String::new(),
                                invitation: None,
                            })
                            .into_iter()
                            .collect(),
                        invitations: Vec::new(),
                        blocked: Vec::new(),
                        service: Service::Available,
                        request_mode: RequestMode::Invitation,
                    })),
                    _ => continue,
                };
                model.apply(Update::Done {
                    command: id,
                    outcome,
                });
            }
        }
        assert_eq!(model.current(), Some(identity(1)));
        assert_eq!(waiting(&model), 1);
        let contact = UiContactId::new(identity(2), [9; 16]);
        for sequence in 1..=3 {
            model.apply(Update::Event(RuntimeEvent::MessageReceived(
                UiMessageId::new(contact, sequence),
            )));
        }
        assert_eq!(waiting(&model), 4);
        assert_eq!(title(0), "Monolith");
        assert_eq!(title(4), "Monolith (4)");
    }

    #[test]
    fn the_title_changes_with_the_count_and_attention_is_asked_only_in_the_background() {
        assert_eq!(
            signal(0, 0, false),
            Signal {
                title: None,
                attention: false
            }
        );
        assert_eq!(
            signal(0, 2, true),
            Signal {
                title: Some("Monolith (2)".to_owned()),
                attention: false
            }
        );
        assert_eq!(
            signal(2, 3, false),
            Signal {
                title: Some("Monolith (3)".to_owned()),
                attention: true
            }
        );
        // Less waiting, as when it is read, asks for nothing.
        assert_eq!(
            signal(3, 0, false),
            Signal {
                title: Some("Monolith".to_owned()),
                attention: false
            }
        );
    }

    #[test]
    fn a_window_asked_to_close_is_hidden_at_once() {
        let ctx = Context::default();
        let commands = |close: bool| {
            let mut input = egui::RawInput::default();
            if close {
                input
                    .viewports
                    .entry(egui::ViewportId::ROOT)
                    .or_default()
                    .events
                    .push(egui::ViewportEvent::Close);
            }
            ctx.run(input, hide_when_closing)
                .viewport_output
                .get(&egui::ViewportId::ROOT)
                .map(|output| output.commands.clone())
                .unwrap_or_default()
        };
        assert!(!commands(false).contains(&ViewportCommand::Visible(false)));
        assert!(commands(true).contains(&ViewportCommand::Visible(false)));
    }

    /// A model with a profile open, kept on disk or not, with one identity
    /// whose one conversation has `queued` messages waiting.
    fn open_model(ephemeral: bool, queued: usize) -> Model {
        use monolith_ui_model::{
            ConversationView, IdentitySnapshot, IdentityView, RequestMode, Service, UiContactId,
            UiIdentityId,
        };
        let identity = UiIdentityId::from_bytes([1; 16]);
        let mut model = Model::new();
        model.apply(Update::Started {
            profile: ProfileOnDisk::Absent,
            location: String::new(),
        });
        model.start_without_saving();
        for _ in 0..3 {
            for (id, command) in model.take_commands() {
                let outcome = match command {
                    Command::StartWithoutSaving => Outcome::ProfileOpen {
                        ephemeral,
                        message_store_missing: false,
                    },
                    Command::LoadIdentities => Outcome::Identities(vec![IdentityView {
                        id: identity,
                        name: None,
                        fingerprint: String::new(),
                        full_fingerprint: String::new(),
                    }]),
                    Command::LoadIdentity(asked) => Outcome::Identity(Box::new(IdentitySnapshot {
                        identity: asked,
                        contacts: Vec::new(),
                        conversations: vec![ConversationView {
                            contact: UiContactId::new(asked, [9; 16]),
                            fingerprint: String::new(),
                            active: true,
                            messages: queued,
                            queued,
                            latest: None,
                        }],
                        requests: Vec::new(),
                        invitations: Vec::new(),
                        blocked: Vec::new(),
                        service: Service::Available,
                        request_mode: RequestMode::Invitation,
                    })),
                    _ => continue,
                };
                model.apply(Update::Done {
                    command: id,
                    outcome,
                });
            }
        }
        model
    }

    /// A window without a system: frames of `close_or_ask` and the screens.
    struct Closing {
        ctx: Context,
        app: App,
        time: f64,
        /// The texts of the last frame, where they were drawn.
        texts: Vec<(String, egui::Rect)>,
    }

    impl Closing {
        fn new(model: Model) -> Self {
            let ctx = Context::default();
            monolith_ui_kit::fonts::install(&ctx, &[]);
            theme::install(&ctx, Theme::light().with_reduced_motion(true));
            Self {
                ctx,
                app: monolith_ui::App::for_tests(model, 0),
                time: 0.0,
                texts: Vec::new(),
            }
        }

        /// A frame, asked to close by the system if `close`, minimized if
        /// `minimized`; what it asked of the window.
        fn frame(
            &mut self,
            close: bool,
            minimized: bool,
            events: Vec<egui::Event>,
        ) -> Vec<ViewportCommand> {
            self.time += 0.1;
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Theme::light().size.window_default,
                )),
                time: Some(self.time),
                focused: true,
                events,
                ..egui::RawInput::default()
            };
            let viewport = input.viewports.entry(egui::ViewportId::ROOT).or_default();
            if close {
                viewport.events.push(egui::ViewportEvent::Close);
            }
            if minimized {
                viewport.minimized = Some(true);
            }
            let app = &mut self.app;
            let output = self.ctx.run(input, |ctx| {
                close_or_ask(ctx, app);
                monolith_ui::show(ctx, app);
            });
            self.texts.clear();
            for clipped in &output.shapes {
                collect_texts(&clipped.shape, &mut self.texts);
            }
            output
                .viewport_output
                .get(&egui::ViewportId::ROOT)
                .map(|viewport| viewport.commands.clone())
                .unwrap_or_default()
        }

        /// Clicks the text `text`; what the frames asked of the window.
        fn click(&mut self, text: &str) -> Vec<ViewportCommand> {
            let at = self
                .texts
                .iter()
                .find(|(shown, _)| shown == text)
                .map(|(_, rect)| rect.center())
                .unwrap();
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            let mut commands = self.frame(false, false, vec![egui::Event::PointerMoved(at)]);
            commands.extend(self.frame(false, false, vec![button(true)]));
            commands.extend(self.frame(false, false, vec![button(false)]));
            commands
        }

        fn shows(&self, text: &str) -> bool {
            self.texts.iter().any(|(shown, _)| shown == text)
        }
    }

    fn collect_texts(shape: &egui::Shape, into: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect_texts(shape, into);
                }
            }
            egui::Shape::Text(text) => into.push((
                text.galley.text().to_owned(),
                text.galley.rect.translate(text.pos.to_vec2()),
            )),
            _ => {}
        }
    }

    #[test]
    fn closing_what_saves_nothing_asks_first_and_hides_the_window_once_agreed() {
        let mut window = Closing::new(open_model(true, 0));
        for _ in 0..3 {
            window.frame(false, false, Vec::new());
        }
        // Its close button, Alt+F4 or the system: the window stays, shown,
        // and asks; asked again, it still does not close.
        for _ in 0..3 {
            let commands = window.frame(true, false, Vec::new());
            assert!(commands.contains(&ViewportCommand::CancelClose));
            assert!(!commands.contains(&ViewportCommand::Visible(false)));
            assert!(window.app.asking_to_close());
        }
        assert!(window.shows("Close Monolith?"));
        // Asked from the taskbar while minimized: back on the screen.
        let commands = window.frame(true, true, Vec::new());
        assert!(commands.contains(&ViewportCommand::CancelClose));
        assert!(commands.contains(&ViewportCommand::Minimized(false)));
        window.frame(false, false, Vec::new());
        // Close Monolith asks the window to close; it is hidden only when
        // that request comes back, and nothing cancels it.
        let commands = window.click("Close Monolith");
        assert!(commands.contains(&ViewportCommand::Close));
        assert!(!commands.contains(&ViewportCommand::Visible(false)));
        let commands = window.frame(true, false, Vec::new());
        assert!(commands.contains(&ViewportCommand::Visible(false)));
        assert!(!commands.contains(&ViewportCommand::CancelClose));
    }

    #[test]
    fn closing_with_messages_not_delivered_asks_and_cancel_keeps_the_window() {
        let mut window = Closing::new(open_model(false, 2));
        window.frame(false, false, Vec::new());
        let commands = window.frame(true, false, Vec::new());
        assert!(commands.contains(&ViewportCommand::CancelClose));
        window.frame(false, false, Vec::new());
        let commands = window.click("Cancel");
        assert!(!commands.contains(&ViewportCommand::Close));
        assert!(!window.app.asking_to_close());
        window.frame(false, false, Vec::new());
        assert!(!window.shows("Close Monolith?"));
        // Asked again, it asks again.
        let commands = window.frame(true, false, Vec::new());
        assert!(commands.contains(&ViewportCommand::CancelClose));
        assert!(!commands.contains(&ViewportCommand::Visible(false)));
    }

    #[test]
    fn closing_that_costs_nothing_hides_the_window_at_once() {
        let mut window = Closing::new(open_model(false, 0));
        window.frame(false, false, Vec::new());
        let commands = window.frame(true, false, Vec::new());
        assert!(commands.contains(&ViewportCommand::Visible(false)));
        assert!(!commands.contains(&ViewportCommand::CancelClose));
        assert!(!window.app.asking_to_close());
    }

    #[test]
    fn the_first_frame_is_cleared_with_the_colour_of_the_theme() {
        let dir = scratch("clear");
        let dark = open_window(dir.join("dark.conf"), true, ThemeChoice::Dark);
        assert_eq!(dark.clear, Theme::dark().color.bg_app);
        let light = open_window(dir.join("light.conf"), true, ThemeChoice::Light);
        assert_eq!(light.clear, Theme::light().color.bg_app);
        drop((dark, light));
        let _ = std::fs::remove_dir_all(&dir);
    }

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

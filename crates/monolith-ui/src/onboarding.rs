//! The screens before a profile is open: the first start, the passphrase,
//! the progress of opening, and a stopped runtime.

use egui::{Align, Context, Frame, Id, Layout, Sense, Ui, UiBuilder, Vec2};
use monolith_ui_kit::banner::{Banner, InlineError, Tone};
use monolith_ui_kit::button::Size;
use monolith_ui_kit::chrome;
use monolith_ui_kit::spinner;
use monolith_ui_kit::text::Text;
use monolith_ui_kit::text_field::{self, TextField};
use monolith_ui_kit::theme::{self, TextToken, Theme};
use monolith_ui_kit::{Button, icons};
use monolith_ui_model::Phase;
use zeroize::{Zeroize, Zeroizing};

use crate::app::App;

/// Salts of the egui ids of the secret fields.
const WELCOME_PASSPHRASE: &str = "welcome-passphrase";
const WELCOME_AGAIN: &str = "welcome-passphrase-again";
const UNLOCK_PASSPHRASE: &str = "unlock-passphrase";

/// The size a secret buffer starts with, so that typing does not move it.
const SECRET_CAPACITY: usize = 256;

/// The most characters a passphrase field takes. The vault takes at most
/// 1024 bytes, so no passphrase it would take is cut.
const PASSPHRASE_LIMIT: usize = 1024;

/// Draws the screen of the current phase.
pub(crate) fn show(ctx: &Context, app: &mut App) {
    let phase = app.model.phase();
    page(ctx, |ui| match phase {
        Phase::Welcome => welcome(ui, app),
        Phase::Locked => locked(ui, app),
        Phase::Opening => opening(ui),
        Phase::Stopped => stopped(ui),
        Phase::Starting | Phase::Ready => starting(ui),
    });
}

// ---- Layout ----

/// The window: `bg_app`, and a centred column that `add` fills.
fn page(ctx: &Context, add: impl FnOnce(&mut Ui)) {
    let theme = theme::current(ctx);
    let frame = Frame::new().fill(theme.color.bg_app);
    egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
        chrome::scroll_area("onboarding-scroll").show(ui, |ui| {
            let area = ui.available_rect_before_wrap();
            let width = theme
                .size
                .modal
                .min(area.width() - theme.space.xxl * 2.0)
                .max(0.0);
            // Centre vertically with the height the column had last
            // frame.
            let key = Id::new("monolith-onboarding-height");
            let known: f32 = ctx.data(|data| data.get_temp(key)).unwrap_or(0.0);
            let top = ((area.height() - known) / 2.0).max(theme.space.xxl);
            let rect = egui::Rect::from_min_size(
                egui::pos2(area.center().x - width / 2.0, area.top() + top),
                Vec2::new(width, area.height()),
            );
            let used = ui
                .scope_builder(
                    UiBuilder::new()
                        .max_rect(rect)
                        .layout(Layout::top_down(Align::Min)),
                    |ui| {
                        ui.spacing_mut().item_spacing.y = theme.space.sm;
                        add(ui);
                        ui.add_space(theme.space.xxl);
                    },
                )
                .response
                .rect
                .height();
            if (used - known).abs() > 0.5 {
                ctx.data_mut(|data| data.insert_temp(key, used));
                // Laid out again with the height now known, before the
                // misplaced pass is shown; a frame later if egui has
                // already made all the passes it allows.
                ctx.request_discard("the first screen changed height");
                if !ctx.will_discard() {
                    ctx.request_repaint();
                }
            }
        });
    });
}

/// A line of text in a style and a colour.
fn text(ui: &mut Ui, content: &str, token: &TextToken, color: egui::Color32) {
    ui.add(Text::new(content, token, color));
}

/// A centred line of text.
fn centered(ui: &mut Ui, content: &str, token: &TextToken, color: egui::Color32) {
    ui.vertical_centered(|ui| text(ui, content, token, color));
}

/// The wordmark and the line under it.
fn wordmark(ui: &mut Ui, theme: &Theme) {
    ui.add_space(theme.space.xs);
    centered(ui, "Monolith", &theme.text.display, theme.color.text);
    centered(
        ui,
        "Private messages over Tor",
        &theme.text.body,
        theme.color.text_secondary,
    );
    ui.add_space(theme.space.xxl);
}

/// A progress mark, centred.
fn progress(ui: &mut Ui, theme: &Theme) {
    ui.vertical_centered(|ui| {
        let size = theme.size.icon_lg;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        spinner::paint(ui, rect.center(), size, theme.color.accent);
    });
}

/// The card around a form.
fn card(ui: &mut Ui, theme: &Theme, add: impl FnOnce(&mut Ui)) {
    chrome::card_frame(ui.ctx(), theme.space.xxl).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = theme.space.sm;
        add(ui);
    });
}

/// Small print under the card.
fn small_print(ui: &mut Ui, theme: &Theme, content: &str) {
    centered(ui, content, &theme.text.caption, theme.color.text_tertiary);
}

/// The first lines of the card of the first start: the wordmark above it
/// already says what Monolith is.
fn introduction(ui: &mut Ui, theme: &Theme) {
    text(ui, "Welcome", &theme.text.title, theme.color.text);
    text(
        ui,
        "There are no servers and no accounts: your identity is a key kept on this \
         device, and only the people you add can write to you.",
        &theme.text.body,
        theme.color.text_secondary,
    );
}

/// The way to start without the profile on disk, and what it means.
fn use_without_saving(ui: &mut Ui, theme: &Theme, label: &str) -> bool {
    let clicked = ui
        .vertical_centered(|ui| {
            ui.add(
                Button::ghost(label)
                    .icon(icons::CLOUD_OFF)
                    .size(Size::Medium),
            )
            .clicked()
        })
        .inner;
    small_print(ui, theme, "Nothing is kept when you close Monolith.");
    clicked
}

/// Where the profile is kept, said only when asked: the path is of use to
/// few, and reads as noise to the rest.
fn location(ui: &mut Ui, app: &mut App, theme: &Theme) {
    ui.add_space(theme.space.md);
    let label = if app.show_location {
        "Hide where my profile is kept"
    } else {
        "Where is my profile kept?"
    };
    let clicked = ui
        .vertical_centered(|ui| ui.add(Button::ghost(label).size(Size::Small)).clicked())
        .inner;
    if clicked {
        app.show_location = !app.show_location;
    }
    if app.show_location {
        small_print(ui, theme, app.model.location());
    }
}

// ---- Secrets ----

/// The egui id of the editing area of the secret field `salt`.
fn edit_id(salt: &'static str) -> Id {
    let mut none = String::new();
    TextField::new(salt, &mut none).edit_id()
}

/// Empties a secret buffer and gives it room again.
fn wipe(secret: &mut Zeroizing<String>) {
    secret.zeroize();
    *secret = Zeroizing::new(String::with_capacity(SECRET_CAPACITY));
}

/// Takes the typed passphrase out of the window, for a command.
fn take_passphrase(secret: &mut Zeroizing<String>) -> Zeroizing<String> {
    core::mem::replace(
        secret,
        Zeroizing::new(String::with_capacity(SECRET_CAPACITY)),
    )
}

/// Drops everything typed and everything egui kept of it.
fn forget_all(ctx: &Context, app: &mut App) {
    wipe(&mut app.passphrase);
    wipe(&mut app.passphrase_again);
    for salt in [WELCOME_PASSPHRASE, WELCOME_AGAIN, UNLOCK_PASSPHRASE] {
        text_field::forget(ctx, edit_id(salt));
    }
}

/// Whether the first field of a form should take focus: nothing else has
/// it.
fn wants_focus(ctx: &Context) -> bool {
    ctx.memory(|memory| memory.focused().is_none())
}

// ---- Screens ----

fn starting(ui: &mut Ui) {
    let theme = theme::current(ui.ctx());
    wordmark(ui, &theme);
    progress(ui, &theme);
}

fn opening(ui: &mut Ui) {
    let theme = theme::current(ui.ctx());
    wordmark(ui, &theme);
    progress(ui, &theme);
    ui.add_space(theme.space.sm);
    centered(
        ui,
        "Opening your profile...",
        &theme.text.body_strong,
        theme.color.text,
    );
    centered(
        ui,
        "Unlocking can take a few seconds.",
        &theme.text.caption,
        theme.color.text_tertiary,
    );
}

fn stopped(ui: &mut Ui) {
    let theme = theme::current(ui.ctx());
    wordmark(ui, &theme);
    card(ui, &theme, |ui| {
        text(
            ui,
            "Monolith has stopped.",
            &theme.text.title,
            theme.color.text,
        );
        text(
            ui,
            "Close this window and start Monolith again.",
            &theme.text.body,
            theme.color.text_secondary,
        );
    });
}

/// The first start where there is no folder to keep a profile in: on
/// Tails before the Persistent Storage is unlocked
/// (`docs/PLATFORM_TAILS.md` section 5), or with no home folder. Only a
/// session that keeps nothing can start.
fn no_place(ui: &mut Ui, app: &mut App) {
    let ctx = ui.ctx().clone();
    let theme = theme::current(&ctx);
    wordmark(ui, &theme);
    let mut start = false;
    card(ui, &theme, |ui| {
        introduction(ui, &theme);
        ui.add_space(theme.space.md);
        Banner::new(
            "no-profile-folder",
            Tone::Info,
            "There is no folder here where a profile would last. On Tails, unlock the \
             Persistent Storage when Tails starts, then start Monolith again.",
        )
        .show(ui);
        ui.add_space(theme.space.sm);
        start = ui
            .add(
                Button::primary("Start without saving")
                    .size(Size::Large)
                    .full_width(true),
            )
            .clicked();
        text(
            ui,
            "Identities, contacts and messages of this session are gone when you close Monolith.",
            &theme.text.caption,
            theme.color.text_tertiary,
        );
    });
    if start {
        forget_all(&ctx, app);
        app.model.start_without_saving();
    }
}

fn welcome(ui: &mut Ui, app: &mut App) {
    if app.model.location().is_empty() {
        no_place(ui, app);
        return;
    }
    let ctx = ui.ctx().clone();
    let theme = theme::current(&ctx);
    let key = Id::new("monolith-welcome-differ");
    wordmark(ui, &theme);
    let mut create = false;
    card(ui, &theme, |ui| {
        introduction(ui, &theme);
        ui.add_space(theme.space.md);
        text(
            ui,
            "Create a profile",
            &theme.text.heading,
            theme.color.text,
        );
        let focus = wants_focus(&ctx);
        let first = TextField::new(WELCOME_PASSPHRASE, &mut *app.passphrase)
            .label("Passphrase")
            .secret(true)
            .char_limit(PASSPHRASE_LIMIT)
            .request_focus(focus)
            .show(ui);
        ui.add_space(theme.space.xs);
        let second = TextField::new(WELCOME_AGAIN, &mut *app.passphrase_again)
            .label("Repeat the passphrase")
            .secret(true)
            .char_limit(PASSPHRASE_LIMIT)
            .show(ui);
        let mut differ: bool = ctx.data(|data| data.get_temp(key)).unwrap_or(false);
        if first.changed || second.changed {
            differ = false;
        }
        if differ {
            ui.add(InlineError::new("The two passphrases differ."));
        } else if let Some(failure) = app.model.profile_error() {
            ui.add(InlineError::new(failure.message()));
        }
        text(
            ui,
            "Your identities, contacts and messages are kept encrypted on this device under \
             this passphrase. If you lose it, nothing can be recovered.",
            &theme.text.caption,
            theme.color.text_tertiary,
        );
        ui.add_space(theme.space.sm);
        let filled = !app.passphrase.is_empty() && !app.passphrase_again.is_empty();
        let clicked = ui
            .add(
                Button::primary("Create profile")
                    .size(Size::Large)
                    .full_width(true)
                    .enabled(filled),
            )
            .clicked();
        if (clicked || first.submitted || second.submitted) && filled {
            if app.passphrase.as_str() == app.passphrase_again.as_str() {
                create = true;
                differ = false;
            } else {
                differ = true;
            }
        }
        ctx.data_mut(|data| data.insert_temp(key, differ));
    });
    ui.add_space(theme.space.lg);
    let without_saving = use_without_saving(ui, &theme, "Use without saving");
    location(ui, app, &theme);
    if create {
        let passphrase = take_passphrase(&mut app.passphrase);
        app.model.create_profile(passphrase);
        forget_all(&ctx, app);
    } else if without_saving {
        forget_all(&ctx, app);
        app.model.start_without_saving();
    }
}

fn locked(ui: &mut Ui, app: &mut App) {
    let ctx = ui.ctx().clone();
    let theme = theme::current(&ctx);
    wordmark(ui, &theme);
    let mut unlock = false;
    card(ui, &theme, |ui| {
        text(
            ui,
            "Unlock your profile",
            &theme.text.title,
            theme.color.text,
        );
        text(
            ui,
            "Enter the passphrase you chose for it.",
            &theme.text.body,
            theme.color.text_secondary,
        );
        ui.add_space(theme.space.sm);
        let focus = wants_focus(&ctx);
        let field = TextField::new(UNLOCK_PASSPHRASE, &mut *app.passphrase)
            .label("Passphrase")
            .secret(true)
            .char_limit(PASSPHRASE_LIMIT)
            .request_focus(focus)
            .show(ui);
        if let Some(failure) = app.model.profile_error() {
            ui.add(InlineError::new(failure.message()));
        }
        ui.add_space(theme.space.sm);
        let filled = !app.passphrase.is_empty();
        let clicked = ui
            .add(
                Button::primary("Unlock")
                    .size(Size::Large)
                    .full_width(true)
                    .enabled(filled),
            )
            .clicked();
        unlock = (clicked || field.submitted) && filled;
    });
    ui.add_space(theme.space.lg);
    let without_saving = use_without_saving(ui, &theme, "Use without saving instead");
    small_print(
        ui,
        &theme,
        "Your profile is not touched; start Monolith again to open it.",
    );
    location(ui, app, &theme);
    if unlock {
        let passphrase = take_passphrase(&mut app.passphrase);
        app.model.unlock(passphrase);
        forget_all(&ctx, app);
    } else if without_saving {
        forget_all(&ctx, app);
        app.model.start_without_saving();
    }
}

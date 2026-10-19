//! The screens before a profile is open: the first start, the passphrase,
//! the progress of opening, and a stopped runtime.

use egui::{Align, Context, Frame, Id, Layout, Sense, Ui, UiBuilder, Vec2};
use monolith_ui_kit::banner::{Banner, InlineError, Tone};
use monolith_ui_kit::button::Size;
use monolith_ui_kit::chrome;
use monolith_ui_kit::mark::Wordmark;
use monolith_ui_kit::spinner;
use monolith_ui_kit::text::Text;
use monolith_ui_kit::text_field::{self, TextField};
use monolith_ui_kit::theme::{self, TextToken, Theme};
use monolith_ui_kit::{Button, icons};
use monolith_ui_model::present::MAX_PASSPHRASE_CHARS;
use monolith_ui_model::{Failure, Phase};
use zeroize::{Zeroize, Zeroizing};

use crate::app::{App, SECRET_CAPACITY};

/// Salts of the egui ids of the secret fields.
const WELCOME_PASSPHRASE: &str = "welcome-passphrase";
const WELCOME_AGAIN: &str = "welcome-passphrase-again";
const UNLOCK_PASSPHRASE: &str = "unlock-passphrase";

/// Failures that belong to the passphrase field; the others are of the
/// profile folder, and are said under the form.
const PASSPHRASE_ERRORS: &[Failure] = &[Failure::WrongPassphrase, Failure::UnusablePassphrase];

/// The failure of the last attempt, split: the sentence of one that
/// belongs to the passphrase field, and any other.
fn split_error(failure: Option<Failure>) -> (Option<&'static str>, Option<Failure>) {
    match failure {
        Some(failure) if PASSPHRASE_ERRORS.contains(&failure) => (Some(failure.message()), None),
        other => (None, other),
    }
}

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

/// Whether the window is shorter than the first window. The first screens
/// then keep less room around the wordmark and inside the card, so that
/// the main action is in view in the smallest window, and both actions of
/// the first start in a window of 600 points.
fn short(ctx: &Context) -> bool {
    let theme = theme::current(ctx);
    ctx.screen_rect().height() < theme.size.window_default.y
}

/// The window: `bg_app`, and a centred column that `add` fills.
fn page(ctx: &Context, add: impl FnOnce(&mut Ui)) {
    let theme = theme::current(ctx);
    let frame = Frame::new().fill(theme.color.bg_app);
    let margin = if short(ctx) {
        theme.space.sm
    } else {
        theme.space.xxl
    };
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
            let top = ((area.height() - known) / 2.0).max(margin);
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
    let short = short(ui.ctx());
    if !short {
        ui.add_space(theme.space.xs);
    }
    ui.add(Wordmark::new("Monolith"));
    centered(
        ui,
        "Private messages over Tor",
        &theme.text.body,
        theme.color.text_secondary,
    );
    ui.add_space(if short {
        theme.space.md
    } else {
        theme.space.xxl
    });
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
    let padding = if short(ui.ctx()) {
        theme.space.lg
    } else {
        theme.space.xxl
    };
    chrome::card_frame(ui.ctx(), padding).show(ui, |ui| {
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
         device, and only people you accept can send you messages.",
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
/// Tails before the Persistent Storage is unlocked or with a data
/// directory named outside it (`docs/PLATFORM_TAILS.md` section 5), or
/// with no home folder. Only a session that keeps nothing can start.
fn no_place(ui: &mut Ui, app: &mut App) {
    let ctx = ui.ctx().clone();
    let theme = theme::current(&ctx);
    wordmark(ui, &theme);
    let mut start = false;
    let why = if app.data_dir_refused {
        "The data folder Monolith was started with is not in the unlocked Persistent \
         Storage, where a profile would last. Start Monolith again with a folder inside it."
    } else {
        "There is no folder here where a profile would last. On Tails, unlock the \
         Persistent Storage when Tails starts, then start Monolith again."
    };
    card(ui, &theme, |ui| {
        introduction(ui, &theme);
        ui.add_space(theme.space.md);
        Banner::new("no-profile-folder", Tone::Info, why).show(ui);
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
        // That the two passphrases differ is said under the second until
        // either changes, in place of the failure of the last attempt; a
        // failure of the passphrase is said under the first.
        let shown: bool = ctx.data(|data| data.get_temp(key)).unwrap_or(false);
        let (field_error, error) = split_error(app.model.profile_error());
        let first = TextField::new(WELCOME_PASSPHRASE, &mut *app.passphrase)
            .label("Passphrase")
            .help(Some("Use a long phrase of several words."))
            .error(field_error.filter(|_| !shown))
            .secret(true)
            .char_limit(MAX_PASSPHRASE_CHARS)
            .request_focus(focus)
            .show(ui);
        ui.add_space(theme.space.xs);
        let second = TextField::new(WELCOME_AGAIN, &mut *app.passphrase_again)
            .label("Repeat the passphrase")
            .error(shown.then_some("The two passphrases differ."))
            .secret(true)
            .char_limit(MAX_PASSPHRASE_CHARS)
            .show(ui);
        if let Some(failure) = error.filter(|_| !shown) {
            ui.add(InlineError::new(failure.message()));
        }
        let mut differ = shown && !first.changed && !second.changed;
        text(
            ui,
            "Your identities, contacts and messages are kept encrypted on this device under \
             this passphrase. Nothing can be recovered without it.",
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
        if differ != shown {
            // Drawn as it is now in the next frame.
            ctx.request_repaint();
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
        let (field_error, error) = split_error(app.model.profile_error());
        let field = TextField::new(UNLOCK_PASSPHRASE, &mut *app.passphrase)
            .label("Passphrase")
            .error(field_error)
            .secret(true)
            .char_limit(MAX_PASSPHRASE_CHARS)
            .request_focus(focus)
            .show(ui);
        if let Some(failure) = error {
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

#[cfg(test)]
mod tests {
    use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect};
    use monolith_ui_kit::{fonts, theme::Theme};
    use monolith_ui_model::{Model, ProfileOnDisk, Update};

    use super::*;

    /// A frame of the window in `ctx`, with `events`.
    fn frame(ctx: &Context, app: &mut App, time: &mut f64, events: Vec<Event>) {
        *time += 0.1;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                Theme::light().size.window_default,
            )),
            time: Some(*time),
            focused: true,
            events,
            ..RawInput::default()
        };
        let _ = ctx.run(input, |ctx| crate::show(ctx, app));
    }

    #[test]
    fn a_passphrase_typed_or_pasted_to_the_limit_never_moves_its_buffer() {
        let ctx = Context::default();
        fonts::install(&ctx, &[]);
        theme::install(&ctx, Theme::light().with_reduced_motion(true));
        let mut model = Model::new();
        model.apply(Update::Started {
            profile: ProfileOnDisk::Absent,
            location: "/home/user/.local/share/monolith".to_owned(),
        });
        let mut app = App::for_tests(model, 0);
        let mut time = 0.0;
        for _ in 0..3 {
            frame(&ctx, &mut app, &mut time, Vec::new());
        }
        let first = app.passphrase.as_ptr();
        let again = app.passphrase_again.as_ptr();
        // Characters of four bytes, typed a few at a time into the first
        // field, past its limit.
        let four = "\u{1D49C}";
        for _ in 0..30 {
            frame(
                &ctx,
                &mut app,
                &mut time,
                vec![Event::Text(four.repeat(10))],
            );
        }
        assert_eq!(app.passphrase.as_ptr(), first);
        assert_eq!(app.passphrase.chars().count(), 256);
        assert_eq!(app.passphrase.len(), 1024);
        // Pasted into the second, past its limit.
        let tab = Event::Key {
            key: Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        frame(&ctx, &mut app, &mut time, vec![tab]);
        frame(
            &ctx,
            &mut app,
            &mut time,
            vec![Event::Paste(four.repeat(300))],
        );
        assert_eq!(app.passphrase_again.as_ptr(), again);
        assert_eq!(app.passphrase_again.chars().count(), 256);
    }
}

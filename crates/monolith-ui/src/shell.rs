//! The window: which screen is shown, the layout of the main screen
//! (sidebar and content, or one of them in a narrow window), the global
//! notices, the dialogs and the toasts.

use egui::{Context, Key, KeyboardShortcut, Margin, Modifiers, Popup, TopBottomPanel, Vec2};
use monolith_ui_kit::banner::{Banner, Tone};
use monolith_ui_kit::chrome::{self, ContentPanel, SidebarPanel};
use monolith_ui_kit::theme;
use monolith_ui_model::{Phase, Selection};

use crate::app::{App, SettingsPage, View};
use crate::{conversation, dialogs, onboarding, requests, settings, sidebar};

/// Draws the window for this frame.
pub fn show(ctx: &Context, app: &mut App) {
    app.pump_notices();
    for contact in app.model.take_forgotten() {
        conversation::forget(ctx, contact);
    }
    match app.model.phase() {
        Phase::Ready => main(ctx, app),
        Phase::Starting | Phase::Welcome | Phase::Locked | Phase::Opening | Phase::Stopped => {
            onboarding::show(ctx, app);
        }
    }
    dialogs::show(ctx, app);
    let _ = app.toasts.show(ctx);
}

fn main(ctx: &Context, app: &mut App) {
    shortcuts(ctx, app);
    if app.view != View::Settings(SettingsPage::Tor) {
        // "Saved" is said on the visit of the Tor page that saved.
        app.tor_saved = false;
    }
    storage_failed(ctx, app);
    if let View::Settings(page) = app.view {
        settings::show(ctx, app, page);
        return;
    }
    let theme = theme::current(ctx);
    let narrow = ctx.screen_rect().width() < theme.size.narrow_below;
    let selected = app.model.selection();
    if narrow {
        if app.narrow_conversation && selected.is_some() {
            ContentPanel.show(ctx, |ui| content(ui, app));
        } else {
            app.narrow_conversation = false;
            ContentPanel.show(ctx, |ui| sidebar::show(ui, app, true));
        }
        return;
    }
    SidebarPanel::new("sidebar").show(ctx, |ui| sidebar::show(ui, app, false));
    ContentPanel.show(ctx, |ui| content(ui, app));
}

/// A store that failed, said once for the whole window: across its top, on
/// every view, until Monolith starts again.
fn storage_failed(ctx: &Context, app: &App) {
    if !app.model.storage_failed() {
        return;
    }
    let theme = theme::current(ctx);
    TopBottomPanel::top("storage-failed")
        .frame(chrome::surface_frame(
            ctx,
            Margin::from(Vec2::new(theme.space.lg, theme.space.sm)),
        ))
        .show_separator_line(false)
        .show(ctx, |ui| {
            Banner::new(
                "storage-failed",
                Tone::Danger,
                "Monolith can no longer save. Restart Monolith to continue.",
            )
            .show(ui);
        });
}

/// The content pane: the conversation, the requests, or what to do next.
fn content(ui: &mut egui::Ui, app: &mut App) {
    // The bar at the top ends where the identity card of the sidebar ends.
    ui.add_space(theme::current(ui.ctx()).space.sm);
    match app.model.selection() {
        Some(Selection::Conversation(contact)) => conversation::show(ui, app, contact),
        Some(Selection::Requests) => requests::show(ui, app),
        None => conversation::show_nothing_selected(ui, app),
    }
}

/// The keyboard shortcuts of the main window, while no dialog is open and
/// closing is not asked about: Ctrl+F (Cmd+F on macOS) for the filter of
/// the list, Alt+Up and Alt+Down for the conversation before or after the
/// one shown, Escape to leave the settings. Enter sends, Shift+Enter starts a new line, and Escape
/// closes a dialog or a menu, in their components.
fn shortcuts(ctx: &Context, app: &mut App) {
    if app.model.dialog().is_some() || app.block_request.is_some() || app.asking_to_close() {
        return;
    }
    let find = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);
    let previous = KeyboardShortcut::new(Modifiers::ALT, Key::ArrowUp);
    let next = KeyboardShortcut::new(Modifiers::ALT, Key::ArrowDown);
    let in_settings = matches!(app.view, View::Settings(_));
    // Escape belongs to an open menu first.
    let menu = Popup::is_any_open(ctx);
    let (find, previous, next, leave) = ctx.input_mut(|input| {
        (
            input.consume_shortcut(&find),
            input.consume_shortcut(&previous),
            input.consume_shortcut(&next),
            in_settings && !menu && input.consume_key(Modifiers::NONE, Key::Escape),
        )
    });
    if leave {
        app.view = View::Main;
    }
    if find {
        app.view = View::Main;
        app.narrow_conversation = false;
        app.focus_search = true;
    }
    if (previous || next) && app.view == View::Main {
        sidebar::step(app, next);
    }
}

//! The window: which screen is shown, the layout of the main screen
//! (sidebar and content, or one of them in a narrow window), the global
//! notices, the dialogs and the toasts.

use egui::Context;
use monolith_ui_kit::chrome::{ContentPanel, SidebarPanel};
use monolith_ui_kit::theme;
use monolith_ui_model::{Phase, Selection};

use crate::app::{App, View};
use crate::{conversation, dialogs, onboarding, requests, settings, sidebar};

/// Draws the window for this frame.
pub fn show(ctx: &Context, app: &mut App) {
    app.pump_notices();
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

/// The content pane: the conversation, the requests, or what to do next.
fn content(ui: &mut egui::Ui, app: &mut App) {
    match app.model.selection() {
        Some(Selection::Conversation(contact)) => conversation::show(ui, app, contact),
        Some(Selection::Requests) => requests::show(ui, app),
        None => conversation::show_nothing_selected(ui, app),
    }
}

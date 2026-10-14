//! The sidebar: the identity shown, the filter, the requests, and the
//! conversations of the identity.

use egui::{
    Align, Align2, CornerRadius, Layout, Margin, Rect, Sense, TopBottomPanel, Ui, UiBuilder, Vec2,
    WidgetType,
};
use monolith_ui_kit::badge::Badge;
use monolith_ui_kit::button::{Button, Size};
use monolith_ui_kit::chrome::{self, scroll_area};
use monolith_ui_kit::empty_state::EmptyState;
use monolith_ui_kit::icon_button::IconButton;
use monolith_ui_kit::identity_switcher::{IdentityItem, IdentitySwitcher};
use monolith_ui_kit::interact::{self, State};
use monolith_ui_kit::rows::{ConversationRow, RowStatus};
use monolith_ui_kit::search_field::SearchField;
use monolith_ui_kit::{icons, theme};
use monolith_ui_model::{Dialog, RowState, Selection, SidebarRow, UiIdentityId, present};

use crate::app::{App, SettingsPage, View};
use crate::common;

/// The longest preview that is laid out; the row cuts it to its width.
const PREVIEW_CHARS: usize = 120;

/// Draws the sidebar; `narrow` when it is the whole window.
pub(crate) fn show(ui: &mut Ui, app: &mut App, narrow: bool) {
    let theme = theme::current(ui.ctx());
    let pad = theme.space.sm;
    TopBottomPanel::bottom("sidebar-footer")
        .frame(
            if narrow {
                // The sidebar is the whole window, on the surface colour.
                chrome::surface_frame(ui.ctx(), Margin::ZERO)
            } else {
                chrome::sidebar_frame(ui.ctx())
            }
            .inner_margin(Margin::from(Vec2::new(
                theme.space.sm + theme.space.xs,
                theme.space.sm,
            ))),
        )
        .show_separator_line(false)
        .show_inside(ui, |ui| footer(ui, app));
    if app.model.current_identity().is_none() {
        // In a wide window the content pane says what to do; the sidebar
        // says it only when it is the whole window.
        if narrow {
            no_identity(ui, app);
        }
        return;
    }
    ui.add_space(pad);
    inset(ui, pad, |ui| switcher(ui, app));
    ui.add_space(pad);
    inset(ui, pad, |ui| {
        SearchField::new("sidebar-search", &mut app.search)
            .hint("Search")
            .compact(true)
            .show(ui);
    });
    ui.add_space(theme.space.xs);
    requests_entry(ui, app, narrow);
    ui.add_space(theme.space.xs);
    conversations(ui, app, narrow);
}

/// Runs `add` in a column narrower than `ui` by `pad` on both sides.
fn inset<R>(ui: &mut Ui, pad: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let mut rect = ui.available_rect_before_wrap();
    rect.min.x += pad;
    rect.max.x = (rect.max.x - pad).max(rect.min.x);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    let result = add(&mut child);
    let used = child.min_rect();
    ui.allocate_rect(
        Rect::from_min_max(
            egui::pos2(rect.min.x - pad, used.min.y),
            egui::pos2(rect.max.x + pad, used.max.y),
        ),
        Sense::hover(),
    );
    result
}

/// "3 contacts".
fn count_text(count: usize) -> String {
    if count == 1 {
        "1 contact".to_owned()
    } else {
        format!("{count} contacts")
    }
}

/// The identity switcher at the top.
fn switcher(ui: &mut Ui, app: &mut App) {
    let Some(current) = app.model.current() else {
        return;
    };
    let names: Vec<(UiIdentityId, String, String)> = app
        .model
        .identities()
        .iter()
        .map(|identity| {
            let detail = app
                .model
                .slice(identity.id)
                .filter(|slice| slice.loaded)
                .map(|slice| count_text(slice.contacts.len()))
                .unwrap_or_default();
            (identity.id, present::identity_name(identity), detail)
        })
        .collect();
    let fingerprints: Vec<&str> = app
        .model
        .identities()
        .iter()
        .map(|identity| identity.fingerprint.as_str())
        .collect();
    let items: Vec<IdentityItem<'_, UiIdentityId>> = names
        .iter()
        .zip(&fingerprints)
        .map(|((id, name, detail), fingerprint)| IdentityItem {
            key: *id,
            name,
            detail,
            seed: common::seed(fingerprint),
        })
        .collect();
    let caption = app
        .model
        .current_slice()
        .filter(|slice| slice.loaded)
        .map(|slice| count_text(slice.contacts.len()))
        .unwrap_or_default();
    let output = IdentitySwitcher::new(
        "sidebar-identity",
        &current,
        &items,
        "New identity",
        "Manage identities",
    )
    .caption(&caption)
    .show(ui);
    let chosen = output.selected;
    let new_identity = output.new_identity;
    let manage = output.manage;
    if let Some(key) = chosen {
        app.model.select_identity(key);
        app.narrow_conversation = false;
    }
    if new_identity {
        app.model.open(Dialog::NewIdentity {
            name: String::new(),
            busy: None,
            error: None,
        });
    }
    if manage {
        app.view = View::Settings(SettingsPage::Identities);
    }
}

/// The entry that opens the waiting requests, when there are any.
fn requests_entry(ui: &mut Ui, app: &mut App, narrow: bool) {
    let Some(identity) = app.model.current() else {
        return;
    };
    let waiting = app
        .model
        .current_slice()
        .map_or(0, |slice| slice.requests.len());
    let selected = app.model.selection() == Some(Selection::Requests);
    if waiting == 0 {
        return;
    }
    let theme = theme::current(ui.ctx());
    let c = &theme.color;
    let (_, outer) = ui.allocate_space(Vec2::new(ui.available_width(), theme.size.control));
    let rect = outer.shrink2(Vec2::new(theme.space.sm, 0.0));
    let id = egui::Id::new(("sidebar-requests", identity));
    let response = ui.interact(rect, id, Sense::click());
    interact::describe_selectable(&response, WidgetType::SelectableLabel, "Requests", selected);
    if ui.is_rect_visible(rect) {
        let state = State::of(ui, &response, id, &theme, selected);
        let radius = CornerRadius::same(theme.radius.lg);
        let painter = ui.painter();
        if selected {
            painter.rect_filled(rect, radius, c.selected);
        }
        let tint = if state.pressed {
            c.pressed
        } else {
            c.hover.gamma_multiply(state.hover)
        };
        if tint.a() > 0 {
            painter.rect_filled(rect, radius, tint);
        }
        let inner = rect.shrink2(Vec2::new(theme.space.md, 0.0));
        // The icon sits in the column of the avatars.
        let icon_center = egui::pos2(inner.left() + theme.size.avatar / 2.0, inner.center().y);
        icons::INBOX.paint(painter, icon_center, theme.size.icon_md, c.accent_text);
        painter.text(
            egui::pos2(
                inner.left() + theme.size.avatar + theme.space.md,
                inner.center().y,
            ),
            Align2::LEFT_CENTER,
            "Requests",
            theme.text.body_strong.font_id(),
            c.text,
        );
        let mut badge_ui = ui.new_child(
            UiBuilder::new()
                .max_rect(inner)
                .layout(Layout::right_to_left(Align::Center)),
        );
        badge_ui.add(Badge::new(u32::try_from(waiting).unwrap_or(u32::MAX)));
        if state.focused {
            interact::focus_ring(ui.painter(), rect, radius, &theme);
        }
    }
    if response.clicked() {
        app.model.show_requests();
        if narrow {
            app.narrow_conversation = true;
        }
    }
}

/// What the preview of a row says.
fn preview_of(row: &SidebarRow) -> (String, bool) {
    match &row.latest {
        Some(message) => (
            present::preview(&message.text, PREVIEW_CHARS),
            message.status.outgoing(),
        ),
        None => {
            let text = match row.state {
                RowState::Ended => "No longer a contact",
                RowState::Waiting => "Waiting for them to accept",
                RowState::Contact => "No messages yet",
            };
            (text.to_owned(), false)
        }
    }
}

/// The scrolling list of conversations, or what stands in for it.
fn conversations(ui: &mut Ui, app: &mut App, narrow: bool) {
    let Some(slice) = app.model.current_slice() else {
        return;
    };
    if !slice.loaded {
        return;
    }
    let all = app.model.sidebar();
    if all.is_empty() {
        no_contacts(ui, app);
        return;
    }
    let filter = app.search.trim().to_lowercase();
    let rows: Vec<SidebarRow> = all
        .into_iter()
        .filter(|row| filter.is_empty() || row.name.to_lowercase().contains(&filter))
        .collect();
    if rows.is_empty() {
        no_match(ui);
        return;
    }
    let selection = app.model.selection();
    let theme = theme::current(ui.ctx());
    let mut clicked = None;
    scroll_area("sidebar-rows").show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for row in &rows {
            let (preview, outgoing) = preview_of(row);
            let time = row
                .latest
                .as_ref()
                .map(|message| present::short_time(&*app.calendar, message.time))
                .unwrap_or_default();
            let mut statuses = Vec::new();
            match row.state {
                RowState::Waiting => statuses.push(RowStatus::WaitingForAcceptance),
                RowState::Ended => statuses.push(RowStatus::Blocked),
                RowState::Contact => {}
            }
            if row.queued > 0 {
                statuses.push(RowStatus::Queued);
            }
            let mut widget =
                ConversationRow::new(row.contact, &row.name, common::seed(&row.fingerprint))
                    .preview(&preview)
                    .timestamp(&time)
                    .unread(row.unread)
                    .statuses(&statuses)
                    .selected(selection == Some(Selection::Conversation(row.contact)));
            if outgoing {
                widget = widget.outgoing("You: ");
            }
            if ui.add(widget).clicked() {
                clicked = Some(row.contact);
            }
        }
        ui.add_space(theme.space.sm);
    });
    if let Some(contact) = clicked {
        app.model.select_conversation(contact);
        if narrow {
            app.narrow_conversation = true;
        }
    }
}

/// The empty list of an identity without contacts.
fn no_contacts(ui: &mut Ui, app: &mut App) {
    let Some(identity) = app.model.current() else {
        return;
    };
    let output = EmptyState::new("sidebar-no-contacts", icons::USERS, "No contacts yet")
        .body("Add someone using their Monolith card or invitation.")
        .primary("Add contact")
        .secondary("Invite")
        .show(ui);
    if output.primary {
        open_add_contact(app, identity);
    }
    if output.secondary {
        open_invitation(app, identity);
    }
}

/// The filter matches nothing.
fn no_match(ui: &mut Ui) {
    EmptyState::new("sidebar-no-match", icons::SEARCH, "No matches")
        .body("No contact has a name like that.")
        .show(ui);
}

/// The sidebar before the first identity exists.
fn no_identity(ui: &mut Ui, app: &mut App) {
    let output = EmptyState::new(
        "sidebar-no-identity",
        icons::USER_PLUS,
        "Create your first identity",
    )
    .body("An identity is what your contacts know you by.")
    .primary("Create identity")
    .show(ui);
    if output.primary {
        app.model.open(Dialog::NewIdentity {
            name: String::new(),
            busy: None,
            error: None,
        });
    }
}

fn open_add_contact(app: &mut App, identity: UiIdentityId) {
    app.model.open(Dialog::AddContact {
        identity,
        card: String::new(),
        name: String::new(),
        busy: None,
        error: None,
    });
}

fn open_invitation(app: &mut App, identity: UiIdentityId) {
    app.model.open(Dialog::NewInvitation {
        identity,
        label: String::new(),
        created: None,
        busy: None,
        error: None,
    });
}

/// The quiet strip at the bottom: add a contact, invite, settings.
fn footer(ui: &mut Ui, app: &mut App) {
    let theme = theme::current(ui.ctx());
    let identity = app.model.current();
    let mut add = false;
    let mut invite = false;
    let mut settings = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme.space.xs;
        let enabled = identity.is_some();
        add = ui
            .add(
                Button::ghost("Add contact")
                    .icon(icons::USER_PLUS)
                    .size(Size::Small)
                    .enabled(enabled),
            )
            .clicked();
        invite = ui
            .add(
                Button::ghost("Invite")
                    .icon(icons::TICKET)
                    .size(Size::Small)
                    .enabled(enabled),
            )
            .clicked();
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            settings = ui
                .add(IconButton::new(icons::SETTINGS, "Settings").small())
                .clicked();
        });
    });
    if let Some(identity) = identity {
        if add {
            open_add_contact(app, identity);
        }
        if invite {
            open_invitation(app, identity);
        }
    }
    if settings {
        app.view = View::Settings(SettingsPage::Appearance);
    }
}

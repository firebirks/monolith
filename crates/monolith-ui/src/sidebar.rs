//! The sidebar: the identity shown, the filter, the requests, and the
//! conversations of the identity.

use egui::{
    Align, Align2, CornerRadius, Layout, Margin, Rect, Sense, TopBottomPanel, Ui, UiBuilder, Vec2,
    WidgetType,
};
use monolith_ui_kit::badge::Badge;
use monolith_ui_kit::badge::{Pill, Tone as PillTone};
use monolith_ui_kit::button::{Button, Size};
use monolith_ui_kit::chrome::{self, TopBar, scroll_area};
use monolith_ui_kit::empty_state::EmptyState;
use monolith_ui_kit::icon_button::IconButton;
use monolith_ui_kit::identity_switcher::{self, IdentityItem, IdentitySwitcher};
use monolith_ui_kit::interact::{self, State};
use monolith_ui_kit::rows::{ConversationRow, RowStatus};
use monolith_ui_kit::search_field::SearchField;
use monolith_ui_kit::text::Text;
use monolith_ui_kit::tor_status::TorStatus;
use monolith_ui_kit::{icons, theme};
use monolith_ui_model::{Dialog, RowState, Selection, SidebarRow, Slice, UiIdentityId, present};

use crate::app::{App, SettingsPage, View};
use crate::{common, conversation};

/// The key of the identity switcher.
const SWITCHER: &str = "sidebar-identity";

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
    let mut open_tor = false;
    if app.model.current_identity().is_none() {
        // In a wide window the content pane says what to do; the sidebar
        // says it only when it is the whole window, under the state of Tor.
        if narrow {
            ui.add_space(pad);
            let (tor, label) = common::tor_state(&app.model);
            TopBar::new("sidebar-bar").show(
                ui,
                |_| {},
                |ui| open_tor = ui.add(TorStatus::new(tor, label)).clicked(),
            );
            no_identity(ui, app);
        }
        open_tor_details(app, open_tor);
        return;
    }
    ui.add_space(pad);
    inset(ui, pad, |ui| {
        if narrow {
            // The window has no other place that says whether Tor works.
            open_tor = switcher_and_tor(ui, app);
        } else {
            switcher(ui, app);
        }
    });
    open_tor_details(app, open_tor);
    if app.model.ephemeral() {
        // Said for the whole session (docs/PLATFORM_TAILS.md section 5).
        ui.add_space(theme.space.sm);
        inset(ui, pad, |ui| {
            ui.add(
                Pill::new(PillTone::Warning, "This session saves nothing").icon(icons::CLOUD_OFF),
            );
        });
    }
    ui.add_space(pad);
    inset(ui, pad, |ui| {
        SearchField::new("sidebar-search", &mut app.search)
            .hint("Filter contacts")
            .compact(true)
            .request_focus(core::mem::take(&mut app.focus_search))
            .show(ui);
    });
    ui.add_space(theme.space.xs);
    requests_entry(ui, app, narrow);
    // The list starts a little under what is above it, so that what
    // scrolls is not cut against it.
    ui.add_space(theme.space.sm);
    conversations(ui, app, narrow);
}

fn open_tor_details(app: &mut App, open: bool) {
    if open {
        app.model.open(Dialog::TorDetails);
    }
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
    match count {
        0 => "No contacts".to_owned(),
        1 => "1 contact".to_owned(),
        _ => format!("{count} contacts"),
    }
}

/// The line under an identity in the switcher: its contacts, its unread
/// messages and its waiting requests, such as "4 contacts, 6 new, 1
/// request".
fn detail_text(contacts: usize, unread: u32, requests: usize) -> String {
    let mut parts = vec![count_text(contacts)];
    if unread > 0 {
        parts.push(format!("{unread} new"));
    }
    match requests {
        0 => {}
        1 => parts.push("1 request".to_owned()),
        _ => parts.push(format!("{requests} requests")),
    }
    parts.join(", ")
}

/// The line of an identity whose state was read.
fn detail_of(slice: &Slice) -> String {
    let unread = slice
        .unread
        .values()
        .fold(0_u32, |sum, count| sum.saturating_add(*count));
    detail_text(slice.contacts.len(), unread, slice.requests.len())
}

/// The switcher with the state of Tor at its right, in one band.
/// Returns whether the state of Tor was clicked.
fn switcher_and_tor(ui: &mut Ui, app: &mut App) -> bool {
    let theme = theme::current(ui.ctx());
    let (_, band) = ui.allocate_space(Vec2::new(ui.available_width(), theme.size.topbar));
    let (tor, label) = common::tor_state(&app.model);
    let mut right = ui.new_child(
        UiBuilder::new()
            .max_rect(band)
            .layout(Layout::right_to_left(Align::Center)),
    );
    let clicked = right.add(TorStatus::new(tor, label)).clicked();
    let mut left = band;
    left.max.x = (right.min_rect().left() - theme.space.sm).max(left.min.x);
    let mut rest = ui.new_child(
        UiBuilder::new()
            .max_rect(left)
            .layout(Layout::top_down(Align::Min)),
    );
    switcher(&mut rest, app);
    clicked
}

/// The identity switcher at the top.
fn switcher(ui: &mut Ui, app: &mut App) {
    let Some(current) = app.model.current() else {
        return;
    };
    if core::mem::take(&mut app.open_switcher) {
        identity_switcher::set_open(ui.ctx(), SWITCHER, true);
    }
    let names: Vec<(UiIdentityId, String, String)> = app
        .model
        .identities()
        .iter()
        .map(|identity| {
            let detail = app
                .model
                .slice(identity.id)
                .filter(|slice| slice.loaded)
                .map(detail_of)
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
        SWITCHER,
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
        app.narrow_settings_page = true;
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

/// What the preview of a row says, and whether it is a message of the
/// user's.
fn preview_of(row: &SidebarRow) -> (&str, bool) {
    match &row.latest {
        Some(message) => (&row.preview, message.status.outgoing()),
        None => {
            let text = match (row.state, row.waiting) {
                (RowState::Ended, _) => "No longer a contact",
                // As the conversation and the details say it.
                (_, Some(waiting)) => common::waiting_words(waiting),
                (RowState::Waiting | RowState::Contact, None) => "No messages yet",
            };
            (text, false)
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
    if app.model.sidebar_rows().is_empty() {
        no_contacts(ui, app, narrow);
        return;
    }
    let rows = filtered(app.model.sidebar_rows(), &app.search);
    if rows.is_empty() {
        no_match(ui);
        return;
    }
    let selection = app.model.selection();
    let theme = theme::current(ui.ctx());
    let mut clicked = None;
    // The list ends a little above the strip at the bottom, so that what
    // scrolls is not cut against it.
    let mut area = ui.available_rect_before_wrap();
    area.max.y = (area.max.y - theme.space.sm).max(area.min.y);
    let mut list = ui.new_child(
        UiBuilder::new()
            .id_salt("sidebar-list")
            .max_rect(area)
            .layout(Layout::top_down(Align::Min)),
    );
    scroll_area("sidebar-rows").show(&mut list, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        for row in rows {
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
                    .anonymous(!row.named)
                    .preview(preview)
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

/// The empty list of an identity without contacts. In a wide window the
/// content pane says what to do, and the list only where they will be.
fn no_contacts(ui: &mut Ui, app: &mut App, narrow: bool) {
    let Some(identity) = app.model.current() else {
        return;
    };
    if !narrow {
        let theme = theme::current(ui.ctx());
        inset(ui, theme.space.sm + theme.space.md, |ui| {
            ui.add(
                Text::new(
                    "Your contacts will be listed here.",
                    &theme.text.caption,
                    theme.color.text_tertiary,
                )
                .wrap(),
            );
        });
        return;
    }
    // Invite is in the strip below.
    let output = EmptyState::new("sidebar-no-contacts", icons::USERS, "No contacts yet")
        .body(conversation::NO_CONTACTS)
        .primary("Add contact")
        .secondary("Share my card")
        .show(ui);
    if output.primary {
        open_add_contact(app, identity);
    }
    if output.secondary {
        open_own_card(app, identity);
    }
}

/// The rows whose name holds the filter, in the order of the list. The
/// rows are the model's; nothing of them is copied.
fn filtered<'a>(rows: &'a [SidebarRow], search: &str) -> Vec<&'a SidebarRow> {
    let filter = search.trim().to_lowercase();
    rows.iter().filter(|row| row.matches(&filter)).collect()
}

/// Shows the conversation after (`forward`) or before the one shown, among
/// those the list shows, and gives its composer the focus. With none
/// shown, the first or the last.
pub(crate) fn step(app: &mut App, forward: bool) {
    let rows = filtered(app.model.sidebar_rows(), &app.search);
    let current = match app.model.selection() {
        Some(Selection::Conversation(contact)) => {
            rows.iter().position(|row| row.contact == contact)
        }
        _ => None,
    };
    let target = match (current, forward) {
        (Some(index), true) => index.checked_add(1),
        (Some(index), false) => index.checked_sub(1),
        (None, true) => Some(0),
        (None, false) => rows.len().checked_sub(1),
    };
    let Some(contact) = target
        .and_then(|index| rows.get(index))
        .map(|row| row.contact)
    else {
        return;
    };
    app.model.select_conversation(contact);
    app.narrow_conversation = true;
    app.focus_composer = true;
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
        added: None,
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

fn open_own_card(app: &mut App, identity: UiIdentityId) {
    app.model.open(Dialog::OwnCard {
        identity,
        card: None,
        busy: None,
        error: None,
    });
}

/// The quiet strip at the bottom: add a contact, invite, one's own card,
/// and the settings. Before the first identity, only the settings.
fn footer(ui: &mut Ui, app: &mut App) {
    let theme = theme::current(ui.ctx());
    let identity = app.model.current();
    let mut add = false;
    let mut invite = false;
    let mut card = false;
    let mut settings = false;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme.space.xs;
        if identity.is_some() {
            add = ui
                .add(
                    Button::ghost("Add contact")
                        .icon(icons::USER_PLUS)
                        .size(Size::Small),
                )
                .clicked();
            invite = ui
                .add(
                    Button::ghost("Invite")
                        .icon(icons::TICKET)
                        .size(Size::Small),
                )
                .clicked();
            card = ui
                .add(IconButton::new(icons::ID_CARD, "My card").small())
                .clicked();
        }
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
        if card {
            open_own_card(app, identity);
        }
    }
    if settings {
        app.view = View::Settings(SettingsPage::Appearance);
        app.narrow_settings_page = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identity_in_the_switcher_says_its_contacts_new_messages_and_requests() {
        assert_eq!(detail_text(4, 6, 1), "4 contacts, 6 new, 1 request");
        assert_eq!(detail_text(1, 0, 2), "1 contact, 2 requests");
        assert_eq!(detail_text(0, 0, 0), "No contacts");
        assert_eq!(detail_text(3, 1, 0), "3 contacts, 1 new");
    }
}

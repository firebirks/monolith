//! The settings: a column of pages at the left and the page at the right,
//! or, in a narrow window, the list of pages, then a page with a way back.

use egui::{
    Align, Align2, Color32, Context, Id, Layout, Margin, Rect, Sense, Ui, UiBuilder, Vec2,
    WidgetType,
};
use monolith_ui_kit::avatar::{AvatarSize, IdentityAvatar};
use monolith_ui_kit::banner::{Banner, Tone};
use monolith_ui_kit::button::Size;
use monolith_ui_kit::chrome::{self, ContentPanel, SidebarPanel, TopBar};
use monolith_ui_kit::divider::Divider;
use monolith_ui_kit::icon_button::IconButton;
use monolith_ui_kit::interact::{self, State};
use monolith_ui_kit::invitation_row::InvitationRow;
use monolith_ui_kit::menu::{MenuItem, menu_below};
use monolith_ui_kit::section_header::SectionHeader;
use monolith_ui_kit::segmented::Segmented;
use monolith_ui_kit::settings_row::{SettingsGroup, SettingsRow};
use monolith_ui_kit::text::Text;
use monolith_ui_kit::text_field::TextField;
use monolith_ui_kit::theme::{self, TextToken, Theme};
use monolith_ui_kit::toggle::Toggle;
use monolith_ui_kit::tor_status::TorStatus;
use monolith_ui_kit::{Button, Icon, icons};
use monolith_ui_model::present::identity_name;
use monolith_ui_model::{Dialog, RequestMode, TorCheck, UiBlockedId, UiIdentityId, UiInvitationId};

use crate::app::{App, Effect, SettingsPage, ThemeChoice, View};
use crate::common;

/// How many invitations an identity can have active.
const MAX_INVITATIONS: usize = 16;

/// What blocking does and what unblocking does: the runtime keeps no record
/// of someone unblocked (`DESIGN_QUESTIONS.md` P6-21).
const BLOCKED: &str = "People you blocked cannot send you requests or messages. Unblocking \
                       someone does not make them your contact again: they can send you a \
                       request if \"Who can send you requests\" lets them, and you can add \
                       their card.";

/// What the list of blocked people says when it is empty.
const NOBODY_BLOCKED: &str = "Nobody is blocked.";

const THEMES: [(ThemeChoice, &str); 3] = [
    (ThemeChoice::System, "System"),
    (ThemeChoice::Light, "Light"),
    (ThemeChoice::Dark, "Dark"),
];

const MODES: [(RequestMode, &str); 3] = [
    (RequestMode::Invitation, "Invitations only"),
    (RequestMode::Open, "Anyone with my card"),
    (RequestMode::Closed, "Nobody"),
];

/// The pages: what they are called, their icon, and what the list of a
/// narrow window says of them.
const PAGES: [(SettingsPage, &str, Icon, &str); 5] = [
    (
        SettingsPage::Appearance,
        "Appearance",
        icons::SUN,
        "Theme and motion",
    ),
    (
        SettingsPage::Identities,
        "Identities",
        icons::USERS,
        "Cards, requests and invitations",
    ),
    (
        SettingsPage::Tor,
        "Tor",
        icons::NETWORK,
        "How Monolith reaches Tor",
    ),
    (
        SettingsPage::Data,
        "Data",
        icons::FOLDER_OPEN,
        "Where your profile is kept, and what is stored",
    ),
    (
        SettingsPage::About,
        "About",
        icons::INFO,
        "Version and licences",
    ),
];

/// Where the settings go after this frame.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Next {
    Stay,
    /// Back to the conversations.
    Leave,
    /// A page.
    Page(SettingsPage),
    /// The list of pages of a narrow window.
    Pages,
}

/// Draws the settings at `page`.
pub(crate) fn show(ctx: &Context, app: &mut App, page: SettingsPage) {
    let theme = theme::current(ctx);
    let narrow = ctx.screen_rect().width() < theme.size.narrow_below;
    let mut next = Next::Stay;
    if narrow {
        ContentPanel.show(ctx, |ui| {
            if app.narrow_settings_page {
                page_view(ui, app, page, true, &mut next);
            } else {
                pages_view(ui, &mut next);
            }
        });
    } else {
        SidebarPanel::new("settings-nav")
            .resizable(false)
            .show(ctx, |ui| navigation(ui, &theme, page, &mut next));
        ContentPanel.show(ctx, |ui| page_view(ui, app, page, false, &mut next));
    }
    match next {
        Next::Stay => {}
        Next::Leave => app.view = View::Main,
        Next::Page(chosen) => {
            app.view = View::Settings(chosen);
            app.narrow_settings_page = true;
        }
        Next::Pages => app.narrow_settings_page = false,
    }
}

fn title_of(page: SettingsPage) -> &'static str {
    PAGES
        .iter()
        .find(|(candidate, _, _, _)| *candidate == page)
        .map_or("Settings", |(_, label, _, _)| *label)
}

/// The bar at the top of the settings: `title`, a way back to the list of
/// pages when `back`, and "Done". Returns where it asks to go.
fn top_bar(ui: &mut Ui, theme: &Theme, title: &str, back: bool) -> Next {
    let mut pages = false;
    let mut done = false;
    // The bar ends where the title of the column of pages ends.
    ui.add_space(theme.space.sm);
    TopBar::new("settings-top").show(
        ui,
        |ui| {
            pages = back
                && ui
                    .add(IconButton::new(icons::CHEVRON_LEFT, "Back").id_salt("settings-back"))
                    .clicked();
            ui.add(Text::new(title, &theme.text.title, theme.color.text).truncate());
        },
        |ui| done = ui.add(Button::secondary("Done")).clicked(),
    );
    if done {
        Next::Leave
    } else if pages {
        Next::Pages
    } else {
        Next::Stay
    }
}

/// A scrolling column of the width of a page, centred, with the gutters
/// at its sides; it starts a little under the bar, so that what scrolls is
/// not cut against its hairline.
fn column(ui: &mut Ui, theme: &Theme, id_salt: &str, add: impl FnOnce(&mut Ui)) {
    ui.add_space(theme.space.sm);
    chrome::scroll_area(id_salt).show(ui, |ui| {
        let area = ui.available_rect_before_wrap();
        let width = theme
            .size
            .modal_wide
            .min(area.width() - theme.space.xxl * 2.0)
            .max(0.0);
        let rect = Rect::from_min_size(
            egui::pos2(area.center().x - width / 2.0, area.top() + theme.space.lg),
            Vec2::new(width, area.height()),
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::top_down(Align::Min)),
            |ui| {
                ui.spacing_mut().item_spacing.y = theme.space.sm;
                add(ui);
                ui.add_space(theme.space.xxxl);
            },
        );
    });
}

/// A page, under its bar.
fn page_view(ui: &mut Ui, app: &mut App, page: SettingsPage, narrow: bool, next: &mut Next) {
    let theme = theme::current(ui.ctx());
    let asked = top_bar(ui, &theme, title_of(page), narrow);
    if asked != Next::Stay {
        *next = asked;
    }
    column(ui, &theme, "settings-scroll", |ui| match page {
        SettingsPage::Appearance => appearance(ui, app, &theme),
        SettingsPage::Identities => identities(ui, app, &theme),
        SettingsPage::Tor => tor(ui, app, &theme),
        SettingsPage::Data => data(ui, app, &theme),
        SettingsPage::About => about(ui, &theme),
    });
}

/// The list of pages of a narrow window.
fn pages_view(ui: &mut Ui, next: &mut Next) {
    let theme = theme::current(ui.ctx());
    let asked = top_bar(ui, &theme, "Settings", false);
    if asked != Next::Stay {
        *next = asked;
    }
    column(ui, &theme, "settings-pages", |ui| {
        SettingsGroup::new("settings-pages").show(ui, |rows| {
            for (page, label, icon, description) in PAGES {
                let row = SettingsRow::new(("settings-page", page), label)
                    .description(description)
                    .icon(icon)
                    .navigates();
                if rows.row(row).clicked() {
                    *next = Next::Page(page);
                }
            }
        });
    });
}

// ---- Navigation ----

fn navigation(ui: &mut Ui, theme: &Theme, current: SettingsPage, next: &mut Next) {
    // The title sits in a band as high as the bar of the page, and lines
    // up with it.
    ui.add_space(theme.space.sm);
    let (_, band) = ui.allocate_space(Vec2::new(ui.available_width(), theme.size.topbar));
    let mut title = ui.new_child(
        UiBuilder::new()
            .max_rect(band.shrink2(Vec2::new(theme.space.md * 2.0, 0.0)))
            .layout(Layout::left_to_right(Align::Center)),
    );
    title.add(Text::new("Settings", &theme.text.title, theme.color.text).truncate());
    ui.add_space(theme.space.sm);
    egui::Frame::new()
        .inner_margin(Margin::from(Vec2::new(theme.space.md, 0.0)))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = theme.space.xxs;
            for (page, label, icon, _) in PAGES {
                if nav_row(ui, theme, page, label, icon, page == current) {
                    *next = Next::Page(page);
                }
            }
        });
}

/// One selectable page of the navigation.
fn nav_row(
    ui: &mut Ui,
    theme: &Theme,
    page: SettingsPage,
    label: &str,
    icon: Icon,
    selected: bool,
) -> bool {
    let c = &theme.color;
    let size = Vec2::new(ui.available_width(), theme.size.control);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let id = Id::new(("monolith-settings-nav", page));
    let response = ui.interact(rect, id, Sense::click());
    interact::describe_selectable(&response, WidgetType::Button, label, selected);
    let state = State::of(ui, &response, id, theme, selected);
    let radius = egui::CornerRadius::same(theme.radius.md);
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
    let icon_x = rect.left() + theme.space.md + theme.size.icon_md / 2.0;
    let icon_color = if selected {
        c.accent_text
    } else {
        c.text_secondary
    };
    icon.paint(
        painter,
        egui::pos2(icon_x, rect.center().y),
        theme.size.icon_md,
        icon_color,
    );
    let text_x = rect.left() + theme.space.md + theme.size.icon_md + theme.space.md;
    let token = if selected {
        &theme.text.body_strong
    } else {
        &theme.text.body
    };
    painter.text(
        egui::pos2(text_x, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        token.font_id(),
        c.text,
    );
    if state.focused {
        interact::focus_ring(painter, rect, radius, theme);
    }
    response.clicked()
}

// ---- Helpers ----

fn text(ui: &mut Ui, content: &str, token: &TextToken, color: Color32) {
    ui.add(Text::new(content, token, color).wrap());
}

fn caption(ui: &mut Ui, theme: &Theme, content: &str) {
    text(ui, content, &theme.text.caption, theme.color.text_tertiary);
}

/// A header over a group, with room above it.
fn section(ui: &mut Ui, theme: &Theme, title: &str) {
    ui.add_space(theme.space.md);
    SectionHeader::new(title).show(ui);
}

/// Whether a control fits beside its text: the column has the full width
/// of a page. Otherwise it goes under its text.
fn roomy(ui: &Ui, theme: &Theme) -> bool {
    ui.available_width() >= theme.size.modal_wide
}

/// A text and a button in one line when there is room, the button at the
/// right; the button under the text otherwise. Returns whether the button
/// was clicked.
fn text_and_button(ui: &mut Ui, theme: &Theme, content: &str, button: Button<'_>) -> bool {
    if !roomy(ui, theme) {
        text(ui, content, &theme.text.body, theme.color.text_secondary);
        return ui.add(button).clicked();
    }
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            clicked = ui.add(button).clicked();
            ui.add_space(theme.space.md);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                text(ui, content, &theme.text.body, theme.color.text_secondary);
            });
        });
    });
    clicked
}

/// A card with a title, a line under it, and what `add` puts below.
fn choice_card(ui: &mut Ui, theme: &Theme, title: &str, line: &str, add: impl FnOnce(&mut Ui)) {
    chrome::card_frame(ui.ctx(), theme.space.lg).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = theme.space.xs;
        text(ui, title, &theme.text.body_strong, theme.color.text);
        caption(ui, theme, line);
        ui.add_space(theme.space.xs);
        add(ui);
    });
}

// ---- Appearance ----

fn appearance(ui: &mut Ui, app: &mut App, theme: &Theme) {
    const THEME_LINE: &str = "Follow the system, or always light or dark.";
    let mut choice = app.draft_preferences.theme;
    let mut reduced = app.draft_preferences.reduced_motion;
    let beside = roomy(ui, theme);
    if !beside {
        choice_card(ui, theme, "Theme", THEME_LINE, |ui| {
            ui.add(Segmented::new("theme", &mut choice, &THEMES).full_width(true));
        });
    }
    SettingsGroup::new("appearance").show(ui, |rows| {
        if beside {
            rows.row(
                SettingsRow::new("theme", "Theme")
                    .description(THEME_LINE)
                    .icon(icons::SUN)
                    .trailing(|ui| {
                        ui.add(Segmented::new("theme", &mut choice, &THEMES));
                    }),
            );
        }
        rows.row(
            SettingsRow::new("reduced-motion", "Reduced motion")
                .description("Show changes at once instead of animating them.")
                .icon(icons::MONITOR)
                .trailing(|ui| {
                    ui.add(
                        Toggle::new(&mut reduced)
                            .name("Reduced motion")
                            .id_salt("reduced-motion"),
                    );
                }),
        );
    });
    if choice != app.draft_preferences.theme || reduced != app.draft_preferences.reduced_motion {
        app.draft_preferences.theme = choice;
        app.draft_preferences.reduced_motion = reduced;
        let mut preferences = app.preferences.clone();
        preferences.theme = choice;
        preferences.reduced_motion = reduced;
        app.effect(Effect::Preferences(preferences));
    }
}

// ---- Identities ----

/// What the identities page shows of one identity, copied out of the model
/// so that the model can be asked to act while it is drawn.
struct IdentityCard {
    id: UiIdentityId,
    name: String,
    /// The short fingerprint, which seeds the avatar as everywhere else.
    seed: String,
    fingerprint: String,
    mode: Option<RequestMode>,
    invitations: Option<Vec<(UiInvitationId, String)>>,
    /// The people it blocked: their short fingerprint, and whether they are
    /// being unblocked.
    blocked: Vec<(UiBlockedId, String, bool)>,
}

enum Action {
    NewIdentity,
    Mode(UiIdentityId, RequestMode),
    Open(Dialog),
    Unblock(UiBlockedId),
}

/// What is said while requests are refused and invitations are active.
fn closed_warning(active: usize) -> String {
    if active == 1 {
        "Your active invitation does not work while this is set to Nobody.".to_owned()
    } else {
        format!("Your {active} active invitations do not work while this is set to Nobody.")
    }
}

fn identities(ui: &mut Ui, app: &mut App, theme: &Theme) {
    let cards: Vec<IdentityCard> = app
        .model
        .identities()
        .iter()
        .map(|identity| {
            let slice = app.model.slice(identity.id);
            IdentityCard {
                id: identity.id,
                name: identity_name(identity),
                seed: identity.fingerprint.clone(),
                fingerprint: identity.full_fingerprint.clone(),
                mode: slice.and_then(|slice| slice.request_mode),
                invitations: slice.map(|slice| {
                    slice
                        .invitations
                        .iter()
                        .map(|invitation| {
                            (invitation.id, invitation.label.clone().unwrap_or_default())
                        })
                        .collect()
                }),
                blocked: slice.map_or_else(Vec::new, |slice| {
                    slice
                        .blocked
                        .iter()
                        .map(|blocked| {
                            (
                                blocked.id,
                                blocked.fingerprint.clone(),
                                app.model.unblocking(blocked.id),
                            )
                        })
                        .collect()
                }),
            }
        })
        .collect();
    let mut action = None;
    let new = Button::secondary("New identity").icon(icons::PLUS);
    if text_and_button(
        ui,
        theme,
        "Each identity has its own card, contacts and messages.",
        new,
    ) {
        action = Some(Action::NewIdentity);
    }
    let last = cards.len().saturating_sub(1);
    for (position, card) in cards.iter().enumerate() {
        ui.add_space(theme.space.md);
        ui.push_id(("settings-identity", card.id), |ui| {
            identity_card(ui, theme, card, &mut action);
        });
        if position != last {
            ui.add_space(theme.space.lg);
            ui.add(Divider::horizontal());
        }
    }
    match action {
        Some(Action::NewIdentity) => app.model.open(Dialog::NewIdentity {
            name: String::new(),
            busy: None,
            error: None,
        }),
        Some(Action::Mode(identity, mode)) => app.model.set_request_mode(identity, mode),
        Some(Action::Open(dialog)) => app.model.open(dialog),
        // At once: blocking again undoes it.
        Some(Action::Unblock(blocked)) => app.model.unblock(blocked),
        None => {}
    }
}

fn identity_card(ui: &mut Ui, theme: &Theme, card: &IdentityCard, action: &mut Option<Action>) {
    let id = card.id;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme.space.md;
        ui.add(IdentityAvatar::new(&card.name, common::seed(&card.seed)).size(AvatarSize::Large));
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = theme.space.xxs;
            text(ui, &card.name, &theme.text.heading, theme.color.text);
            text(
                ui,
                &card.fingerprint,
                &theme.text.mono,
                theme.color.text_secondary,
            );
        });
    });
    ui.add_space(theme.space.xs);

    let mut mode = card.mode.unwrap_or(RequestMode::Invitation);
    let active = card.invitations.as_ref().map_or(0, Vec::len);
    let fits = roomy(ui, theme);
    choice_card(
        ui,
        theme,
        "Who can send you requests",
        "With Invitations only, a request needs one of your invitations. With Nobody, \
         no request reaches you, not even through an invitation.",
        |ui| {
            if fits {
                ui.add_enabled(
                    card.mode.is_some(),
                    Segmented::new(("request-mode", id), &mut mode, &MODES).full_width(true),
                );
            } else {
                mode_menu(ui, id, card.mode.is_some(), &mut mode);
            }
            if card.mode == Some(RequestMode::Closed) && active > 0 {
                ui.add_space(theme.space.xs);
                Banner::new(
                    ("closed-invitations", id),
                    Tone::Warning,
                    &closed_warning(active),
                )
                .show(ui);
            }
        },
    );
    ui.add_space(theme.space.xs);
    SettingsGroup::new(("identity-rows", id)).show(ui, |rows| {
        let share = rows.row(
            SettingsRow::new(("share-card", id), "Share my card")
                .description("Show the card and its QR code, or save it as a file.")
                .icon(icons::SHARE_2)
                .navigates(),
        );
        if share.clicked() {
            *action = Some(Action::Open(Dialog::OwnCard {
                identity: id,
                card: None,
                busy: None,
                error: None,
            }));
        }
        let delete = rows.row(
            SettingsRow::new(("delete-identity", id), "Delete identity")
                .description("Removes its keys, contacts and messages from this device.")
                .icon(icons::TRASH_2)
                .danger(),
        );
        if delete.clicked() {
            *action = Some(Action::Open(Dialog::DeleteIdentity {
                identity: id,
                busy: None,
                error: None,
            }));
        }
    });
    if card.mode.is_some() && Some(mode) != card.mode {
        *action = Some(Action::Mode(id, mode));
    }

    // The invitations of this identity.
    ui.add_space(theme.space.md);
    SectionHeader::new("Invitations").show(ui);
    caption(
        ui,
        theme,
        "Anyone who has one of your invitations can send you a request until you revoke it. \
         Make one for each person, and revoke it once they are your contact.",
    );
    let Some(invitations) = &card.invitations else {
        caption(ui, theme, "Loading...");
        return;
    };
    if !invitations.is_empty() {
        ui.add_space(theme.space.xs);
        chrome::card_frame(ui.ctx(), theme.space.sm).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = theme.space.xxs;
            let last = invitations.len().saturating_sub(1);
            for (position, (invitation, label)) in invitations.iter().enumerate() {
                let out =
                    InvitationRow::new(invitation, label, "Invitation", "Show", "Revoke").show(ui);
                if out.show {
                    *action = Some(Action::Open(Dialog::ShowInvitation {
                        invitation: *invitation,
                        card: None,
                        busy: None,
                        error: None,
                    }));
                }
                if out.revoke {
                    *action = Some(Action::Open(Dialog::RevokeInvitation {
                        invitation: *invitation,
                        discard_requests: false,
                        busy: None,
                        error: None,
                    }));
                }
                if position != last {
                    ui.add(Divider::horizontal().inset(theme.space.sm));
                }
            }
        });
    }
    ui.add_space(theme.space.xs);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme.space.md;
        let full = invitations.len() >= MAX_INVITATIONS;
        let new = Button::secondary("New invitation")
            .icon(icons::TICKET)
            .enabled(!full);
        if ui.add(new).clicked() {
            *action = Some(Action::Open(Dialog::NewInvitation {
                identity: id,
                label: String::new(),
                created: None,
                busy: None,
                error: None,
            }));
        }
        caption(
            ui,
            theme,
            &format!("{} of {MAX_INVITATIONS} active", invitations.len()),
        );
    });

    // The people this identity blocked.
    ui.add_space(theme.space.md);
    SectionHeader::new("Blocked").show(ui);
    if card.blocked.is_empty() {
        caption(ui, theme, NOBODY_BLOCKED);
        return;
    }
    caption(ui, theme, BLOCKED);
    ui.add_space(theme.space.xs);
    chrome::card_frame(ui.ctx(), theme.space.md).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = theme.space.sm;
        let last = card.blocked.len().saturating_sub(1);
        for (position, (blocked, fingerprint, busy)) in card.blocked.iter().enumerate() {
            ui.push_id(("blocked", *blocked), |ui| {
                if blocked_row(ui, theme, fingerprint, *busy) {
                    *action = Some(Action::Unblock(*blocked));
                }
            });
            if position != last {
                ui.add(Divider::horizontal());
            }
        }
    });
}

/// Someone blocked: their short fingerprint, and the button that unblocks
/// them, busy while that is under way. Returns whether it was clicked.
fn blocked_row(ui: &mut Ui, theme: &Theme, fingerprint: &str, busy: bool) -> bool {
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let unblock = Button::secondary("Unblock").size(Size::Small).busy(busy);
            clicked = ui.add(unblock).clicked();
            ui.add_space(theme.space.md);
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                text(ui, fingerprint, &theme.text.mono, theme.color.text);
            });
        });
    });
    clicked
}

/// The request modes as a button that opens them as a menu, for a column
/// too narrow for the three side by side.
fn mode_menu(ui: &mut Ui, id: UiIdentityId, enabled: bool, mode: &mut RequestMode) {
    let current = MODES
        .iter()
        .find(|(option, _)| option == mode)
        .map_or("", |(_, label)| *label);
    let button = ui.push_id(("request-mode-menu", id), |ui| {
        ui.add(
            Button::secondary(current)
                .icon(icons::CHEVRON_DOWN)
                .enabled(enabled),
        )
    });
    menu_below(&button.inner, |ui| {
        for (option, label) in MODES {
            let icon = match option {
                RequestMode::Invitation => icons::TICKET,
                RequestMode::Open => icons::ID_CARD,
                RequestMode::Closed => icons::BAN,
            };
            if ui.add(MenuItem::new(label).icon(icon)).clicked() {
                *mode = option;
            }
        }
    });
}

// ---- Tor ----

fn check_words(check: Option<&TorCheck>) -> String {
    match check {
        None => "Not checked yet.".to_owned(),
        Some(TorCheck::Ready { version, .. }) => format!("Tor {version} is running and ready."),
        Some(TorCheck::Connecting {
            progress: Some(percent),
        }) => {
            format!("Tor is starting to connect ({percent}%).")
        }
        Some(TorCheck::Connecting { progress: None }) => "Tor is starting to connect.".to_owned(),
        Some(TorCheck::NotRunning) => {
            "Tor is not running, or Monolith is set up to reach it elsewhere.".to_owned()
        }
        Some(TorCheck::SocksUnavailable) => {
            "Monolith cannot reach the SOCKS port of Tor. Check the endpoint below.".to_owned()
        }
        Some(TorCheck::ControlUnavailable) => {
            "Monolith cannot reach the control port of Tor. Check the endpoint below.".to_owned()
        }
        Some(TorCheck::AuthenticationFailed) => {
            "Tor refused Monolith. Check the cookie file below.".to_owned()
        }
        Some(TorCheck::TooOld { version }) => {
            format!("Tor {version} is too old to be used. Update Tor.")
        }
    }
}

fn tor(ui: &mut Ui, app: &mut App, theme: &Theme) {
    let words = check_words(app.model.tor_check());
    let (state, label) = common::tor_state(&app.model);
    let checking = app.model.tor_checking();
    let outdated = match app.model.tor_check() {
        Some(TorCheck::Ready {
            version,
            outdated: true,
        }) => Some(format!(
            "Tor {version} is older than recommended. Update Tor."
        )),
        _ => None,
    };
    let mut check = false;
    SettingsGroup::new("tor-status").show(ui, |rows| {
        rows.row(
            SettingsRow::new("status", "Tor")
                .description(&words)
                .icon(icons::NETWORK)
                .trailing(|ui| {
                    ui.add(TorStatus::new(state, label));
                }),
        );
        rows.row(
            SettingsRow::new("check", "Connection check")
                .description("Asks Tor how it is now.")
                .icon(icons::REFRESH_CW)
                .trailing(|ui| {
                    check = ui
                        .add(
                            Button::secondary("Check again")
                                .size(Size::Small)
                                .busy(checking),
                        )
                        .clicked();
                }),
        );
    });
    if check {
        app.model.check_tor();
    }
    if let Some(note) = &outdated {
        ui.add_space(theme.space.xs);
        Banner::new("tor-outdated", Tone::Warning, note).show(ui);
    }

    section(ui, theme, "Connection");
    let mut save = false;
    let mut picked: Option<usize> = None;
    let hint = app
        .tor_presets
        .first()
        .map(|preset| preset.settings.clone())
        .unwrap_or_else(|| app.preferences.tor.clone());
    let names: Vec<String> = app
        .tor_presets
        .iter()
        .map(|preset| preset.name.clone())
        .collect();
    let changed = app.draft_preferences.tor != app.preferences.tor;
    let saved = app.tor_saved && !changed;
    chrome::card_frame(ui.ctx(), theme.space.xl).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = theme.space.md;
        let tor = &mut app.draft_preferences.tor;
        TextField::new("tor-socks", &mut tor.socks)
            .label("SOCKS endpoint")
            .hint(&hint.socks)
            .show(ui);
        TextField::new("tor-control", &mut tor.control)
            .label("Control endpoint")
            .hint(&hint.control)
            .show(ui);
        TextField::new("tor-cookie", &mut tor.cookie_file)
            .label("Cookie file")
            .hint(&hint.cookie_file)
            .help(Some(
                "Monolith signs in to Tor with this file. Tor Browser keeps it in its \
                 folder, under Browser/TorBrowser/Data/Tor/control_auth_cookie.",
            ))
            .show(ui);
        if !names.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = theme.space.sm;
                ui.add(Text::new(
                    "Use the settings of",
                    &theme.text.caption,
                    theme.color.text_tertiary,
                ));
                for (index, name) in names.iter().enumerate() {
                    if ui
                        .add(Button::ghost(name.as_str()).size(Size::Small))
                        .clicked()
                    {
                        picked = Some(index);
                    }
                }
            });
        }
        ui.add(Divider::horizontal());
        if saved {
            Banner::new(
                "tor-saved",
                Tone::Success,
                "Saved. Close and reopen Monolith to use these settings. Check again tests \
                 the settings in use now.",
            )
            .show(ui);
        }
        // The action at the right, where dialogs have theirs.
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                save = ui.add(Button::primary("Save").enabled(changed)).clicked();
                ui.add_space(theme.space.md);
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    caption(ui, theme, "Takes effect when Monolith starts again.");
                });
            });
        });
    });
    if let Some(preset) = picked.and_then(|index| app.tor_presets.get(index)) {
        app.draft_preferences.tor = preset.settings.clone();
    }
    if save {
        let mut preferences = app.preferences.clone();
        preferences.tor = app.draft_preferences.tor.clone();
        app.effect(Effect::Preferences(preferences));
        app.tor_saved = true;
        // The desktop applies them after this frame; the next one says so.
        ui.ctx().request_repaint();
    }
    ui.add_space(theme.space.xs);
    caption(
        ui,
        theme,
        "Monolith only reaches other people through Tor. There is no direct connection.",
    );
}

// ---- Data ----

fn data(ui: &mut Ui, app: &mut App, theme: &Theme) {
    if app.model.message_store_missing() {
        Banner::new(
            "message-store-missing",
            Tone::Warning,
            "The messages from before were not found when the profile opened. \
             Monolith started with no message history.",
        )
        .show(ui);
        ui.add_space(theme.space.xs);
    }
    let ephemeral = app.model.ephemeral();
    let location = app.model.location().to_owned();
    SettingsGroup::new("data-where").show(ui, |rows| {
        if ephemeral {
            rows.row(
                SettingsRow::new("session", "This session saves nothing")
                    .description(
                        "Your identities, contacts and messages are gone when you close Monolith.",
                    )
                    .icon(icons::CLOUD_OFF),
            );
        } else {
            rows.row(
                SettingsRow::new("folder", "Profile folder")
                    .description(&location)
                    .icon(icons::FOLDER_OPEN),
            );
            rows.row(
                SettingsRow::new("stored", "What is stored")
                    .description(
                        "Your identities, contacts and messages, encrypted under \
                         your passphrase. Nothing else.",
                    )
                    .icon(icons::LOCK),
            );
        }
    });
    section(ui, theme, "Privacy");
    SettingsGroup::new("data-privacy").show(ui, |rows| {
        rows.row(
            SettingsRow::new("no-logs", "No logs, no analytics")
                .description("Monolith writes no log file and sends no usage data.")
                .icon(icons::SHIELD_OFF),
        );
        rows.row(
            SettingsRow::new("deleting", "Deleting")
                .description(
                    "Deleting messages or an identity removes them from Monolith's files. \
                     It cannot guarantee that older copies, backups or freed disk blocks \
                     are erased.",
                )
                .icon(icons::TRASH_2),
        );
    });
}

// ---- About ----

fn about(ui: &mut Ui, theme: &Theme) {
    let version = format!(
        "Version {}. Private messages over Tor, with no servers and no accounts.",
        env!("CARGO_PKG_VERSION")
    );
    SettingsGroup::new("about").show(ui, |rows| {
        rows.row(
            SettingsRow::new("version", "Monolith")
                .description(&version)
                .icon(icons::MESSAGE_SQUARE),
        );
        rows.row(
            SettingsRow::new("telemetry", "No telemetry")
                .description("No telemetry, no analytics, no update checks.")
                .icon(icons::SHIELD_OFF),
        );
    });
    section(ui, theme, "Fonts and icons");
    SettingsGroup::new("about-licences").show(ui, |rows| {
        for (name, licence) in [
            ("Inter", "SIL Open Font License 1.1"),
            ("JetBrains Mono", "SIL Open Font License 1.1"),
            ("Noto Emoji", "SIL Open Font License 1.1"),
            ("Lucide", "ISC License"),
        ] {
            rows.row(SettingsRow::new(name, name).description(licence));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_warning_under_nobody_counts_the_active_invitations() {
        assert_eq!(
            closed_warning(1),
            "Your active invitation does not work while this is set to Nobody."
        );
        assert_eq!(
            closed_warning(3),
            "Your 3 active invitations do not work while this is set to Nobody."
        );
    }
}

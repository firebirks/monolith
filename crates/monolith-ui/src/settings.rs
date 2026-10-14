//! The settings: a column of pages at the left, the page at the right.

use egui::{Align, Align2, Color32, Context, Id, Layout, Sense, Ui, UiBuilder, Vec2, WidgetType};
use monolith_ui_kit::avatar::{AvatarSize, IdentityAvatar};
use monolith_ui_kit::banner::{Banner, Tone};
use monolith_ui_kit::button::Size;
use monolith_ui_kit::chrome::{self, ContentPanel, SidebarPanel, TopBar};
use monolith_ui_kit::divider::Divider;
use monolith_ui_kit::interact::{self, State};
use monolith_ui_kit::invitation_row::InvitationRow;
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
use monolith_ui_model::{Dialog, RequestMode, TorCheck, UiIdentityId, UiInvitationId};

use crate::app::{App, Effect, SettingsPage, ThemeChoice, View};
use crate::common;

/// How many invitations an identity can have active.
const MAX_INVITATIONS: usize = 16;

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

const PAGES: [(SettingsPage, &str, Icon); 5] = [
    (SettingsPage::Appearance, "Appearance", icons::SUN),
    (SettingsPage::Identities, "Identities", icons::USERS),
    (SettingsPage::Tor, "Tor", icons::NETWORK),
    (SettingsPage::Data, "Data", icons::FOLDER_OPEN),
    (SettingsPage::About, "About", icons::INFO),
];

/// Draws the settings at `page`.
pub(crate) fn show(ctx: &Context, app: &mut App, page: SettingsPage) {
    let theme = theme::current(ctx);
    let mut chosen = page;
    SidebarPanel::new("settings-nav")
        .resizable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .inner_margin(theme.space.md)
                .show(ui, |ui| navigation(ui, &theme, page, &mut chosen));
        });
    let mut done = false;
    ContentPanel.show(ctx, |ui| {
        let title = PAGES
            .iter()
            .find(|(candidate, _, _)| *candidate == page)
            .map_or("Settings", |(_, label, _)| *label);
        TopBar::new("settings-top").show(
            ui,
            |ui| {
                ui.add(Text::new(title, &theme.text.title, theme.color.text));
            },
            |ui| {
                done = ui
                    .add(Button::secondary("Done").size(Size::Small))
                    .clicked();
            },
        );
        chrome::scroll_area("settings-scroll").show(ui, |ui| {
            let area = ui.available_rect_before_wrap();
            let width = theme
                .size
                .modal_wide
                .min(area.width() - theme.space.xxl * 2.0)
                .max(theme.size.sidebar_min);
            let rect = egui::Rect::from_min_size(
                egui::pos2(area.center().x - width / 2.0, area.top() + theme.space.xxl),
                Vec2::new(width, area.height()),
            );
            ui.scope_builder(
                UiBuilder::new()
                    .max_rect(rect)
                    .layout(Layout::top_down(Align::Min)),
                |ui| {
                    ui.spacing_mut().item_spacing.y = theme.space.sm;
                    match page {
                        SettingsPage::Appearance => appearance(ui, app),
                        SettingsPage::Identities => identities(ui, app, &theme),
                        SettingsPage::Tor => tor(ui, app, &theme),
                        SettingsPage::Data => data(ui, app, &theme),
                        SettingsPage::About => about(ui, &theme),
                    }
                    ui.add_space(theme.space.xxxl);
                },
            );
        });
    });
    if done {
        app.view = View::Main;
    } else if chosen != page {
        app.view = View::Settings(chosen);
    }
}

// ---- Navigation ----

fn navigation(ui: &mut Ui, theme: &Theme, current: SettingsPage, chosen: &mut SettingsPage) {
    ui.spacing_mut().item_spacing.y = theme.space.xxs;
    ui.add_space(theme.space.xs);
    ui.horizontal(|ui| {
        ui.add_space(theme.space.sm);
        text(ui, "Settings", &theme.text.title, theme.color.text);
    });
    ui.add_space(theme.space.md);
    for (page, label, icon) in PAGES {
        if nav_row(ui, theme, page, label, icon, page == current) {
            *chosen = page;
        }
    }
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
    ui.add(Text::new(content, token, color));
}

fn caption(ui: &mut Ui, theme: &Theme, content: &str) {
    text(ui, content, &theme.text.caption, theme.color.text_tertiary);
}

/// A header over a group, with room above it.
fn section(ui: &mut Ui, theme: &Theme, title: &str) {
    ui.add_space(theme.space.md);
    SectionHeader::new(title).show(ui);
}

// ---- Appearance ----

fn appearance(ui: &mut Ui, app: &mut App) {
    let mut choice = app.draft_preferences.theme;
    let mut reduced = app.draft_preferences.reduced_motion;
    SettingsGroup::new("appearance").show(ui, |rows| {
        rows.row(
            SettingsRow::new("theme", "Theme")
                .description("Follow the system, or always light or dark.")
                .icon(icons::SUN)
                .trailing(|ui| {
                    ui.add(Segmented::new("theme", &mut choice, &THEMES));
                }),
        );
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
    fingerprint: String,
    mode: Option<RequestMode>,
    invitations: Option<Vec<(UiInvitationId, String)>>,
}

enum Action {
    NewIdentity,
    Mode(UiIdentityId, RequestMode),
    Open(Dialog),
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
            }
        })
        .collect();
    let mut action = None;
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let new = Button::secondary("New identity")
                .icon(icons::PLUS)
                .size(Size::Medium);
            if ui.add(new).clicked() {
                action = Some(Action::NewIdentity);
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                text(
                    ui,
                    "Each identity has its own card, contacts and messages.",
                    &theme.text.body,
                    theme.color.text_secondary,
                );
            });
        });
    });
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
        None => {}
    }
}

fn identity_card(ui: &mut Ui, theme: &Theme, card: &IdentityCard, action: &mut Option<Action>) {
    let id = card.id;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = theme.space.md;
        ui.add(
            IdentityAvatar::new(&card.name, crate::common::seed(&card.fingerprint))
                .size(AvatarSize::Large),
        );
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
    chrome::card_frame(ui.ctx(), theme.space.lg).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = theme.space.xs;
        text(
            ui,
            "Who can send you requests",
            &theme.text.body_strong,
            theme.color.text,
        );
        caption(
            ui,
            theme,
            "A request asks to talk to this identity. An invitation always works, whatever you choose.",
        );
        ui.add_space(theme.space.xs);
        ui.add_enabled(
            card.mode.is_some(),
            Segmented::new(("request-mode", id), &mut mode, &MODES).full_width(true),
        );
    });
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
    let Some(invitations) = &card.invitations else {
        caption(ui, theme, "Loading...");
        return;
    };
    if invitations.is_empty() {
        caption(
            ui,
            theme,
            "No active invitations. An invitation is a card that lets one \
             person send you a request, whatever the setting above.",
        );
    } else {
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
        let full = invitations.len() >= MAX_INVITATIONS;
        let new = Button::secondary("New invitation")
            .icon(icons::TICKET)
            .size(Size::Medium)
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
                text(
                    ui,
                    "Use the settings of",
                    &theme.text.caption,
                    theme.color.text_tertiary,
                );
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
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = theme.space.md;
            save = ui
                .add(
                    Button::primary("Save")
                        .size(Size::Medium)
                        .enabled(app.draft_preferences.tor != app.preferences.tor),
                )
                .clicked();
            caption(ui, theme, "Takes effect when Monolith starts again.");
        });
    });
    if let Some(preset) = picked.and_then(|index| app.tor_presets.get(index)) {
        app.draft_preferences.tor = preset.settings.clone();
    }
    if save {
        let mut preferences = app.preferences.clone();
        preferences.tor = app.draft_preferences.tor.clone();
        app.effect(Effect::Preferences(preferences));
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
                    .icon(icons::LOCK),
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

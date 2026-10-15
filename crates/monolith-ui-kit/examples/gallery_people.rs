//! Renders sheets of the people-related components in every state, light
//! and dark, to PNG files:
//! `cargo run -p monolith-ui-kit --example gallery_people -- <directory>`.

#![allow(clippy::print_stderr, clippy::expect_used)]

use std::path::{Path, PathBuf};

use egui::{Color32, Context, Pos2, Vec2};
use monolith_ui_kit::avatar::{Avatar, AvatarSize, IdentityAvatar};
use monolith_ui_kit::banner::{Banner, InlineError, Tone};
use monolith_ui_kit::empty_state::EmptyState;
use monolith_ui_kit::fingerprint::Fingerprint;
use monolith_ui_kit::identity_switcher::{IdentityItem, IdentitySwitcher, set_open};
use monolith_ui_kit::invitation_row::InvitationRow;
use monolith_ui_kit::request_card::{RequestAction, RequestCard};
use monolith_ui_kit::rows::{ContactRow, ConversationRow, RowStatus, Secondary};
use monolith_ui_kit::segmented::Segmented;
use monolith_ui_kit::settings_row::{SettingsGroup, SettingsRow};
use monolith_ui_kit::{Button, Theme, fonts, icons, interact, theme};

/// A fingerprint in its full form: 13 groups of four.
const FINGERPRINT: &str = "F6QW 8NRT 3MKZ 5DPL 2XEC 9BVH 5TQN 2WRK 8MZP 3XEL 6VDB 9HCA Q4KM";
use monolith_ui_snapshot::{Options, render};

#[derive(Clone, Copy)]
enum Ground {
    App,
    Sidebar,
    Surface,
}

struct Sheet<'a> {
    name: &'a str,
    size: Vec2,
    ground: Ground,
    pointer: Option<Pos2>,
    margin: i8,
}

fn sheet(spec: &Sheet<'_>, dir: &Path, mut body: impl FnMut(&mut egui::Ui)) {
    for (suffix, theme) in [("light", Theme::light()), ("dark", Theme::dark())] {
        let ctx = Context::default();
        fonts::install(&ctx, &[]);
        let background = match spec.ground {
            Ground::App => theme.color.bg_app,
            Ground::Sidebar => theme.color.bg_sidebar,
            Ground::Surface => theme.color.bg_surface,
        };
        theme::install(&ctx, theme.with_reduced_motion(true));
        // Focus set by hand shows its ring, as if Tab had moved it.
        interact::set_focus_visible(&ctx, true);
        let options = Options {
            size: spec.size,
            background,
            pointer: spec.pointer,
            ..Options::default()
        };
        let margin = spec.margin;
        let image = render(&ctx, &options, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().inner_margin(margin))
                .show(ctx, |ui| body(ui));
        });
        let path = dir.join(format!("{}-{suffix}.png", spec.name));
        image.save_png(&path).expect("write the PNG");
        eprintln!("{}", path.display());
    }
}

fn conversations(ui: &mut egui::Ui, focus: bool, selected: Option<u32>) {
    let seeds: [&[u8]; 6] = [b"a1", b"b2", b"c3", b"d4", b"e5", b"f6"];
    let waiting = [RowStatus::WaitingForAcceptance];
    let ended = [RowStatus::Ended];
    let queued = [RowStatus::Queued];
    ui.add(
        ConversationRow::new(1_u32, "Alice Martin", seeds[0])
            .preview("See you at the station at eight")
            .timestamp("09:41")
            .selected(selected == Some(1)),
    );
    ui.add(
        ConversationRow::new(2_u32, "Bob at work", seeds[1])
            .preview("Sounds good, sending the file now")
            .outgoing("You: ")
            .timestamp("Mon")
            .unread(3)
            .selected(selected == Some(2)),
    );
    ui.add(
        ConversationRow::new(3_u32, "Contact 7F3A", seeds[2])
            .preview("Waiting for them to accept")
            .timestamp("Sun")
            .statuses(&waiting)
            .anonymous(true)
            .selected(selected == Some(3)),
    );
    ui.add(
        ConversationRow::new(4_u32, "Former contact D4XM", seeds[3])
            .preview("Talk soon.")
            .timestamp("12 Oct")
            .statuses(&ended)
            .anonymous(true)
            .selected(selected == Some(4)),
    );
    ui.add(
        ConversationRow::new(
            5_u32,
            "A very long display name that certainly cannot fit in the sidebar width",
            seeds[4],
        )
        .preview("This is a long message preview that goes on and on and must end with an ellipsis")
        .outgoing("You: ")
        .timestamp("Fri")
        .unread(120)
        .statuses(&queued)
        .selected(selected == Some(5)),
    );
    ui.add(
        ConversationRow::new(6_u32, "\u{5f20}\u{4f1f} \u{1f60a}", seeds[5])
            .preview("\u{4f60}\u{597d}\u{ff0c}\u{660e}\u{5929}\u{89c1}")
            .timestamp("Thu")
            .selected(selected == Some(6)),
    );
    if focus {
        ui.memory_mut(|m| m.request_focus(ConversationRow::id_for(2_u32)));
    }
}

fn switcher(ui: &mut egui::Ui, open: bool) {
    let items = [
        IdentityItem {
            key: 1_u32,
            name: "Personal",
            detail: "7F3A 91C2 0B44",
            seed: b"p",
        },
        IdentityItem {
            key: 2_u32,
            name: "Work",
            detail: "21DD 8E05 C3A9",
            seed: b"w",
        },
        IdentityItem {
            key: 3_u32,
            name: "Anonymous reading list",
            detail: "5B00 17AF 66E1",
            seed: b"r",
        },
    ];
    if open {
        set_open(ui.ctx(), "switcher", true);
    }
    IdentitySwitcher::new(
        "switcher",
        &1_u32,
        &items,
        "New identity",
        "Manage identities",
    )
    .caption("3 contacts")
    .show(ui);
}

fn main() {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "gallery-people".to_owned()),
    );

    let side = |name, pointer| Sheet {
        name,
        size: Vec2::new(340.0, 560.0),
        ground: Ground::Sidebar,
        pointer,
        margin: 0,
    };
    sheet(&side("sidebar", None), &dir, |ui| {
        ui.add_space(8.0);
        ui.scope(|ui| {
            ui.set_width(ui.available_width());
            let inner = egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 0));
            inner.show(ui, |ui| switcher(ui, false));
        });
        ui.add_space(8.0);
        conversations(ui, false, Some(1));
    });
    sheet(&side("sidebar-open", None), &dir, |ui| {
        ui.add_space(8.0);
        let inner = egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 0));
        inner.show(ui, |ui| switcher(ui, true));
        ui.add_space(8.0);
        conversations(ui, false, Some(1));
    });
    // Pointer over the second row; keyboard focus on it as well in the
    // next sheet.
    sheet(
        &side("sidebar-hover", Some(Pos2::new(160.0, 72.0 + 64.0 + 32.0))),
        &dir,
        |ui| {
            ui.add_space(8.0);
            ui.add_space(56.0);
            ui.add_space(8.0);
            conversations(ui, false, Some(1));
        },
    );
    sheet(&side("sidebar-focus", None), &dir, |ui| {
        ui.add_space(72.0);
        conversations(ui, true, Some(1));
    });

    let wide = |name, h| Sheet {
        name,
        size: Vec2::new(if name == "contacts" { 700.0 } else { 520.0 }, h),
        ground: Ground::App,
        pointer: None,
        margin: 24,
    };
    sheet(&wide("contacts", 420.0), &dir, |ui| {
        let seed: &[u8] = b"zz";
        ui.add(
            ContactRow::new(1_u32, "Alice Martin", seed)
                .secondary(Secondary::Mono("7F3A 91C2"))
                .verified(true),
        );
        ui.add(
            ContactRow::new(2_u32, "Contact 21DD", b"yy")
                .anonymous(true)
                .secondary(Secondary::Text("Waiting for them to accept"))
                .trailing(|ui| {
                    ui.add(Button::secondary("Cancel").size(monolith_ui_kit::button::Size::Small));
                }),
        );
        ui.add(
            ContactRow::new(
                3_u32,
                "Selected contact with a rather long name indeed",
                b"xx",
            )
            .secondary(Secondary::Mono("5B00 17AF 66E1 0000 1111"))
            .selected(true),
        );
        ui.add(
            ContactRow::new(4_u32, "Disabled contact", b"ww")
                .secondary(Secondary::Text("Unavailable"))
                .enabled(false),
        );
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            for size in [
                AvatarSize::Small,
                AvatarSize::Medium,
                AvatarSize::Large,
                AvatarSize::Extra,
            ] {
                ui.add(Avatar::new("Alice Martin", b"s1").size(size));
            }
            ui.add(Avatar::new("", b"s2"));
            ui.add(Avatar::new("Contact C7TQ", b"s5").anonymous(true));
            ui.add(
                Avatar::new("Bob", b"s3")
                    .verified(true)
                    .size(AvatarSize::Large),
            );
            ui.add(
                Avatar::new("Carol", b"s4")
                    .verified(true)
                    .size(AvatarSize::Extra),
            );
            ui.add(IdentityAvatar::new("Personal", b"p").size(AvatarSize::Large));
            ui.add(IdentityAvatar::new("Work", b"w").size(AvatarSize::Extra));
        });
    });
    sheet(&wide("invitations", 300.0), &dir, |ui| {
        let frame = egui::Frame::new().inner_margin(egui::Margin::symmetric(4, 0));
        frame.show(ui, |ui| {
            InvitationRow::new(1_u32, "Friends", "Invitation", "Show", "Revoke")
                .caption("Created yesterday, not used")
                .show(ui);
            InvitationRow::new(2_u32, "", "Invitation", "Show", "Revoke")
                .caption("Created 3 days ago, used once")
                .show(ui);
            InvitationRow::new(3_u32, "Being revoked", "Invitation", "Show", "Revoke")
                .caption("Revoking")
                .busy(true)
                .show(ui);
        });
    });
    sheet(&wide("requests", 700.0), &dir, |ui| {
        RequestCard::new(1_u32, "Bob wants to connect", "Accept", "Decline", "Block")
            .sender("Bob", b"bob")
            .introduction(
                "Their message",
                "Hi, it is Bob from the climbing gym. I got your invitation from Dana. AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            )
            .fingerprint(FINGERPRINT)
            .caption("Through your invitation Friends")
            .show(ui);
        ui.add_space(16.0);
        RequestCard::new(
            2_u32,
            "Someone wants to connect",
            "Accept",
            "Decline",
            "Block",
        )
        .fingerprint(FINGERPRINT)
        .busy(Some(RequestAction::Accept))
        .show(ui);
    });
    sheet(&wide("fingerprints", 300.0), &dir, |ui| {
        let theme = theme::current(ui.ctx());
        ui.add(Fingerprint::new(FINGERPRINT));
        ui.add_space(12.0);
        ui.add(
            Fingerprint::new(FINGERPRINT)
                .compact(true)
                .color(theme.color.text_secondary),
        );
        ui.add_space(12.0);
        // Too narrow for six groups: fewer on each row.
        ui.allocate_ui(Vec2::new(180.0, 120.0), |ui| {
            ui.add(Fingerprint::new(FINGERPRINT));
        });
    });
    sheet(&wide("settings", 420.0), &dir, |ui| {
        SettingsGroup::new("g").show(ui, |rows| {
            rows.row(
                SettingsRow::new("a", "Appearance")
                    .description("Follow the system, or choose light or dark.")
                    .icon(icons::SUN)
                    .navigates(),
            );
            rows.row(
                SettingsRow::new("t", "Theme")
                    .icon(icons::MONITOR)
                    .trailing(|ui| {
                        let mut choice = 2_u8;
                        let options = [(0_u8, "System"), (1_u8, "Light"), (2_u8, "Dark")];
                        ui.add(Segmented::new("theme", &mut choice, &options));
                    }),
            );
            rows.row(
                SettingsRow::new("b", "Reduce motion")
                    .description("Turns animations off.")
                    .icon(icons::MONITOR)
                    .trailing(|ui| {
                        ui.add(Button::secondary("Off").size(monolith_ui_kit::button::Size::Small));
                    }),
            );
            rows.row(
                SettingsRow::new("c", "Delete this identity")
                    .icon(icons::TRASH_2)
                    .danger(),
            );
        });
    });
    sheet(
        &Sheet {
            name: "empty",
            size: Vec2::new(520.0, 360.0),
            ground: Ground::Surface,
            pointer: None,
            margin: 0,
        },
        &dir,
        |ui| {
            EmptyState::new("e", icons::INBOX, "No requests")
                .body("When someone sends you a request through one of your invitations it appears here.")
                .primary("Create an invitation")
                .secondary("Learn more")
                .show(ui);
        },
    );
    sheet(&wide("banners", 520.0), &dir, |ui| {
        for (i, tone) in [Tone::Info, Tone::Success, Tone::Warning, Tone::Danger]
            .into_iter()
            .enumerate()
        {
            Banner::new(i, tone, "Messages are queued until Tor is connected.")
                .title("Tor is starting")
                .action("Details")
                .dismissible("Dismiss")
                .show(ui);
            ui.add_space(8.0);
        }
        Banner::new("plain", Tone::Info, "A plain one-line notice.").show(ui);
        ui.add_space(12.0);
        ui.add(InlineError::new(
            "This name is already used by another contact, choose a different one please.",
        ));
        let _ = Color32::TRANSPARENT;
    });
}

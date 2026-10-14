//! Renders sheets of the conversation, overlay and chrome components, light
//! and dark, to PNG files: `cargo run -p monolith-ui-kit --example
//! gallery_messages -- <directory>`.

#![allow(
    clippy::print_stderr,
    clippy::expect_used,
    clippy::too_many_lines,
    deprecated
)]

use std::path::{Path, PathBuf};

use egui::{Context, Pos2, Vec2};
use monolith_ui_kit::bubble::{DaySeparator, Direction, MessageBubble};
use monolith_ui_kit::chrome::{self, ContentPanel, TopBar};
use monolith_ui_kit::menu::{self, MenuItem};
use monolith_ui_kit::message_status::{MessageStatus, Status};
use monolith_ui_kit::modal::Modal;
use monolith_ui_kit::qr::{QrCode, QrMatrix};
use monolith_ui_kit::toast::{Kind, Toasts};
use monolith_ui_kit::tor_status::{TorState, TorStatus};
use monolith_ui_kit::{Button, TextField, Theme, fonts, icons, theme};
use monolith_ui_snapshot::{Options, render};

fn sheet<S>(
    name: &str,
    size: Vec2,
    pixels_per_point: f32,
    dir: &Path,
    make: impl Fn() -> S,
    mut body: impl FnMut(&mut S, &Context),
) {
    for (suffix, theme) in [("light", Theme::light()), ("dark", Theme::dark())] {
        let ctx = Context::default();
        fonts::install(&ctx, &[]);
        let background = theme.color.bg_app;
        theme::install(&ctx, theme.with_reduced_motion(true));
        let options = Options {
            size,
            pixels_per_point,
            background,
            ..Options::default()
        };
        let mut state = make();
        let image = render(&ctx, &options, |ctx| body(&mut state, ctx));
        let path = dir.join(format!("{name}-{suffix}.png"));
        image.save_png(&path).expect("write the PNG");
        eprintln!("{}", path.display());
    }
}

const LONG_STRING: &str =
    "mon1qz9x8c7v6b5n4m3a2s1d0f9g8h7j6k5l4p3o2i1u7y6t5r4e3w2q1zxcvbnmasdfghjklqwertyuiopmnbvcxz";

const LONG_MESSAGE_START: &str = "I went through the notes from the meeting again and I think we should\nwrite down the three decisions before we forget what we agreed on:\nthe schedule, the budget and who talks to the supplier.";

fn conversation(ctx: &Context, with_menu: bool) {
    TopBar::new("conversation").panel(
        ctx,
        |ui| {
            ui.label(
                egui::RichText::new("Alice Moreau")
                    .font(theme::current(ui.ctx()).text.heading.font_id())
                    .color(theme::current(ui.ctx()).color.text),
            );
        },
        |ui| {
            ui.add(TorStatus::new(TorState::Connected, "Connected through Tor"));
        },
    );
    ContentPanel.show(ctx, |ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        chrome::scroll_area("messages").show(ui, |ui| {
            ui.set_width(ui.available_width());
            let theme = theme::current(ui.ctx());
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add_space(theme.space.sm);
            // Content margin: the list is inset from the panel edges.
            let inner = ui
                .available_rect_before_wrap()
                .shrink2(Vec2::new(theme.space.xl, 0.0));
            ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
                ui.add(DaySeparator::new("Yesterday"));
                bubbles(ui, with_menu);
            });
        });
    });
}

fn bubbles(ui: &mut egui::Ui, with_menu: bool) {
    let incoming = Direction::Incoming;
    let outgoing = Direction::Outgoing;
    let show = |ui: &mut egui::Ui, bubble: MessageBubble<'_>, menu_here: bool| {
        let output = bubble.show(ui);
        if with_menu && menu_here {
            ui.ctx().memory_mut(|memory| {
                memory.open_popup_at(
                    output.response.id.with("popup"),
                    Some(Pos2::new(
                        output.response.rect.right() - 60.0,
                        output.response.rect.center().y,
                    )),
                );
            });
            menu::context_menu(&output.response, |ui| {
                ui.add(MenuItem::new("Copy text").icon(icons::COPY));
                ui.add(MenuItem::new("Message details").icon(icons::INFO));
                ui.add(MenuItem::new("Edit").icon(icons::PENCIL).enabled(false));
                menu::menu_separator(ui);
                ui.add(
                    MenuItem::new("Delete for me")
                        .icon(icons::TRASH_2)
                        .danger(true),
                );
            });
        }
    };
    show(
        ui,
        MessageBubble::new(1, incoming, "Hey! Are we still on for Thursday?", "21:14")
            .last_in_group(false),
        false,
    );
    show(
        ui,
        MessageBubble::new(2, incoming, "I booked the table for 7.", "21:14").first_in_group(false),
        false,
    );
    show(
        ui,
        MessageBubble::new(3, outgoing, "Yes, 7 works for me.", "21:20")
            .status(Status::Delivered, "Delivered")
            .last_in_group(false),
        false,
    );
    show(
        ui,
        MessageBubble::new(
            4,
            outgoing,
            "Should I bring anything?\nI can pick up some wine on the way.",
            "21:20",
        )
        .status(Status::Delivered, "Delivered")
        .first_in_group(false),
        true,
    );
    ui.add(DaySeparator::new("Today"));
    show(
        ui,
        MessageBubble::new(5, incoming, "No need. Just yourself.", "09:02"),
        false,
    );
    show(
        ui,
        MessageBubble::new(6, outgoing, LONG_STRING, "09:05")
            .status(Status::Delivered, "Delivered"),
        false,
    );
    show(
        ui,
        MessageBubble::new(
            7,
            incoming,
            "Ha, that looks like a key. Please do not paste those into a chat, even this one.",
            "09:06",
        ),
        false,
    );
    show(
        ui,
        MessageBubble::new(8, incoming, LONG_MESSAGE_START, "09:30").collapsed("Show more"),
        false,
    );
    show(
        ui,
        MessageBubble::new(9, outgoing, "Ok", "09:41")
            .status(Status::Delivered, "Delivered")
            .last_in_group(false),
        false,
    );
    show(
        ui,
        MessageBubble::new(
            10,
            outgoing,
            "See you at https://example.org/page?x=1 then",
            "09:42",
        )
        .status(Status::Queued, "Queued")
        .first_in_group(false),
        false,
    );
    ui.add_space(16.0);
}

fn dim_backdrop(ctx: &Context) {
    // A screen behind the modal, so that the scrim has something to cover.
    conversation(ctx, false);
}

fn main() {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "gallery-messages".to_owned()),
    );

    sheet(
        "conversation",
        Vec2::new(760.0, 900.0),
        1.0,
        &dir,
        || (),
        |(), ctx| conversation(ctx, false),
    );

    sheet(
        "narrow",
        Vec2::new(420.0, 700.0),
        1.0,
        &dir,
        || (),
        |(), ctx| conversation(ctx, false),
    );

    sheet(
        "context-menu",
        Vec2::new(760.0, 900.0),
        1.0,
        &dir,
        || (),
        |(), ctx| conversation(ctx, true),
    );

    sheet(
        "tor-and-status",
        Vec2::new(480.0, 300.0),
        1.0,
        &dir,
        || (),
        |(), ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().inner_margin(24))
                .show(ctx, |ui| {
                    for (state, label) in [
                        (TorState::Starting, "Starting"),
                        (TorState::Connecting, "Connecting to Tor"),
                        (TorState::Connected, "Connected through Tor"),
                        (TorState::Unavailable, "Tor unavailable"),
                    ] {
                        ui.add(TorStatus::new(state, label));
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.add(MessageStatus::new("a", Status::Queued, "Queued").show_label(true));
                        ui.add_space(16.0);
                        ui.add(
                            MessageStatus::new("b", Status::Delivered, "Delivered")
                                .show_label(true),
                        );
                    });
                });
        },
    );

    sheet(
        "modal-form",
        Vec2::new(760.0, 600.0),
        1.0,
        &dir,
        || "Alice at work".to_owned(),
        |name, ctx| {
            dim_backdrop(ctx);
            Modal::new("rename", "Rename contact")
                .subtitle("Only you see this name.")
                .show(
                    ctx,
                    |ui| {
                        TextField::new("rename-field", name)
                            .label("Name")
                            .hint("For example, Alice")
                            .show(ui);
                    },
                    |ui| {
                        ui.add(Button::primary("Save"));
                        ui.add(Button::secondary("Cancel"));
                    },
                );
        },
    );

    sheet(
        "modal-confirm",
        Vec2::new(760.0, 600.0),
        1.0,
        &dir,
        || (),
        |(), ctx| {
            dim_backdrop(ctx);
            let _ = Modal::new("delete", "Delete this conversation?").confirm(
                ctx,
                "The messages of this conversation are deleted from this device. This cannot be undone.",
                "Cancel",
                "Delete",
                false,
            );
        },
    );

    sheet(
        "toasts",
        Vec2::new(640.0, 360.0),
        1.0,
        &dir,
        || {
            let mut toasts = Toasts::new().dismiss_label("Dismiss");
            toasts.push(Kind::Success, "Invitation copied to the clipboard.");
            toasts.push_with_action(
                Kind::Error,
                "The message could not be sent. It stays queued and will be sent when Tor is back.",
                "Details",
            );
            toasts.push(Kind::Info, "Alice is now a contact.");
            toasts.push(Kind::Warning, "Tor is slow right now.");
            toasts
        },
        |toasts, ctx| {
            egui::CentralPanel::default().show(ctx, |_| {});
            let _ = toasts.show(ctx);
        },
    );

    let secret: String = (0..30)
        .map(|n| format!("monolith-card-{n:02}-AbCdEfGhIjKl"))
        .collect::<Vec<_>>()
        .join("")
        .chars()
        .take(300)
        .collect();
    for ppp in [1.0_f32, 2.0] {
        let suffix = if ppp > 1.0 { "qr-2x" } else { "qr" };
        sheet(
            suffix,
            Vec2::new(520.0, 460.0),
            ppp,
            &dir,
            || QrMatrix::encode(&secret).expect("a matrix"),
            |matrix, ctx| {
                dim_backdrop_small(ctx);
                Modal::new("qr", "Invitation")
                    .subtitle("Scan it with Monolith on the other device.")
                    .show(
                        ctx,
                        |ui| {
                            ui.vertical_centered(|ui| {
                                ui.add(QrCode::new(matrix, "QR code of the invitation"));
                            });
                        },
                        |ui| {
                            ui.add(Button::secondary("Close"));
                            ui.add(Button::secondary("Copy").icon(icons::COPY));
                        },
                    );
            },
        );
    }

    sheet(
        "menu-below",
        Vec2::new(520.0, 300.0),
        1.0,
        &dir,
        || (),
        |(), ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    let more = ui.add(Button::secondary("More").icon(icons::ELLIPSIS_VERTICAL));
                    egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&more));
                    menu::menu_below(&more, |ui| {
                        ui.add(MenuItem::new("Rename").icon(icons::PENCIL));
                        ui.add(MenuItem::new("Block contact").icon(icons::BAN));
                        menu::menu_separator(ui);
                        ui.add(MenuItem::new("Delete").icon(icons::TRASH_2).danger(true));
                    });
                });
            });
        },
    );
}

fn dim_backdrop_small(ctx: &Context) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.label("Settings");
    });
}

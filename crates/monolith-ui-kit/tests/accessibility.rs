//! What controls that take no input tell a screen reader and the pointer,
//! and which widgets are stops of the Tab key.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use common::{Rig, focused_labels, node};
use egui::{CursorIcon, Event, Id, Key, Ui};
use monolith_ui_kit::bubble::{Direction, MessageBubble};
use monolith_ui_kit::checkbox::Checkbox;
use monolith_ui_kit::chrome::{ContentPanel, SidebarPanel, scroll_area};
use monolith_ui_kit::composer::Composer;
use monolith_ui_kit::icon_button::IconButton;
use monolith_ui_kit::menu::MenuItem;
use monolith_ui_kit::rows::{ContactRow, ConversationRow};
use monolith_ui_kit::search_field::SearchField;
use monolith_ui_kit::segmented::Segmented;
use monolith_ui_kit::settings_row::{SettingsGroup, SettingsRow};
use monolith_ui_kit::text::Text;
use monolith_ui_kit::toggle::Toggle;
use monolith_ui_kit::{Button, TextField, Theme, icons, theme};

/// Adds one control and returns the id of its accessible node.
type Make = fn(&mut Ui) -> Id;

fn controls_that_take_no_input() -> Vec<(&'static str, Make)> {
    vec![
        ("a disabled button", |ui| {
            ui.add(Button::primary("Create profile").enabled(false)).id
        }),
        ("a busy button", |ui| {
            ui.add(Button::primary("Creating").busy(true)).id
        }),
        ("a disabled icon button", |ui| {
            ui.add(IconButton::new(icons::COPY, "Copy").enabled(false))
                .id
        }),
        ("a disabled toggle", |ui| {
            let mut on = true;
            ui.add(Toggle::new(&mut on).label("Reduce motion").enabled(false))
                .id
        }),
        ("a disabled checkbox", |ui| {
            let mut on = false;
            ui.add(Checkbox::new(&mut on, "I wrote it down").enabled(false))
                .id
        }),
        ("a disabled segmented control", |ui| {
            let mut value = 1_u8;
            let options = [(1_u8, "Light"), (2_u8, "Dark")];
            ui.add(Segmented::new("mode", &mut value, &options).enabled(false));
            Id::new(("monolith-segmented", "mode")).with(("segment", 2_u8))
        }),
        ("a disabled menu item", |ui| {
            ui.add(MenuItem::new("Delete").enabled(false)).id
        }),
        ("a disabled conversation row", |ui| {
            ui.add(ConversationRow::new("a", "Alice", &[1]).enabled(false))
                .id
        }),
        ("a disabled contact row", |ui| {
            ui.add(ContactRow::new("a", "Alice", &[1]).enabled(false))
                .id
        }),
        ("a disabled settings row", |ui| {
            SettingsGroup::new("group").show(ui, |rows| {
                rows.row(SettingsRow::new("row", "Export").navigates().enabled(false))
                    .id
            })
        }),
        ("a disabled text field", |ui| {
            let mut text = String::from("Alice");
            let field = TextField::new("name", &mut text).enabled(false);
            let id = field.edit_id();
            field.show(ui);
            id
        }),
        ("a disabled search field", |ui| {
            let mut text = String::new();
            let field = SearchField::new("filter", &mut text).enabled(false);
            let id = field.edit_id();
            field.show(ui);
            id
        }),
    ]
}

/// Shows what `make` adds, with AccessKit on; returns the rig and the id
/// of the control.
fn render(make: Make) -> (Rig, Id) {
    let mut rig = Rig::new();
    rig.ctx.enable_accesskit();
    let mut id = Id::NULL;
    rig.settle(2, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| id = make(ui));
    });
    (rig, id)
}

#[test]
fn controls_that_take_no_input_tell_accessibility_they_are_disabled() {
    for (what, make) in controls_that_take_no_input() {
        let (mut rig, mut id) = render(make);
        let output = rig.frame(vec![], |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| id = make(ui));
        });
        let node = node(&output, id).unwrap_or_else(|| panic!("{what} has a node"));
        assert!(node.is_disabled(), "{what}");
    }
}

#[test]
fn an_enabled_button_is_not_reported_disabled() {
    let make: Make = |ui| ui.add(Button::primary("Create profile")).id;
    let (mut rig, mut id) = render(make);
    let output = rig.frame(vec![], |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| id = make(ui));
    });
    assert!(!node(&output, id).unwrap().is_disabled());
}

#[test]
fn the_pointer_over_a_control_that_takes_no_input_stays_an_arrow() {
    let pointer_over = |make: Make| {
        let (mut rig, mut id) = render(make);
        let rect = rig.ctx.read_response(id).expect("shown").rect;
        let mut outputs = Vec::new();
        for _ in 0..2 {
            outputs.push(rig.frame(vec![Event::PointerMoved(rect.center())], |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| id = make(ui));
            }));
        }
        outputs.last().unwrap().platform_output.cursor_icon
    };
    assert_eq!(
        pointer_over(|ui| ui.add(Button::primary("Create profile")).id),
        CursorIcon::PointingHand
    );
    for make in [
        (|ui| ui.add(Button::primary("Create profile").enabled(false)).id) as Make,
        |ui| ui.add(Button::primary("Creating").busy(true)).id,
        |ui| ui.add(MenuItem::new("Delete").enabled(false)).id,
        |ui| {
            ui.add(ConversationRow::new("a", "Alice", &[1]).enabled(false))
                .id
        },
    ] {
        assert_eq!(pointer_over(make), CursorIcon::Default);
    }
}

#[test]
fn tab_passes_over_message_bubbles_and_the_frame_of_the_search_field() {
    let mut search_id = Id::NULL;
    let mut after_id = Id::NULL;
    let mut run = |ctx: &egui::Context| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let mut filter = String::new();
            let field = SearchField::new("filter", &mut filter);
            search_id = field.edit_id();
            field.show(ui);
            for (key, direction) in [(1, Direction::Incoming), (2, Direction::Outgoing)] {
                MessageBubble::new(key, direction, "See you at nine", "09:41").show(ui);
            }
            after_id = ui.add(Button::primary("After")).id;
        });
    };
    let mut rig = Rig::new();
    rig.settle(2, &mut run);
    let mut stops = Vec::new();
    for _ in 0..6 {
        rig.key(Key::Tab, &mut run);
        if let Some(id) = rig.ctx.memory(|memory| memory.focused()) {
            if !stops.contains(&id) {
                stops.push(id);
            }
        }
    }
    assert_eq!(stops, vec![search_id, after_id]);
}

#[test]
fn tab_passes_over_scroll_bars_drag_areas_and_the_edge_of_the_sidebar() {
    // A sidebar that can be resized, with a list that scrolls, beside a
    // composer whose draft is taller than it: each has parts that egui
    // makes stops of the Tab key unless told otherwise.
    let mut draft = "A line of the draft\n".repeat(30);
    let mut run = |ctx: &egui::Context| {
        SidebarPanel::new("side").show(ctx, |ui| {
            ui.add(Button::secondary("Left"));
            scroll_area("list").show(ui, |ui| {
                for index in 0..60 {
                    ui.add(Text::new(
                        &format!("Row {index}"),
                        &theme::current(ui.ctx()).text.body,
                        theme::current(ui.ctx()).color.text,
                    ));
                }
            });
        });
        ContentPanel.show(ctx, |ui| {
            ui.add(Button::secondary("Right"));
            Composer::new("composer", &mut draft).show(ui);
        });
    };
    let mut rig = Rig::sized(Theme::light(), egui::vec2(900.0, 600.0));
    rig.settle(3, &mut run);
    let mut stops = Vec::new();
    for _ in 0..16 {
        let outputs = rig.key(Key::Tab, &mut run);
        let labels = focused_labels(&outputs);
        let focused = rig.ctx.memory(|memory| memory.focused());
        assert!(
            !labels.is_empty(),
            "Tab stopped on {focused:?}, which says nothing, after {stops:?}"
        );
        stops.extend(labels);
    }
    // Every control was reached.
    for label in ["Left", "Right", "Send message"] {
        assert!(stops.iter().any(|stop| stop == label), "{label}: {stops:?}");
    }
}

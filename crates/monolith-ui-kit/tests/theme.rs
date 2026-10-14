//! What the theme sets in egui's own style: motion, text selection, the
//! scroll bar and the edge of the sidebar.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use std::cell::Cell;

use common::Rig;
use egui::epaint::Shape;
use egui::style::ScrollAnimation;
use egui::{Color32, Context, CursorIcon, Event, FullOutput, Rect, Ui, pos2};
use monolith_ui_kit::chrome::{ContentPanel, SidebarPanel};
use monolith_ui_kit::text::Text;
use monolith_ui_kit::{Theme, theme};

#[test]
fn reduced_motion_stops_the_caret_blinking_and_scrolling_from_gliding() {
    let moving = Theme::light().egui_style();
    assert!(moving.visuals.text_cursor.blink);
    assert_ne!(moving.scroll_animation, ScrollAnimation::none());
    let still = Theme::dark().with_reduced_motion(true).egui_style();
    assert!(!still.visuals.text_cursor.blink);
    assert_eq!(still.scroll_animation, ScrollAnimation::none());
}

#[test]
fn a_scroll_bar_shows_a_faint_handle_at_rest() {
    for theme in [Theme::light(), Theme::dark()] {
        let scroll = theme.egui_style().spacing.scroll;
        assert!(scroll.dormant_handle_opacity >= 0.2, "{scroll:?}");
        assert!(scroll.dormant_handle_opacity < scroll.active_handle_opacity);
    }
}

/// The pointer over the text added by `add`.
fn cursor_over(add: fn(&mut Ui) -> egui::Response) -> CursorIcon {
    let rect = Cell::new(Rect::NOTHING);
    let run = |ctx: &Context| {
        egui::CentralPanel::default().show(ctx, |ui| rect.set(add(ui).rect));
    };
    let mut rig = Rig::new();
    rig.settle(2, run);
    let at = rect.get().center();
    rig.frame(vec![Event::PointerMoved(at)], run);
    rig.frame(vec![], run).platform_output.cursor_icon
}

#[test]
fn text_is_a_text_target_only_where_it_is_selectable() {
    let plain: fn(&mut Ui) -> egui::Response = |ui| {
        let theme = theme::current(ui.ctx());
        ui.add(Text::new("Settings", &theme.text.body, theme.color.text))
    };
    let label: fn(&mut Ui) -> egui::Response = |ui| ui.label("Settings");
    let selectable: fn(&mut Ui) -> egui::Response = |ui| {
        let theme = theme::current(ui.ctx());
        ui.add(Text::new("AF3K 72PQ", &theme.text.mono, theme.color.text).selectable(true))
    };
    assert_ne!(cursor_over(plain), CursorIcon::Text);
    assert_ne!(cursor_over(label), CursorIcon::Text);
    assert_eq!(cursor_over(selectable), CursorIcon::Text);
}

/// The colours of the vertical lines painted at `x`, in paint order.
fn lines_at(output: &FullOutput, x: f32) -> Vec<Color32> {
    let mut found = Vec::new();
    for clipped in &output.shapes {
        if let Shape::LineSegment { points, stroke } = &clipped.shape {
            if (points[0].x - x).abs() < 1.0 && (points[1].x - x).abs() < 1.0 {
                found.push(stroke.color);
            }
        }
    }
    found
}

/// A sidebar and the content; returns the right edge of the sidebar.
fn window(ctx: &Context) -> f32 {
    let sidebar = SidebarPanel::new("sidebar").show(ctx, |ui| {
        ui.label("Conversations");
    });
    ContentPanel.show(ctx, |ui| {
        ui.label("Messages");
    });
    sidebar.response.rect.right()
}

#[test]
fn the_sidebar_edge_under_the_pointer_takes_a_colour_of_the_theme() {
    let theme = Theme::light();
    let mut rig = Rig::new();
    let edge = Cell::new(0.0);
    let run = |ctx: &Context| edge.set(window(ctx));
    rig.settle(2, run);
    let at = pos2(edge.get(), 300.0);
    rig.frame(vec![Event::PointerMoved(at)], run);
    let out = rig.frame(vec![], run);
    // egui marks the edge in the colour of text; the theme's mark is
    // painted last, over it.
    let lines = lines_at(&out, edge.get() - 0.5);
    assert_eq!(lines.last(), Some(&theme.color.border_strong), "{lines:?}");
}

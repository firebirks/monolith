//! The frame of a window without stops of the Tab key of its own: the bar
//! of a scroll area and the edge of the sidebar still follow the pointer.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use std::cell::Cell;

use common::{Rig, button, paints};
use egui::{Context, CursorIcon, Event, Pos2, Rect, pos2, vec2};
use monolith_ui_kit::chrome::{ContentPanel, SidebarPanel, scroll_area};
use monolith_ui_kit::text::Text;
use monolith_ui_kit::{Theme, theme};

/// A list of a hundred rows in a scroll area; `view` is set to where the
/// area shows them.
fn list(ctx: &Context, view: &Cell<Rect>) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let theme = theme::current(ui.ctx());
        let output = scroll_area("list").show(ui, |ui| {
            for index in 0..100 {
                ui.add(Text::new(
                    &format!("Row {index}"),
                    &theme.text.body,
                    theme.color.text,
                ));
            }
        });
        view.set(output.inner_rect);
    });
}

/// Presses at `from`, moves to `to` and lets go, a frame for each, then
/// lets layout settle.
fn drag(rig: &mut Rig, from: Pos2, to: Pos2, mut run: impl FnMut(&Context)) {
    rig.frame(vec![Event::PointerMoved(from)], &mut run);
    rig.frame(vec![button(from, true)], &mut run);
    for step in 1..=4 {
        let at = from + (to - from) * (step as f32 / 4.0);
        rig.frame(vec![Event::PointerMoved(at)], &mut run);
    }
    rig.frame(vec![button(to, false)], &mut run);
    rig.settle(3, &mut run);
}

#[test]
fn the_bar_of_a_scroll_area_is_dragged_with_the_pointer() {
    let view = Cell::new(Rect::NOTHING);
    let mut rig = Rig::new();
    let before = rig.settle(3, |ctx| list(ctx, &view));
    assert!(paints(&before, "Row 0") && !paints(&before, "Row 99"));
    let area = view.get();
    let x = area.right() - 4.0;
    // The handle is at the top; it is taken there and moved to the bottom.
    drag(
        &mut rig,
        pos2(x, area.top() + 4.0),
        pos2(x, area.bottom() + 40.0),
        |ctx| list(ctx, &view),
    );
    let after = rig.frame(vec![], |ctx| list(ctx, &view));
    assert!(
        paints(&after, "Row 99"),
        "the end of the list is not in view"
    );
    assert!(!paints(&after, "Row 0"));
    // And back to the top.
    drag(
        &mut rig,
        pos2(x, area.bottom() - 4.0),
        pos2(x, area.top() - 40.0),
        |ctx| list(ctx, &view),
    );
    let back = rig.frame(vec![], |ctx| list(ctx, &view));
    assert!(paints(&back, "Row 0"));
}

#[test]
fn a_press_on_the_bar_below_the_handle_scrolls_there() {
    let view = Cell::new(Rect::NOTHING);
    let mut rig = Rig::new();
    rig.settle(3, |ctx| list(ctx, &view));
    let area = view.get();
    let at = pos2(area.right() - 4.0, area.bottom() - 2.0);
    rig.frame(vec![Event::PointerMoved(at)], |ctx| list(ctx, &view));
    rig.frame(vec![button(at, true)], |ctx| list(ctx, &view));
    rig.frame(vec![button(at, false)], |ctx| list(ctx, &view));
    rig.settle(3, |ctx| list(ctx, &view));
    let after = rig.frame(vec![], |ctx| list(ctx, &view));
    assert!(paints(&after, "Row 99"));
}

#[test]
fn the_wheel_scrolls_the_list_and_text_is_not_dragged() {
    let view = Cell::new(Rect::NOTHING);
    let mut rig = Rig::new();
    rig.settle(3, |ctx| list(ctx, &view));
    let area = view.get();
    // A drag over the rows is no scroll.
    drag(
        &mut rig,
        area.center(),
        area.center() - vec2(0.0, 200.0),
        |ctx| list(ctx, &view),
    );
    let dragged = rig.frame(vec![], |ctx| list(ctx, &view));
    assert!(paints(&dragged, "Row 0"));
    rig.frame(
        vec![
            Event::PointerMoved(area.center()),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -10_000.0),
                modifiers: egui::Modifiers::NONE,
            },
        ],
        |ctx| list(ctx, &view),
    );
    rig.settle(3, |ctx| list(ctx, &view));
    let wheeled = rig.frame(vec![], |ctx| list(ctx, &view));
    assert!(paints(&wheeled, "Row 99"));
}

/// A sidebar, its contents as wide as it is, beside the content; `edge` is
/// set to where its right edge is.
fn window(ctx: &Context, edge: &Cell<f32>) {
    let shown = SidebarPanel::new("side").show(ctx, |ui| {
        ui.allocate_space(ui.available_size());
    });
    edge.set(shown.response.rect.right());
    ContentPanel.show(ctx, |_| {});
}

#[test]
fn the_edge_of_the_sidebar_is_dragged_with_the_pointer_within_its_range() {
    let theme = Theme::light();
    let edge = Cell::new(0.0);
    let mut rig = Rig::sized(theme.clone(), vec2(900.0, 600.0));
    rig.settle(3, |ctx| window(ctx, &edge));
    assert_eq!(edge.get(), theme.size.sidebar);
    let at = pos2(edge.get(), 300.0);
    rig.frame(vec![Event::PointerMoved(at)], |ctx| window(ctx, &edge));
    let over = rig.frame(vec![], |ctx| window(ctx, &edge));
    assert_eq!(
        over.platform_output.cursor_icon,
        CursorIcon::ResizeHorizontal
    );
    drag(&mut rig, at, pos2(380.0, 300.0), |ctx| window(ctx, &edge));
    assert_eq!(edge.get(), 380.0);
    // No wider than the theme allows.
    drag(&mut rig, pos2(380.0, 300.0), pos2(700.0, 300.0), |ctx| {
        window(ctx, &edge);
    });
    assert_eq!(edge.get(), theme.size.sidebar_max);
}

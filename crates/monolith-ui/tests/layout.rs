//! The window at the sizes it can have: what does not fit can be scrolled
//! to.

#![cfg(feature = "fixtures")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use egui::{Context, Event, MouseWheelUnit, Pos2, Vec2};
use monolith_ui::fixtures::fixture;
use monolith_ui_kit::{Theme, fonts, theme};
use monolith_ui_snapshot::{Image, Options, render};

fn context() -> Context {
    let ctx = Context::default();
    fonts::install(&ctx, &[]);
    theme::install(&ctx, Theme::light().with_reduced_motion(true));
    ctx
}

/// Renders `name` at the smallest window, then again after the wheel
/// turned down over the middle of the window.
fn before_and_after_scrolling(name: &str) -> (Image, Image) {
    let ctx = context();
    let mut app = fixture(name).unwrap();
    let size = Theme::light().size.window_min;
    let middle = Pos2::new(size.x / 2.0, size.y / 2.0);
    let options = Options {
        size,
        pointer: Some(middle),
        background: Theme::light().color.bg_app,
        ..Options::default()
    };
    let before = render(&ctx, &options, |ctx| monolith_ui::show(ctx, &mut app));
    let wheel = Options {
        events: vec![Event::MouseWheel {
            unit: MouseWheelUnit::Point,
            delta: Vec2::new(0.0, -400.0),
            modifiers: egui::Modifiers::NONE,
        }],
        start_time: 60.0,
        ..options
    };
    let after = render(&ctx, &wheel, |ctx| monolith_ui::show(ctx, &mut app));
    (before, after)
}

#[test]
fn the_first_start_scrolls_in_the_smallest_window() {
    let (before, after) = before_and_after_scrolling("welcome");
    assert!(
        before.differs(&after, 0.01),
        "the welcome screen did not move under the wheel"
    );
}

#[test]
fn the_settings_scroll_in_the_smallest_window() {
    let (before, after) = before_and_after_scrolling("settings-identities");
    assert!(
        before.differs(&after, 0.01),
        "the identities page did not move under the wheel"
    );
}

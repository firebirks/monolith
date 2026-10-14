//! Toasts: where they sit, how they stack, and what a screen reader is
//! told of them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use common::{Rig, node};
use egui::accesskit::Live;
use egui::{Id, Rect, pos2};
use monolith_ui_kit::Theme;
use monolith_ui_kit::toast::{Kind, Toasts};

fn card(rig: &Rig, toast: u64) -> Rect {
    rig.ctx
        .read_response(Id::new(("monolith-toast", toast)))
        .expect("the toast was shown")
        .rect
}

#[test]
fn a_toast_sits_at_the_top_centre_under_the_top_bar() {
    let theme = Theme::light();
    let mut rig = Rig::new();
    let mut toasts = Toasts::new();
    let id = toasts.push(Kind::Success, "Card copied");
    rig.settle(3, |ctx| {
        toasts.show(ctx);
    });
    let rect = card(&rig, id);
    let screen = rig.screen();
    assert!(
        (rect.center().x - screen.center().x).abs() < 1.0,
        "{rect:?}"
    );
    assert!(rect.top() >= theme.size.topbar, "{rect:?}");
    assert!(rect.bottom() < screen.center().y, "{rect:?}");
}

#[test]
fn a_new_toast_goes_under_the_last_and_inside_the_window_in_the_frame_it_comes() {
    let mut rig = Rig::new();
    let mut toasts = Toasts::new();
    let first = toasts.push(Kind::Info, "Tor is connecting");
    rig.settle(3, |ctx| {
        toasts.show(ctx);
    });
    let second = toasts.push(Kind::Error, "The message could not be sent");
    rig.frame(vec![], |ctx| {
        toasts.show(ctx);
    });
    let (above, below) = (card(&rig, first), card(&rig, second));
    assert!(below.top() >= above.bottom(), "{above:?} {below:?}");
    assert!(rig.screen().contains_rect(below), "{below:?}");
}

#[test]
fn toasts_shown_in_an_area_are_centred_on_it() {
    let theme = Theme::light();
    let mut rig = Rig::new();
    let mut toasts = Toasts::new();
    let id = toasts.push(Kind::Info, "Card copied");
    let content = Rect::from_min_max(pos2(120.0, 0.0), pos2(400.0, 600.0));
    rig.settle(3, |ctx| {
        toasts.show_in(ctx, content);
    });
    let rect = card(&rig, id);
    assert!(
        (rect.center().x - content.center().x).abs() < 1.0,
        "{rect:?}"
    );
    assert!(rect.top() >= content.top() + theme.size.topbar);
}

#[test]
fn a_toast_is_a_live_region_and_an_error_interrupts() {
    let mut rig = Rig::new();
    rig.ctx.enable_accesskit();
    let mut toasts = Toasts::new();
    let info = toasts.push(Kind::Info, "Card copied");
    let error = toasts.push(Kind::Error, "The message could not be sent");
    let out = rig.settle(3, |ctx| {
        toasts.show(ctx);
    });
    let live = |toast: u64| {
        node(&out, Id::new(("monolith-toast", toast)))
            .expect("the toast has a node")
            .live()
    };
    assert_eq!(live(info), Some(Live::Polite));
    assert_eq!(live(error), Some(Live::Assertive));
}

//! A window that nothing happens to does not draw itself again and again:
//! once its layout and animations settle, a frame with no input asks for no
//! other frame. Only a spinner keeps drawing, while it turns.
//!
//! The text cursor blinks (twice a second, a frame each time) while a
//! field has the focus; that is switched off here, so that what remains is
//! what the screens ask for.

#![cfg(feature = "fixtures")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use core::time::Duration;

use egui::{Context, Pos2, RawInput, Rect, ViewportId};
use monolith_ui::fixtures::{NAMES, fixture};
use monolith_ui_kit::{Theme, fonts, theme};

/// The repaint the last of `frames` frames of `name` asks for, with
/// `theme` and time going on half a second a frame.
fn repaint_after_settling(name: &str, theme: Theme, frames: u32) -> Duration {
    let ctx = Context::default();
    fonts::install(&ctx, &[]);
    theme::install(&ctx, theme);
    ctx.style_mut(|style| style.visuals.text_cursor.blink = false);
    let mut app = fixture(name).unwrap();
    let size = Theme::light().size.window_default;
    let mut delay = Duration::ZERO;
    for frame in 0..frames {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            time: Some(f64::from(frame) * 0.5),
            focused: true,
            ..RawInput::default()
        };
        let output = ctx.run(input, |ctx| monolith_ui::show(ctx, &mut app));
        delay = output
            .viewport_output
            .get(&ViewportId::ROOT)
            .map_or(Duration::MAX, |viewport| viewport.repaint_delay);
    }
    delay
}

#[test]
fn a_settled_window_asks_for_no_frame() {
    for name in NAMES.iter().filter(|name| **name != "opening") {
        for (theme, reduced) in [(Theme::light(), false), (Theme::dark(), true)] {
            let delay = repaint_after_settling(name, theme.with_reduced_motion(reduced), 30);
            assert_eq!(
                delay,
                Duration::MAX,
                "{name} (reduced motion {reduced}) asks for a frame in {delay:?}"
            );
        }
    }
}

#[test]
fn a_spinner_draws_while_it_turns() {
    let delay = repaint_after_settling("opening", Theme::light(), 30);
    assert!(delay < Duration::from_millis(100), "{delay:?}");
}

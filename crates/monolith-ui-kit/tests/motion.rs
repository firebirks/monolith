//! How often components ask for another frame.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use core::time::Duration;

use common::Rig;
use egui::epaint::Shape;
use egui::{Context, FullOutput, ViewportId};
use monolith_ui_kit::empty_state::EmptyState;
use monolith_ui_kit::tor_status::{TorState, TorStatus};
use monolith_ui_kit::{Theme, icons, theme};

fn repaint_delay(output: &FullOutput) -> Duration {
    output
        .viewport_output
        .get(&ViewportId::ROOT)
        .map_or(Duration::MAX, |viewport| viewport.repaint_delay)
}

fn tor(ctx: &Context, state: TorState) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.add(TorStatus::new(state, "Connecting to Tor"));
    });
}

#[test]
fn the_connecting_pulse_asks_for_four_frames_a_second() {
    let mut rig = Rig::new();
    theme::install(&rig.ctx, Theme::light());
    let out = rig.settle(3, |ctx| tor(ctx, TorState::Connecting));
    // egui takes the expected time of a frame off the delay asked for.
    let delay = repaint_delay(&out);
    assert!(delay > Duration::from_millis(200), "{delay:?}");
    assert!(delay <= Duration::from_millis(250), "{delay:?}");
}

#[test]
fn the_connecting_pulse_is_still_with_reduced_motion() {
    let mut rig = Rig::new();
    let out = rig.settle(3, |ctx| tor(ctx, TorState::Connecting));
    assert_eq!(repaint_delay(&out), Duration::MAX);
}

fn empty(ctx: &Context) {
    egui::CentralPanel::default().show(ctx, |ui| {
        EmptyState::new("none", icons::MESSAGE_SQUARE, "No conversations yet")
            .body("Add a contact to start.")
            .primary("Add contact")
            .secondary("Invite")
            .show(ui);
    });
}

/// Where the label `needle` is painted in `output`: its left and right.
fn label_span(output: &FullOutput, needle: &str) -> (f32, f32) {
    output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            Shape::Text(text) if text.galley.text() == needle => {
                Some((text.pos.x, text.pos.x + text.galley.size().x))
            }
            _ => None,
        })
        .expect("the label is painted")
}

#[test]
fn the_first_frame_of_an_empty_state_shows_its_actions_centred() {
    let mut rig = Rig::new();
    let first = rig.frame(vec![], empty);
    // The pass that measured the actions was laid out again, not shown.
    assert_eq!(first.platform_output.num_completed_passes, 2);
    let (left, _) = label_span(&first, "Add contact");
    let (_, right) = label_span(&first, "Invite");
    let centre = rig.screen().center().x;
    assert!(
        ((left + right) / 2.0 - centre).abs() < 1.5,
        "{left} {right}"
    );
    let next = rig.frame(vec![], empty);
    assert_eq!(next.platform_output.num_completed_passes, 1);
}

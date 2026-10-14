//! Modal state: what closes it, what does not, and what it shields.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use common::{Rig, paints};
use egui::{Context, Id, Key, pos2};
use monolith_ui_kit::Button;
use monolith_ui_kit::modal::{Decision, Modal};

/// Shows a modal and tells whether it asked to close.
fn show(ctx: &Context, dismissable: bool, requested: &mut bool) {
    let out = Modal::new("dialog", "Rename")
        .subtitle("Pick a new name")
        .dismissable(dismissable)
        .show(
            ctx,
            |ui| {
                ui.label("body text");
            },
            |ui| {
                ui.add(Button::primary("Save"));
            },
        );
    *requested |= out.close_requested;
}

fn open(dismissable: bool) -> Rig {
    let mut rig = Rig::new();
    let mut requested = false;
    rig.settle(3, |ctx| show(ctx, dismissable, &mut requested));
    assert!(!requested);
    rig
}

#[test]
fn a_modal_paints_its_title_subtitle_body_and_actions() {
    let mut rig = Rig::new();
    let mut requested = false;
    let out = rig.settle(3, |ctx| show(ctx, true, &mut requested));
    for text in ["Rename", "Pick a new name", "body text", "Save"] {
        assert!(paints(&out, text), "{text}");
    }
}

#[test]
fn escape_asks_a_dismissable_modal_to_close() {
    let mut rig = open(true);
    let mut requested = false;
    rig.key(Key::Escape, |ctx| show(ctx, true, &mut requested));
    assert!(requested);
}

#[test]
fn escape_does_not_close_a_modal_that_is_not_dismissable() {
    let mut rig = open(false);
    let mut requested = false;
    rig.key(Key::Escape, |ctx| show(ctx, false, &mut requested));
    assert!(!requested);
}

#[test]
fn a_click_on_the_scrim_asks_a_dismissable_modal_to_close() {
    let mut rig = open(true);
    let mut requested = false;
    rig.click(pos2(5.0, 5.0), |ctx| show(ctx, true, &mut requested));
    assert!(requested);
}

#[test]
fn a_click_on_the_scrim_does_not_close_a_modal_that_is_not_dismissable() {
    let mut rig = open(false);
    let mut requested = false;
    rig.click(pos2(5.0, 5.0), |ctx| show(ctx, false, &mut requested));
    assert!(!requested);
}

#[test]
fn a_click_on_the_card_does_not_close_the_modal() {
    let mut rig = open(true);
    let card = rig
        .ctx
        .read_response(Id::new(("monolith-modal", "dialog")).with("card"))
        .expect("the card was shown")
        .rect;
    let mut requested = false;
    rig.click(card.center(), |ctx| show(ctx, true, &mut requested));
    assert!(!requested);
}

#[test]
fn nothing_behind_a_modal_can_be_clicked() {
    let mut requested = false;
    let mut frame = |ctx: &Context, clicks: &mut u32, modal: bool| {
        egui::CentralPanel::default().show(ctx, |ui| {
            if ui.add(Button::primary("Behind")).clicked() {
                *clicks += 1;
            }
        });
        if modal {
            show(ctx, true, &mut requested);
        }
    };
    // The button sits at the top left of the window, outside the card. It
    // takes the click when no modal is open.
    let mut rig = Rig::new();
    let mut clicks = 0;
    rig.settle(3, |ctx| frame(ctx, &mut clicks, false));
    rig.click(pos2(30.0, 25.0), |ctx| frame(ctx, &mut clicks, false));
    assert_eq!(clicks, 1);
    let mut rig = Rig::new();
    let mut clicks = 0;
    rig.settle(3, |ctx| frame(ctx, &mut clicks, true));
    rig.click(pos2(30.0, 25.0), |ctx| frame(ctx, &mut clicks, true));
    assert_eq!(clicks, 0);
}

fn confirm(ctx: &Context, busy: bool, decision: &mut Option<Decision>) {
    if let Some(answer) = Modal::new("confirm", "Delete").confirm(
        ctx,
        "This cannot be undone",
        "Cancel",
        "Delete",
        busy,
    ) {
        *decision = Some(answer);
    }
}

#[test]
fn a_confirmation_shows_its_message_and_both_answers() {
    let mut rig = Rig::new();
    let mut decision = None;
    let out = rig.settle(3, |ctx| confirm(ctx, false, &mut decision));
    for text in ["Delete", "This cannot be undone", "Cancel"] {
        assert!(paints(&out, text), "{text}");
    }
    assert_eq!(decision, None);
}

#[test]
fn escape_cancels_a_confirmation() {
    let mut rig = Rig::new();
    let mut decision = None;
    rig.settle(3, |ctx| confirm(ctx, false, &mut decision));
    rig.key(Key::Escape, |ctx| confirm(ctx, false, &mut decision));
    assert_eq!(decision, Some(Decision::Cancel));
}

#[test]
fn enter_on_a_new_confirmation_cancels_instead_of_destroying() {
    let mut rig = Rig::new();
    let mut decision = None;
    rig.settle(3, |ctx| confirm(ctx, false, &mut decision));
    rig.key(Key::Enter, |ctx| confirm(ctx, false, &mut decision));
    assert_eq!(decision, Some(Decision::Cancel));
}

#[test]
fn a_busy_confirmation_answers_nothing() {
    let mut rig = Rig::new();
    let mut decision = None;
    rig.settle(3, |ctx| confirm(ctx, true, &mut decision));
    rig.key(Key::Escape, |ctx| confirm(ctx, true, &mut decision));
    rig.key(Key::Enter, |ctx| confirm(ctx, true, &mut decision));
    rig.click(pos2(5.0, 5.0), |ctx| confirm(ctx, true, &mut decision));
    assert_eq!(decision, None);
}

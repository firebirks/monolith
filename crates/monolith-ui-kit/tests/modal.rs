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
use egui::{Context, Event, Id, Key, Modifiers, MouseWheelUnit, Rect, pos2, vec2};
use monolith_ui_kit::modal::{Decision, Modal};
use monolith_ui_kit::{Button, TextField, Theme};

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

#[test]
fn the_keyboard_reaches_nothing_behind_a_modal() {
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
    // Without a modal, Tab reaches the button and Enter presses it.
    let mut rig = Rig::new();
    let mut clicks = 0;
    rig.settle(3, |ctx| frame(ctx, &mut clicks, false));
    rig.key(Key::Tab, |ctx| frame(ctx, &mut clicks, false));
    rig.key(Key::Enter, |ctx| frame(ctx, &mut clicks, false));
    assert_eq!(clicks, 1);
    // With one, no number of Tabs gives it the focus.
    let mut rig = Rig::new();
    let mut clicks = 0;
    rig.settle(3, |ctx| frame(ctx, &mut clicks, true));
    for _ in 0..4 {
        rig.key(Key::Tab, |ctx| frame(ctx, &mut clicks, true));
        rig.key(Key::Enter, |ctx| frame(ctx, &mut clicks, true));
        rig.key(Key::Space, |ctx| frame(ctx, &mut clicks, true));
    }
    assert_eq!(clicks, 0);
}

/// Where the parts of a tall modal were, and whether Save was clicked.
#[derive(Clone, Copy)]
struct Tall {
    card: Rect,
    save: Rect,
    field_id: Id,
    save_id: Id,
    cancel_id: Id,
    saved: bool,
}

impl Default for Tall {
    fn default() -> Self {
        Self {
            card: Rect::NOTHING,
            save: Rect::NOTHING,
            field_id: Id::NULL,
            save_id: Id::NULL,
            cancel_id: Id::NULL,
            saved: false,
        }
    }
}

/// A modal with a field and forty lines of body, as tall as a contact's
/// details.
fn tall(ctx: &Context, seen: &mut Tall) {
    Modal::new("tall", "Contact").show(
        ctx,
        |ui| {
            let mut name = String::from("Alice");
            let field = TextField::new("name", &mut name).label("Name");
            seen.field_id = field.edit_id();
            field.show(ui);
            for line in 0..40 {
                ui.label(format!("Line {line}"));
            }
        },
        |ui| {
            let save = ui.add(Button::primary("Save"));
            let cancel = ui.add(Button::secondary("Cancel"));
            seen.save = save.rect;
            seen.save_id = save.id;
            seen.cancel_id = cancel.id;
            seen.saved |= save.clicked();
        },
    );
    if let Some(card) = ctx.read_response(Id::new(("monolith-modal", "tall")).with("card")) {
        seen.card = card.rect;
    }
}

/// The smallest window the client allows.
fn smallest() -> Rig {
    Rig::sized(Theme::light(), Theme::light().size.window_min)
}

#[test]
fn a_tall_modal_keeps_its_title_and_actions_inside_the_smallest_window() {
    let mut rig = smallest();
    let mut seen = Tall::default();
    rig.settle(4, |ctx| tall(ctx, &mut seen));
    let screen = rig.screen();
    assert!(screen.contains_rect(seen.card), "{:?}", seen.card);
    assert!(screen.contains_rect(seen.save), "{:?}", seen.save);
    let at = seen.save.center();
    rig.click(at, |ctx| tall(ctx, &mut seen));
    assert!(seen.saved);
}

#[test]
fn a_short_modal_is_only_as_tall_as_its_content() {
    let mut rig = Rig::new();
    let mut requested = false;
    rig.settle(3, |ctx| show(ctx, true, &mut requested));
    let card = rig
        .ctx
        .read_response(Id::new(("monolith-modal", "dialog")).with("card"))
        .expect("the card was shown")
        .rect;
    assert!(card.height() < 260.0, "{card:?}");
    assert!((card.center().y - rig.screen().center().y).abs() < 1.0);
}

#[test]
fn the_body_of_a_tall_modal_scrolls_under_the_wheel() {
    let mut rig = smallest();
    let mut seen = Tall::default();
    let before = rig.settle(4, |ctx| tall(ctx, &mut seen));
    assert!(paints(&before, "Line 0"));
    assert!(!paints(&before, "Line 39"));
    let over = seen.card.center();
    rig.frame(vec![Event::PointerMoved(over)], |ctx| tall(ctx, &mut seen));
    let wheel = Event::MouseWheel {
        unit: MouseWheelUnit::Point,
        delta: vec2(0.0, -4000.0),
        modifiers: Modifiers::NONE,
    };
    rig.frame(vec![wheel], |ctx| tall(ctx, &mut seen));
    let after = rig.settle(3, |ctx| tall(ctx, &mut seen));
    assert!(paints(&after, "Line 39"));
    assert!(paints(&after, "Contact") && paints(&after, "Save"));
}

#[test]
fn the_scrim_and_the_card_are_no_tab_stops() {
    let mut rig = Rig::new();
    let mut seen = Tall::default();
    rig.settle(3, |ctx| tall(ctx, &mut seen));
    let mut stops = Vec::new();
    for _ in 0..10 {
        rig.key(Key::Tab, |ctx| tall(ctx, &mut seen));
        if let Some(id) = rig.ctx.memory(|memory| memory.focused()) {
            if !stops.contains(&id) {
                stops.push(id);
            }
        }
    }
    // The field, the two actions and the scroll bar of the body, which
    // egui makes a stop of its own; nothing else.
    assert_eq!(stops.len(), 4, "{stops:?}");
    for id in [seen.field_id, seen.save_id, seen.cancel_id] {
        assert!(stops.contains(&id));
    }
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

//! The focus ring: shown when the keyboard moved the focus, not when a
//! click or a screen gave it; and the composer's own quiet mark of focus.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use common::{Rig, button, paints_focus_ring, rect_strokes};
use egui::{Context, Id, Key, pos2};
use monolith_ui_kit::composer::Composer;
use monolith_ui_kit::{Button, TextField, Theme};

/// Two buttons; the first asks for the focus while `request` is set.
fn buttons(ctx: &Context, request: bool, first: &mut Id) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let response = ui.add(Button::primary("Send"));
        if request {
            response.request_focus();
        }
        *first = response.id;
        ui.add(Button::secondary("Other"));
    });
}

#[test]
fn focus_given_by_a_screen_shows_no_ring_and_focus_reached_with_tab_does() {
    let theme = Theme::light();
    let mut rig = Rig::new();
    let mut first = Id::NULL;
    rig.settle(2, |ctx| buttons(ctx, true, &mut first));
    let given = rig.frame(vec![], |ctx| buttons(ctx, false, &mut first));
    assert!(rig.ctx.memory(|memory| memory.has_focus(first)));
    assert!(!paints_focus_ring(&given, &theme));

    let tabbed = rig.key(Key::Tab, |ctx| buttons(ctx, false, &mut first));
    assert!(paints_focus_ring(tabbed.last().unwrap(), &theme));
}

#[test]
fn a_pointer_press_hides_the_ring_until_the_next_tab() {
    let theme = Theme::light();
    let mut rig = Rig::new();
    let mut first = Id::NULL;
    rig.settle(2, |ctx| buttons(ctx, false, &mut first));
    rig.key(Key::Tab, |ctx| buttons(ctx, false, &mut first));
    let shown = rig.frame(vec![], |ctx| buttons(ctx, false, &mut first));
    assert!(paints_focus_ring(&shown, &theme));

    // A press on an empty part of the window.
    let empty = pos2(300.0, 500.0);
    rig.frame(vec![button(empty, true)], |ctx| {
        buttons(ctx, false, &mut first)
    });
    rig.frame(vec![button(empty, false)], |ctx| {
        buttons(ctx, false, &mut first)
    });
    rig.ctx.memory_mut(|memory| memory.request_focus(first));
    let after = rig.frame(vec![], |ctx| buttons(ctx, false, &mut first));
    assert!(rig.ctx.memory(|memory| memory.has_focus(first)));
    assert!(!paints_focus_ring(&after, &theme));

    rig.key(Key::Tab, |ctx| buttons(ctx, false, &mut first));
    let again = rig.frame(vec![], |ctx| buttons(ctx, false, &mut first));
    assert!(paints_focus_ring(&again, &theme));
}

/// A button, then a text field that asks for the focus while `request`.
fn field(ctx: &Context, request: bool) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.add(Button::secondary("Before"));
        let mut text = String::from("Alice");
        TextField::new("name", &mut text)
            .label("Name")
            .request_focus(request)
            .show(ui);
    });
}

#[test]
fn a_text_field_focused_by_a_screen_has_its_accent_border_and_no_ring() {
    let theme = Theme::light();
    let mut rig = Rig::new();
    rig.settle(3, |ctx| field(ctx, true));
    let out = rig.frame(vec![], |ctx| field(ctx, true));
    assert!(rect_strokes(&out).contains(&theme.color.accent));
    assert!(!paints_focus_ring(&out, &theme));
}

#[test]
fn a_text_field_reached_with_tab_has_the_ring() {
    let theme = Theme::light();
    let mut rig = Rig::new();
    rig.settle(2, |ctx| field(ctx, false));
    rig.key(Key::Tab, |ctx| field(ctx, false));
    let outs = rig.key(Key::Tab, |ctx| field(ctx, false));
    let out = outs.last().unwrap();
    assert!(rect_strokes(out).contains(&theme.color.accent));
    assert!(paints_focus_ring(out, &theme));
}

/// A button, then a composer that asks for the focus while `request`.
fn show_composer(ctx: &Context, draft: &mut String, request: bool) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.add(Button::secondary("Before"));
        Composer::new("chat", draft).request_focus(request).show(ui);
    });
}

#[test]
fn the_composer_focused_by_a_screen_has_a_quiet_border_and_no_ring() {
    let theme = Theme::light();
    let c = &theme.color;
    let mut rig = Rig::new();
    let mut draft = String::new();
    rig.settle(3, |ctx| show_composer(ctx, &mut draft, true));
    let out = rig.frame(vec![], |ctx| show_composer(ctx, &mut draft, true));
    let strokes = rect_strokes(&out);
    assert!(strokes.contains(&c.border_strong), "{strokes:?}");
    assert!(!strokes.contains(&c.accent));
    assert!(!paints_focus_ring(&out, &theme));
}

#[test]
fn the_composer_reached_with_tab_has_the_ring() {
    let theme = Theme::light();
    let mut rig = Rig::new();
    let mut draft = String::new();
    rig.settle(2, |ctx| show_composer(ctx, &mut draft, false));
    rig.key(Key::Tab, |ctx| show_composer(ctx, &mut draft, false));
    let outs = rig.key(Key::Tab, |ctx| show_composer(ctx, &mut draft, false));
    assert!(paints_focus_ring(outs.last().unwrap(), &theme));
}

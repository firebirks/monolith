//! Button states: what a press does in each of them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use common::{Rig, button, clicked_infos, dump};
use egui::{Context, Event, FullOutput, Id, Key, Pos2, Rect, WidgetType, pos2};
use monolith_ui_kit::Button;
use monolith_ui_kit::button::Size;

#[derive(Clone, Copy)]
struct Shown {
    rect: Rect,
    id: Id,
    clicked: bool,
}

fn show(ctx: &Context, shown: &mut Shown, button: Button<'_>) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let response = ui.add(button);
        shown.rect = response.rect;
        shown.id = response.id;
        shown.clicked |= response.clicked();
    });
}

/// A rig with `make` laid out and settled.
fn start(make: &dyn Fn() -> Button<'static>) -> (Rig, Shown) {
    let mut shown = Shown {
        rect: Rect::NOTHING,
        id: Id::NULL,
        clicked: false,
    };
    let mut rig = Rig::new();
    rig.settle(2, |ctx| show(ctx, &mut shown, make()));
    (rig, shown)
}

/// Presses and releases the pointer on the button made by `make`.
fn pointer_click(make: &dyn Fn() -> Button<'static>) -> (Shown, Vec<FullOutput>) {
    let (mut rig, mut shown) = start(make);
    let center = shown.rect.center();
    let outputs = rig.click(center, |ctx| show(ctx, &mut shown, make()));
    (shown, outputs)
}

/// Focuses the button made by `make`, then presses and releases `key`.
fn key_press(make: &dyn Fn() -> Button<'static>, key: Key) -> Shown {
    let (mut rig, mut shown) = start(make);
    rig.ctx.memory_mut(|memory| memory.request_focus(shown.id));
    rig.frame(vec![], |ctx| show(ctx, &mut shown, make()));
    rig.key(key, |ctx| show(ctx, &mut shown, make()));
    shown
}

#[test]
fn an_enabled_button_is_clicked_by_a_press_and_release_on_it() {
    let (shown, _) = pointer_click(&|| Button::primary("Send"));
    assert!(shown.clicked);
}

#[test]
fn every_kind_and_size_of_button_is_clicked_alike() {
    for size in [Size::Small, Size::Medium, Size::Large] {
        let makes: [&dyn Fn() -> Button<'static>; 4] = [
            &|| Button::primary("Go").size(size),
            &|| Button::secondary("Go").size(size),
            &|| Button::ghost("Go").size(size),
            &|| Button::danger("Go").size(size),
        ];
        for make in makes {
            let (shown, _) = pointer_click(make);
            assert!(shown.clicked, "{:?}", make());
        }
    }
}

#[test]
fn a_disabled_button_is_never_clicked() {
    let (shown, _) = pointer_click(&|| Button::primary("Send").enabled(false));
    assert!(!shown.clicked);
}

#[test]
fn a_busy_button_is_not_clicked() {
    let (shown, _) = pointer_click(&|| Button::danger("Delete").busy(true));
    assert!(!shown.clicked);
}

#[test]
fn a_button_in_a_disabled_ui_is_not_clicked() {
    let (mut rig, shown) = start(&|| Button::primary("Send"));
    let mut clicked = false;
    rig.click(shown.rect.center(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_enabled_ui(false, |ui| {
                clicked |= ui.add(Button::primary("Send")).clicked();
            });
        });
    });
    assert!(!clicked);
}

#[test]
fn a_press_that_leaves_the_button_before_the_release_is_not_a_click() {
    let make = || Button::primary("Send");
    let (mut rig, mut shown) = start(&make);
    let inside = shown.rect.center();
    let outside: Pos2 = pos2(inside.x, inside.y + 200.0);
    let mut run = |ctx: &Context| show(ctx, &mut shown, make());
    rig.frame(vec![Event::PointerMoved(inside)], &mut run);
    rig.frame(vec![button(inside, true)], &mut run);
    rig.frame(vec![Event::PointerMoved(outside)], &mut run);
    rig.frame(vec![button(outside, false)], &mut run);
    assert!(!shown.clicked);
}

#[test]
fn a_focused_button_is_activated_by_enter_and_by_space() {
    for key in [Key::Enter, Key::Space] {
        let shown = key_press(&|| Button::primary("Send"), key);
        assert!(shown.clicked, "{key:?}");
    }
}

#[test]
fn a_disabled_or_busy_button_is_not_activated_from_the_keyboard() {
    let makes: [&dyn Fn() -> Button<'static>; 2] =
        [&|| Button::primary("Send").enabled(false), &|| {
            Button::primary("Send").busy(true)
        }];
    for make in makes {
        for key in [Key::Enter, Key::Space] {
            assert!(!key_press(make, key).clicked, "{:?} {key:?}", make());
        }
    }
}

#[test]
fn a_click_reports_a_labelled_enabled_button_to_accessibility() {
    let (shown, outputs) = pointer_click(&|| Button::primary("Send"));
    assert!(shown.clicked);
    let infos = clicked_infos(&outputs);
    assert_eq!(infos.len(), 1, "{infos:?}");
    assert_eq!(infos[0].typ, WidgetType::Button);
    assert_eq!(infos[0].label.as_deref(), Some("Send"));
    assert!(infos[0].enabled);
}

#[test]
fn a_disabled_and_a_busy_button_look_different_from_an_enabled_one() {
    let looks = |make: &dyn Fn() -> Button<'static>| {
        let (mut rig, mut shown) = start(make);
        dump(&rig.frame(vec![], |ctx| show(ctx, &mut shown, make())))
    };
    let on = looks(&|| Button::primary("Send"));
    let off = looks(&|| Button::primary("Send").enabled(false));
    let busy = looks(&|| Button::primary("Send").busy(true));
    assert_ne!(on, off);
    assert_ne!(on, busy);
}

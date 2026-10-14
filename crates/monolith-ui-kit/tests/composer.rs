//! The composer: what it keeps of a draft in egui's memory once the draft
//! is gone.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use common::{Rig, command};
use egui::{Context, Id, Key, TextEdit};
use monolith_ui_kit::composer::{self, Composer};

fn show(ctx: &Context, draft: &mut String, sent: &mut bool) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let out = Composer::new("chat", draft).request_focus(true).show(ui);
        *sent |= out.send_requested;
    });
}

fn edit_id() -> Id {
    Composer::new("chat", &mut String::new()).edit_id()
}

/// A composer with the focus, after typing a draft in two bursts a few
/// seconds apart (as egui groups undo steps by time).
fn typed(draft: &mut String) -> Rig {
    let mut rig = Rig::new();
    let mut sent = false;
    rig.settle(3, |ctx| show(ctx, draft, &mut sent));
    assert!(rig.ctx.memory(|memory| memory.has_focus(edit_id())));
    rig.type_text("meet at the", |ctx| show(ctx, draft, &mut sent));
    rig.frame_after(3.0, vec![], |ctx| show(ctx, draft, &mut sent));
    rig.type_text(" north gate", |ctx| show(ctx, draft, &mut sent));
    rig.frame_after(3.0, vec![], |ctx| show(ctx, draft, &mut sent));
    assert_eq!(draft, "meet at the north gate");
    rig
}

#[test]
fn undo_after_the_sent_draft_was_cleared_brings_nothing_back() {
    let mut draft = String::new();
    let mut rig = typed(&mut draft);
    let mut sent = false;
    rig.key(Key::Enter, |ctx| show(ctx, &mut draft, &mut sent));
    assert!(sent);
    // The runtime accepted the message: the caller clears the draft.
    draft.clear();
    rig.frame(vec![], |ctx| show(ctx, &mut draft, &mut sent));
    rig.key_with(Key::Z, command(), |ctx| show(ctx, &mut draft, &mut sent));
    assert_eq!(draft, "");
}

#[test]
fn undo_while_typing_keeps_the_draft_as_it_is() {
    let mut draft = String::new();
    let mut rig = typed(&mut draft);
    let mut sent = false;
    rig.key_with(Key::Z, command(), |ctx| show(ctx, &mut draft, &mut sent));
    assert_eq!(draft, "meet at the north gate");
}

#[test]
fn forget_removes_the_state_egui_keeps_for_the_composer() {
    let mut draft = String::new();
    let rig = typed(&mut draft);
    assert!(TextEdit::load_state(&rig.ctx, edit_id()).is_some());
    composer::forget(&rig.ctx, edit_id());
    assert!(TextEdit::load_state(&rig.ctx, edit_id()).is_none());
}

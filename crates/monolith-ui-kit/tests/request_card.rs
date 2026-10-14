//! The request card: the order of its answers on screen and from the
//! keyboard.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use common::{Rig, focused_labels};
use egui::epaint::Shape;
use egui::{Context, FullOutput, Key, Pos2};
use monolith_ui_kit::request_card::{Output, RequestCard};

fn show(ctx: &Context, output: &mut Output) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let out = RequestCard::new(
            "frank",
            "Someone wants to connect",
            "Accept",
            "Decline",
            "Block",
        )
        .sender("Frank", &[3])
        .introduction("Their message", "Hi, it is Frank from the climbing gym.")
        .fingerprint("F6QW 8NRT 3MKZ 5DPL 2XEC 9BVH 5TQN 2WRK 8MZP 3XEL 6VDB 9HCA Q4KM")
        .show(ui);
        output.accept |= out.accept;
        output.decline |= out.decline;
        output.block |= out.block;
    });
}

/// Where the text `needle` is painted in `output`.
fn painted_at(output: &FullOutput, needle: &str) -> Option<Pos2> {
    output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            Shape::Text(text) if text.galley.text() == needle => Some(text.pos),
            _ => None,
        })
}

#[test]
fn block_is_at_the_left_and_accept_at_the_right() {
    let mut rig = Rig::new();
    let mut output = Output::default();
    let out = rig.settle(3, |ctx| show(ctx, &mut output));
    let block = painted_at(&out, "Block").expect("Block is painted");
    let decline = painted_at(&out, "Decline").expect("Decline is painted");
    let accept = painted_at(&out, "Accept").expect("Accept is painted");
    assert!(block.x < decline.x && decline.x < accept.x);
}

#[test]
fn tab_reaches_accept_and_decline_before_block() {
    let mut rig = Rig::new();
    let mut output = Output::default();
    rig.settle(3, |ctx| show(ctx, &mut output));
    let mut outputs = Vec::new();
    for _ in 0..3 {
        outputs.extend(rig.key(Key::Tab, |ctx| show(ctx, &mut output)));
    }
    assert_eq!(focused_labels(&outputs), ["Accept", "Decline", "Block"]);
}

#[test]
fn enter_on_the_first_stop_accepts_and_never_blocks() {
    let mut rig = Rig::new();
    let mut output = Output::default();
    rig.settle(3, |ctx| show(ctx, &mut output));
    rig.key(Key::Tab, |ctx| show(ctx, &mut output));
    rig.key(Key::Enter, |ctx| show(ctx, &mut output));
    assert!(output.accept);
    assert!(!output.block && !output.decline);
}

//! The request card: the order of its answers on screen and from the
//! keyboard, and the well that holds the words of the sender, the name
//! they chose among them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use std::sync::Arc;

use common::{Rig, focused_labels};
use egui::epaint::{RectShape, Shape};
use egui::{Context, FullOutput, Galley, Key, Pos2};
use monolith_ui_kit::Theme;
use monolith_ui_kit::line::Line;
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

/// The rectangles painted in `output`.
fn rects(output: &FullOutput) -> Vec<RectShape> {
    fn walk(shape: &Shape, into: &mut Vec<RectShape>) {
        match shape {
            Shape::Vec(shapes) => {
                for shape in shapes {
                    walk(shape, into);
                }
            }
            Shape::Rect(rect) => into.push(rect.clone()),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &output.shapes {
        walk(&clipped.shape, &mut found);
    }
    found
}

#[test]
fn the_message_sits_in_a_well_with_an_edge_in_both_themes() {
    // On the card of the dark theme the fill of the well alone is hardly
    // told from the card's.
    for theme in [Theme::light(), Theme::dark()] {
        let mut rig = Rig::with_theme(theme.clone());
        let mut output = Output::default();
        let out = rig.settle(3, |ctx| show(ctx, &mut output));
        let well = rects(&out)
            .into_iter()
            .find(|rect| rect.fill == theme.color.bg_sunken)
            .expect("the well is painted");
        assert!(well.stroke.width > 0.0, "{:?}", theme.mode);
        assert_eq!(well.stroke.color, theme.color.border_subtle);
    }
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

/// The galley painted with exactly the text `needle`, with where it is.
fn galley_of(output: &FullOutput, needle: &str) -> Option<(Pos2, Arc<Galley>)> {
    output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            Shape::Text(text) if text.galley.text() == needle => {
                Some((text.pos, text.galley.clone()))
            }
            _ => None,
        })
}

#[test]
fn a_name_a_sender_chose_stays_in_the_box_of_their_words() {
    // Quotation marks, a sentence of the card, instructions: whatever the
    // name holds, it is drawn alone, under the label of the client, in the
    // well, and the caption of the card holds none of it (P7-2).
    let hostile = "Sam\". Through your invitation \"Family";
    let caption = Line::said("Through your invitation ")
        .name("Climbing gym")
        .and(".");
    let theme = Theme::light();
    let mut rig = Rig::with_theme(theme.clone());
    let out = rig.settle(3, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let _ = RequestCard::new(
                "sam",
                "Someone wants to connect",
                "Accept",
                "Decline",
                "Block",
            )
            .sender(hostile, &[3])
            .anonymous(true)
            .name("Name they chose", hostile)
            .introduction("Their message", "Hello")
            .caption(caption.clone())
            .show(ui);
        });
    });
    let shown = common::texts(&out);
    let (caption_at, caption_galley) =
        galley_of(&out, "Through your invitation Climbing gym.").expect("the caption is painted");
    assert!(
        shown.iter().filter(|text| text.contains("Family")).count() == 1,
        "{shown:?}"
    );
    let (name_at, _) = galley_of(&out, hostile).expect("the name is painted alone");
    let (label_at, _) = galley_of(&out, "Name they chose").expect("under its label");
    assert!(label_at.y < name_at.y && caption_at.y < label_at.y);
    // The name sits in the well.
    let well = rects(&out)
        .into_iter()
        .find(|rect| rect.fill == theme.color.bg_sunken)
        .expect("the well is painted");
    assert!(well.rect.contains(name_at), "{well:?} {name_at:?}");
    assert!(!well.rect.contains(caption_at));
    // The label of the invitation is drawn as a name: the strong style.
    let strong = theme.text.caption_strong.font_id();
    let plain = theme.text.caption.font_id();
    let text = caption_galley.job.text.as_str();
    let start = text.find("Climbing gym").expect("the label");
    for section in &caption_galley.job.sections {
        let range = section.byte_range.clone();
        let is_name = range.start == start && range.end == start + "Climbing gym".len();
        let expected = if is_name { &strong } else { &plain };
        assert_eq!(&section.format.font_id, expected, "{:?}", &text[range]);
    }
}

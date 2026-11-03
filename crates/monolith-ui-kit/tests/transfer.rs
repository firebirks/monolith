//! The components of file transfers: the progress bar, from whole bytes
//! and never a guess; the transfer card, whose filename stands apart from
//! the client's words; and the attach button of the composer, which says
//! why it takes no input when it takes none.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    clippy::cast_possible_truncation
)]

mod common;

use core::time::Duration;

use common::{Rig, texts};
use egui::accesskit::{Node, Role};
use egui::{Context, FullOutput, ViewportId};
use monolith_ui_kit::composer::Composer;
use monolith_ui_kit::icons;
use monolith_ui_kit::progress::{Fill, Progress};
use monolith_ui_kit::transfer_card::{ActionKind, CardAction, Tone, TransferCard};
use monolith_ui_kit::{Theme, theme};

fn repaint_delay(output: &FullOutput) -> Duration {
    output
        .viewport_output
        .get(&ViewportId::ROOT)
        .map_or(Duration::MAX, |viewport| viewport.repaint_delay)
}

/// Every AccessKit node of `output` with its label.
fn nodes(output: &FullOutput) -> Vec<Node> {
    output
        .platform_output
        .accesskit_update
        .as_ref()
        .map(|update| update.nodes.iter().map(|(_, node)| node.clone()).collect())
        .unwrap_or_default()
}

/// The first text painted in `output` that starts with `start`.
fn galley_of(output: &FullOutput, start: &str) -> Option<std::sync::Arc<egui::Galley>> {
    fn find(shape: &egui::epaint::Shape, start: &str) -> Option<std::sync::Arc<egui::Galley>> {
        match shape {
            egui::epaint::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, start)),
            egui::epaint::Shape::Text(text) if text.galley.text().starts_with(start) => {
                Some(text.galley.clone())
            }
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|clipped| find(&clipped.shape, start))
}

fn bar(ctx: &Context, fill: Fill) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.add(Progress::new(fill, "Sending report.pdf"));
    });
}

fn progress_node(output: &FullOutput) -> Node {
    nodes(output)
        .into_iter()
        .find(|node| node.role() == Role::ProgressIndicator)
        .expect("a progress indicator")
}

#[test]
fn a_bar_says_its_share_in_whole_per_cent_and_a_file_of_no_bytes_is_all_there() {
    for (fill, share) in [
        (Fill::Share { done: 0, total: 0 }, 100.0),
        (Fill::Share { done: 0, total: 64 }, 0.0),
        (
            Fill::Share {
                done: 41_000_000,
                total: 64_000_000,
            },
            64.0,
        ),
        (
            Fill::Share {
                done: u64::MAX,
                total: u64::MAX,
            },
            100.0,
        ),
        (Fill::Share { done: 9, total: 4 }, 100.0),
    ] {
        let mut rig = Rig::new();
        rig.ctx.enable_accesskit();
        let out = rig.settle(2, |ctx| bar(ctx, fill));
        let node = progress_node(&out);
        assert_eq!(node.numeric_value(), Some(share), "{fill:?}");
        assert_eq!(node.label(), Some("Sending report.pdf"));
    }
}

#[test]
fn a_bar_of_bytes_not_known_has_no_share_and_moves_only_when_motion_is_allowed() {
    let mut rig = Rig::new();
    rig.ctx.enable_accesskit();
    let out = rig.settle(2, |ctx| bar(ctx, Fill::Unknown));
    assert_eq!(progress_node(&out).numeric_value(), None);
    // Reduced motion: still, and no repaint asked for it.
    assert_eq!(repaint_delay(&out), Duration::MAX);
    let mut moving = Rig::new();
    theme::install(&moving.ctx, Theme::light().with_reduced_motion(false));
    let out = moving.settle(2, |ctx| bar(ctx, Fill::Unknown));
    assert!(repaint_delay(&out) < Duration::from_millis(100));
    // A known share never asks for a frame.
    let out = moving.settle(2, |ctx| {
        bar(ctx, Fill::Share { done: 1, total: 2 });
    });
    assert_eq!(repaint_delay(&out), Duration::MAX);
}

fn card(ctx: &Context, name: &str, busy: bool, clicked: &mut Option<CardAction>) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let out = TransferCard::new("t1", name)
            .incoming(true)
            .status("64\u{a0}MB \u{b7} Offered to you", Tone::Attention)
            .note(icons::LOCK, "Encrypted connection through Tor")
            .action(CardAction::Accept, "Accept", ActionKind::Primary)
            .action(CardAction::Reject, "Reject", ActionKind::Secondary)
            .busy(busy)
            .show(ui);
        if out.action.is_some() {
            *clicked = out.action;
        }
    });
}

#[test]
fn a_filename_stands_on_its_own_line_and_never_in_the_words_of_the_card() {
    // A name that tries to read as the client's words.
    let name = "photo.jpg\" verified by Monolith";
    let mut rig = Rig::new();
    let mut clicked = None;
    let out = rig.settle(2, |ctx| card(ctx, name, false, &mut clicked));
    let painted = texts(&out);
    assert!(painted.iter().any(|text| text == name), "{painted:?}");
    for text in &painted {
        if text != name {
            assert!(!text.contains("photo.jpg"), "{text:?}");
        }
    }
    // A long name with no break stays on one line: its width is bounded.
    let long = "x".repeat(4000);
    let out = rig.settle(2, |ctx| card(ctx, &long, false, &mut clicked));
    let galley = galley_of(&out, "xxx").expect("the name");
    assert_eq!(galley.rows.len(), 1);
    assert!(galley.rect.width() <= rig.screen().width());
}

#[test]
fn a_card_whose_command_is_under_way_takes_no_action() {
    for busy in [false, true] {
        let mut rig = Rig::new();
        rig.ctx.enable_accesskit();
        let mut clicked = None;
        let out = rig.settle(3, |ctx| card(ctx, "a.txt", busy, &mut clicked));
        let accept = nodes(&out)
            .into_iter()
            .find(|node| node.label() == Some("Accept"))
            .expect("the accept button");
        assert_eq!(accept.is_disabled(), busy);
        let bounds = accept.bounds().unwrap();
        let at = egui::pos2(
            ((bounds.x0 + bounds.x1) / 2.0) as f32,
            ((bounds.y0 + bounds.y1) / 2.0) as f32,
        );
        rig.click(at, |ctx| card(ctx, "a.txt", busy, &mut clicked));
        assert_eq!(
            clicked,
            (!busy).then_some(CardAction::Accept),
            "busy {busy}"
        );
    }
}

const NOT_YET: &str = "Files can be sent once this contact has accepted you.";

fn composer(ctx: &Context, enabled: bool, asked: &mut bool) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let mut draft = String::new();
        let label = if enabled { "Send a file" } else { NOT_YET };
        let out = Composer::new("chat", &mut draft)
            .attach(label, enabled)
            .show(ui);
        *asked |= out.attach_requested;
    });
}

#[test]
fn an_attach_button_that_takes_no_input_says_why_and_asks_for_nothing() {
    for enabled in [true, false] {
        let mut rig = Rig::new();
        rig.ctx.enable_accesskit();
        let mut asked = false;
        let out = rig.settle(3, |ctx| composer(ctx, enabled, &mut asked));
        let label = if enabled { "Send a file" } else { NOT_YET };
        let attach = nodes(&out)
            .into_iter()
            .find(|node| node.label() == Some(label))
            .expect("the attach button");
        assert_eq!(attach.is_disabled(), !enabled);
        let bounds = attach.bounds().unwrap();
        let at = egui::pos2(
            ((bounds.x0 + bounds.x1) / 2.0) as f32,
            ((bounds.y0 + bounds.y1) / 2.0) as f32,
        );
        rig.click(at, |ctx| composer(ctx, enabled, &mut asked));
        assert_eq!(asked, enabled);
    }
}

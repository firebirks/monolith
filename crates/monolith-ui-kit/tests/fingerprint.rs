//! The fingerprint: its groups in rows of seven, so that the 13 groups of
//! a full fingerprint make rows of seven and six; fewer where there is no
//! room; and the text as it is written.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use std::sync::Arc;

use common::Rig;
use egui::epaint::Shape;
use egui::{Color32, FullOutput, Galley};
use monolith_ui_kit::Theme;
use monolith_ui_kit::fingerprint::Fingerprint;

const FINGERPRINT: &str = "AF3K 72PQ 9XZT M4LB D8EV 61RC 5TQN 2WRK 8MZP 3XEL 6VDB 9HCA Q4KM";

/// The galley that holds the fingerprint in `output`.
fn galley(output: &FullOutput) -> Arc<Galley> {
    output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            Shape::Text(text) if text.galley.text().starts_with("AF3K") => {
                Some(text.galley.clone())
            }
            _ => None,
        })
        .expect("the fingerprint is painted")
}

/// The groups on each row of the fingerprint laid out `width` points wide.
fn rows(width: f32, compact: bool) -> (Vec<usize>, String) {
    let mut rig = Rig::new();
    let out = rig.settle(2, |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.allocate_ui(egui::vec2(width, 300.0), |ui| {
                ui.add(Fingerprint::new(FINGERPRINT).compact(compact));
            });
        });
    });
    let galley = galley(&out);
    let groups = galley
        .rows
        .iter()
        .map(|row| {
            let text: String = row.glyphs.iter().map(|glyph| glyph.chr).collect();
            text.split_whitespace().count()
        })
        .collect();
    (groups, galley.text().to_owned())
}

#[test]
fn a_fingerprint_is_laid_out_in_rows_of_seven_and_six_groups() {
    for compact in [false, true] {
        let (groups, text) = rows(380.0, compact);
        assert_eq!(groups, [7, 6], "compact {compact}");
        // It is copied as it is written.
        assert_eq!(text, FINGERPRINT);
    }
}

#[test]
fn a_narrow_fingerprint_puts_fewer_groups_on_a_row() {
    let (groups, _) = rows(200.0, false);
    assert!(groups.iter().all(|count| *count <= groups[0]));
    assert!(groups[0] < 6 && groups[0] >= 1, "{groups:?}");
    assert_eq!(groups.iter().sum::<usize>(), 13);
}

#[test]
fn a_row_of_seven_needs_no_room_for_the_space_after_it() {
    // The fingerprint beside a QR code has the width of seven groups and
    // little more.
    let mut rig = Rig::new();
    rig.settle(1, |_| {});
    let seven = "AF3K 72PQ 9XZT M4LB D8EV 61RC 5TQN";
    let font = Theme::light().text.mono.font_id();
    let width = rig.ctx.fonts(|fonts| {
        fonts
            .layout_no_wrap(seven.to_owned(), font, Color32::WHITE)
            .size()
            .x
    });
    let (groups, _) = rows(width + 1.0, false);
    assert_eq!(groups, [7, 6]);
}

//! Conversation and contact rows: selection, clicks, ids and what a screen
//! reader is told.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use common::{Rig, clicked_infos, dump, paints};
use egui::{Context, Rect, WidgetType};
use monolith_ui_kit::rows::{ContactRow, ConversationRow, Secondary};

#[derive(Clone, Copy, PartialEq)]
enum Style {
    Conversation,
    Contact,
}

#[derive(Clone, Copy)]
struct Setup {
    style: Style,
    selected: usize,
    disabled: Option<usize>,
}

impl Setup {
    fn new(style: Style) -> Self {
        Self {
            style,
            selected: usize::MAX,
            disabled: None,
        }
    }
}

const NAMES: [&str; 2] = ["Alice Example", "Bob Example"];
const SEEDS: [[u8; 1]; 2] = [[0], [1]];

/// Shows two rows with the keys "a" and "b"; returns their rectangles and
/// which of them were clicked.
fn show(ctx: &Context, setup: Setup, rects: &mut [Rect; 2], clicked: &mut [bool; 2]) {
    egui::CentralPanel::default().show(ctx, |ui| {
        for (index, key) in ["a", "b"].into_iter().enumerate() {
            let selected = setup.selected == index;
            let enabled = setup.disabled != Some(index);
            let response = match setup.style {
                Style::Conversation => ui.add(
                    ConversationRow::new(key, NAMES[index], &SEEDS[index])
                        .preview("See you soon")
                        .timestamp("09:41")
                        .selected(selected)
                        .enabled(enabled),
                ),
                Style::Contact => ui.add(
                    ContactRow::new(key, NAMES[index], &SEEDS[index])
                        .secondary(Secondary::Text("Online"))
                        .selected(selected)
                        .enabled(enabled),
                ),
            };
            rects[index] = response.rect;
            clicked[index] |= response.clicked();
        }
    });
}

const STYLES: [Style; 2] = [Style::Conversation, Style::Contact];

#[test]
fn a_click_selects_the_row_that_was_clicked_and_no_other() {
    for style in STYLES {
        for target in 0..2 {
            let setup = Setup::new(style);
            let mut rig = Rig::new();
            let mut rects = [Rect::NOTHING; 2];
            let mut clicked = [false; 2];
            rig.settle(2, |ctx| show(ctx, setup, &mut rects, &mut clicked));
            let at = rects[target].center();
            rig.click(at, |ctx| show(ctx, setup, &mut rects, &mut clicked));
            let mut expected = [false; 2];
            expected[target] = true;
            assert_eq!(clicked, expected);
        }
    }
}

#[test]
fn rows_do_not_overlap_in_one_frame() {
    for style in STYLES {
        let mut rig = Rig::new();
        let mut rects = [Rect::NOTHING; 2];
        let mut clicked = [false; 2];
        rig.settle(2, |ctx| {
            show(ctx, Setup::new(style), &mut rects, &mut clicked)
        });
        assert!(rects[0].bottom() <= rects[1].top());
        assert!(rects[0].width() > 100.0);
    }
}

#[test]
fn a_disabled_row_is_not_clicked_and_its_neighbour_still_is() {
    for style in STYLES {
        let setup = Setup {
            disabled: Some(0),
            ..Setup::new(style)
        };
        let mut rig = Rig::new();
        let mut rects = [Rect::NOTHING; 2];
        let mut clicked = [false; 2];
        rig.settle(2, |ctx| show(ctx, setup, &mut rects, &mut clicked));
        let (first, second) = (rects[0].center(), rects[1].center());
        rig.click(first, |ctx| show(ctx, setup, &mut rects, &mut clicked));
        assert_eq!(clicked, [false, false]);
        rig.click(second, |ctx| show(ctx, setup, &mut rects, &mut clicked));
        assert_eq!(clicked, [false, true]);
    }
}

#[test]
fn a_selected_row_renders_differently_from_an_unselected_one() {
    for style in STYLES {
        let looks = |selected: usize| {
            let setup = Setup {
                selected,
                ..Setup::new(style)
            };
            let mut rig = Rig::new();
            let mut rects = [Rect::NOTHING; 2];
            let mut clicked = [false; 2];
            rig.settle(2, |ctx| show(ctx, setup, &mut rects, &mut clicked));
            dump(&rig.frame(vec![], |ctx| show(ctx, setup, &mut rects, &mut clicked)))
        };
        let none = looks(usize::MAX);
        let first = looks(0);
        let second = looks(1);
        assert_ne!(none, first);
        assert_ne!(none, second);
        assert_ne!(first, second);
    }
}

#[test]
fn a_disabled_row_renders_differently_from_an_enabled_one() {
    for style in STYLES {
        let looks = |disabled: Option<usize>| {
            let setup = Setup {
                disabled,
                ..Setup::new(style)
            };
            let mut rig = Rig::new();
            let mut rects = [Rect::NOTHING; 2];
            let mut clicked = [false; 2];
            rig.settle(2, |ctx| show(ctx, setup, &mut rects, &mut clicked));
            dump(&rig.frame(vec![], |ctx| show(ctx, setup, &mut rects, &mut clicked)))
        };
        assert_ne!(looks(None), looks(Some(0)));
    }
}

#[test]
fn rows_paint_their_names() {
    for style in STYLES {
        let mut rig = Rig::new();
        let mut rects = [Rect::NOTHING; 2];
        let mut clicked = [false; 2];
        let out = rig.settle(2, |ctx| {
            show(ctx, Setup::new(style), &mut rects, &mut clicked)
        });
        assert!(paints(&out, "Alice Example"));
        assert!(paints(&out, "Bob Example"));
    }
}

#[test]
fn distinct_keys_give_distinct_ids_and_equal_keys_equal_ones() {
    assert_ne!(ConversationRow::id_for("a"), ConversationRow::id_for("b"));
    assert_eq!(ConversationRow::id_for("a"), ConversationRow::id_for("a"));
    assert_ne!(ContactRow::id_for("a"), ContactRow::id_for("b"));
    assert_ne!(ContactRow::id_for(1_u32), ContactRow::id_for(2_u32));
    // The same key in the two kinds of list does not clash.
    assert_ne!(ConversationRow::id_for("a"), ContactRow::id_for("a"));
}

#[test]
fn a_click_reports_a_selectable_row_with_its_label_and_state_to_accessibility() {
    for style in STYLES {
        let setup = Setup {
            selected: 1,
            ..Setup::new(style)
        };
        let mut rig = Rig::new();
        let mut rects = [Rect::NOTHING; 2];
        let mut clicked = [false; 2];
        rig.settle(2, |ctx| show(ctx, setup, &mut rects, &mut clicked));
        let at = rects[1].center();
        let outputs = rig.click(at, |ctx| show(ctx, setup, &mut rects, &mut clicked));
        let infos = clicked_infos(&outputs);
        assert_eq!(infos.len(), 1, "{infos:?}");
        assert_eq!(infos[0].typ, WidgetType::SelectableLabel);
        assert!(infos[0].label.as_deref().unwrap().contains("Bob Example"));
        assert_eq!(infos[0].selected, Some(true));
    }
}

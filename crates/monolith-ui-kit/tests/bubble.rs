//! Message bubbles: side, delivery status, grouping and "Show more".

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use common::{Rig, dump, paints};
use egui::{Context, Event, FullOutput, Id, OutputCommand, Rect, pos2};
use monolith_ui_kit::bubble::{Direction, MessageBubble};
use monolith_ui_kit::icons;
use monolith_ui_kit::message_status::{MessageStatus, Status};

/// Where a bubble was put and what it reported.
#[derive(Default)]
struct Seen {
    bubble: Option<Rect>,
    panel: Option<Rect>,
    show_more: bool,
}

fn show(ctx: &Context, seen: &mut Seen, build: &dyn Fn() -> MessageBubble<'static>) {
    egui::CentralPanel::default().show(ctx, |ui| {
        seen.panel = Some(ui.max_rect());
        let out = build().show(ui);
        seen.bubble = Some(out.response.rect);
        seen.show_more |= out.show_more;
    });
}

fn render(build: &dyn Fn() -> MessageBubble<'static>) -> (FullOutput, Seen) {
    let mut rig = Rig::new();
    let mut seen = Seen::default();
    rig.settle(2, |ctx| show(ctx, &mut seen, build));
    let output = rig.frame(vec![], |ctx| show(ctx, &mut seen, build));
    (output, seen)
}

fn glyph(icon: icons::Icon) -> String {
    icon.char().to_string()
}

fn outgoing(status: Status) -> MessageBubble<'static> {
    MessageBubble::new(1, Direction::Outgoing, "See you at nine", "09:41").status(status, "status")
}

#[test]
fn a_queued_and_a_delivered_message_draw_different_glyphs() {
    let (queued, _) = render(&|| outgoing(Status::Queued));
    let (delivered, _) = render(&|| outgoing(Status::Delivered));
    assert!(paints(&queued, &glyph(icons::CLOCK)));
    assert!(!paints(&queued, &glyph(icons::CHECK_CHECK)));
    assert!(paints(&delivered, &glyph(icons::CHECK_CHECK)));
    assert!(!paints(&delivered, &glyph(icons::CLOCK)));
    assert_ne!(dump(&queued), dump(&delivered));
}

#[test]
fn an_outgoing_message_without_a_status_draws_no_glyph() {
    let (bare, _) =
        render(&|| MessageBubble::new(1, Direction::Outgoing, "See you at nine", "09:41"));
    assert!(!paints(&bare, &glyph(icons::CLOCK)));
    assert!(!paints(&bare, &glyph(icons::CHECK_CHECK)));
}

#[test]
fn a_received_message_ignores_a_status_it_is_given() {
    let incoming = |status: Option<Status>| {
        let mut bubble = MessageBubble::new(1, Direction::Incoming, "See you at nine", "09:41");
        if let Some(status) = status {
            bubble = bubble.status(status, "status");
        }
        bubble
    };
    let (with, _) = render(&|| incoming(Some(Status::Delivered)));
    let (without, _) = render(&|| incoming(None));
    assert!(!paints(&with, &glyph(icons::CHECK_CHECK)));
    assert_eq!(dump(&with), dump(&without));
}

#[test]
fn a_message_paints_its_text_and_its_time() {
    let (out, _) = render(&|| outgoing(Status::Queued));
    assert!(paints(&out, "See you at nine"));
    assert!(paints(&out, "09:41"));
}

#[test]
fn text_that_looks_like_a_link_is_painted_as_it_is() {
    let text = "https://example.invalid/a?b=c";
    let (out, _) = render(&|| MessageBubble::new(2, Direction::Incoming, text, "09:41"));
    assert!(paints(&out, text));
}

#[test]
fn outgoing_bubbles_sit_at_the_right_and_incoming_ones_at_the_left() {
    let (_, out) = render(&|| outgoing(Status::Queued));
    let (_, inc) = render(&|| MessageBubble::new(3, Direction::Incoming, "Hello", "09:40"));
    let (out_bubble, out_panel) = (out.bubble.unwrap(), out.panel.unwrap());
    let (inc_bubble, inc_panel) = (inc.bubble.unwrap(), inc.panel.unwrap());
    assert!((out_bubble.right() - out_panel.right()).abs() < 1.0);
    assert!(out_bubble.left() > out_panel.left() + 50.0);
    assert!((inc_bubble.left() - inc_panel.left()).abs() < 1.0);
    assert!(inc_bubble.right() < inc_panel.right() - 50.0);
}

#[test]
fn a_bubble_and_its_colours_differ_by_direction() {
    let (out, _) = render(&|| MessageBubble::new(4, Direction::Outgoing, "Hello", "09:40"));
    let (inc, _) = render(&|| MessageBubble::new(4, Direction::Incoming, "Hello", "09:40"));
    assert_ne!(dump(&out), dump(&inc));
}

#[test]
fn a_long_message_wraps_inside_its_share_of_the_width() {
    let long = "word ".repeat(200);
    let long: &'static str = Box::leak(long.into_boxed_str());
    let (_, seen) = render(&|| MessageBubble::new(5, Direction::Incoming, long, "09:40"));
    let (bubble, panel) = (seen.bubble.unwrap(), seen.panel.unwrap());
    assert!(bubble.width() <= panel.width() * 0.72 + 1.0, "{bubble:?}");
    assert!(bubble.height() > 100.0);
}

#[test]
fn only_the_last_bubble_of_a_run_has_the_pointed_corner() {
    let make = |last: bool| {
        move || MessageBubble::new(6, Direction::Incoming, "Hello", "09:40").last_in_group(last)
    };
    let (last, _) = render(&make(true));
    let (middle, _) = render(&make(false));
    assert_ne!(dump(&last), dump(&middle));
}

#[test]
fn show_more_is_reported_when_its_control_is_clicked() {
    let make = || {
        MessageBubble::new(7, Direction::Incoming, "A long start", "09:40").collapsed("Show more")
    };
    let mut rig = Rig::new();
    let mut seen = Seen::default();
    let first = rig.settle(2, |ctx| show(ctx, &mut seen, &make));
    assert!(paints(&first, "Show more"));
    let id = Id::new(("monolith-bubble", 7)).with("more");
    let rect = rig
        .ctx
        .read_response(id)
        .expect("the control was shown")
        .rect;
    assert!(!seen.show_more);
    rig.click(rect.center(), |ctx| show(ctx, &mut seen, &make));
    assert!(seen.show_more);
}

#[test]
fn a_bubble_without_a_collapse_has_no_show_more() {
    let (out, seen) = render(&|| MessageBubble::new(8, Direction::Incoming, "Hi", "09:40"));
    assert!(!paints(&out, "Show more"));
    assert!(!seen.show_more);
}

#[test]
fn the_status_mark_says_its_state_in_words_under_the_pointer() {
    let make = || {
        MessageBubble::new(1, Direction::Outgoing, "See you at nine", "09:41")
            .status(Status::Queued, "Queued: not delivered yet")
    };
    let mut rig = Rig::new();
    let mut seen = Seen::default();
    let before = rig.settle(2, |ctx| show(ctx, &mut seen, &make));
    assert!(!paints(&before, "Queued: not delivered yet"));
    let mark = rig
        .ctx
        .read_response(Id::new(("monolith-bubble", 1)).with("status"))
        .expect("the mark was shown")
        .rect;
    rig.frame(vec![Event::PointerMoved(mark.center())], |ctx| {
        show(ctx, &mut seen, &make)
    });
    let mut after = rig.frame_after(1.0, vec![], |ctx| show(ctx, &mut seen, &make));
    for _ in 0..3 {
        after = rig.frame(vec![], |ctx| show(ctx, &mut seen, &make));
    }
    assert!(paints(&after, "Queued: not delivered yet"));
}

/// The text a frame asked the platform to copy.
fn copied(outputs: &[FullOutput]) -> Vec<String> {
    outputs
        .iter()
        .flat_map(|output| output.platform_output.commands.iter())
        .filter_map(|command| match command {
            OutputCommand::CopyText(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Shows `bubble` under `above` lines of other text.
fn list(ctx: &Context, above: usize, seen: &mut Seen) {
    egui::CentralPanel::default().show(ctx, |ui| {
        for line in 0..above {
            ui.label(format!("Earlier {line}"));
        }
        let out = MessageBubble::new(9, Direction::Incoming, "See you at nine", "09:41").show(ui);
        seen.bubble = Some(out.response.rect);
    });
}

/// Double-clicks the first word of the bubble and returns the rig.
fn select_first_word(seen: &mut Seen) -> Rig {
    let mut rig = Rig::new();
    rig.settle(2, |ctx| list(ctx, 0, seen));
    let text = rig
        .ctx
        .read_response(Id::new(("monolith-bubble", 9)).with("text"))
        .expect("the text was shown")
        .rect;
    let at = pos2(text.left() + 6.0, text.center().y);
    for _ in 0..2 {
        rig.click(at, |ctx| list(ctx, 0, seen));
    }
    rig
}

#[test]
fn a_double_click_selects_a_word_and_copy_takes_only_that_word() {
    let mut seen = Seen::default();
    let mut rig = select_first_word(&mut seen);
    let out = rig.frame(vec![Event::Copy], |ctx| list(ctx, 0, &mut seen));
    assert_eq!(copied(&[out]), ["See"]);
}

#[test]
fn a_selection_stays_with_its_message_when_the_list_above_changes() {
    let mut seen = Seen::default();
    let mut rig = select_first_word(&mut seen);
    rig.settle(2, |ctx| list(ctx, 3, &mut seen));
    let out = rig.frame(vec![Event::Copy], |ctx| list(ctx, 3, &mut seen));
    assert_eq!(copied(&[out]), ["See"]);
}

fn status_widget(ctx: &Context, status: Status) {
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.add(MessageStatus::new(9, status, "label").show_label(true));
    });
}

#[test]
fn the_status_widget_draws_one_glyph_per_state_and_the_label() {
    let mut rig = Rig::new();
    rig.settle(2, |ctx| status_widget(ctx, Status::Queued));
    let queued = rig.frame(vec![], |ctx| status_widget(ctx, Status::Queued));
    let mut rig = Rig::new();
    rig.settle(2, |ctx| status_widget(ctx, Status::Delivered));
    let delivered = rig.frame(vec![], |ctx| status_widget(ctx, Status::Delivered));
    assert!(paints(&queued, &glyph(icons::CLOCK)));
    assert!(!paints(&queued, &glyph(icons::CHECK_CHECK)));
    assert!(paints(&delivered, &glyph(icons::CHECK_CHECK)));
    assert!(!paints(&delivered, &glyph(icons::CLOCK)));
    assert!(paints(&queued, "label") && paints(&delivered, "label"));
}

#[test]
fn a_message_is_delivered_only_when_the_caller_says_so() {
    let mut rig = Rig::new();
    rig.settle(3, |ctx| status_widget(ctx, Status::Queued));
    let later = rig.frame_after(10.0, vec![], |ctx| status_widget(ctx, Status::Queued));
    assert!(!paints(&later, &glyph(icons::CHECK_CHECK)));
}

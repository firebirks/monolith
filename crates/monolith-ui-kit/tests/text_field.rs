//! Text field behaviour: typing, limits, notes under the field, and what
//! a secret field and a field without history leave in egui's memory.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

mod common;

use common::{Rig, command, dump, paints};
use egui::{Context, Event, FullOutput, Id, ImeEvent, Key, TextEdit};
use monolith_ui_kit::TextField;
use monolith_ui_kit::text_field;

/// The options a test sets on the field.
#[derive(Clone, Copy, Default)]
struct Cfg {
    secret: bool,
    no_history: bool,
    enabled_off: bool,
    limit: Option<usize>,
    bytes: Option<usize>,
    error: Option<&'static str>,
    help: Option<&'static str>,
    focus: bool,
}

/// What a frame of the field tells.
#[derive(Default)]
struct Seen {
    height: f32,
    changed: bool,
    submitted: bool,
}

fn show(ctx: &Context, text: &mut String, cfg: Cfg, seen: &mut Seen) {
    egui::CentralPanel::default().show(ctx, |ui| {
        let top = ui.cursor().top();
        let mut field = TextField::new("field", text)
            .label("Name")
            .secret(cfg.secret)
            .no_history(cfg.no_history)
            .enabled(!cfg.enabled_off)
            .error(cfg.error)
            .help(cfg.help)
            .request_focus(cfg.focus);
        if let Some(limit) = cfg.limit {
            field = field.char_limit(limit);
        }
        if let Some(bytes) = cfg.bytes {
            field = field.byte_limit(bytes);
        }
        let out = field.show(ui);
        seen.height = ui.cursor().top() - top;
        seen.changed |= out.changed;
        seen.submitted |= out.submitted;
    });
}

fn edit_id() -> Id {
    TextField::new("field", &mut String::new()).edit_id()
}

/// A rig whose field has the focus.
fn focused(text: &mut String, cfg: Cfg) -> Rig {
    let mut rig = Rig::new();
    let cfg = Cfg { focus: true, ..cfg };
    let mut seen = Seen::default();
    rig.settle(3, |ctx| show(ctx, text, cfg, &mut seen));
    assert!(rig.ctx.memory(|memory| memory.has_focus(edit_id())));
    rig
}

fn frame(rig: &mut Rig, text: &mut String, cfg: Cfg) -> (FullOutput, Seen) {
    let mut seen = Seen::default();
    let output = rig.frame(vec![], |ctx| show(ctx, text, cfg, &mut seen));
    (output, seen)
}

#[test]
fn typed_text_reaches_the_bound_string() {
    let mut text = String::new();
    let mut rig = focused(&mut text, Cfg::default());
    let mut seen = Seen::default();
    for chunk in ["hel", "lo"] {
        rig.type_text(chunk, |ctx| show(ctx, &mut text, Cfg::default(), &mut seen));
    }
    assert_eq!(text, "hello");
    assert!(seen.changed);
}

#[test]
fn a_field_that_is_not_enabled_takes_no_text() {
    let mut text = String::from("kept");
    let cfg = Cfg {
        enabled_off: true,
        ..Cfg::default()
    };
    let mut rig = focused_even_if_off(&mut text, cfg);
    let mut seen = Seen::default();
    rig.type_text("more", |ctx| show(ctx, &mut text, cfg, &mut seen));
    assert_eq!(text, "kept");
    assert!(!seen.changed);
}

fn focused_even_if_off(text: &mut String, cfg: Cfg) -> Rig {
    let mut rig = Rig::new();
    let mut seen = Seen::default();
    rig.ctx.memory_mut(|memory| memory.request_focus(edit_id()));
    rig.settle(3, |ctx| show(ctx, text, cfg, &mut seen));
    rig
}

#[test]
fn the_character_limit_cuts_typed_and_pasted_text() {
    let cfg = Cfg {
        limit: Some(5),
        ..Cfg::default()
    };
    let mut text = String::new();
    let mut rig = focused(&mut text, cfg);
    let mut seen = Seen::default();
    rig.type_text("abc", |ctx| show(ctx, &mut text, cfg, &mut seen));
    assert_eq!(text, "abc");
    rig.type_text("defgh", |ctx| show(ctx, &mut text, cfg, &mut seen));
    assert_eq!(text, "abcde");
    rig.type_text("x", |ctx| show(ctx, &mut text, cfg, &mut seen));
    assert_eq!(text, "abcde");
}

#[test]
fn the_byte_limit_cuts_typed_and_pasted_text_at_a_character() {
    let cfg = Cfg {
        limit: Some(5),
        bytes: Some(7),
        ..Cfg::default()
    };
    let mut text = String::new();
    let mut rig = focused(&mut text, cfg);
    let mut seen = Seen::default();
    // Three bytes each: the third would pass the limit of bytes before the
    // limit of characters.
    rig.type_text("\u{4E2D}\u{6587}\u{5B57}", |ctx| {
        show(ctx, &mut text, cfg, &mut seen);
    });
    assert_eq!(text, "\u{4E2D}\u{6587}");
    rig.type_text("ab", |ctx| show(ctx, &mut text, cfg, &mut seen));
    assert_eq!(text, "\u{4E2D}\u{6587}a");
    rig.type_text("\u{4E2D}", |ctx| show(ctx, &mut text, cfg, &mut seen));
    assert_eq!(text, "\u{4E2D}\u{6587}a");
    // Two bytes do not fit in the one left once a character is deleted.
    rig.key(Key::Backspace, |ctx| show(ctx, &mut text, cfg, &mut seen));
    rig.type_text("\u{E9}b", |ctx| show(ctx, &mut text, cfg, &mut seen));
    assert_eq!(text, "\u{4E2D}\u{6587}");
}

#[test]
fn enter_in_a_focused_field_reports_a_submission() {
    let mut text = String::from("done");
    let mut rig = focused(&mut text, Cfg::default());
    let mut seen = Seen::default();
    rig.key(Key::Enter, |ctx| {
        show(ctx, &mut text, Cfg::default(), &mut seen)
    });
    assert!(seen.submitted);
    assert_eq!(text, "done");
}

#[test]
fn an_enter_that_commits_an_input_method_is_no_submission() {
    let ime = |event: ImeEvent| Event::Ime(event);
    let enter = || {
        vec![
            common::key_event(Key::Enter, true),
            common::key_event(Key::Enter, false),
        ]
    };
    let mut text = String::new();
    let mut rig = focused(&mut text, Cfg::default());
    let mut seen = Seen::default();
    rig.frame(
        vec![
            ime(ImeEvent::Enabled),
            ime(ImeEvent::Preedit("\u{304B}".to_owned())),
        ],
        |ctx| show(ctx, &mut text, Cfg::default(), &mut seen),
    );
    // The Enter before the commit, and the commit with its Enter.
    rig.frame(enter(), |ctx| {
        show(ctx, &mut text, Cfg::default(), &mut seen);
    });
    assert!(!seen.submitted);
    let mut text = String::new();
    let mut rig = focused(&mut text, Cfg::default());
    let mut seen = Seen::default();
    rig.frame(vec![ime(ImeEvent::Preedit("ka".to_owned()))], |ctx| {
        show(ctx, &mut text, Cfg::default(), &mut seen);
    });
    let mut events = vec![ime(ImeEvent::Commit("\u{304B}".to_owned()))];
    events.extend(enter());
    rig.frame(events, |ctx| {
        show(ctx, &mut text, Cfg::default(), &mut seen)
    });
    assert!(!seen.submitted);
    assert_eq!(text, "\u{304B}");
}

#[test]
fn an_error_is_shown_under_the_field_and_makes_it_taller() {
    let mut text = String::new();
    let mut rig = Rig::new();
    rig.settle(3, |ctx| {
        show(ctx, &mut text, Cfg::default(), &mut Seen::default())
    });
    let (plain, plain_seen) = frame(&mut rig, &mut text, Cfg::default());
    let cfg = Cfg {
        error: Some("Use at least twelve characters"),
        ..Cfg::default()
    };
    let (wrong, wrong_seen) = frame(&mut rig, &mut text, cfg);
    assert!(!paints(&plain, "twelve"));
    assert!(paints(&wrong, "Use at least twelve characters"));
    assert!(wrong_seen.height > plain_seen.height);
    assert_ne!(dump(&plain), dump(&wrong));
}

#[test]
fn help_is_shown_unless_there_is_an_error() {
    let mut text = String::new();
    let mut rig = Rig::new();
    let help = Cfg {
        help: Some("Shown to nobody"),
        ..Cfg::default()
    };
    let both = Cfg {
        error: Some("Too short"),
        ..help
    };
    let (with_help, _) = frame(&mut rig, &mut text, help);
    assert!(paints(&with_help, "Shown to nobody"));
    let (with_both, _) = frame(&mut rig, &mut text, both);
    assert!(paints(&with_both, "Too short"));
    assert!(!paints(&with_both, "Shown to nobody"));
}

#[test]
fn a_secret_field_masks_what_it_shows() {
    let mut text = String::from("hunter2-passphrase");
    let cfg = Cfg {
        secret: true,
        ..Cfg::default()
    };
    let mut rig = Rig::new();
    rig.settle(2, |ctx| show(ctx, &mut text, cfg, &mut Seen::default()));
    let (secret, _) = frame(&mut rig, &mut text, cfg);
    assert!(!paints(&secret, "hunter2"));
    let (open, _) = frame(&mut rig, &mut text, Cfg::default());
    assert!(paints(&open, "hunter2-passphrase"));
}

/// Types two bursts a few seconds apart, then asks the field to undo.
fn type_then_undo(cfg: Cfg) -> String {
    let mut text = String::new();
    let mut rig = focused(&mut text, cfg);
    let mut seen = Seen::default();
    rig.frame_after(2.0, vec![], |ctx| show(ctx, &mut text, cfg, &mut seen));
    rig.type_text("first", |ctx| show(ctx, &mut text, cfg, &mut seen));
    rig.frame_after(3.0, vec![], |ctx| show(ctx, &mut text, cfg, &mut seen));
    rig.type_text(" second", |ctx| show(ctx, &mut text, cfg, &mut seen));
    rig.frame_after(3.0, vec![], |ctx| show(ctx, &mut text, cfg, &mut seen));
    assert_eq!(text, "first second");
    rig.key_with(Key::Z, command(), |ctx| {
        show(ctx, &mut text, cfg, &mut seen)
    });
    text
}

#[test]
fn an_ordinary_field_can_undo_and_a_secret_one_keeps_no_history() {
    assert_eq!(type_then_undo(Cfg::default()), "first");
    let secret = Cfg {
        secret: true,
        ..Cfg::default()
    };
    assert_eq!(type_then_undo(secret), "first second");
}

#[test]
fn a_field_without_history_shows_what_is_typed_and_cannot_undo_it() {
    let cfg = Cfg {
        no_history: true,
        ..Cfg::default()
    };
    assert_eq!(type_then_undo(cfg), "first second");
    // Unlike a secret, it is shown as it is.
    let mut text = String::from("monolith:card:1:ABC");
    let mut rig = Rig::new();
    rig.settle(2, |ctx| show(ctx, &mut text, cfg, &mut Seen::default()));
    let (output, _) = frame(&mut rig, &mut text, cfg);
    assert!(paints(&output, "monolith:card:1:ABC"));
}

/// Whether a focused field with `cfg` asks the platform for its input
/// method.
fn asks_for_the_input_method(cfg: Cfg) -> bool {
    let mut text = String::from("abc");
    let mut rig = focused(&mut text, cfg);
    let (output, _) = frame(&mut rig, &mut text, cfg);
    output.platform_output.ime.is_some()
}

#[test]
fn a_focused_secret_field_turns_the_input_method_off() {
    assert!(asks_for_the_input_method(Cfg::default()));
    let secret = Cfg {
        secret: true,
        ..Cfg::default()
    };
    assert!(!asks_for_the_input_method(secret));
}

#[test]
fn a_secret_field_cuts_typed_and_pasted_text_at_its_limit() {
    let cfg = Cfg {
        secret: true,
        limit: Some(12),
        ..Cfg::default()
    };
    let mut text = String::new();
    let mut rig = focused(&mut text, cfg);
    let mut seen = Seen::default();
    rig.type_text("correct horse", |ctx| show(ctx, &mut text, cfg, &mut seen));
    assert_eq!(text, "correct hors");
    rig.frame(vec![egui::Event::Paste("battery".to_owned())], |ctx| {
        show(ctx, &mut text, cfg, &mut seen)
    });
    assert_eq!(text, "correct hors");
}

#[test]
fn forget_removes_the_state_egui_keeps_for_the_field() {
    let mut text = String::new();
    let cfg = Cfg {
        secret: true,
        ..Cfg::default()
    };
    let mut rig = focused(&mut text, cfg);
    let mut seen = Seen::default();
    rig.type_text("abc", |ctx| show(ctx, &mut text, cfg, &mut seen));
    assert!(TextEdit::load_state(&rig.ctx, edit_id()).is_some());
    text_field::forget(&rig.ctx, edit_id());
    assert!(TextEdit::load_state(&rig.ctx, edit_id()).is_none());
}

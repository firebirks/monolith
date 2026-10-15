//! What the window shows, and where a click or a key takes it: the
//! settings of a narrow window, Escape out of the settings, a failed store
//! on every view, the state of Tor in a narrow window, the counts of the
//! identity switcher, empty states, the Tor settings once saved, the people
//! an identity blocked, the first screens, what a contact added by a card
//! or by an invitation waits for, the question before closing, what a
//! conversation says of delivery, and the composer while a message is being
//! queued.

#![cfg(feature = "fixtures")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use egui::epaint::{ClippedShape, Shape};
use egui::{
    Color32, Context, Event, FullOutput, Id, Key, Modifiers, OutputCommand, PointerButton, Pos2,
    RawInput, Rect, Vec2, ViewportCommand, ViewportId, pos2,
};
use monolith_ui::fixtures::{NOW, conversation_window, fixture, long_text};
use monolith_ui::{App, Effect, EffectResult};
use monolith_ui_kit::avatar::color_index;
use monolith_ui_kit::{TextField, Theme, fonts, icons, theme};
use monolith_ui_model::{
    Command, Deferral, Dialog, Failure, MessageView, Outcome, RuntimeEvent, Status, UiBlockedId,
    UiContactId, UiIdentityId, UiInvitationId, UiMessageId, UiRequestId, Update,
};
use zeroize::Zeroizing;

/// The sizes of the reviews.
const WIDE: Vec2 = Vec2::new(1180.0, 760.0);
const NARROW: Vec2 = Vec2::new(820.0, 700.0);
const SMALLEST: Vec2 = Vec2::new(760.0, 520.0);

const STORAGE: &str = "Monolith can no longer save. Restart Monolith to continue.";

const SAVES_NOTHING: &str = "This session saves nothing. Closing Monolith deletes your identities, \
                             contacts and messages for good.";

const BOTH_OPEN: &str = "Messages can be delivered only while Monolith is open on both sides.";
/// What the identities page says over the people an identity blocked.
const BLOCKED: &str = "People you blocked cannot send you requests or messages. Unblocking \
                       someone does not make them your contact again: they can send you a \
                       request if \"Who can send you requests\" lets them, and you can add \
                       their card.";

/// What the toast says once someone is unblocked.
const UNBLOCKED: &str = "Unblocked. They become a contact again only if you add their card or \
                         accept a request from them.";

/// The short fingerprints of Dana and Greg, whom the personal identity
/// blocked.
const DANA: &str = "D4XM 9QTB 2KWE 7RLN 5PZC 8HVA";
const GREG: &str = "G3VD 8KTN 5QWZ 2MLE 7XRC 4PBH";

struct Window {
    ctx: Context,
    app: App,
    size: Vec2,
    time: f64,
    /// The texts of the last frame, where they were drawn.
    texts: Vec<(String, Rect)>,
    /// The outlined rectangles of the last frame, with the colour of their
    /// outline.
    outlines: Vec<(Rect, Color32)>,
    /// Everything the last frame drew.
    shapes: Vec<ClippedShape>,
    /// What the frames since the last look asked of the window.
    commands: Vec<ViewportCommand>,
    /// What the last frame copied to the clipboard.
    copied: Vec<String>,
}

impl Window {
    fn new(name: &str, size: Vec2) -> Self {
        Self::with(fixture(name).unwrap(), size)
    }

    fn with(app: App, size: Vec2) -> Self {
        let mut window = Self::unsettled(app, size);
        window.settle();
        window
    }

    /// A window that has drawn no frame yet.
    fn unsettled(app: App, size: Vec2) -> Self {
        let ctx = Context::default();
        fonts::install(&ctx, &[]);
        theme::install(&ctx, Theme::light().with_reduced_motion(true));
        Self {
            ctx,
            app,
            size,
            time: 0.0,
            texts: Vec::new(),
            outlines: Vec::new(),
            shapes: Vec::new(),
            commands: Vec::new(),
            copied: Vec::new(),
        }
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.1;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
            time: Some(self.time),
            focused: true,
            events,
            ..RawInput::default()
        };
        let app = &mut self.app;
        let output = self.ctx.run(input, |ctx| monolith_ui::show(ctx, app));
        self.texts = texts(&output);
        self.outlines = outlines(&output);
        self.shapes.clone_from(&output.shapes);
        if let Some(viewport) = output.viewport_output.get(&ViewportId::ROOT) {
            self.commands.extend(viewport.commands.iter().cloned());
        }
        self.copied = output
            .platform_output
            .commands
            .iter()
            .filter_map(|command| match command {
                OutputCommand::CopyText(text) => Some(text.clone()),
                _ => None,
            })
            .collect();
    }

    /// Takes what the window was asked since the last call.
    fn take_commands(&mut self) -> Vec<ViewportCommand> {
        core::mem::take(&mut self.commands)
    }

    /// Turns the wheel over `at` by `delta` points (up when positive), a
    /// frame for each of `turns`, then lets the scrolling settle.
    fn scroll(&mut self, at: Pos2, delta: f32, turns: usize) {
        for _ in 0..turns {
            self.frame(vec![
                Event::PointerMoved(at),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: Vec2::new(0.0, delta),
                    modifiers: Modifiers::NONE,
                },
            ]);
        }
        for _ in 0..10 {
            self.frame(Vec::new());
        }
    }

    /// Where the text of message `sequence` of Alice was drawn in the last
    /// frame, if it was in view.
    fn message(&self, sequence: u64) -> Option<Rect> {
        let start = format!("Message {sequence}.");
        self.texts
            .iter()
            .find(|(text, _)| text.starts_with(&start))
            .map(|(_, rect)| *rect)
    }

    /// Whether message `sequence` of Alice was laid out in the last frame,
    /// in view or not.
    fn laid_out(&self, sequence: u64) -> bool {
        let id = Id::new(("monolith-bubble", UiMessageId::new(alice(), sequence))).with("text");
        self.ctx.read_response(id).is_some()
    }

    /// Whether a text drawn holds `part`.
    fn shows_part(&self, part: &str) -> bool {
        self.texts.iter().any(|(shown, _)| shown.contains(part))
    }

    /// Frames without input, so that layout settles.
    fn settle(&mut self) {
        for _ in 0..3 {
            self.frame(Vec::new());
        }
    }

    fn press(&mut self, key: Key) {
        self.frame(vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]);
        self.settle();
    }

    /// Clicks the middle of the first text that reads `text`.
    fn click(&mut self, text: &str) {
        let Some((_, rect)) = self.texts.iter().find(|(shown, _)| shown == text) else {
            panic!("{text:?} is not shown: {:#?}", self.shown());
        };
        self.click_at(rect.center());
    }

    /// Clicks the middle of the last text that reads `text`: the one drawn
    /// on top, in a dialog.
    fn click_last(&mut self, text: &str) {
        let Some((_, rect)) = self.texts.iter().rev().find(|(shown, _)| shown == text) else {
            panic!("{text:?} is not shown: {:#?}", self.shown());
        };
        self.click_at(rect.center());
    }

    /// Clicks the text that reads `text` on the line of the text `beside`.
    fn click_beside(&mut self, text: &str, beside: &str) {
        let Some((_, row)) = self.texts.iter().find(|(shown, _)| shown == beside) else {
            panic!("{beside:?} is not shown: {:#?}", self.shown());
        };
        let line = row.center().y;
        let Some((_, rect)) = self
            .texts
            .iter()
            .find(|(shown, rect)| shown == text && (rect.top()..=rect.bottom()).contains(&line))
        else {
            panic!(
                "{text:?} is not shown beside {beside:?}: {:#?}",
                self.shown()
            );
        };
        self.click_at(rect.center());
    }

    fn click_at(&mut self, at: Pos2) {
        let button = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        self.frame(vec![Event::PointerMoved(at)]);
        self.frame(vec![button(true)]);
        self.frame(vec![button(false)]);
        self.settle();
    }

    fn shown(&self) -> Vec<&str> {
        self.texts.iter().map(|(text, _)| text.as_str()).collect()
    }

    fn shows(&self, text: &str) -> bool {
        self.count(text) > 0
    }

    fn count(&self, text: &str) -> usize {
        self.texts.iter().filter(|(shown, _)| shown == text).count()
    }

    /// How many texts drawn hold `part`.
    fn count_part(&self, part: &str) -> usize {
        self.texts
            .iter()
            .filter(|(shown, _)| shown.contains(part))
            .count()
    }

    /// The outlines drawn in the colour of an error: the fields that hold
    /// one.
    fn error_outlines(&self) -> Vec<Rect> {
        let danger = Theme::light().color.danger;
        self.outlines
            .iter()
            .filter(|(_, color)| *color == danger)
            .map(|(rect, _)| *rect)
            .collect()
    }

    /// Types `text` into what has the focus.
    fn type_text(&mut self, text: &str) {
        self.frame(vec![Event::Text(text.to_owned())]);
    }

    /// Answers the commands asked since the last look that `answer` has an
    /// answer for, and returns every command asked.
    fn answer(&mut self, mut answer: impl FnMut(&Command) -> Option<Outcome>) -> Vec<Command> {
        let mut asked = Vec::new();
        for (command, what) in self.app.model.take_commands() {
            if let Some(outcome) = answer(&what) {
                self.app.model.apply(Update::Done { command, outcome });
            }
            asked.push(what);
        }
        self.settle();
        asked
    }

    /// The groups on each row of every full fingerprint (13 groups of four
    /// characters) drawn in the last frame.
    fn fingerprints(&self) -> Vec<Vec<usize>> {
        let mut found = Vec::new();
        each_shape(&self.shapes, &mut |shape, _| {
            let Shape::Text(text) = shape else {
                return;
            };
            let groups: Vec<&str> = text.galley.text().split_whitespace().collect();
            if groups.len() != 13 || groups.iter().any(|group| group.len() != 4) {
                return;
            }
            let rows = text
                .galley
                .rows
                .iter()
                .map(|row| {
                    let line: String = row.glyphs.iter().map(|glyph| glyph.chr).collect();
                    line.split_whitespace().count()
                })
                .collect();
            found.push(rows);
        });
        found
    }

    /// The rectangles the last frame filled with `color`, such as the
    /// notices of one tone.
    fn filled(&self, color: Color32) -> Vec<Rect> {
        let mut found = Vec::new();
        each_shape(&self.shapes, &mut |shape, _| {
            if let Shape::Rect(rect) = shape {
                if rect.fill == color {
                    found.push(rect.rect);
                }
            }
        });
        found
    }

    /// The fills of the rectangles the last frame drew.
    fn fills(&self) -> Vec<Color32> {
        let mut found = Vec::new();
        each_shape(&self.shapes, &mut |shape, _| {
            if let Shape::Rect(rect) = shape {
                found.push(rect.fill);
            }
        });
        found
    }

    /// The clip of the first text that reads `text`.
    fn clip_of(&self, text: &str) -> Option<Rect> {
        let mut found = None;
        each_shape(&self.shapes, &mut |shape, clip| {
            if let Shape::Text(shown) = shape {
                if found.is_none() && shown.galley.text() == text {
                    found = Some(clip);
                }
            }
        });
        found
    }

    /// Where the first toast was drawn in the last frame.
    fn toast(&self) -> Rect {
        (0..8_u64)
            .find_map(|toast| self.ctx.read_response(Id::new(("monolith-toast", toast))))
            .expect("a toast is shown")
            .rect
    }

    /// Where the first text that holds `part` was drawn.
    fn rect_of(&self, part: &str) -> Rect {
        self.texts
            .iter()
            .find(|(shown, _)| shown.contains(part))
            .map(|(_, rect)| *rect)
            .unwrap_or_else(|| panic!("{part:?} is not shown"))
    }
}

/// Calls `visit` with every shape of `shapes`, those inside a list
/// included, and the clip it was drawn in.
fn each_shape(shapes: &[ClippedShape], visit: &mut impl FnMut(&Shape, Rect)) {
    fn walk(shape: &Shape, clip: Rect, visit: &mut impl FnMut(&Shape, Rect)) {
        if let Shape::Vec(list) = shape {
            for shape in list {
                walk(shape, clip, visit);
            }
        } else {
            visit(shape, clip);
        }
    }
    for clipped in shapes {
        walk(&clipped.shape, clipped.clip_rect, visit);
    }
}

fn collect(shape: &Shape, into: &mut Vec<(String, Rect)>) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, into);
            }
        }
        Shape::Text(text) => into.push((
            text.galley.text().to_owned(),
            text.galley.rect.translate(text.pos.to_vec2()),
        )),
        _ => {}
    }
}

/// Every text painted in `output`, with where it was painted.
fn texts(output: &FullOutput) -> Vec<(String, Rect)> {
    let mut found = Vec::new();
    for clipped in &output.shapes {
        collect(&clipped.shape, &mut found);
    }
    found
}

fn collect_outlines(shape: &Shape, into: &mut Vec<(Rect, Color32)>) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                collect_outlines(shape, into);
            }
        }
        Shape::Rect(rect) if rect.stroke.width > 0.0 => into.push((rect.rect, rect.stroke.color)),
        _ => {}
    }
}

/// Every outlined rectangle painted in `output`, with its outline colour.
fn outlines(output: &FullOutput) -> Vec<(Rect, Color32)> {
    let mut found = Vec::new();
    for clipped in &output.shapes {
        collect_outlines(&clipped.shape, &mut found);
    }
    found
}

fn back() -> String {
    icons::CHEVRON_LEFT.char().to_string()
}

fn alice() -> UiContactId {
    UiContactId::new(UiIdentityId::from_bytes([1; 16]), [10; 16])
}

/// Message `n` of Alice as `conversation_window(first, count, 200)` makes
/// it; one before `first` is older, one after the window newer.
fn alice_message(first: u64, count: u64, n: u64) -> MessageView {
    let status = match n.checked_sub(first).map(|index| index % 3) {
        Some(1) => Status::Delivered,
        Some(2) => Status::Queued,
        _ => Status::Received,
    };
    MessageView {
        id: UiMessageId::new(alice(), n),
        status,
        time: NOW - (first + count).saturating_sub(n) * 180,
        text: long_text(usize::try_from(n).unwrap(), 200),
    }
}

/// Answers the reads of Alice's messages the window asked for with
/// `messages`, and returns how many there were.
fn answer_messages(window: &mut Window, messages: &[MessageView]) -> usize {
    let mut answered = 0;
    for (command, asked) in window.app.model.take_commands() {
        let complete = false;
        let outcome = match asked {
            Command::LoadMessages { contact, .. } | Command::LoadEarlier { contact, .. }
                if contact == alice() =>
            {
                Outcome::Messages {
                    contact,
                    messages: messages.to_vec(),
                    complete,
                }
            }
            _ => continue,
        };
        window.app.model.apply(Update::Done { command, outcome });
        answered += 1;
    }
    answered
}

/// The middle of the list of messages in a wide window.
const MESSAGES: Pos2 = pos2(760.0, 360.0);

#[test]
fn a_long_conversation_opens_at_its_newest_message_and_lays_out_only_what_is_in_view() {
    let window = Window::with(conversation_window(1, 2_000, 200), WIDE);
    let newest = window
        .message(2_000)
        .unwrap_or_else(|| panic!("the newest message is not shown: {:#?}", window.shown()));
    // Its whole bubble is in view, just above the composer.
    let composer = window
        .texts
        .iter()
        .find(|(text, _)| text == "Write a message")
        .map(|(_, rect)| *rect)
        .unwrap();
    assert!(newest.bottom() < composer.top(), "{newest:?} {composer:?}");
    assert!(
        composer.top() - newest.bottom() < 80.0,
        "{newest:?} {composer:?}"
    );
    // The oldest messages are neither shown nor laid out.
    assert!(window.message(1).is_none());
    assert!(!window.laid_out(1));
    assert!(!window.laid_out(1_000));
    assert!(window.laid_out(2_000));
    let laid_out = (1..=2_000).filter(|n| window.laid_out(*n)).count();
    assert!(laid_out < 40, "{laid_out} messages were laid out");
    // Higher up, neither are the newest.
    let mut window = window;
    window.scroll(MESSAGES, 2_000.0, 5);
    assert!(!window.laid_out(2_000));
    let laid_out = (1..=2_000).filter(|n| window.laid_out(*n)).count();
    assert!(
        (1..40).contains(&laid_out),
        "{laid_out} messages were laid out"
    );
}

#[test]
fn the_first_frame_of_a_long_conversation_shows_its_newest_messages_in_place() {
    // Messages of 1.5 KB have more lines than their length tells, so their
    // heights are known only once they are laid out.
    let mut window = Window::unsettled(conversation_window(1, 2_000, 1_536), WIDE);
    window.frame(Vec::new());
    let first: Vec<Option<Rect>> = (1_990..=2_000).map(|n| window.message(n)).collect();
    assert!(first[10].is_some(), "the newest message is not shown");
    // No message is drawn over another.
    let shown: Vec<Rect> = first.iter().flatten().copied().collect();
    assert!(shown.len() >= 2);
    for pair in shown.windows(2) {
        assert!(pair[1].top() >= pair[0].bottom(), "{shown:#?}");
    }
    // And all are where they stay.
    window.settle();
    let settled: Vec<Option<Rect>> = (1_990..=2_000).map(|n| window.message(n)).collect();
    assert_eq!(first, settled);
}

#[test]
fn a_contact_without_a_name_has_a_person_for_an_avatar() {
    // Carol has no name and Dana is no longer a contact: in the bar of
    // Carol's conversation and in the list, a person, never initials.
    let window = Window::new("waiting", WIDE);
    let person = icons::USER.char().to_string();
    assert!(window.count(&person) >= 3, "{:#?}", window.shown());
    assert!(!window.shows("CC"));
    assert!(!window.shows("FC"));
    assert!(window.shows("AM"));
}

#[test]
fn the_composer_of_a_conversation_whose_messages_are_deleted_is_forgotten() {
    let mut window = Window::new("conversation", WIDE);
    window.frame(vec![Event::Text("draft".to_owned())]);
    let edit = Id::new(("monolith-composer", ("composer", alice()))).with("edit");
    let bob = UiContactId::new(UiIdentityId::from_bytes([1; 16]), [11; 16]);
    window.app.model.select_conversation(bob);
    window.settle();
    assert!(egui::TextEdit::load_state(&window.ctx, edit).is_some());
    window.app.model.open(Dialog::DeleteConversation {
        contact: alice(),
        busy: None,
        error: None,
    });
    window.app.model.confirm();
    for (command, asked) in window.app.model.take_commands() {
        if matches!(asked, Command::DeleteConversation(_)) {
            window.app.model.apply(Update::Done {
                command,
                outcome: Outcome::Done,
            });
        }
    }
    window.settle();
    assert!(egui::TextEdit::load_state(&window.ctx, edit).is_none());
}

#[test]
fn reading_older_messages_keeps_the_reader_where_they_are() {
    // Messages 301 to 500 are read; 201 to 300 come with the next page.
    let mut window = Window::with(conversation_window(301, 200, 200), WIDE);
    let mut turns = 0;
    while !window.shows("Load earlier messages") {
        window.scroll(MESSAGES, 2_000.0, 5);
        turns += 1;
        assert!(turns < 40, "the top was never reached");
    }
    let before = window.message(301).unwrap();
    window.click("Load earlier messages");
    let older: Vec<MessageView> = (201..=300).map(|n| alice_message(301, 200, n)).collect();
    let asked: Vec<Command> = window
        .app
        .model
        .take_commands()
        .into_iter()
        .map(|(command, asked)| {
            if matches!(asked, Command::LoadEarlier { .. }) {
                window.app.model.apply(Update::Done {
                    command,
                    outcome: Outcome::Messages {
                        contact: alice(),
                        messages: older.clone(),
                        complete: false,
                    },
                });
            }
            asked
        })
        .collect();
    assert!(
        asked.iter().any(|command| matches!(
            command,
            Command::LoadEarlier {
                before: 301,
                limit: 100,
                ..
            }
        )),
        "{asked:?}"
    );
    window.settle();
    assert_eq!(
        window.app.model.thread(alice()).unwrap().messages.len(),
        300
    );
    // The message at the top stays where it was; the older ones are above
    // it, out of view, with the offer of more.
    let after = window.message(301).unwrap();
    assert!(
        (after.top() - before.top()).abs() < 1.0,
        "{before:?} {after:?}"
    );
    assert!(
        window
            .message(300)
            .is_none_or(|rect| rect.bottom() <= after.top())
    );
    window.scroll(MESSAGES, 2_000.0, 20);
    assert!(window.message(201).is_some());
    assert!(window.shows("Load earlier messages"));
}

#[test]
fn a_selection_in_a_message_stays_with_it_when_new_messages_arrive() {
    let mut window = Window::with(conversation_window(1, 50, 200), WIDE);
    // A double click on the first word of the newest message selects it.
    let text = window.message(50).unwrap();
    let at = pos2(text.left() + 8.0, text.top() + 8.0);
    window.frame(vec![Event::PointerMoved(at)]);
    for pressed in [true, false, true, false] {
        window.frame(vec![Event::PointerButton {
            pos: at,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        }]);
    }
    window.settle();
    // Two messages arrive and are read.
    for sequence in [51, 52] {
        window
            .app
            .model
            .apply(Update::Event(RuntimeEvent::MessageReceived(
                UiMessageId::new(alice(), sequence),
            )));
        let messages: Vec<MessageView> = (1..=sequence).map(|n| alice_message(1, 50, n)).collect();
        assert_eq!(answer_messages(&mut window, &messages), 1);
        window.settle();
    }
    // The newest is in view, and so is the one selected, which copies the
    // word selected.
    assert!(window.message(52).is_some());
    assert!(window.message(50).is_some());
    window.frame(vec![Event::Copy]);
    assert_eq!(window.copied, ["Message"]);
}

#[test]
fn a_reader_who_scrolled_up_stays_put_and_each_conversation_keeps_its_place() {
    let mut window = Window::with(conversation_window(1, 300, 200), WIDE);
    window.scroll(MESSAGES, 120.0, 30);
    let (seen, before) = (1..=300)
        .rev()
        .find_map(|n| window.message(n).map(|rect| (n, rect)))
        .unwrap();
    assert!(window.message(300).is_none(), "the view did not move up");
    // A new message does not move what the reader sees.
    window
        .app
        .model
        .apply(Update::Event(RuntimeEvent::MessageReceived(
            UiMessageId::new(alice(), 301),
        )));
    let messages: Vec<MessageView> = (1..=301).map(|n| alice_message(1, 300, n)).collect();
    assert_eq!(answer_messages(&mut window, &messages), 1);
    window.settle();
    assert_eq!(window.message(seen), Some(before));
    // Another conversation, and back: the place is kept.
    let bob = UiContactId::new(UiIdentityId::from_bytes([1; 16]), [11; 16]);
    window.app.model.select_conversation(bob);
    window.settle();
    assert!(window.message(seen).is_none());
    window.app.model.select_conversation(alice());
    window.settle();
    assert_eq!(window.message(seen), Some(before));
}

#[test]
fn a_narrow_window_lists_the_settings_pages_then_shows_one_with_a_way_back() {
    for size in [NARROW, SMALLEST] {
        let mut window = Window::new("settings-list", size);
        for page in ["Appearance", "Identities", "Tor", "Data", "About"] {
            assert!(window.shows(page), "{page} is not listed");
        }
        assert!(!window.shows("Who can send you requests"));
        assert!(!window.shows(&back()), "the list has nowhere to go back to");

        window.click("Identities");
        assert!(window.shows("Who can send you requests"));
        assert!(!window.shows("Cards, requests and invitations"));

        window.click(&back());
        assert!(window.shows("Cards, requests and invitations"));
        assert!(!window.shows("Who can send you requests"));

        window.click("Done");
        assert!(window.shows("Filter contacts"), "Done did not leave");
    }
}

#[test]
fn a_wide_window_shows_the_pages_beside_the_page() {
    let window = Window::new("settings-list", WIDE);
    assert!(window.shows("Identities"));
    assert!(window.shows("Theme"));
    assert!(!window.shows(&back()));
}

#[test]
fn escape_leaves_the_settings() {
    for size in [WIDE, NARROW] {
        let mut window = Window::new("settings-tor", size);
        assert!(window.shows("SOCKS endpoint"));
        window.press(Key::Escape);
        assert!(!window.shows("SOCKS endpoint"));
        assert!(window.shows("Filter contacts"));
    }
}

#[test]
fn escape_closes_a_dialog_over_the_settings_before_it_leaves_them() {
    let mut window = Window::new("settings-identities", WIDE);
    window.app.model.open(Dialog::NewIdentity {
        name: String::new(),
        busy: None,
        error: None,
    });
    window.settle();
    window.press(Key::Escape);
    assert!(window.app.model.dialog().is_none());
    assert!(window.shows("Who can send you requests"));
    window.press(Key::Escape);
    assert!(!window.shows("Who can send you requests"));
}

#[test]
fn a_failed_store_is_said_on_every_view() {
    let views = [
        "conversation",
        "narrow-conversation",
        "requests",
        "narrow-requests",
        "empty-identity",
        "settings-list",
        "settings-tor",
        "about",
    ];
    for name in views {
        for size in [WIDE, NARROW, SMALLEST] {
            let mut app = fixture(name).unwrap();
            let before = Window::with(fixture(name).unwrap(), size);
            assert!(!before.shows(STORAGE), "{name} says it before");
            app.model.apply(Update::Event(RuntimeEvent::StorageFailed));
            let window = Window::with(app, size);
            assert!(window.shows(STORAGE), "{name} at {size:?} does not say it");
        }
    }
}

#[test]
fn a_narrow_list_says_the_state_of_tor() {
    for (name, words) in [
        ("conversation", "Tor connected"),
        ("tor-unavailable", "Tor unavailable"),
        ("tor-connecting", "Connecting to Tor"),
    ] {
        let window = Window::new(name, NARROW);
        assert!(window.shows("Filter contacts"), "{name} shows no list");
        assert!(window.shows(words), "{name} does not say {words:?}");
    }
}

#[test]
fn the_switcher_gives_each_identity_its_counts() {
    let window = Window::new("identity-switcher", WIDE);
    assert!(window.shows("4 contacts, 3 new, 1 request"));
    assert!(window.shows("No contacts"));
}

#[test]
fn my_card_is_one_click_from_the_list() {
    let mut window = Window::new("conversation", WIDE);
    window.click(&icons::ID_CARD.char().to_string());
    assert!(matches!(
        window.app.model.dialog(),
        Some(Dialog::OwnCard { .. })
    ));
}

#[test]
fn the_identity_switcher_shares_the_card_of_the_identity_shown() {
    let mut window = Window::new("identity-switcher", WIDE);
    let shown = window.app.model.current();
    window.click("Share my card");
    assert!(matches!(
        window.app.model.dialog(),
        Some(Dialog::OwnCard { identity, .. }) if Some(*identity) == shown
    ));
}

#[test]
fn an_identity_without_contacts_is_offered_them_once() {
    let wide = Window::new("empty-identity", WIDE);
    assert_eq!(wide.count("No contacts yet"), 1);
    assert!(wide.shows("Your contacts will be listed here."));
    let narrow = Window::new("empty-identity", NARROW);
    assert_eq!(narrow.count("No contacts yet"), 1);
}

#[test]
fn before_the_first_identity_the_list_offers_nothing_it_cannot_do() {
    for size in [WIDE, NARROW] {
        let window = Window::new("no-identity", size);
        assert!(window.shows("Create identity"));
        assert!(!window.shows("Add contact"));
        assert!(!window.shows("Invite"));
    }
}

#[test]
fn the_tor_settings_say_when_they_are_saved_and_how_they_take_effect() {
    const SAVED: &str = "Saved. Close and reopen Monolith to use these settings. Check \
                         again tests the settings in use now.";
    let mut window = Window::new("settings-tor", WIDE);
    assert!(!window.shows(SAVED));
    window.click("Tor Browser");
    window.click("Save");
    let saved = window
        .app
        .take_effects()
        .into_iter()
        .find_map(|effect| match effect {
            Effect::Preferences(preferences) => Some(preferences),
            _ => None,
        })
        .expect("Save asked for nothing");
    assert_eq!(saved.tor.socks, "127.0.0.1:9150");
    // The desktop applies them.
    window.app.set_preferences(saved);
    window.settle();
    assert!(window.shows(SAVED));
    // Gone once the settings change again.
    window.click("System Tor");
    assert!(!window.shows(SAVED));
    window.click("Tor Browser");
    assert!(window.shows(SAVED));
    // And on the next visit of the page.
    window.press(Key::Escape);
    window.click(&icons::SETTINGS.char().to_string());
    window.click("Tor");
    assert!(window.shows("SOCKS endpoint"));
    assert!(!window.shows(SAVED));
}

#[test]
fn the_first_screens_say_where_the_profile_is_kept_only_when_asked() {
    for name in ["welcome", "unlock"] {
        let mut window = Window::new(name, WIDE);
        let path = "/home/user/.local/share/monolith";
        assert!(!window.shows(path), "{name} shows the path at once");
        window.click("Where is my profile kept?");
        assert!(window.shows(path), "{name} does not show the path");
    }
}

#[test]
fn a_passphrase_is_cut_at_the_longest_the_vault_takes() {
    let mut window = Window::new("welcome", WIDE);
    // 256 characters of four bytes are the 1024 bytes the vault takes.
    let long = "\u{1D49C}".repeat(300);
    // The first field has the focus; type, move to the second, type.
    window.frame(vec![Event::Text(long.clone())]);
    window.frame(vec![Event::Key {
        key: Key::Tab,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    }]);
    window.frame(vec![Event::Text(long)]);
    window.press(Key::Enter);
    let created =
        window
            .app
            .model
            .take_commands()
            .into_iter()
            .find_map(|(_, command)| match command {
                Command::CreateProfile { passphrase } => {
                    Some((passphrase.chars().count(), passphrase.len()))
                }
                _ => None,
            });
    assert_eq!(created, Some((256, 1024)));
}

/// What a contact added by a card, not an invitation, waits for.
const CARD_ADDED: &str = "This is their card, not an invitation. Contact C7TQ must add your \
                          card too before your messages can arrive.";
const WAITING_CARD: &str = "You added their card, not an invitation. Contact C7TQ must add \
                            your card too before your messages can arrive.";
const WAITING_INVITATION: &str = "Your request goes out with your messages. Contact C7TQ should \
                                  see it through the invitation they gave you, with your \
                                  fingerprint only, and can read your messages after \
                                  accepting. Tell them it is you.";

/// The fixture `name`, with the conversation of Carol shown also in a
/// narrow window.
fn conversation_of_carol(name: &str, size: Vec2) -> Window {
    let mut window = Window::new(name, size);
    if size != WIDE {
        window.click("Contact C7TQ");
    }
    window
}

#[test]
fn a_conversation_says_whether_a_card_or_an_invitation_was_added() {
    for size in [WIDE, NARROW, SMALLEST] {
        let window = conversation_of_carol("waiting", size);
        assert!(window.shows(WAITING_INVITATION), "{size:?}");
        assert!(window.shows("Waiting for them to accept your request"));
        assert!(!window.shows(WAITING_CARD));
        assert!(!window.shows("Show my card"));

        let mut window = conversation_of_carol("waiting-card", size);
        assert!(window.shows(WAITING_CARD), "{size:?}");
        assert!(window.shows("Waiting for them to add your card"));
        assert!(!window.shows(WAITING_INVITATION));
        window.click("Show my card");
        assert!(matches!(
            window.app.model.dialog(),
            Some(Dialog::OwnCard { identity, .. }) if *identity == window.app.model.current().unwrap()
        ));
    }
    // Accepted, the contact waits for nothing.
    let window = Window::new("conversation", WIDE);
    assert!(!window.shows(WAITING_CARD) && !window.shows(WAITING_INVITATION));
}

#[test]
fn a_card_added_says_what_the_other_side_has_to_do_and_offers_the_own_card() {
    for size in [WIDE, NARROW, SMALLEST] {
        let mut window = Window::new("card-added", size);
        assert!(window.shows("Contact added"));
        assert!(window.shows(CARD_ADDED), "{size:?}: {:#?}", window.shown());
        window.click("Done");
        assert!(window.app.model.dialog().is_none());
        // The conversation behind it, in a wide window, says the same for
        // as long as the contact waits; a narrow one shows the list.
        assert_eq!(window.shows(WAITING_CARD), size == WIDE);

        // The conversation behind offers the card too; the dialog's button
        // is drawn on top.
        let mut window = Window::new("card-added", size);
        let behind = usize::from(size == WIDE);
        assert_eq!(window.count("Show my card"), 1 + behind);
        window.click_last("Show my card");
        assert!(matches!(
            window.app.model.dialog(),
            Some(Dialog::OwnCard { .. })
        ));
        assert!(!window.shows(CARD_ADDED));
    }
}

#[test]
fn the_own_card_says_what_a_card_alone_can_do_under_each_request_mode() {
    const INVITATIONS: &str = "Share this card with people you want to talk to. It contains no \
                               secret. Someone who has only your card can reach you once you \
                               have added their card too. To let someone reach you first, give \
                               them an invitation instead.";
    const ANYONE: &str = "Share this card with people you want to talk to. It contains no \
                          secret. Anyone who has it can send you a request.";
    const NOBODY: &str = "Share this card with people you want to talk to. It contains no \
                          secret. Requests are set to Nobody, so no request arrives at all. \
                          Someone who has your card can reach you once you have added their \
                          card too.";
    let all = [INVITATIONS, ANYONE, NOBODY];
    for (name, said) in [
        ("conversation", INVITATIONS),
        ("own-card-open", ANYONE),
        ("settings-nobody", NOBODY),
    ] {
        let mut app = fixture(name).unwrap();
        if app.model.dialog().is_none() {
            let identity = app.model.current().unwrap();
            app.model.open(Dialog::OwnCard {
                identity,
                card: None,
                busy: None,
                error: None,
            });
            for (command, asked) in app.model.take_commands() {
                if matches!(asked, Command::OwnCard(_)) {
                    app.model.apply(Update::Done {
                        command,
                        outcome: Outcome::Card(Zeroizing::new("MONOLITH1:AAAA".to_owned())),
                    });
                }
            }
        }
        let window = Window::with(app, WIDE);
        for text in all {
            assert_eq!(window.shows(text), text == said, "{name}: {text}");
        }
    }
}

#[test]
fn closing_a_session_that_saves_nothing_says_everything_goes() {
    for size in [WIDE, NARROW, SMALLEST] {
        let window = Window::new("close-ephemeral", size);
        assert!(window.shows("Close Monolith?"), "{:#?}", window.shown());
        assert!(window.shows(SAVES_NOTHING));
        // Carol's request waits to leave, and never will.
        assert!(window.shows("1 message has not been delivered yet and never will be."));
        assert!(window.shows("Close Monolith"));
        assert!(window.shows("Cancel"));
    }
}

#[test]
fn closing_with_messages_not_delivered_says_when_they_are_tried_again() {
    for size in [WIDE, NARROW, SMALLEST] {
        let window = Window::new("close-queued", size);
        assert!(window.shows("Close Monolith?"));
        assert!(window.shows(
            "2 messages have not been delivered yet. Monolith tries to send them again the next \
             time it is open."
        ));
        assert!(!window.shows(SAVES_NOTHING));
    }
}

#[test]
fn the_question_before_closing_keeps_the_window_unless_close_is_chosen() {
    let mut window = Window::new("close-queued", WIDE);
    let _ = window.take_commands();
    // Cancel has the focus: Enter keeps the window.
    window.press(Key::Enter);
    assert!(!window.app.asking_to_close());
    assert!(!window.shows("Close Monolith?"));
    assert!(!window.take_commands().contains(&ViewportCommand::Close));
    // So do Escape and Cancel.
    assert!(!window.app.close_requested());
    window.settle();
    window.press(Key::Escape);
    assert!(!window.app.asking_to_close());
    assert!(!window.app.close_requested());
    window.settle();
    window.click("Cancel");
    assert!(!window.app.asking_to_close());
    assert!(!window.take_commands().contains(&ViewportCommand::Close));
    // Close Monolith closes it, and the window then closes without asking.
    assert!(!window.app.close_requested());
    window.settle();
    window.click("Close Monolith");
    assert!(window.take_commands().contains(&ViewportCommand::Close));
    assert!(!window.app.asking_to_close());
    assert!(window.app.close_requested());
    assert!(!window.app.asking_to_close());
}

#[test]
fn the_question_before_closing_puts_another_dialog_aside() {
    let mut app = fixture("contact-details").unwrap();
    assert!(!app.close_requested());
    let mut window = Window::with(app, WIDE);
    assert!(window.shows("Close Monolith?"));
    assert!(!window.shows("Contact"));
    // Escape answers the question, not the dialog behind it.
    window.press(Key::Escape);
    assert!(!window.app.asking_to_close());
    assert!(matches!(
        window.app.model.dialog(),
        Some(Dialog::ContactDetails { .. })
    ));
    assert!(window.shows("Contact"));
}

#[test]
fn escape_answers_the_question_before_closing_and_leaves_the_settings_open() {
    let mut app = fixture("settings-tor").unwrap();
    assert!(!app.close_requested());
    let mut window = Window::with(app, WIDE);
    assert!(window.shows("Close Monolith?"));
    window.press(Key::Escape);
    assert!(!window.app.asking_to_close());
    assert!(window.shows("SOCKS endpoint"));
}

#[test]
fn a_waiting_delivery_says_both_sides_must_be_open_once_and_when_it_is_tried_next() {
    let window = Window::new("conversation-deferred", WIDE);
    let banner = "Alice Martin cannot be reached right now. Their Monolith may be closed or \
                  offline. Monolith tries again from time to time; your messages stay queued \
                  on this device. Messages can be delivered only while Monolith is open on \
                  both sides. Next try in about 4 minutes.";
    assert!(window.shows(banner), "{:#?}", window.shown());
    assert_eq!(
        window
            .texts
            .iter()
            .filter(|(shown, _)| shown.contains(BOTH_OPEN))
            .count(),
        1
    );
    // Later, by the clock of the window.
    for (later, said) in [
        (200, Some("Next try in less than a minute.")),
        (240, None),
        (900, None),
    ] {
        let mut app = fixture("conversation-deferred").unwrap();
        let model = core::mem::take(&mut app.model);
        let window = Window::with(App::for_tests(model, NOW + later), WIDE);
        assert!(window.shows_part(BOTH_OPEN));
        assert_eq!(window.shows_part("Next try"), said.is_some(), "{later}");
        if let Some(said) = said {
            assert!(window.shows_part(said), "{later}");
        }
    }
}

#[test]
fn no_next_try_is_said_without_a_deferral_or_while_tor_is_down() {
    let window = Window::new("conversation-queued", WIDE);
    assert!(!window.shows_part("Next try"));
    let mut app = fixture("tor-unavailable").unwrap();
    app.model.set_time(NOW);
    app.model
        .apply(Update::Event(RuntimeEvent::DeliveryDeferred {
            contact: alice(),
            reason: Deferral::Tor,
            retry_in: 300,
        }));
    let window = Window::with(app, WIDE);
    assert!(window.shows_part("Tor is not available"));
    assert!(window.shows_part(BOTH_OPEN));
    assert!(!window.shows_part("Next try"));
}

#[test]
fn what_is_typed_while_a_message_is_queued_stays_in_the_composer() {
    let mut window = Window::new("conversation", WIDE);
    let _ = window.app.model.take_commands();
    window.frame(vec![Event::Text("See you at eight.".to_owned())]);
    window.press(Key::Enter);
    let sent: Vec<_> = window
        .app
        .model
        .take_commands()
        .into_iter()
        .filter(|(_, command)| matches!(command, Command::Send { .. }))
        .collect();
    assert_eq!(sent.len(), 1);
    // Typed on while the runtime queues it.
    window.frame(vec![Event::Text(" Bring the map.".to_owned())]);
    window.settle();
    assert!(window.shows("See you at eight. Bring the map."));
    window.app.model.apply(Update::Done {
        command: sent[0].0,
        outcome: Outcome::MessageQueued(UiMessageId::new(alice(), 20)),
    });
    window.settle();
    assert_eq!(
        window.app.model.thread(alice()).unwrap().draft,
        "Bring the map."
    );
    assert!(window.shows("Bring the map."));
}

/// What the question before unblocking says: what it lets them do, and
/// that nothing is kept of them to block them by.
const UNBLOCK: &str = "They can send you requests again, as \"Who can send you requests\" \
                       allows. Monolith keeps no record of them after this, so you can block \
                       them again only from a request they send or once you have added their \
                       card.";

#[test]
fn the_identities_page_lists_who_is_blocked_and_unblocks_after_asking() {
    // Tall enough to show the whole page, wide and narrow.
    for size in [Vec2::new(1180.0, 2000.0), Vec2::new(760.0, 2200.0)] {
        let mut window = Window::new("settings-blocked", size);
        assert!(window.shows(BLOCKED), "{size:?}");
        assert!(window.shows(DANA));
        assert!(window.shows(GREG));
        assert_eq!(window.count("Unblock"), 2);
        // Work blocked nobody: one line says so.
        assert_eq!(window.count("Nobody is blocked."), 1);

        // A question first, which says what unblocking does; nothing is
        // unblocked by asking, and Enter keeps the block: Cancel has the
        // focus.
        let _ = window.app.model.take_commands();
        window.click_beside("Unblock", DANA);
        assert!(
            window.shows("Unblock this person?"),
            "{:#?}",
            window.shown()
        );
        assert!(window.shows(UNBLOCK));
        assert!(window.app.model.take_commands().is_empty());
        window.press(Key::Enter);
        assert!(!window.shows("Unblock this person?"));
        assert!(window.app.model.take_commands().is_empty());
        // Asked again and confirmed: the person on the row is unblocked.
        window.click_beside("Unblock", DANA);
        window.click_last("Unblock");
        assert!(!window.shows("Unblock this person?"));
        let dana = UiBlockedId::new(UiIdentityId::from_bytes([1; 16]), 1);
        let commands = window.app.model.take_commands();
        let asked: Vec<&Command> = commands.iter().map(|(_, command)| command).collect();
        assert_eq!(asked, [&Command::Unblock(dana)]);
        // While it is under way, the button takes no click.
        window.click_beside("Unblock", DANA);
        assert!(!window.shows("Unblock this person?"));
        assert!(window.app.model.take_commands().is_empty());

        window.app.model.apply(Update::Done {
            command: commands[0].0,
            outcome: Outcome::Done,
        });
        window.settle();
        assert!(window.shows(UNBLOCKED), "the toast does not say it");

        // The button of the other row names the other person.
        window.click_beside("Unblock", GREG);
        window.click_last("Unblock");
        let greg = UiBlockedId::new(UiIdentityId::from_bytes([1; 16]), 2);
        let unblocks: Vec<Command> = window
            .app
            .model
            .take_commands()
            .into_iter()
            .map(|(_, command)| command)
            .filter(|command| matches!(command, Command::Unblock(_)))
            .collect();
        assert_eq!(unblocks, [Command::Unblock(greg)]);
    }
}

#[test]
fn the_shortcuts_of_the_window_wait_while_unblocking_is_asked() {
    let mut window = Window::new("settings-blocked", Vec2::new(1180.0, 2000.0));
    let _ = window.app.model.take_commands();
    window.click_beside("Unblock", DANA);
    assert!(window.shows("Unblock this person?"));
    // Escape answers the question; it does not leave the settings.
    window.press(Key::Escape);
    assert!(!window.shows("Unblock this person?"));
    assert!(window.shows(DANA));
    assert!(window.app.model.take_commands().is_empty());
}

#[test]
fn an_identity_that_blocked_nobody_says_so_in_one_line() {
    let window = Window::new("settings-identities", Vec2::new(1180.0, 2000.0));
    assert_eq!(window.count("Nobody is blocked."), 2);
    assert!(!window.shows(BLOCKED));
    assert!(!window.shows("Unblock"));
}

fn carol() -> UiContactId {
    UiContactId::new(UiIdentityId::from_bytes([1; 16]), [12; 16])
}

/// How many texts drawn say that Monolith must be started again.
fn restart_said(window: &Window) -> usize {
    window.count_part("Restart Monolith")
}

#[test]
fn a_failed_store_is_said_once_and_not_again_where_it_was_met() {
    let mut window = Window::new("conversation", WIDE);
    let _ = window.app.model.take_commands();
    // A send meets it.
    window.type_text("See you at eight.");
    window.press(Key::Enter);
    window.answer(|command| match command {
        Command::Send { .. } => Some(Outcome::Failed(Failure::Storage)),
        _ => None,
    });
    window
        .app
        .model
        .apply(Update::Event(RuntimeEvent::StorageFailed));
    window.settle();
    assert_eq!(restart_said(&window), 1, "{:#?}", window.shown());
    // So does the action of a dialog.
    window.app.model.open(Dialog::ContactDetails {
        contact: alice(),
        name: "Alice".to_owned(),
        busy: None,
        error: None,
    });
    window.app.model.confirm();
    window.answer(|command| match command {
        Command::SetContactName { .. } => Some(Outcome::Failed(Failure::Storage)),
        _ => None,
    });
    assert_eq!(restart_said(&window), 1, "{:#?}", window.shown());
    // And a command without a dialog.
    window.app.model.close();
    window.app.model.set_verified(alice(), false);
    window.answer(|command| match command {
        Command::SetVerified { .. } => Some(Outcome::Failed(Failure::Storage)),
        _ => None,
    });
    assert_eq!(restart_said(&window), 1, "{:#?}", window.shown());
}

#[test]
fn a_message_refused_for_want_of_room_says_what_to_do() {
    let mut window = Window::new("conversation", WIDE);
    window
        .app
        .model
        .apply(Update::Event(RuntimeEvent::MessageRefused(alice())));
    window.settle();
    assert!(
        window.shows_part("There is no more room for messages on this device"),
        "{:#?}",
        window.shown()
    );
    assert!(window.shows_part("To make room, delete"));
}

#[test]
fn a_passphrase_that_fails_marks_its_field() {
    // The wrong passphrase, under the field that holds it.
    let window = Window::new("unlock-error", WIDE);
    assert!(window.shows("That passphrase does not open this profile."));
    assert_eq!(window.error_outlines().len(), 1, "{:#?}", window.outlines);
    // Two that differ mark the second field.
    let mut window = Window::new("welcome", WIDE);
    assert!(window.error_outlines().is_empty());
    window.type_text("first words");
    window.press(Key::Tab);
    window.type_text("other words");
    window.press(Key::Enter);
    assert!(window.shows("The two passphrases differ."));
    let marked = window.error_outlines();
    assert_eq!(marked.len(), 1, "{:#?}", window.outlines);
    let (_, again) = window
        .texts
        .iter()
        .find(|(text, _)| text == "Repeat the passphrase")
        .unwrap();
    assert!(marked[0].top() > again.bottom(), "{marked:?} {again:?}");
}

#[test]
fn the_first_start_says_how_to_choose_a_passphrase_and_who_can_write() {
    let window = Window::new("welcome", WIDE);
    assert!(window.shows("Use a long phrase of several words."));
    assert!(window.shows_part("only people you accept can send you messages"));
    assert!(!window.shows_part("only the people you add can write to you"));
}

/// What invitations do while requests are set to Nobody.
const NOBODY_INVITATIONS: &str = "Requests are set to Nobody: no request arrives, not even \
                                  through an invitation, until you change who can send you \
                                  requests.";

/// What every text said of an invitation, true only while requests are
/// not set to Nobody.
const UNTIL_REVOKED: &str = "can send you a request until you revoke it";

#[test]
fn under_nobody_no_text_says_that_an_invitation_brings_a_request() {
    // The personal identity is set to Nobody; Work is not.
    let mut window = Window::new("settings-nobody", Vec2::new(1180.0, 2000.0));
    assert_eq!(
        window.count_part(NOBODY_INVITATIONS),
        1,
        "{:#?}",
        window.shown()
    );
    assert_eq!(window.count_part(UNTIL_REVOKED), 1);
    window.press(Key::Escape);
    let personal = UiIdentityId::from_bytes([1; 16]);
    window.app.model.open(Dialog::NewInvitation {
        identity: personal,
        label: String::new(),
        created: None,
        busy: None,
        error: None,
    });
    window.settle();
    assert!(
        window.shows_part(NOBODY_INVITATIONS),
        "{:#?}",
        window.shown()
    );
    assert!(!window.shows_part(UNTIL_REVOKED));
    window.app.model.confirm();
    window.answer(|command| match command {
        Command::CreateInvitation { .. } => Some(Outcome::InvitationCreated {
            invitation: UiInvitationId::new(personal, 3),
            card: Zeroizing::new("MONOLITH1:AAAA".to_owned()),
        }),
        _ => None,
    });
    assert!(window.shows("New invitation"));
    assert!(
        window.shows_part(NOBODY_INVITATIONS),
        "{:#?}",
        window.shown()
    );
    assert!(!window.shows_part(UNTIL_REVOKED));
}

#[test]
fn accepting_a_request_that_came_through_an_invitation_offers_to_revoke_it() {
    let mut window = Window::new("requests", WIDE);
    let _ = window.app.model.take_commands();
    window.click("Accept");
    window.click_last("Accept");
    let frank = UiContactId::new(UiIdentityId::from_bytes([1; 16]), [15; 16]);
    window.answer(|command| match command {
        Command::AcceptRequest { .. } => Some(Outcome::RequestAccepted(frank)),
        _ => None,
    });
    assert!(
        window.shows_part("\"Climbing gym\""),
        "{:#?}",
        window.shown()
    );
    window.click("Revoke");
    assert!(
        matches!(
            window.app.model.dialog(),
            Some(Dialog::RevokeInvitation { invitation, .. })
                if *invitation == UiInvitationId::new(UiIdentityId::from_bytes([1; 16]), 1)
        ),
        "{:?}",
        window.app.model.dialog()
    );
}

/// What the question before declining says.
const DECLINE: &str = "Nothing is sent to them. Their later requests are dropped without being \
                       shown, and Monolith does not show whom you declined, so this cannot be \
                       undone here. To talk with them later, add their card.";

#[test]
fn declining_a_request_asks_first_and_says_it_cannot_be_undone() {
    let mut window = Window::new("requests", WIDE);
    let _ = window.app.model.take_commands();
    window.click("Decline");
    assert!(
        window.shows("Decline this request?"),
        "{:#?}",
        window.shown()
    );
    assert!(window.shows(DECLINE));
    assert!(window.app.model.take_commands().is_empty());
    // Enter keeps the request: Cancel has the focus.
    window.press(Key::Enter);
    assert!(!window.shows("Decline this request?"));
    assert!(window.app.model.take_commands().is_empty());
    // Asked again and confirmed: the request is declined.
    window.click("Decline");
    window.click_last("Decline");
    let asked: Vec<Command> = window
        .app
        .model
        .take_commands()
        .into_iter()
        .map(|(_, command)| command)
        .collect();
    assert_eq!(
        asked,
        [Command::DeclineRequest(UiRequestId::new(
            UiIdentityId::from_bytes([1; 16]),
            1
        ))]
    );
}

#[test]
fn the_shortcuts_of_the_window_wait_while_declining_is_asked() {
    let mut window = Window::new("requests", WIDE);
    window.click("Decline");
    let selected = window.app.model.selection();
    window.frame(vec![Event::Key {
        key: Key::ArrowDown,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::ALT,
    }]);
    window.settle();
    assert_eq!(window.app.model.selection(), selected);
    assert!(window.shows("Decline this request?"));
}

#[test]
fn a_waiting_contact_is_said_to_wait_for_the_same_thing_everywhere() {
    for (name, said, places) in [
        // Nothing written yet: the header, the list and the details.
        ("waiting-new", "Request not sent yet", 3),
        // Written to: the header and the details; the list shows the
        // message.
        ("waiting", "Waiting for them to accept your request", 2),
        ("waiting-card", "Waiting for them to add your card", 2),
    ] {
        let mut window = Window::new(name, WIDE);
        window.app.model.open(Dialog::ContactDetails {
            contact: carol(),
            name: String::new(),
            busy: None,
            error: None,
        });
        window.settle();
        assert_eq!(window.count(said), places, "{name}: {:#?}", window.shown());
        assert!(!window.shows_part("Your first message sends your request"));
    }
    let window = Window::new("waiting-new", WIDE);
    assert!(window.shows_part("Your request goes out with your first message."));
}

#[test]
fn an_ended_conversation_says_once_that_it_ended() {
    let mut window = Window::new("conversation", WIDE);
    window.click("Former contact D4XM");
    assert_eq!(
        window.count_part("removed or blocked"),
        1,
        "{:#?}",
        window.shown()
    );
    assert!(window.shows("No longer a contact"));
}

#[test]
fn tor_is_said_the_same_in_its_details_and_in_the_settings() {
    let mut window = Window::new("tor-unavailable", WIDE);
    window.app.model.open(Dialog::TorDetails);
    window.settle();
    let said: Vec<String> = window
        .texts
        .iter()
        .filter(|(text, _)| text.starts_with("Tor is not running"))
        .map(|(text, _)| text.clone())
        .collect();
    assert_eq!(said.len(), 1, "{:#?}", window.shown());
    assert!(said[0].contains("Check again"), "{said:?}");
    window.click("Connection settings");
    assert!(window.shows("SOCKS endpoint"));
    assert!(window.shows(&said[0]), "{:#?}", window.shown());
}

#[test]
fn notices_end_with_a_period_and_empty_states_have_titles_without_one() {
    let mut window = Window::new("conversation", WIDE);
    window.app.model.open(Dialog::OwnCard {
        identity: UiIdentityId::from_bytes([1; 16]),
        card: None,
        busy: None,
        error: None,
    });
    window.answer(|command| match command {
        Command::OwnCard(_) => Some(Outcome::Card(Zeroizing::new("MONOLITH1:AAAA".to_owned()))),
        _ => None,
    });
    window.click("Copy card");
    assert!(window.shows("Card copied."), "{:#?}", window.shown());
    window.app.effect_done(EffectResult::CardSaved {
        path: "/home/user/Downloads/monolith-card.txt".to_owned(),
    });
    window.settle();
    assert!(window.shows("Saved to /home/user/Downloads/monolith-card.txt."));

    let mut window = Window::new("empty-identity", WIDE);
    window.app.model.show_requests();
    window.settle();
    assert!(window.shows("No requests waiting"), "{:#?}", window.shown());
}

#[test]
fn the_data_and_about_pages_say_what_is_kept_apart_and_nothing_twice() {
    let window = Window::new("settings-data", WIDE);
    assert!(window.shows_part("not encrypted"), "{:#?}", window.shown());
    assert!(window.shows_part("Unread counts are not kept"));
    let window = Window::new("about", WIDE);
    assert!(!window.shows_part("0.0.0"), "{:#?}", window.shown());
    assert_eq!(window.count_part("No telemetry"), 1);
}

#[test]
fn the_details_of_a_contact_say_what_is_saved_when() {
    let window = Window::new("contact-details", WIDE);
    assert!(window.shows("Changes to Verified are saved at once; the name when you press Save."));
    assert!(!window.shows("Saved at once"));
}

#[test]
fn an_identity_without_contacts_offers_its_card() {
    for size in [WIDE, NARROW] {
        let mut window = Window::new("empty-identity", size);
        window.click("Share my card");
        assert!(
            matches!(
                window.app.model.dialog(),
                Some(Dialog::OwnCard { identity, .. })
                    if *identity == UiIdentityId::from_bytes([2; 16])
            ),
            "{size:?}"
        );
    }
}

#[test]
fn a_draft_cleared_and_typed_again_while_a_message_is_queued_stays_whole() {
    let mut window = Window::new("conversation", WIDE);
    let _ = window.app.model.take_commands();
    window.type_text("ok");
    window.press(Key::Enter);
    let sent: Vec<_> = window
        .app
        .model
        .take_commands()
        .into_iter()
        .filter(|(_, command)| matches!(command, Command::Send { .. }))
        .collect();
    assert_eq!(sent.len(), 1);
    // Cleared while the runtime queues it, and something new typed that
    // starts the same way.
    window.press(Key::Backspace);
    window.press(Key::Backspace);
    assert_eq!(window.app.model.thread(alice()).unwrap().draft, "");
    window.type_text("ok, I will call");
    window.settle();
    window.app.model.apply(Update::Done {
        command: sent[0].0,
        outcome: Outcome::MessageQueued(UiMessageId::new(alice(), 20)),
    });
    window.settle();
    assert_eq!(
        window.app.model.thread(alice()).unwrap().draft,
        "ok, I will call"
    );
}

#[test]
fn deleting_the_messages_of_a_contact_that_has_not_accepted_says_its_request_waits_too() {
    let mut window = Window::new("waiting", WIDE);
    window.app.model.open(Dialog::DeleteConversation {
        contact: carol(),
        busy: None,
        error: None,
    });
    window.settle();
    assert!(
        window.shows_part(
            "Your request goes out only with a message, so it is not sent either until you write again."
        ),
        "{:#?}",
        window.shown()
    );
}

#[test]
fn a_full_fingerprint_is_laid_out_in_rows_of_seven_and_six_everywhere() {
    // People comparing a fingerprint read it on the request card, in the
    // contact dialog and under their own identity: the same rows on each.
    for (name, size) in [
        ("requests", WIDE),
        ("narrow-requests", SMALLEST),
        ("settings-identities", WIDE),
        ("settings-identities", SMALLEST),
        ("contact-details", WIDE),
        ("contact-details", SMALLEST),
        ("own-card-open", SMALLEST),
    ] {
        let window = Window::new(name, size);
        let found = window.fingerprints();
        assert!(!found.is_empty(), "{name} at {size:?} shows no fingerprint");
        for rows in found {
            assert_eq!(rows, [7, 6], "{name} at {size:?}");
        }
    }
}

#[test]
fn a_request_and_a_contact_without_a_name_have_a_person_for_an_avatar() {
    let person = icons::USER.char().to_string();
    // The name a sender chose is not who they are: no initials of it.
    let window = Window::new("requests", WIDE);
    assert!(window.shows_part("Calls themselves \"Frank\"."));
    assert!(!window.shows("F"), "{:#?}", window.shown());
    // Carol and Dana in the list, and the card.
    assert_eq!(window.count(&person), 3);

    // Carol has no name of her own.
    let mut window = Window::new("waiting", WIDE);
    let carol = UiContactId::new(UiIdentityId::from_bytes([1; 16]), [12; 16]);
    window.app.model.open(Dialog::ContactDetails {
        contact: carol,
        name: String::new(),
        busy: None,
        error: None,
    });
    window.settle();
    assert!(window.shows("Contact C7TQ"));
    assert!(!window.shows("CC"), "{:#?}", window.shown());
}

#[test]
fn every_ended_conversation_has_the_same_neutral_mark_in_the_list() {
    // Dana was removed or blocked; the list does not say which.
    let window = Window::new("conversation", WIDE);
    assert!(!window.shows(&icons::BAN.char().to_string()));
    assert_eq!(window.count(&icons::CIRCLE_DASHED.char().to_string()), 1);
}

#[test]
fn the_request_modes_stay_side_by_side_beside_the_pages() {
    // Wide enough for the pages beside the page, never narrower than the
    // page of a narrow window.
    for width in [880.0, 900.0, 960.0, 1040.0] {
        let window = Window::new("settings-identities", Vec2::new(width, 700.0));
        assert!(window.shows("Appearance"), "{width}");
        assert!(window.shows("Anyone with my card"), "{width}");
        assert!(window.shows("Nobody"), "{width}");
    }
}

#[test]
fn the_contact_dialog_offers_quiet_buttons_and_a_question_confirms_with_a_solid_one() {
    let danger = Theme::light().color.danger_fill;
    let window = Window::new("contact-details", WIDE);
    for label in ["Block", "Remove", "Delete messages"] {
        assert!(window.shows(label), "{label}");
    }
    assert!(!window.fills().contains(&danger));
    let window = Window::new("confirm-block", WIDE);
    let solid = window
        .fills()
        .into_iter()
        .filter(|fill| *fill == danger)
        .count();
    assert_eq!(solid, 1);
}

#[test]
fn messages_are_cut_clear_of_the_bar_above_them() {
    let theme = Theme::light();
    // The newest message is in view, and older ones are cut at the top:
    // as far under the bar as the list of conversations and the settings
    // are cut under what is above them.
    let window = Window::new("narrow-conversation", SMALLEST);
    let bar = theme.space.sm + theme.size.topbar;
    let clip = window
        .clip_of("Agreed. I will send a summary tonight.")
        .expect("the newest message is shown");
    assert!(
        clip.top() >= bar + theme.space.md,
        "messages are cut at {} under a bar that ends at {bar}",
        clip.top()
    );
}

fn add_contact(window: &mut Window) -> UiIdentityId {
    let identity = UiIdentityId::from_bytes([1; 16]);
    window.app.model.open(Dialog::AddContact {
        identity,
        card: String::new(),
        name: String::new(),
        busy: None,
        error: None,
        added: None,
    });
    window.settle();
    identity
}

#[test]
fn a_click_on_the_place_to_drop_a_card_file_goes_to_its_path() {
    let mut window = Window::new("conversation", WIDE);
    let identity = add_contact(&mut window);
    let path = TextField::new(("add-contact-path", identity), &mut String::new()).edit_id();
    assert_ne!(window.ctx.memory(|memory| memory.focused()), Some(path));
    window.click("Drop a card file here");
    assert_eq!(window.ctx.memory(|memory| memory.focused()), Some(path));
    // It opens nothing.
    assert!(window.app.take_effects().is_empty());
}

#[test]
fn the_card_typed_in_add_contact_is_kept_in_no_undo_history() {
    let mut window = Window::new("conversation", WIDE);
    let identity = add_contact(&mut window);
    let card = TextField::new(("add-contact-card", identity), &mut String::new()).edit_id();
    window.ctx.memory_mut(|memory| memory.request_focus(card));
    window.settle();
    let typed = |window: &Window| match window.app.model.dialog() {
        Some(Dialog::AddContact { card, .. }) => card.clone(),
        _ => panic!("Add contact is not open"),
    };
    // Two bursts, far enough apart for an undo history to tell them apart.
    for burst in ["monolith:card:1:FIRST", "SECOND"] {
        window.frame(vec![Event::Text(burst.to_owned())]);
        window.time += 3.0;
        window.settle();
    }
    assert_eq!(typed(&window), "monolith:card:1:FIRSTSECOND");
    window.frame(vec![Event::Key {
        key: Key::Z,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND,
    }]);
    window.settle();
    assert_eq!(typed(&window), "monolith:card:1:FIRSTSECOND");
}

/// The short fingerprints of the identities of the fixtures.
const PERSONAL: &str = "M0N0 L1TH 7QZK 4WPE 9RTD 2XBC";

const WORK: &str = "W0RK 4ZQP 8TNM 2KLE 6XRD 9VBC";

#[test]
fn an_identity_has_the_same_colour_in_the_list_and_in_the_settings() {
    let theme = Theme::light();
    let colour = |fingerprint: &str| theme.color.avatar[color_index(fingerprint.as_bytes())];
    // The full fingerprint would give Personal another colour.
    let full = format!("{PERSONAL} 5TQN 2WRK 8MZP 3XEL 6VDB 9HCA Q4KM");
    assert_ne!(colour(&full), colour(PERSONAL));
    // Identities are drawn as rounded squares, contacts as circles.
    let identities = |window: &Window| -> Vec<Color32> {
        window
            .fills()
            .into_iter()
            .filter(|fill| theme.color.avatar.contains(fill))
            .collect()
    };
    let list = Window::new("conversation", WIDE);
    assert_eq!(identities(&list), [colour(PERSONAL)]);
    let settings = Window::new("settings-identities", Vec2::new(1180.0, 2000.0));
    assert_eq!(identities(&settings), [colour(PERSONAL), colour(WORK)]);
}

#[test]
fn a_toast_covers_neither_the_notices_of_a_conversation_nor_the_title_and_actions_of_a_dialog() {
    for size in [WIDE, SMALLEST] {
        let window = Window::new("toast-banners", size);
        assert!(window.shows("Card copied"));
        let notices = window.filled(Theme::light().color.accent_subtle);
        assert!(!notices.is_empty(), "at {size:?} no notice is shown");
        for notice in notices {
            assert!(
                !window.toast().intersects(notice),
                "at {size:?} the toast {:?} covers the notice {notice:?}",
                window.toast()
            );
        }

        let window = Window::new("toast-dialog", size);
        assert!(window.shows("Card copied"));
        for part in ["Your card", "Copy card", "Save card", "Close"] {
            let covered = window.rect_of(part);
            assert!(
                !window.toast().intersects(covered),
                "at {size:?} the toast {:?} covers {part:?} at {covered:?}",
                window.toast()
            );
        }
    }
}

#[test]
fn the_picture_of_the_blocked_people_has_them_in_view() {
    // Below the fold of the first window unless scrolled to.
    let window = Window::new("settings-blocked", WIDE);
    assert!(window.shows(DANA), "{:#?}", window.shown());
    assert!(window.shows(GREG));
}

#[test]
fn the_first_screens_show_the_mark_beside_the_name() {
    let accent = Theme::light().color.accent;
    for name in ["welcome", "unlock", "opening"] {
        let window = Window::new(name, WIDE);
        assert!(window.shows("Monolith"), "{name}");
        assert!(window.fills().contains(&accent), "{name} has no mark");
    }
}

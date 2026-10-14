//! The window without a mouse: Ctrl+F reaches the filter, Alt+Down and
//! Alt+Up go through the conversations of the list and hand the focus to
//! the composer, and no shortcut acts while a dialog is open.

#![cfg(feature = "fixtures")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use egui::{Context, Event, Key, Modifiers, Pos2, RawInput, Rect};
use monolith_ui::App;
use monolith_ui::fixtures::fixture;
use monolith_ui_kit::search_field::SearchField;
use monolith_ui_kit::{Theme, fonts, theme};
use monolith_ui_model::{Dialog, Selection, UiContactId, UiIdentityId};

struct Window {
    ctx: Context,
    app: App,
    time: f64,
}

impl Window {
    fn new(name: &str) -> Self {
        let ctx = Context::default();
        fonts::install(&ctx, &[]);
        theme::install(&ctx, Theme::light().with_reduced_motion(true));
        let mut window = Self {
            ctx,
            app: fixture(name).unwrap(),
            time: 0.0,
        };
        for _ in 0..3 {
            window.frame(Vec::new());
        }
        window
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 0.1;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                Theme::light().size.window_default,
            )),
            time: Some(self.time),
            focused: true,
            events,
            ..RawInput::default()
        };
        let app = &mut self.app;
        let _ = self.ctx.run(input, |ctx| monolith_ui::show(ctx, app));
    }

    fn press(&mut self, key: Key, modifiers: Modifiers) {
        self.frame(vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]);
        self.frame(Vec::new());
    }

    fn shown(&self) -> Option<UiContactId> {
        match self.app.model.selection() {
            Some(Selection::Conversation(contact)) => Some(contact),
            _ => None,
        }
    }
}

fn contact(record: u8) -> UiContactId {
    UiContactId::new(UiIdentityId::from_bytes([1; 16]), [record; 16])
}

#[test]
fn alt_and_the_arrows_go_through_the_conversations_of_the_list() {
    let mut window = Window::new("conversation");
    let order: Vec<UiContactId> = window
        .app
        .model
        .sidebar()
        .iter()
        .map(|row| row.contact)
        .collect();
    assert_eq!(window.shown(), Some(contact(10)));
    let start = order.iter().position(|row| *row == contact(10)).unwrap();
    window.press(Key::ArrowDown, Modifiers::ALT);
    assert_eq!(window.shown(), Some(order[start + 1]));
    window.press(Key::ArrowUp, Modifiers::ALT);
    assert_eq!(window.shown(), Some(order[start]));
    // Past the ends nothing moves.
    for _ in 0..order.len() {
        window.press(Key::ArrowUp, Modifiers::ALT);
    }
    assert_eq!(window.shown(), Some(order[0]));
    // Without Alt the arrows are the composer's.
    window.press(Key::ArrowDown, Modifiers::NONE);
    assert_eq!(window.shown(), Some(order[0]));
}

#[test]
fn ctrl_f_gives_the_filter_the_focus() {
    let mut window = Window::new("conversation");
    let filter = SearchField::new("sidebar-search", &mut String::new()).edit_id();
    assert_ne!(window.ctx.memory(|memory| memory.focused()), Some(filter));
    window.press(Key::F, Modifiers::COMMAND);
    assert_eq!(window.ctx.memory(|memory| memory.focused()), Some(filter));
}

#[test]
fn no_shortcut_acts_under_a_dialog() {
    let mut window = Window::new("conversation");
    window.app.model.open(Dialog::NewIdentity {
        name: String::new(),
        busy: None,
        error: None,
    });
    window.frame(Vec::new());
    window.press(Key::ArrowDown, Modifiers::ALT);
    assert_eq!(window.shown(), Some(contact(10)));
    let filter = SearchField::new("sidebar-search", &mut String::new()).edit_id();
    window.press(Key::F, Modifiers::COMMAND);
    assert_ne!(window.ctx.memory(|memory| memory.focused()), Some(filter));
}

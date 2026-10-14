//! What the window shows, and where a click or a key takes it: the
//! settings of a narrow window, Escape out of the settings, a failed store
//! on every view, the state of Tor in a narrow window, the counts of the
//! identity switcher, empty states, the Tor settings once saved, the first
//! screens, and what a contact added by a card or by an invitation waits
//! for.

#![cfg(feature = "fixtures")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use egui::epaint::Shape;
use egui::{Context, Event, FullOutput, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};
use monolith_ui::fixtures::fixture;
use monolith_ui::{App, Effect};
use monolith_ui_kit::{Theme, fonts, icons, theme};
use monolith_ui_model::{Command, Dialog, Outcome, RuntimeEvent, Update};
use zeroize::Zeroizing;

/// The sizes of the reviews.
const WIDE: Vec2 = Vec2::new(1180.0, 760.0);
const NARROW: Vec2 = Vec2::new(820.0, 700.0);
const SMALLEST: Vec2 = Vec2::new(760.0, 520.0);

const STORAGE: &str = "Monolith can no longer save. Restart Monolith to continue.";

struct Window {
    ctx: Context,
    app: App,
    size: Vec2,
    time: f64,
    /// The texts of the last frame, where they were drawn.
    texts: Vec<(String, Rect)>,
}

impl Window {
    fn new(name: &str, size: Vec2) -> Self {
        Self::with(fixture(name).unwrap(), size)
    }

    fn with(app: App, size: Vec2) -> Self {
        let ctx = Context::default();
        fonts::install(&ctx, &[]);
        theme::install(&ctx, Theme::light().with_reduced_motion(true));
        let mut window = Self {
            ctx,
            app,
            size,
            time: 0.0,
            texts: Vec::new(),
        };
        window.settle();
        window
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

fn back() -> String {
    icons::CHEVRON_LEFT.char().to_string()
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
    let long = "a".repeat(1100);
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
                Command::CreateProfile { passphrase } => Some(passphrase.len()),
                _ => None,
            });
    assert_eq!(created, Some(1024));
}

/// What a contact added by a card, not an invitation, waits for.
const CARD_ADDED: &str = "This is their card, not an invitation. Contact C7TQ must add your \
                          card too before your messages can arrive.";
const WAITING_CARD: &str = "You added their card, not an invitation. Contact C7TQ must add \
                            your card too before your messages can arrive.";
const WAITING_INVITATION: &str = "Your request leaves with your first message. Contact C7TQ sees \
                                  it through the invitation they gave you, with your \
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

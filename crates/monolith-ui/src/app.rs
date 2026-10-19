//! The state of the window: the UI model, and what only the screens need
//! (notices on screen, the QR code shown, the passphrase being typed, the
//! filter of the sidebar, which settings page is open, and what a narrow
//! window shows).

use std::collections::BTreeMap;
use std::path::PathBuf;

use egui::Rect;
use monolith_ui_kit::qr::QrMatrix;
use monolith_ui_kit::request_card::RequestAction;
use monolith_ui_kit::toast::{Kind, Toasts};
use monolith_ui_model::present::{Calendar, UtcCalendar};
use monolith_ui_model::{
    Dialog, Failure, Model, NoticeKind, Phase, UiBlockedId, UiContactId, UiIdentityId,
    UiInvitationId, UiRequestId,
};
use zeroize::{Zeroize, Zeroizing};

use crate::common;

/// The room a buffer of a passphrase has from the start: the most a field
/// takes, `present::MAX_PASSPHRASE_CHARS` characters of at most four
/// bytes, so that typing or pasting never moves it and leaves no copy
/// behind.
pub(crate) const SECRET_CAPACITY: usize = 1024;

/// How the window looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeChoice {
    /// As the system says.
    System,
    /// Always light.
    Light,
    /// Always dark.
    Dark,
}

/// How Monolith reaches the system Tor. Plain text as the user typed it;
/// the desktop checks it before it is used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TorSettings {
    /// The SOCKS endpoint, such as `127.0.0.1:9050`.
    pub socks: String,
    /// The control endpoint, such as `127.0.0.1:9051` or
    /// `unix:/run/tor/control`.
    pub control: String,
    /// The cookie file of SAFECOOKIE authentication, by its full path. The
    /// desktop takes no settings without one: no setting turns
    /// authentication off.
    pub cookie_file: String,
}

/// Tor settings the user can pick with one click, named; given by the
/// desktop for its platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TorPreset {
    /// What the user picks: "System Tor", "Tor Browser".
    pub name: String,
    /// The settings.
    pub settings: TorSettings,
}

/// The preferences kept in the settings file. Nothing else is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preferences {
    /// The theme.
    pub theme: ThemeChoice,
    /// Reduced motion.
    pub reduced_motion: bool,
    /// The Tor endpoints.
    pub tor: TorSettings,
}

/// Something only the desktop can do: files and preferences. The screens
/// ask; the desktop does it and reports with [`EffectResult`].
#[derive(Clone, PartialEq, Eq)]
pub enum Effect {
    /// Save a card as a new file in the user's downloads folder (or home).
    SaveCard {
        /// The file name to use.
        name: String,
        /// The card; it can carry an invitation, so it is wiped when
        /// dropped.
        contents: Zeroizing<String>,
    },
    /// Read a card file the user named or dropped on the window, for the
    /// "Add contact" dialog that asked; its result is taken by that dialog
    /// only, if it is still open.
    ReadCard {
        /// The path.
        path: PathBuf,
        /// The identity of the dialog.
        identity: UiIdentityId,
        /// The dialog, as [`Model::dialog_instance`] numbers it.
        dialog: u64,
    },
    /// Keep and apply these preferences.
    Preferences(Preferences),
}

impl core::fmt::Debug for Effect {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::SaveCard { .. } => f.write_str("SaveCard"),
            Self::ReadCard { .. } => f.write_str("ReadCard"),
            Self::Preferences(_) => f.write_str("Preferences"),
        }
    }
}

/// What came of an effect.
#[derive(Clone, PartialEq, Eq)]
pub enum EffectResult {
    /// A card was saved, at this path (for display).
    CardSaved {
        /// Where.
        path: String,
    },
    /// A card file was read, for the dialog of [`Effect::ReadCard`].
    CardRead {
        /// The identity of the dialog that asked.
        identity: UiIdentityId,
        /// The dialog that asked.
        dialog: u64,
        /// Its text. It can carry an invitation, so it is wiped when
        /// dropped.
        text: Zeroizing<String>,
    },
    /// It failed.
    Failed(Failure),
}

impl core::fmt::Debug for EffectResult {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CardSaved { .. } => f.write_str("CardSaved"),
            Self::CardRead { .. } => f.write_str("CardRead"),
            Self::Failed(failure) => write!(f, "Failed({failure:?})"),
        }
    }
}

/// A page of the settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SettingsPage {
    /// Theme and motion.
    Appearance,
    /// The local identities.
    Identities,
    /// Tor.
    Tor,
    /// Where the data is, and what Monolith keeps.
    Data,
    /// What Monolith is, its version once released, and the licences.
    About,
}

/// What the window shows besides dialogs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// The conversations.
    Main,
    /// The settings, at a page.
    Settings(SettingsPage),
}

/// What closing the window now would cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CloseWarning {
    /// The session saves nothing: its identities, contacts and messages
    /// go with it.
    pub loses_everything: bool,
    /// Messages not delivered yet, over every identity. The runtime sends
    /// only while it runs: they wait until Monolith is open again, or, in
    /// a session that saves nothing, are never delivered.
    pub undelivered: usize,
}

/// What closing the window now would cost, `None` for nothing: only an
/// open profile has anything to lose or to send.
pub fn close_warning(model: &Model) -> Option<CloseWarning> {
    if model.phase() != Phase::Ready {
        return None;
    }
    let warning = CloseWarning {
        loses_everything: model.ephemeral() && !model.identities().is_empty(),
        undelivered: model.undelivered(),
    };
    (warning.loses_everything || warning.undelivered > 0).then_some(warning)
}

/// A QR code shown, with the text it encodes. Held only while the dialog
/// that shows it is open; never in egui's memory.
pub(crate) struct ShownQr {
    pub(crate) text: Zeroizing<String>,
    pub(crate) matrix: QrMatrix,
}

/// A card file asked for in "Add contact".
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CardFile {
    /// Its name, ready to draw.
    pub(crate) name: String,
    /// The card in the field is the one read from it.
    pub(crate) read: bool,
}

/// The state of the window.
pub struct App {
    /// The UI model.
    pub model: Model,
    /// What the screens show of time.
    pub(crate) calendar: Box<dyn Calendar>,
    /// The preferences as they are now.
    pub(crate) preferences: Preferences,
    /// The preferences being edited on the settings pages.
    pub(crate) draft_preferences: Preferences,
    /// Notices on screen.
    pub(crate) toasts: Toasts,
    /// What the window shows.
    pub(crate) view: View,
    /// In a narrow window, whether the conversation is shown instead of
    /// the list.
    pub(crate) narrow_conversation: bool,
    /// In a narrow window, whether the settings show a page instead of the
    /// list of pages.
    pub(crate) narrow_settings_page: bool,
    /// The Tor settings were saved on this visit of the Tor page.
    pub(crate) tor_saved: bool,
    /// The first screens show where the profile is kept.
    pub(crate) show_location: bool,
    /// The identity switcher opens its list at the next frame (for
    /// pictures of it).
    pub(crate) open_switcher: bool,
    /// The identities page scrolls its first list of blocked people into
    /// view at the next frame (for pictures of it).
    pub(crate) reveal_blocked: bool,
    /// The filter of the sidebar.
    pub(crate) search: String,
    /// The passphrase being typed.
    pub(crate) passphrase: Zeroizing<String>,
    /// The passphrase typed again, when creating a profile.
    pub(crate) passphrase_again: Zeroizing<String>,
    /// The QR code shown.
    pub(crate) qr: Option<ShownQr>,
    /// A path the user typed to read a card from.
    pub(crate) card_path: String,
    /// The card file last asked for in "Add contact", while it is open.
    pub(crate) card_file: Option<CardFile>,
    /// The request whose sender the user is asked to block, captured when
    /// the question opened.
    pub(crate) block_request: Option<UiRequestId>,
    /// The request the user is asked to decline, captured when the question
    /// opened.
    pub(crate) decline_request: Option<UiRequestId>,
    /// Someone the user is asked to unblock, captured when the question
    /// opened.
    pub(crate) unblock: Option<UiBlockedId>,
    /// The notices on screen that offer to revoke an invitation, by the
    /// number of their toast.
    revoke_offers: BTreeMap<u64, UiInvitationId>,
    /// The question asked before the window closes, while it is open, with
    /// what closing would cost.
    pub(crate) closing: Option<CloseWarning>,
    /// The window closes without asking: the person agreed.
    close_allowed: bool,
    /// The answer given to each request while it is under way, so that its
    /// card shows progress on the right button.
    pub(crate) request_answers: BTreeMap<UiRequestId, RequestAction>,
    /// The composer should take focus.
    pub(crate) focus_composer: bool,
    /// The filter of the sidebar should take focus.
    pub(crate) focus_search: bool,
    /// The conversation shown last, to keep its scroll position apart.
    pub(crate) last_conversation: Option<UiContactId>,
    /// The Tor settings offered on the Tor page, the default first.
    pub(crate) tor_presets: Vec<TorPreset>,
    /// The data directory named when Monolith was started was refused
    /// (`docs/PLATFORM_TAILS.md` section 5).
    pub(crate) data_dir_refused: bool,
    /// Where the view drawn in this frame wants its toasts: the part of it
    /// under its bar and its notices.
    pub(crate) toast_area: Option<Rect>,
    effects: Vec<Effect>,
}

impl core::fmt::Debug for App {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("App")
            .field("view", &self.view)
            .finish_non_exhaustive()
    }
}

impl App {
    /// A window with `model`, the time zone of `calendar`, and the
    /// preferences read at start.
    pub fn new(model: Model, calendar: Box<dyn Calendar>, preferences: Preferences) -> Self {
        Self {
            model,
            calendar,
            draft_preferences: preferences.clone(),
            preferences,
            toasts: Toasts::new().dismiss_label("Dismiss"),
            view: View::Main,
            narrow_conversation: false,
            narrow_settings_page: false,
            tor_saved: false,
            show_location: false,
            open_switcher: false,
            reveal_blocked: false,
            search: String::new(),
            passphrase: Zeroizing::new(String::with_capacity(SECRET_CAPACITY)),
            passphrase_again: Zeroizing::new(String::with_capacity(SECRET_CAPACITY)),
            qr: None,
            card_path: String::new(),
            card_file: None,
            block_request: None,
            decline_request: None,
            unblock: None,
            revoke_offers: BTreeMap::new(),
            closing: None,
            close_allowed: false,
            request_answers: BTreeMap::new(),
            focus_composer: false,
            focus_search: false,
            last_conversation: None,
            tor_presets: Vec::new(),
            data_dir_refused: false,
            toast_area: None,
            effects: Vec::new(),
        }
    }

    /// Says that the data directory named when Monolith was started was
    /// refused, so that the first screen says why there is no place for a
    /// profile.
    pub fn set_data_dir_refused(&mut self, refused: bool) {
        self.data_dir_refused = refused;
    }

    /// Sets the Tor settings offered on the Tor page, the default first.
    pub fn set_tor_presets(&mut self, presets: Vec<TorPreset>) {
        self.tor_presets = presets;
    }

    /// A window for tests and screenshots: a fixed time, default
    /// preferences.
    pub fn for_tests(model: Model, now: u64) -> Self {
        let system = TorSettings {
            socks: "127.0.0.1:9050".to_owned(),
            control: "unix:/run/tor/control".to_owned(),
            cookie_file: "/run/tor/control.authcookie".to_owned(),
        };
        let mut app = Self::new(
            model,
            Box::new(UtcCalendar { now }),
            Preferences {
                theme: ThemeChoice::Light,
                reduced_motion: true,
                tor: system.clone(),
            },
        );
        app.set_tor_presets(vec![
            TorPreset {
                name: "System Tor".to_owned(),
                settings: system,
            },
            TorPreset {
                name: "Tor Browser".to_owned(),
                settings: TorSettings {
                    socks: "127.0.0.1:9150".to_owned(),
                    control: "127.0.0.1:9151".to_owned(),
                    cookie_file: "/home/user/tor-browser/Browser/TorBrowser/Data/Tor/\
                                  control_auth_cookie"
                        .to_owned(),
                },
            },
        ]);
        app
    }

    /// The preferences now.
    pub fn preferences(&self) -> &Preferences {
        &self.preferences
    }

    /// Records preferences that the desktop applied.
    pub fn set_preferences(&mut self, preferences: Preferences) {
        self.draft_preferences = preferences.clone();
        self.preferences = preferences;
    }

    /// The time now, in seconds since the epoch, as the calendar says.
    pub fn now(&self) -> u64 {
        self.calendar.now()
    }

    /// The window was asked to close: by its close button, Alt+F4, the
    /// system, or the person's confirmation. True if it may close now:
    /// closing costs nothing, or the person agreed. Otherwise the question
    /// opens, or stays open, and the window stays.
    pub fn close_requested(&mut self) -> bool {
        if self.close_allowed {
            return true;
        }
        let Some(warning) = close_warning(&self.model) else {
            return true;
        };
        self.closing = Some(warning);
        false
    }

    /// Whether the question before closing is open.
    pub fn asking_to_close(&self) -> bool {
        self.closing.is_some()
    }

    /// Lets the window close without asking, as when the person agreed.
    pub fn allow_close(&mut self) {
        self.close_allowed = true;
        self.closing = None;
    }

    pub(crate) fn effect(&mut self, effect: Effect) {
        self.effects.push(effect);
    }

    /// Takes what the desktop is asked to do.
    pub fn take_effects(&mut self) -> Vec<Effect> {
        core::mem::take(&mut self.effects)
    }

    /// Takes what an effect came to.
    pub fn effect_done(&mut self, result: EffectResult) {
        match result {
            EffectResult::CardSaved { path } => {
                self.toasts.push(Kind::Success, format!("Saved to {path}."));
            }
            EffectResult::CardRead {
                identity,
                dialog,
                text,
            } => {
                // Only the dialog that asked takes it: not one opened since,
                // of the same identity or another.
                if self.model.dialog_instance() != Some(dialog) {
                    return;
                }
                // Not once the contact is added: the card has gone.
                if let Some(Dialog::AddContact {
                    identity: owner,
                    card,
                    error,
                    added: None,
                    ..
                }) = self.model.dialog_mut()
                {
                    if *owner == identity {
                        card.zeroize();
                        card.push_str(&text);
                        *error = None;
                    }
                }
            }
            EffectResult::Failed(failure) => {
                if failure.said_in_place() {
                    self.toasts.push(Kind::Error, failure.message());
                }
            }
        }
    }

    /// Moves the notices of the model onto the screen.
    pub(crate) fn pump_notices(&mut self) {
        // An offer goes with its toast: none is left once no toast is. Nor
        // once another identity is shown: its invitation is not one of the
        // identity shown (ADR 0011, P7-23).
        if self.toasts.is_empty() {
            self.revoke_offers.clear();
        }
        let shown = self.model.current();
        self.revoke_offers
            .retain(|_, invitation| Some(invitation.identity()) == shown);
        for notice in self.model.take_notices() {
            let kind = match notice.kind {
                NoticeKind::Success => Kind::Success,
                NoticeKind::Info => Kind::Info,
                NoticeKind::Error => Kind::Error,
            };
            match notice.revoke {
                Some(invitation) => {
                    let toast =
                        self.toasts
                            .push_with_action(kind, common::line(&notice.text), "Revoke");
                    self.revoke_offers.insert(toast, invitation);
                }
                None => {
                    self.toasts.push(kind, common::line(&notice.text));
                }
            }
        }
    }

    /// The action of the toast `toast` was chosen: a Revoke asks whether
    /// to revoke its invitation, as revoking from the settings does.
    pub(crate) fn toast_action(&mut self, toast: u64) {
        if let Some(invitation) = self.revoke_offers.remove(&toast) {
            self.model.open(Dialog::RevokeInvitation {
                invitation,
                discard_requests: false,
                busy: None,
                error: None,
            });
        }
    }

    /// Drops the QR code shown, and with it the card it encodes.
    pub(crate) fn drop_qr(&mut self) {
        self.qr = None;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use monolith_ui_kit::theme::{self, Theme};
    use monolith_ui_model::{
        Command, ContactView, ConversationView, IdentitySnapshot, IdentityView, Outcome,
        ProfileOnDisk, RequestMode, RuntimeEvent, Service, UiMessageId, Update,
    };

    use super::*;

    fn identity(n: u8) -> UiIdentityId {
        UiIdentityId::from_bytes([n; 16])
    }

    fn add_contact(identity: UiIdentityId) -> Dialog {
        Dialog::AddContact {
            identity,
            card: String::new(),
            name: String::new(),
            busy: None,
            error: Some(Failure::InvalidCard),
            added: None,
        }
    }

    fn read(identity: UiIdentityId, dialog: u64, text: &str) -> EffectResult {
        EffectResult::CardRead {
            identity,
            dialog,
            text: Zeroizing::new(text.to_owned()),
        }
    }

    /// The card and the error of the open "Add contact" dialog.
    fn shown(app: &App) -> (String, Option<Failure>) {
        match app.model.dialog() {
            Some(Dialog::AddContact { card, error, .. }) => (card.clone(), *error),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_dropped_card_file_is_read_for_the_dialog_it_was_dropped_on() {
        let ctx = egui::Context::default();
        monolith_ui_kit::fonts::install(&ctx, &[]);
        theme::install(&ctx, Theme::light().with_reduced_motion(true));
        let mut app = App::for_tests(Model::new(), 0);
        app.model.open(add_contact(identity(1)));
        let input = egui::RawInput {
            dropped_files: vec![egui::DroppedFile {
                path: Some(PathBuf::from("card.txt")),
                ..egui::DroppedFile::default()
            }],
            ..egui::RawInput::default()
        };
        let _ = ctx.run(input, |ctx| crate::show(ctx, &mut app));
        let effects = app.take_effects();
        assert_eq!(
            effects,
            [Effect::ReadCard {
                path: PathBuf::from("card.txt"),
                identity: identity(1),
                dialog: app.model.dialog_instance().unwrap(),
            }]
        );
    }

    #[test]
    fn a_card_file_read_is_taken_only_by_the_dialog_that_asked() {
        let mut app = App::for_tests(Model::new(), 0);
        app.model.open(add_contact(identity(1)));
        let asked = app.model.dialog_instance().unwrap();
        // The dialog of A is cancelled and one of B opened before the file
        // is read: B's field stays as it is.
        app.model.close();
        app.model.open(add_contact(identity(2)));
        app.effect_done(read(identity(1), asked, "MONOLITH card of A"));
        assert_eq!(shown(&app), (String::new(), Some(Failure::InvalidCard)));
        // A's dialog opened again is not the one that asked either.
        app.model.close();
        app.model.open(add_contact(identity(1)));
        app.effect_done(read(identity(1), asked, "MONOLITH card of A"));
        assert_eq!(shown(&app), (String::new(), Some(Failure::InvalidCard)));
        // The dialog that asked takes it, for its own identity only.
        let now = app.model.dialog_instance().unwrap();
        app.effect_done(read(identity(2), now, "MONOLITH card of B"));
        assert_eq!(shown(&app), (String::new(), Some(Failure::InvalidCard)));
        app.effect_done(read(identity(1), now, "MONOLITH card"));
        assert_eq!(shown(&app), ("MONOLITH card".to_owned(), None));
    }

    #[test]
    fn a_card_file_read_after_the_contact_was_added_is_dropped() {
        let mut app = App::for_tests(Model::new(), 0);
        let mut dialog = add_contact(identity(1));
        if let Dialog::AddContact { added, .. } = &mut dialog {
            *added = Some(UiContactId::new(identity(1), [2; 16]));
        }
        app.model.open(dialog);
        let now = app.model.dialog_instance().unwrap();
        app.effect_done(read(identity(1), now, "MONOLITH card"));
        assert_eq!(shown(&app), (String::new(), Some(Failure::InvalidCard)));
    }

    /// What the first identity of [`open_profile`] holds: a contact (record
    /// 10) whose conversation has `queued` messages waiting, and an ended
    /// conversation (record 11) with `ended` messages it will never send.
    fn first_identity(queued: usize, ended: usize) -> IdentitySnapshot {
        let conversation = |record: u8, active: bool, queued: usize| ConversationView {
            contact: UiContactId::new(identity(1), [record; 16]),
            fingerprint: String::new(),
            active,
            messages: queued,
            queued,
            latest: None,
        };
        IdentitySnapshot {
            identity: identity(1),
            contacts: vec![ContactView {
                id: UiContactId::new(identity(1), [10; 16]),
                name: None,
                fingerprint: String::new(),
                full_fingerprint: String::new(),
                accepted: true,
                verified: false,
                invitation: false,
            }],
            conversations: vec![
                conversation(10, true, queued),
                conversation(11, false, ended),
            ],
            requests: Vec::new(),
            invitations: Vec::new(),
            blocked: Vec::new(),
            service: Service::Available,
            request_mode: RequestMode::Invitation,
        }
    }

    /// A model with a profile open, kept on disk or not, with `identities`
    /// identities; the first has a conversation with `queued` messages
    /// waiting and an ended one with `ended` messages it will never send.
    fn open_profile(ephemeral: bool, identities: u8, queued: usize, ended: usize) -> Model {
        let mut model = Model::new();
        model.apply(Update::Started {
            profile: ProfileOnDisk::Absent,
            location: String::new(),
        });
        model.start_without_saving();
        for _ in 0..3 {
            for (id, command) in model.take_commands() {
                let outcome = match command {
                    Command::StartWithoutSaving => Outcome::ProfileOpen {
                        ephemeral,
                        message_store_missing: false,
                    },
                    Command::LoadIdentities => Outcome::Identities(
                        (1..=identities)
                            .map(|n| IdentityView {
                                id: identity(n),
                                name: None,
                                fingerprint: String::new(),
                                full_fingerprint: String::new(),
                            })
                            .collect(),
                    ),
                    Command::LoadIdentity(asked) if asked == identity(1) => {
                        Outcome::Identity(Box::new(first_identity(queued, ended)))
                    }
                    Command::LoadIdentity(asked) => Outcome::Identity(Box::new(IdentitySnapshot {
                        identity: asked,
                        contacts: Vec::new(),
                        conversations: Vec::new(),
                        requests: Vec::new(),
                        invitations: Vec::new(),
                        blocked: Vec::new(),
                        service: Service::Available,
                        request_mode: RequestMode::Invitation,
                    })),
                    _ => continue,
                };
                model.apply(Update::Done {
                    command: id,
                    outcome,
                });
            }
        }
        assert_eq!(model.phase(), Phase::Ready);
        model
    }

    #[test]
    fn closing_counts_a_message_being_queued_and_one_queued_before_it_is_read_again() {
        let mut model = open_profile(false, 1, 0, 0);
        assert_eq!(close_warning(&model), None);
        let contact = UiContactId::new(identity(1), [10; 16]);
        model.select_conversation(contact);
        for (id, command) in model.take_commands() {
            if matches!(command, Command::LoadMessages { .. }) {
                model.apply(Update::Done {
                    command: id,
                    outcome: Outcome::Messages {
                        contact,
                        messages: Vec::new(),
                        complete: true,
                    },
                });
            }
        }
        model
            .draft_mut(contact)
            .unwrap()
            .push_str("See you at eight.");
        model.send(contact);
        let commands = model.take_commands();
        let one = Some(CloseWarning {
            loses_everything: false,
            undelivered: 1,
        });
        // While the runtime queues it.
        assert_eq!(close_warning(&model), one);
        // Queued, before the identity is read again.
        let message = UiMessageId::new(contact, 1);
        model.apply(Update::Event(RuntimeEvent::MessageQueued(message)));
        for (id, command) in commands {
            if matches!(command, Command::Send { .. }) {
                model.apply(Update::Done {
                    command: id,
                    outcome: Outcome::MessageQueued(message),
                });
            }
        }
        assert_eq!(close_warning(&model), one);
        // Read again: it is counted once, as the runtime says.
        for (id, command) in model.take_commands() {
            if command == Command::LoadIdentity(identity(1)) {
                model.apply(Update::Done {
                    command: id,
                    outcome: Outcome::Identity(Box::new(first_identity(1, 0))),
                });
            }
        }
        assert_eq!(close_warning(&model), one);
    }

    #[test]
    fn closing_costs_nothing_without_an_open_profile_or_anything_to_lose() {
        assert_eq!(close_warning(&Model::new()), None);
        // Kept on disk, with nothing waiting but what will never be sent.
        assert_eq!(close_warning(&open_profile(false, 1, 0, 4)), None);
        // Saving nothing, but nothing made yet.
        assert_eq!(close_warning(&open_profile(true, 0, 0, 0)), None);
    }

    #[test]
    fn closing_warns_of_a_session_that_saves_nothing_and_of_messages_not_delivered() {
        assert_eq!(
            close_warning(&open_profile(true, 2, 0, 0)),
            Some(CloseWarning {
                loses_everything: true,
                undelivered: 0
            })
        );
        assert_eq!(
            close_warning(&open_profile(false, 1, 2, 3)),
            Some(CloseWarning {
                loses_everything: false,
                undelivered: 2
            })
        );
        assert_eq!(
            close_warning(&open_profile(true, 1, 1, 0)),
            Some(CloseWarning {
                loses_everything: true,
                undelivered: 1
            })
        );
    }

    #[test]
    fn the_window_asks_before_it_closes_until_the_person_agrees() {
        let mut app = App::for_tests(open_profile(false, 1, 2, 0), 0);
        assert!(!app.asking_to_close());
        // Asked again while the question is open: still not without an
        // answer.
        for _ in 0..2 {
            assert!(!app.close_requested());
            assert!(app.asking_to_close());
        }
        app.allow_close();
        assert!(!app.asking_to_close());
        assert!(app.close_requested());
        assert!(!app.asking_to_close());
        // Nothing to lose: it closes at once, without a question.
        let mut app = App::for_tests(open_profile(false, 1, 0, 0), 0);
        assert!(app.close_requested());
        assert!(!app.asking_to_close());
    }
}

//! The state of the window: the UI model, and what only the screens need
//! (notices on screen, the QR code shown, the passphrase being typed, the
//! filter of the sidebar, which settings page is open).

use std::path::PathBuf;

use monolith_ui_kit::qr::QrMatrix;
use monolith_ui_kit::toast::{Kind, Toasts};
use monolith_ui_model::present::{Calendar, UtcCalendar};
use monolith_ui_model::{Failure, Model, NoticeKind, UiContactId};
use zeroize::Zeroizing;

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
    /// The cookie file of SAFECOOKIE authentication; empty for a trusted
    /// filter.
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
    /// open "Add contact" dialog.
    ReadCard {
        /// The path.
        path: PathBuf,
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
    /// A card file was read.
    CardRead {
        /// Its text.
        text: String,
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
    /// Version and licences.
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

/// A QR code shown, with the text it encodes. Held only while the dialog
/// that shows it is open; never in egui's memory.
pub(crate) struct ShownQr {
    pub(crate) text: Zeroizing<String>,
    pub(crate) matrix: QrMatrix,
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
    /// The composer should take focus.
    pub(crate) focus_composer: bool,
    /// The conversation shown last, to keep its scroll position apart.
    pub(crate) last_conversation: Option<UiContactId>,
    /// The Tor settings offered on the Tor page, the default first.
    pub(crate) tor_presets: Vec<TorPreset>,
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
            search: String::new(),
            passphrase: Zeroizing::new(String::with_capacity(256)),
            passphrase_again: Zeroizing::new(String::with_capacity(256)),
            qr: None,
            card_path: String::new(),
            focus_composer: false,
            last_conversation: None,
            tor_presets: Vec::new(),
            effects: Vec::new(),
        }
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
                    cookie_file: "/home/user/tor-browser/Browser/TorBrowser/Data/Tor/                                  control_auth_cookie"
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
                self.toasts.push(Kind::Success, format!("Saved to {path}"));
            }
            EffectResult::CardRead { text } => {
                if let Some(monolith_ui_model::Dialog::AddContact { card, error, .. }) =
                    self.model.dialog_mut()
                {
                    *card = text;
                    *error = None;
                }
            }
            EffectResult::Failed(failure) => {
                self.toasts.push(Kind::Error, failure.message());
            }
        }
    }

    /// Moves the notices of the model onto the screen.
    pub(crate) fn pump_notices(&mut self) {
        for notice in self.model.take_notices() {
            let kind = match notice.kind {
                NoticeKind::Success => Kind::Success,
                NoticeKind::Info => Kind::Info,
                NoticeKind::Error => Kind::Error,
            };
            self.toasts.push(kind, notice.text);
        }
    }

    /// Drops the QR code shown, and with it the card it encodes.
    pub(crate) fn drop_qr(&mut self) {
        self.qr = None;
    }
}

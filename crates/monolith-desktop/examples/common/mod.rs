//! What the examples of the desktop client share: a client driven by
//! script through its window code. Each client has a profile on disk, the
//! real bridge and runtime, and a thread for its files; the script does
//! what the screens do when the user acts (the actions of the UI model) and
//! waits for what the runtime says, in real time; nothing is answered in
//! its place. Every wait is bounded by the patience of the client, and one
//! that ends without its result fails with what it waited for.

// Each example uses a part of this.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use core::time::Duration;

use egui::{Context, DroppedFile, Pos2, RawInput, Rect};
use monolith_desktop::bridge::{Bridge, Setup};
use monolith_desktop::calendar::SystemCalendar;
use monolith_desktop::files::{Disk, Job};
use monolith_runtime::TorBackend;
use monolith_ui::{App, Effect, EffectResult, Preferences, ThemeChoice, TorSettings};
use monolith_ui_kit::{Theme, fonts, theme};
use monolith_ui_model::{
    Dialog, Failure, Model, Phase, Selection, Status, TorCheck, UiContactId, UiIdentityId,
    UiRequestId,
};
use monolith_ui_snapshot::{Options, render};
use zeroize::Zeroizing;

pub const PASSPHRASE: &str = "correct horse battery staple";

/// The Tor settings the window shows on the in-memory Tor. Nothing is
/// reached through them.
pub fn in_memory_tor() -> TorSettings {
    TorSettings {
        socks: "127.0.0.1:9050".to_owned(),
        control: "127.0.0.1:9051".to_owned(),
        cookie_file: String::new(),
    }
}

/// One client: its window state, its bridge, its file thread, and a
/// context to draw it.
pub struct Client {
    pub name: &'static str,
    pub app: App,
    bridge: Option<Bridge>,
    disk: Disk,
    ctx: Context,
    started: Instant,
    /// How long a wait lasts at most.
    patience: Duration,
    /// Where "Save card" saves.
    saves: Option<PathBuf>,
    /// The cards saved, in order.
    saved: Vec<PathBuf>,
    /// What the file thread failed to do, in order.
    file_failures: Vec<Failure>,
    /// Where pictures go, and how many were taken.
    shots: Option<PathBuf>,
    shot: u32,
}

impl Client {
    /// Starts a client on the data directory `data`, with the Tor backends
    /// `backend` makes and the Tor settings `tor` in its preferences.
    pub fn start<B: TorBackend + 'static>(
        name: &'static str,
        data: &Path,
        backend: impl FnMut() -> B + Send + 'static,
        tor: TorSettings,
        patience: Duration,
    ) -> Self {
        let bridge = Bridge::start(
            Setup {
                data_dir: Some(data.to_owned()),
                backend: Box::new(backend),
                shutdown_deadline: Duration::from_secs(3),
                close_wait: Duration::from_secs(10),
            },
            Arc::new(|| {}),
        )
        .expect("bridge");
        let disk = Disk::start(Arc::new(|| {})).expect("file thread");
        let preferences = Preferences {
            theme: ThemeChoice::Light,
            reduced_motion: true,
            tor,
        };
        let ctx = Context::default();
        fonts::install(&ctx, &[]);
        theme::install(&ctx, Theme::light().with_reduced_motion(true));
        Self {
            name,
            app: App::new(Model::new(), Box::new(SystemCalendar::new()), preferences),
            bridge: Some(bridge),
            disk,
            ctx,
            started: Instant::now(),
            patience,
            saves: None,
            saved: Vec::new(),
            file_failures: Vec::new(),
            shots: None,
            shot: 0,
        }
    }

    /// Writes the pictures of [`Client::picture`] to `dir`.
    pub fn with_pictures(mut self, dir: &Path) -> Self {
        self.shots = Some(dir.to_owned());
        self
    }

    /// Saves the cards of "Save card" in `dir`, where the window saves
    /// them in the downloads folder.
    pub fn saving_to(mut self, dir: &Path) -> Self {
        self.saves = Some(dir.to_owned());
        self
    }

    pub fn model(&mut self) -> &mut Model {
        &mut self.app.model
    }

    /// One frame: what the bridge and the file thread said goes to the
    /// window, the window is drawn, and what it asks goes to the bridge and
    /// the file thread, as `Window::update` does.
    pub fn frame(&mut self) {
        self.frame_with(Vec::new());
    }

    /// One frame with `path` dropped on the window.
    pub fn drop_file(&mut self, path: &Path) {
        self.frame_with(vec![DroppedFile {
            path: Some(path.to_owned()),
            ..DroppedFile::default()
        }]);
    }

    fn frame_with(&mut self, dropped_files: Vec<DroppedFile>) {
        let bridge = self.bridge.as_mut().expect("running");
        while let Some(update) = bridge.try_update() {
            self.app.model.apply(update);
        }
        while let Some(result) = self.disk.try_result() {
            match &result {
                EffectResult::CardSaved { path } => self.saved.push(PathBuf::from(path)),
                EffectResult::Failed(failure) => self.file_failures.push(*failure),
                EffectResult::CardRead { .. } => {}
            }
            self.app.effect_done(result);
        }
        let size = Theme::light().size.window_default;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            time: Some(self.started.elapsed().as_secs_f64()),
            focused: true,
            dropped_files,
            ..RawInput::default()
        };
        let app = &mut self.app;
        let _ = self.ctx.run(input, |ctx| monolith_ui::show(ctx, app));
        for (command, what) in self.app.model.take_commands() {
            bridge.send(command, what);
        }
        bridge.flush();
        for effect in self.app.take_effects() {
            self.effect(effect);
        }
    }

    /// Does what the screens asked of the desktop, as `Window::effect`
    /// does, with the folder of [`Client::saving_to`] for saved cards. The
    /// preferences are not kept.
    pub fn effect(&mut self, effect: Effect) {
        let job = match effect {
            Effect::SaveCard { name, contents } => Job::SaveCard {
                dir: self.saves.clone(),
                name,
                contents,
            },
            Effect::ReadCard {
                path,
                identity,
                dialog,
            } => Job::ReadCard {
                path,
                identity,
                dialog,
            },
            Effect::Preferences(_) => return,
        };
        if let Some(failed) = self.disk.ask(job) {
            if let EffectResult::Failed(failure) = &failed {
                self.file_failures.push(*failure);
            }
            self.app.effect_done(failed);
        }
    }

    /// Runs frames until `ready` gives a value, says so, and returns it.
    pub fn until_some<T>(&mut self, what: &str, ready: impl FnMut(&mut Self) -> Option<T>) -> T {
        let waited = Instant::now();
        let value = self.quietly_until(what, ready);
        println!(
            "  {}: {what} ({:.1} s)",
            self.name,
            waited.elapsed().as_secs_f64()
        );
        value
    }

    /// [`Client::until_some`], saying nothing unless the wait fails.
    fn quietly_until<T>(&mut self, what: &str, mut ready: impl FnMut(&mut Self) -> Option<T>) -> T {
        let deadline = Instant::now() + self.patience;
        loop {
            self.frame();
            if let Some(value) = ready(self) {
                return value;
            }
            assert!(
                Instant::now() < deadline,
                "{}: {what} did not happen in {:?}",
                self.name,
                self.patience
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Runs frames until `done` holds.
    pub fn until(&mut self, what: &str, done: impl Fn(&Model) -> bool) {
        self.until_some(what, |client| done(&client.app.model).then_some(()));
    }

    /// Runs frames for `time`.
    pub fn pause(&mut self, time: Duration) {
        let end = Instant::now() + time;
        while Instant::now() < end {
            self.frame();
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Writes a picture of the window. It is drawn in a context of its own,
    /// which the software renderer sees from its first frame, textures
    /// included; the state shown is the client's.
    pub fn picture(&mut self, step: &str) {
        self.frame();
        self.shot += 1;
        let ctx = Context::default();
        fonts::install(&ctx, &[]);
        theme::install(&ctx, Theme::light().with_reduced_motion(true));
        let options = Options {
            size: Theme::light().size.window_default,
            // The clock of the client, which the notices' timers follow.
            start_time: self.started.elapsed().as_secs_f64(),
            frames: 4,
            frame_time: 0.05,
            background: Theme::light().color.bg_app,
            ..Options::default()
        };
        let app = &mut self.app;
        let image = render(&ctx, &options, |ctx| monolith_ui::show(ctx, app));
        let path = self
            .shots
            .as_ref()
            .expect("a folder for pictures")
            .join(format!("{}-{:02}-{step}.png", self.name, self.shot));
        image.save_png(&path).expect("write the picture");
    }

    /// Closes the window: the runtime stops, as when the user closes it.
    pub fn close(&mut self) {
        self.app.model.shut_down();
        self.until("the runtime stopped", |model| {
            model.phase() == Phase::Stopped
        });
        drop(self.bridge.take());
    }

    pub fn current(&self) -> UiIdentityId {
        self.app.model.current().expect("an identity is shown")
    }

    /// Makes a profile, or opens the one there.
    pub fn open_profile(&mut self) {
        self.until("started", |model| {
            matches!(model.phase(), Phase::Welcome | Phase::Locked)
        });
        let passphrase = Zeroizing::new(PASSPHRASE.to_owned());
        if self.model().phase() == Phase::Welcome {
            self.model().create_profile(passphrase);
        } else {
            self.model().unlock(passphrase);
        }
        self.until("the profile is open", |model| {
            model.phase() == Phase::Ready && model.identities_loaded()
        });
    }

    /// Checks Tor, as the Tor details dialog does, every two seconds until
    /// the check says Tor works, for `within` at most, and returns the
    /// version it names. The check proves the SOCKS endpoint as a dial
    /// does. Each other answer is printed when it first comes.
    pub fn tor_works(&mut self, within: Duration) -> String {
        let checked = |client: &mut Self| (!client.app.model.tor_checking()).then_some(());
        // A check under way was asked before this one.
        self.quietly_until("a check of Tor under way ends", checked);
        let deadline = Instant::now() + within;
        let mut said = None;
        loop {
            self.model().check_tor();
            self.frame();
            self.quietly_until("Tor is checked", checked);
            let check = self.app.model.tor_check().cloned();
            if let Some(TorCheck::Ready { version, .. }) = check {
                return version;
            }
            if check != said {
                println!("  {}: the Tor check says {check:?}", self.name);
            }
            assert!(
                Instant::now() < deadline,
                "{}: Tor did not work in {within:?}; the last check said {check:?}",
                self.name
            );
            said = check;
            self.pause(Duration::from_secs(2));
        }
    }

    pub fn create_identity(&mut self, name: &str) -> UiIdentityId {
        let before = self.app.model.identities().len();
        self.model().open(Dialog::NewIdentity {
            name: name.to_owned(),
            busy: None,
            error: None,
        });
        self.model().confirm();
        let wanted = name.to_owned();
        self.until(
            &format!("identity {name} is made and shown"),
            move |model| {
                model.identities().len() == before + 1
                    && model.dialog().is_none()
                    && model
                        .current_identity()
                        .is_some_and(|view| view.name.as_deref() == Some(wanted.as_str()))
            },
        );
        self.current()
    }

    /// Makes an invitation and returns its card; its dialog stays open.
    pub fn create_invitation(&mut self, label: &str) -> String {
        let identity = self.current();
        self.model().open(Dialog::NewInvitation {
            identity,
            label: label.to_owned(),
            created: None,
            busy: None,
            error: None,
        });
        self.model().confirm();
        self.until("the invitation is made", |model| {
            matches!(
                model.dialog(),
                Some(Dialog::NewInvitation {
                    created: Some(_),
                    ..
                })
            )
        });
        let Some(Dialog::NewInvitation {
            created: Some((_, card)),
            ..
        }) = self.app.model.dialog()
        else {
            unreachable!()
        };
        card.to_string()
    }

    /// Saves `card` as the "Save card" button of an invitation's dialog
    /// asks, and returns the file it was saved to.
    pub fn save_invitation(&mut self, card: &str) -> PathBuf {
        let before = self.saved.len();
        self.effect(Effect::SaveCard {
            name: "monolith-invitation.txt".to_owned(),
            contents: Zeroizing::new(card.to_owned()),
        });
        self.until_some("the card is saved", |client| {
            client.no_file_failure();
            client.saved.get(before).cloned()
        })
    }

    fn no_file_failure(&self) {
        assert!(
            self.file_failures.is_empty(),
            "{}: the file thread failed: {:?}",
            self.name,
            self.file_failures
        );
    }

    /// Adds the contact of `card` as pasted in "Add contact".
    pub fn add_contact(&mut self, card: &str, name: &str) -> UiContactId {
        let identity = self.current();
        self.model().open(Dialog::AddContact {
            identity,
            card: card.to_owned(),
            name: name.to_owned(),
            busy: None,
            error: None,
            added: None,
        });
        self.confirm_contact()
    }

    /// Adds the contact of the card file `path`, dropped on "Add contact".
    pub fn add_contact_from_file(&mut self, path: &Path, name: &str) -> UiContactId {
        let identity = self.current();
        self.model().open(Dialog::AddContact {
            identity,
            card: String::new(),
            name: name.to_owned(),
            busy: None,
            error: None,
            added: None,
        });
        self.drop_file(path);
        self.until_some("the card file is read into the dialog", |client| {
            client.no_file_failure();
            matches!(
                client.app.model.dialog(),
                Some(Dialog::AddContact { card, .. }) if !card.is_empty()
            )
            .then_some(())
        });
        self.confirm_contact()
    }

    fn confirm_contact(&mut self) -> UiContactId {
        self.model().confirm();
        self.until("the contact is added and shown", |model| {
            model.dialog().is_none()
                && matches!(model.selection(), Some(Selection::Conversation(_)))
        });
        match self.app.model.selection() {
            Some(Selection::Conversation(contact)) => contact,
            _ => unreachable!(),
        }
    }

    /// Waits for a request to the identity shown, shows the requests, and
    /// returns the first.
    pub fn wait_request(&mut self) -> UiRequestId {
        let request = self.until_some("a request arrives", |client| {
            client
                .app
                .model
                .current_slice()
                .and_then(|slice| slice.requests.first())
                .map(|request| request.id)
        });
        self.model().show_requests();
        request
    }

    /// Accepts `request`, naming the contact `name`, and returns the
    /// contact, whose conversation is then shown.
    pub fn accept_request(&mut self, request: UiRequestId, name: &str) -> UiContactId {
        self.model().open(Dialog::AcceptRequest {
            request,
            name: name.to_owned(),
            busy: None,
            error: None,
        });
        self.model().confirm();
        self.until("the request is accepted and shown", |model| {
            model.dialog().is_none()
                && matches!(model.selection(), Some(Selection::Conversation(_)))
        });
        match self.app.model.selection() {
            Some(Selection::Conversation(contact)) => contact,
            _ => unreachable!(),
        }
    }

    /// Shows the conversation of `contact` once its identity is read.
    pub fn open_conversation(&mut self, contact: UiContactId) {
        self.until("the identity is read", move |model| {
            model
                .slice(contact.identity())
                .is_some_and(|slice| slice.loaded)
        });
        self.model().select_conversation(contact);
        self.until("the conversation is shown", move |model| {
            model.selection() == Some(Selection::Conversation(contact))
        });
    }

    pub fn send(&mut self, contact: UiContactId, text: &str) {
        self.model()
            .draft_mut(contact)
            .expect("the draft can be edited")
            .push_str(text);
        self.model().send(contact);
        let wanted = text.to_owned();
        self.until(&format!("\"{text}\" is queued"), move |model| {
            model.thread(contact).is_some_and(|thread| {
                thread.draft.is_empty()
                    && thread.messages.iter().any(|message| message.text == wanted)
            })
        });
    }

    pub fn status_of(&self, contact: UiContactId, text: &str) -> Option<Status> {
        self.app.model.thread(contact).and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|message| message.text == text)
                .map(|message| message.status)
        })
    }

    pub fn wait_status(&mut self, contact: UiContactId, text: &str, status: Status) {
        let wanted = text.to_owned();
        self.until(&format!("\"{text}\" is {status:?}"), move |model| {
            model.thread(contact).is_some_and(|thread| {
                thread
                    .messages
                    .iter()
                    .any(|message| message.text == wanted && message.status == status)
            })
        });
    }
}

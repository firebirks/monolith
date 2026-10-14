//! The workflows of the desktop client, A to G of the Phase 6 brief, run
//! by script: two clients, each with a profile on disk, the real bridge and
//! the real runtime, on the mock Tor network, in real time. The script does
//! what the screens do when the user acts (the actions of the UI model) and
//! waits for what the runtime says; nothing is answered in its place. A
//! picture of the window is written at each step:
//! `cargo run --release -p monolith-desktop --example scenarios -- <dir>`.
//!
//! It takes a few minutes: the runtime waits between delivery attempts as
//! it always does.

#![allow(
    clippy::print_stdout,
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::unreachable,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use core::time::Duration;

use egui::{Context, Pos2, RawInput, Rect};
use monolith_desktop::bridge::{Bridge, Setup};
use monolith_desktop::calendar::SystemCalendar;
use monolith_tor::MockNetwork;
use monolith_ui::{App, Preferences, ThemeChoice, TorSettings};
use monolith_ui_kit::{Theme, fonts, theme};
use monolith_ui_model::{
    Dialog, Model, Phase, RowState, Selection, Status, TorIndicator, UiContactId, UiIdentityId,
};
use monolith_ui_snapshot::{Options, render};
use zeroize::Zeroizing;

const PASSPHRASE: &str = "correct horse battery staple";
const PATIENCE: Duration = Duration::from_secs(240);

/// One client: its window state, its bridge, and a context to draw it.
struct Client {
    name: &'static str,
    app: App,
    bridge: Option<Bridge>,
    ctx: Context,
    started: Instant,
    shots: PathBuf,
    shot: u32,
}

impl Client {
    fn start(network: &MockNetwork, name: &'static str, data: &Path, shots: &Path) -> Self {
        let network = network.clone();
        let bridge = Bridge::start(
            Setup {
                data_dir: Some(data.to_owned()),
                backend: Box::new(move || network.backend()),
                shutdown_deadline: Duration::from_secs(3),
                close_wait: Duration::from_secs(10),
            },
            Arc::new(|| {}),
        )
        .expect("bridge");
        let preferences = Preferences {
            theme: ThemeChoice::Light,
            reduced_motion: true,
            tor: TorSettings {
                socks: "127.0.0.1:9050".to_owned(),
                control: "127.0.0.1:9051".to_owned(),
                cookie_file: String::new(),
            },
        };
        let ctx = Context::default();
        fonts::install(&ctx, &[]);
        theme::install(&ctx, Theme::light().with_reduced_motion(true));
        Self {
            name,
            app: App::new(Model::new(), Box::new(SystemCalendar::new()), preferences),
            bridge: Some(bridge),
            ctx,
            started: Instant::now(),
            shots: shots.to_owned(),
            shot: 0,
        }
    }

    fn model(&mut self) -> &mut Model {
        &mut self.app.model
    }

    /// One frame: what the bridge said goes to the model, the window is
    /// drawn, and what the model asks goes to the bridge.
    fn frame(&mut self) {
        let bridge = self.bridge.as_mut().expect("running");
        while let Some(update) = bridge.try_update() {
            self.app.model.apply(update);
        }
        let size = Theme::light().size.window_default;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            time: Some(self.started.elapsed().as_secs_f64()),
            focused: true,
            ..RawInput::default()
        };
        let app = &mut self.app;
        let _ = self.ctx.run(input, |ctx| monolith_ui::show(ctx, app));
        for (command, what) in self.app.model.take_commands() {
            bridge.send(command, what);
        }
        bridge.flush();
        let _ = self.app.take_effects();
    }

    /// Runs frames until `done` holds.
    fn until(&mut self, what: &str, done: impl Fn(&Model) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        let waited = Instant::now();
        loop {
            self.frame();
            if done(&self.app.model) {
                println!(
                    "  {}: {what} ({:.1} s)",
                    self.name,
                    waited.elapsed().as_secs_f64()
                );
                return;
            }
            assert!(
                Instant::now() < deadline,
                "{}: {what} did not happen in {PATIENCE:?}",
                self.name
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// Writes a picture of the window. It is drawn in a context of its own,
    /// which the software renderer sees from its first frame, textures
    /// included; the state shown is the client's.
    fn picture(&mut self, step: &str) {
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
            .join(format!("{}-{:02}-{step}.png", self.name, self.shot));
        image.save_png(&path).expect("write the picture");
    }

    /// Closes the window: the runtime stops, as when the user closes it.
    fn close(&mut self) {
        self.app.model.shut_down();
        self.frame();
        drop(self.bridge.take());
    }

    fn current(&self) -> UiIdentityId {
        self.app.model.current().expect("an identity is shown")
    }

    /// Makes a profile, or opens the one there.
    fn open_profile(&mut self) {
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

    fn create_identity(&mut self, name: &str) -> UiIdentityId {
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

    fn create_invitation(&mut self, label: &str) -> String {
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

    fn add_contact(&mut self, card: &str, name: &str) -> UiContactId {
        let identity = self.current();
        self.model().open(Dialog::AddContact {
            identity,
            card: card.to_owned(),
            name: name.to_owned(),
            busy: None,
            error: None,
            added: None,
        });
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

    /// Shows the conversation of `contact` once its identity is read.
    fn open_conversation(&mut self, contact: UiContactId) {
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

    fn send(&mut self, contact: UiContactId, text: &str) {
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

    fn status_of(&self, contact: UiContactId, text: &str) -> Option<Status> {
        self.app.model.thread(contact).and_then(|thread| {
            thread
                .messages
                .iter()
                .find(|message| message.text == text)
                .map(|message| message.status)
        })
    }

    fn wait_status(&mut self, contact: UiContactId, text: &str, status: Status) {
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

fn scratch(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    let _ = std::fs::remove_dir_all(&path);
    path
}

fn main() {
    let out = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "scenarios".to_owned()),
    );
    std::fs::create_dir_all(&out).expect("output folder");
    let data = std::env::temp_dir().join(format!("monolith-scenarios-{}", std::process::id()));
    let data_a = scratch(&data, "a");
    let data_b = scratch(&data, "b");
    let network = MockNetwork::new();
    let begun = Instant::now();

    println!("A. First launch, an identity, an invitation");
    let mut alice = Client::start(&network, "alice", &data_a, &out);
    alice.until("the first start is shown", |model| {
        model.phase() == Phase::Welcome
    });
    alice.picture("first-start");
    alice.open_profile();
    alice.picture("no-identity");
    let personal = alice.create_identity("Personal");
    let invitation = alice.create_invitation("For Bob");
    alice.picture("invitation-made");
    alice.model().close();

    println!("B. Two clients become contacts and talk");
    let mut bob = Client::start(&network, "bob", &data_b, &out);
    bob.open_profile();
    bob.create_identity("Bob");
    let at_bob = bob.add_contact(&invitation, "Alice");
    bob.picture("contact-added");
    bob.send(at_bob, "Hello Alice, it is Bob.");
    alice.until("Bob's request arrives", |model| {
        model
            .current_slice()
            .is_some_and(|slice| !slice.requests.is_empty())
    });
    alice.model().show_requests();
    alice.picture("request");
    let request = alice.app.model.current_slice().unwrap().requests[0].id;
    alice.model().open(Dialog::AcceptRequest {
        request,
        name: "Bob".to_owned(),
        busy: None,
        error: None,
    });
    alice.model().confirm();
    alice.until("Bob is a contact and his conversation is shown", |model| {
        model.dialog().is_none() && matches!(model.selection(), Some(Selection::Conversation(_)))
    });
    let at_alice = match alice.app.model.selection() {
        Some(Selection::Conversation(contact)) => contact,
        _ => unreachable!(),
    };
    assert_eq!(at_alice.identity(), personal);
    bob.wait_status(at_bob, "Hello Alice, it is Bob.", Status::Delivered);
    alice.wait_status(at_alice, "Hello Alice, it is Bob.", Status::Received);
    alice.send(at_alice, "Hi Bob, welcome.");
    alice.wait_status(at_alice, "Hi Bob, welcome.", Status::Delivered);
    bob.wait_status(at_bob, "Hi Bob, welcome.", Status::Received);
    alice.picture("conversation");
    bob.picture("conversation");

    println!("C. Bob is offline: messages wait, then are delivered");
    bob.close();
    alice.send(at_alice, "Are you there?");
    alice.send(at_alice, "I will wait.");
    assert_eq!(
        alice.status_of(at_alice, "Are you there?"),
        Some(Status::Queued)
    );
    alice.until("an attempt to deliver failed", |model| {
        model
            .current_slice()
            .is_some_and(|slice| slice.deferrals.contains_key(&at_alice))
    });
    alice.picture("queued-while-offline");
    let mut bob = Client::start(&network, "bob", &data_b, &out);
    bob.open_profile();
    alice.wait_status(at_alice, "Are you there?", Status::Delivered);
    alice.wait_status(at_alice, "I will wait.", Status::Delivered);
    alice.picture("delivered-after-return");

    println!("D. Tor stops, the window stays usable, Tor comes back");
    network.set_socks_available(false);
    network.set_control_available(false);
    alice.model().check_tor();
    alice.until("Tor is shown unavailable", |model| {
        model.tor_indicator() == TorIndicator::Unavailable
    });
    alice.send(at_alice, "Sent while Tor was down.");
    assert_eq!(
        alice.status_of(at_alice, "Sent while Tor was down."),
        Some(Status::Queued)
    );
    alice.picture("tor-unavailable");
    network.set_control_available(true);
    network.set_socks_available(true);
    alice.model().check_tor();
    alice.until("Tor is shown working again", |model| {
        model.tor_indicator() != TorIndicator::Unavailable
    });
    alice.wait_status(at_alice, "Sent while Tor was down.", Status::Delivered);
    alice.picture("tor-back");

    println!("E. Two identities, switched while messages arrive");
    let work = alice.create_identity("Work");
    bob.open_conversation(at_bob);
    for round in 0..6 {
        let text = format!("Message {round} while Alice switches");
        bob.send(at_bob, &text);
        alice
            .model()
            .select_identity(if round % 2 == 0 { personal } else { work });
        alice.frame();
        if alice.app.model.current() == Some(work) {
            let slice = alice.app.model.current_slice().unwrap();
            assert!(
                slice.contacts.iter().all(|view| view.id.identity() == work),
                "Work shows a contact of Personal"
            );
            assert_eq!(alice.app.model.selection(), None);
        }
    }
    alice.model().select_identity(work);
    alice.picture("work-identity");
    alice.model().select_identity(personal);
    alice.model().select_conversation(at_alice);
    alice.wait_status(at_alice, "Message 5 while Alice switches", Status::Received);
    for round in 0..6 {
        assert_eq!(
            alice.status_of(at_alice, &format!("Message {round} while Alice switches")),
            Some(Status::Received)
        );
    }
    alice.picture("personal-after-switching");

    println!("F. Blocking and deleting while the conversation is open");
    alice.model().open(Dialog::BlockContact {
        contact: at_alice,
        busy: None,
        error: None,
    });
    alice.picture("confirm-block");
    alice.model().confirm();
    alice.until("the conversation is ended", |model| {
        model
            .sidebar()
            .iter()
            .any(|row| row.contact == at_alice && row.state == RowState::Ended)
    });
    assert!(!alice.app.model.can_send(at_alice));
    alice.picture("blocked");
    alice.model().open(Dialog::DeleteConversation {
        contact: at_alice,
        busy: None,
        error: None,
    });
    alice.model().confirm();
    alice.until("the conversation is gone", |model| {
        model.dialog().is_none() && model.sidebar().iter().all(|row| row.contact != at_alice)
    });
    alice.picture("deleted");

    println!("G. Alice's client starts again and shows what was kept");
    let identities: Vec<_> = alice
        .app
        .model
        .identities()
        .iter()
        .map(|view| (view.id, view.name.clone()))
        .collect();
    alice.close();
    let mut alice = Client::start(&network, "alice", &data_a, &out);
    alice.open_profile();
    let again: Vec<_> = alice
        .app
        .model
        .identities()
        .iter()
        .map(|view| (view.id, view.name.clone()))
        .collect();
    assert_eq!(again, identities);
    alice.model().select_identity(personal);
    alice.until("Personal is read", |model| {
        model.current_slice().is_some_and(|slice| slice.loaded)
    });
    assert!(
        alice
            .app
            .model
            .sidebar()
            .iter()
            .all(|row| row.contact != at_alice),
        "the deleted conversation is back"
    );
    alice.picture("after-restart");

    alice.close();
    bob.close();
    let _ = std::fs::remove_dir_all(&data);
    println!(
        "all scenarios passed in {:.0} s; pictures in {}",
        begun.elapsed().as_secs_f64(),
        out.display()
    );
}

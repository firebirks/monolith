//! The bridge against a runtime on the mock Tor network: commands are
//! answered with the handles they carry, handles of one identity name
//! nothing of another, and the runtime stops when the window goes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use monolith_desktop::bridge::{Bridge, Setup};
use monolith_tor::{MockNetwork, MockTorBackend};
use monolith_ui_model::{
    Command, CommandId, Failure, Outcome, ProfileOnDisk, RequestMode, RuntimeEvent, UiIdentityId,
    UiInvitationId, UiRequestId, Update,
};
use zeroize::Zeroizing;

const PATIENCE: Duration = Duration::from_secs(60);

struct Driver {
    bridge: Bridge,
    next: u64,
    events: Vec<RuntimeEvent>,
    wakes: Arc<AtomicUsize>,
}

impl Driver {
    fn start(network: &MockNetwork, data_dir: Option<PathBuf>) -> Self {
        let wakes = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&wakes);
        let network = network.clone();
        let bridge = Bridge::start(
            Setup {
                data_dir,
                backend: Box::new(move || network.backend()),
                shutdown_deadline: Duration::from_secs(2),
                close_wait: Duration::from_secs(10),
            },
            Arc::new(move || {
                counter.fetch_add(1, Ordering::Relaxed);
            }),
        )
        .unwrap();
        Self {
            bridge,
            next: 0,
            events: Vec::new(),
            wakes,
        }
    }

    /// The next update that is not an event; events are kept.
    fn update(&mut self) -> Update {
        let deadline = Instant::now() + PATIENCE;
        loop {
            match self.bridge.try_update() {
                Some(Update::Event(event)) => self.events.push(event),
                Some(update) => return update,
                None => {
                    assert!(Instant::now() < deadline, "no update came");
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
    }

    /// Sends `command` and waits for its answer.
    fn ask(&mut self, command: Command) -> Outcome {
        self.next += 1;
        let id = CommandId::new(self.next);
        self.bridge.send(id, command);
        match self.update() {
            Update::Done { command, outcome } => {
                assert_eq!(command, id, "an answer to another command");
                outcome
            }
            other => panic!("not an answer: {other:?}"),
        }
    }

    /// Waits for an event that `wanted` accepts.
    fn wait_event(&mut self, wanted: impl Fn(&RuntimeEvent) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Some(index) = self.events.iter().position(&wanted) {
                self.events.remove(index);
                return;
            }
            match self.bridge.try_update() {
                Some(Update::Event(event)) => self.events.push(event),
                Some(other) => panic!("not an event: {other:?}"),
                None => {
                    assert!(
                        Instant::now() < deadline,
                        "the event did not come: {:?}",
                        self.events
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
        }
    }

    fn identity(&mut self, name: &str) -> UiIdentityId {
        match self.ask(Command::CreateIdentity {
            name: Some(name.to_owned()),
        }) {
            Outcome::IdentityCreated(identity) => identity,
            other => panic!("{other:?}"),
        }
    }

    fn invitation(&mut self, identity: UiIdentityId) -> (UiInvitationId, Zeroizing<String>) {
        match self.ask(Command::CreateInvitation {
            identity,
            label: Some("For Dana".to_owned()),
        }) {
            Outcome::InvitationCreated { invitation, card } => (invitation, card),
            other => panic!("{other:?}"),
        }
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("monolith-bridge-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn start_without_saving(driver: &mut Driver) {
    assert!(matches!(driver.update(), Update::Started { .. }));
    assert_eq!(
        driver.ask(Command::StartWithoutSaving),
        Outcome::ProfileOpen {
            ephemeral: true,
            message_store_missing: false,
        }
    );
}

#[test]
fn a_session_is_answered_command_by_command() {
    let network = MockNetwork::new();
    let dir = scratch("session");
    let mut driver = Driver::start(&network, Some(dir.clone()));
    assert_eq!(
        driver.update(),
        Update::Started {
            profile: ProfileOnDisk::Absent,
            location: dir.display().to_string(),
        }
    );
    // Before a profile is open, nothing else is done.
    assert_eq!(
        driver.ask(Command::LoadIdentities),
        Outcome::Failed(Failure::Unexpected)
    );
    // No vault: an empty passphrase is refused before anything is read,
    // and opening finds nothing.
    assert_eq!(
        driver.ask(Command::CreateProfile {
            passphrase: Zeroizing::new(String::new()),
        }),
        Outcome::Failed(Failure::UnusablePassphrase)
    );
    assert_eq!(
        driver.ask(Command::OpenProfile {
            passphrase: Zeroizing::new("correct horse".to_owned()),
        }),
        Outcome::Failed(Failure::Unexpected)
    );
    assert_eq!(
        driver.ask(Command::StartWithoutSaving),
        Outcome::ProfileOpen {
            ephemeral: true,
            message_store_missing: false,
        }
    );
    assert!(!dir.exists(), "a session without saving wrote to the disk");

    let personal = driver.identity("Personal");
    match driver.ask(Command::LoadIdentities) {
        Outcome::Identities(list) => {
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].id, personal);
            assert_eq!(list[0].name.as_deref(), Some("Personal"));
        }
        other => panic!("{other:?}"),
    }
    match driver.ask(Command::LoadIdentity(personal)) {
        Outcome::Identity(snapshot) => {
            assert_eq!(snapshot.identity, personal);
            assert!(snapshot.contacts.is_empty());
            assert!(snapshot.requests.is_empty());
            assert_eq!(snapshot.request_mode, RequestMode::Invitation);
        }
        other => panic!("{other:?}"),
    }
    let own = match driver.ask(Command::OwnCard(personal)) {
        Outcome::Card(card) => card,
        other => panic!("{other:?}"),
    };
    assert!(own.starts_with("MONOLITH"));
    assert_eq!(
        driver.ask(Command::AddContact {
            identity: personal,
            card: own.clone(),
            name: None,
        }),
        Outcome::Failed(Failure::OwnCard)
    );

    let (invitation, card) = driver.invitation(personal);
    assert_eq!(
        driver.ask(Command::InvitationCard(invitation)),
        Outcome::Card(card)
    );
    assert_eq!(
        driver.ask(Command::SetRequestMode {
            identity: personal,
            mode: RequestMode::Closed,
        }),
        Outcome::Done
    );
    match driver.ask(Command::LoadIdentity(personal)) {
        Outcome::Identity(snapshot) => {
            assert_eq!(snapshot.request_mode, RequestMode::Closed);
            assert_eq!(snapshot.invitations.len(), 1);
            assert_eq!(snapshot.invitations[0].id, invitation);
            assert_eq!(snapshot.invitations[0].label.as_deref(), Some("For Dana"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        driver.ask(Command::RevokeInvitation {
            invitation,
            discard_requests: true,
        }),
        Outcome::Revoked { discarded: 0 }
    );
    assert_eq!(
        driver.ask(Command::InvitationCard(invitation)),
        Outcome::Failed(Failure::NoSuchInvitation)
    );

    assert_eq!(driver.ask(Command::DeleteIdentity(personal)), Outcome::Done);
    driver.wait_event(|event| *event == RuntimeEvent::IdentityClosed(personal));
    assert_eq!(
        driver.ask(Command::LoadIdentity(personal)),
        Outcome::Failed(Failure::NoSuchIdentity)
    );

    assert_eq!(driver.ask(Command::Shutdown), Outcome::Done);
    assert_eq!(driver.update(), Update::Stopped);
    assert!(driver.wakes.load(Ordering::Relaxed) > 0);
}

#[test]
fn a_handle_of_one_identity_names_nothing_of_another() {
    let network = MockNetwork::new();
    let mut driver = Driver::start(&network, None);
    start_without_saving(&mut driver);
    let alice = driver.identity("Alice");
    let bob = driver.identity("Bob");

    // Alice asks Bob through his invitation; the request leaves with her
    // first message (P6-6).
    let (bob_invitation, card) = driver.invitation(bob);
    let at_alice = match driver.ask(Command::AddContact {
        identity: alice,
        card: card.clone(),
        name: Some("Bob".to_owned()),
    }) {
        Outcome::ContactAdded {
            contact,
            invitation: true,
        } => contact,
        other => panic!("{other:?}"),
    };
    assert_eq!(at_alice.identity(), alice);
    assert!(matches!(
        driver.ask(Command::Send {
            contact: at_alice,
            text: "hello Bob".to_owned(),
        }),
        Outcome::MessageQueued(_)
    ));
    driver.wait_event(|event| {
        *event
            == RuntimeEvent::RequestReceived {
                identity: bob,
                queued: true,
            }
    });
    let request = match driver.ask(Command::LoadIdentity(bob)) {
        Outcome::Identity(snapshot) => {
            assert_eq!(snapshot.requests.len(), 1);
            let request = &snapshot.requests[0];
            assert_eq!(request.id.identity(), bob);
            assert_eq!(request.invitation, Some(bob_invitation));
            request.id
        }
        other => panic!("{other:?}"),
    };

    // The same numbers, said of Alice, name nothing.
    let foreign_request = UiRequestId::new(alice, request.handle());
    let foreign_invitation = UiInvitationId::new(alice, bob_invitation.handle());
    for command in [
        Command::AcceptRequest {
            request: foreign_request,
            name: None,
        },
        Command::DeclineRequest(foreign_request),
        Command::BlockRequest(foreign_request),
    ] {
        assert_eq!(driver.ask(command), Outcome::Failed(Failure::NoSuchRequest));
    }
    assert_eq!(
        driver.ask(Command::InvitationCard(foreign_invitation)),
        Outcome::Failed(Failure::NoSuchInvitation)
    );
    assert_eq!(
        driver.ask(Command::RevokeInvitation {
            invitation: foreign_invitation,
            discard_requests: true,
        }),
        Outcome::Failed(Failure::NoSuchInvitation)
    );
    // A number never given names nothing either.
    assert_eq!(
        driver.ask(Command::DeclineRequest(UiRequestId::new(bob, u64::MAX))),
        Outcome::Failed(Failure::NoSuchRequest)
    );

    // Bob's own handle works, once.
    let contact = match driver.ask(Command::AcceptRequest {
        request,
        name: Some("Alice".to_owned()),
    }) {
        Outcome::RequestAccepted(contact) => contact,
        other => panic!("{other:?}"),
    };
    assert_eq!(contact.identity(), bob);
    assert_eq!(
        driver.ask(Command::AcceptRequest {
            request,
            name: None
        }),
        Outcome::Failed(Failure::NoSuchRequest)
    );
    match driver.ask(Command::LoadIdentity(bob)) {
        Outcome::Identity(snapshot) => {
            assert!(snapshot.requests.is_empty());
            let named = snapshot
                .contacts
                .iter()
                .find(|view| view.id == contact)
                .unwrap();
            assert_eq!(named.name.as_deref(), Some("Alice"));
        }
        other => panic!("{other:?}"),
    }
    // A message to the new contact is queued, under Bob's handle.
    match driver.ask(Command::Send {
        contact,
        text: "hello".to_owned(),
    }) {
        Outcome::MessageQueued(message) => assert_eq!(message.contact(), contact),
        other => panic!("{other:?}"),
    }
    match driver.ask(Command::LoadMessages { contact, limit: 50 }) {
        Outcome::Messages {
            contact: answered,
            messages,
            complete,
        } => {
            assert_eq!(answered, contact);
            assert!(complete);
            // Alice's first message may arrive before or after Bob's;
            // every message is of this conversation.
            assert!(
                messages
                    .iter()
                    .any(|view| view.text == "hello" && view.status.outgoing())
            );
            assert!(messages.iter().all(|view| view.id.contact() == contact));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_contact_added_says_whether_its_card_was_an_invitation() {
    let network = MockNetwork::new();
    let mut driver = Driver::start(&network, None);
    start_without_saving(&mut driver);
    let alice = driver.identity("Alice");
    let bob = driver.identity("Bob");
    let carol = driver.identity("Carol");
    let (_, invitation) = driver.invitation(bob);
    let carol_card = match driver.ask(Command::OwnCard(carol)) {
        Outcome::Card(card) => card,
        other => panic!("{other:?}"),
    };
    let add = |driver: &mut Driver, owner: UiIdentityId, card: &str| match driver.ask(
        Command::AddContact {
            identity: owner,
            card: Zeroizing::new(card.to_owned()),
            name: None,
        },
    ) {
        Outcome::ContactAdded {
            contact,
            invitation,
        } => {
            assert_eq!(contact.identity(), owner);
            (contact, invitation)
        }
        other => panic!("{other:?}"),
    };
    let (bob_at_alice, invited) = add(&mut driver, alice, &invitation);
    assert!(invited);
    let (carol_at_alice, invited) = add(&mut driver, alice, &carol_card);
    assert!(!invited);
    let flags = |driver: &mut Driver| match driver.ask(Command::LoadIdentity(alice)) {
        Outcome::Identity(snapshot) => {
            let of = |contact| {
                let view = snapshot
                    .contacts
                    .iter()
                    .find(|view| view.id == contact)
                    .unwrap();
                (view.invitation, view.accepted)
            };
            (of(bob_at_alice), of(carol_at_alice))
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(flags(&mut driver), ((true, false), (false, false)));

    // Carol, in the default mode, drops Alice's request; once she adds
    // Alice's card too and writes, the two are contacts.
    assert!(matches!(
        driver.ask(Command::Send {
            contact: carol_at_alice,
            text: "hello Carol".to_owned(),
        }),
        Outcome::MessageQueued(_)
    ));
    let alice_card = match driver.ask(Command::OwnCard(alice)) {
        Outcome::Card(card) => card,
        other => panic!("{other:?}"),
    };
    let (alice_at_carol, invited) = add(&mut driver, carol, &alice_card);
    assert!(!invited);
    assert!(matches!(
        driver.ask(Command::Send {
            contact: alice_at_carol,
            text: "hello Alice".to_owned(),
        }),
        Outcome::MessageQueued(_)
    ));
    driver.wait_event(|event| *event == RuntimeEvent::ContactAccepted(carol_at_alice));
    assert_eq!(flags(&mut driver), ((true, false), (false, true)));
}

#[test]
fn closing_the_window_stops_the_runtime() {
    let network = MockNetwork::new();
    let mut driver = Driver::start(&network, None);
    start_without_saving(&mut driver);
    let _ = driver.identity("Personal");
    let started = Instant::now();
    drop(driver);
    assert!(started.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_runtime_thread_that_panics_is_said_to_have_stopped() {
    // The backend is made when a profile starts; this one cannot be, and
    // the thread of the runtime unwinds without a word.
    let mut bridge = Bridge::start(
        Setup {
            data_dir: None,
            backend: Box::new(|| -> MockTorBackend { panic!("no backend") }),
            shutdown_deadline: Duration::from_secs(2),
            close_wait: Duration::from_secs(10),
        },
        Arc::new(|| {}),
    )
    .unwrap();
    bridge.send(CommandId::new(1), Command::StartWithoutSaving);
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut updates = Vec::new();
    while !updates.contains(&Update::Stopped) {
        match bridge.try_update() {
            Some(update) => updates.push(update),
            None => {
                assert!(Instant::now() < deadline, "silence after {updates:?}");
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
    assert!(matches!(updates.first(), Some(Update::Started { .. })));
    assert_eq!(updates.last(), Some(&Update::Stopped));
    assert_eq!(updates.len(), 2, "{updates:?}");
    // It is said once.
    for _ in 0..10 {
        assert_eq!(bridge.try_update(), None);
    }
}

#[test]
fn messages_are_read_in_pages_up_to_the_limit() {
    let network = MockNetwork::new();
    let mut driver = Driver::start(&network, None);
    start_without_saving(&mut driver);
    let alice = driver.identity("Alice");
    let bob = driver.identity("Bob");
    let (_, card) = driver.invitation(bob);
    let contact = match driver.ask(Command::AddContact {
        identity: alice,
        card: card.clone(),
        name: None,
    }) {
        Outcome::ContactAdded {
            contact,
            invitation: true,
        } => contact,
        other => panic!("{other:?}"),
    };
    for number in 0..12 {
        assert!(matches!(
            driver.ask(Command::Send {
                contact,
                text: format!("message {number}"),
            }),
            Outcome::MessageQueued(_)
        ));
    }
    let texts = |outcome: Outcome| match outcome {
        Outcome::Messages {
            messages, complete, ..
        } => (
            messages
                .into_iter()
                .map(|view| view.text)
                .collect::<Vec<_>>(),
            complete,
        ),
        other => panic!("{other:?}"),
    };
    let (newest, complete) = texts(driver.ask(Command::LoadMessages { contact, limit: 5 }));
    assert_eq!(newest.first().unwrap(), "message 7");
    assert_eq!(newest.last().unwrap(), "message 11");
    assert!(!complete);
    let (all, complete) = texts(driver.ask(Command::LoadMessages { contact, limit: 12 }));
    assert_eq!(all.len(), 12);
    assert!(complete);
    let before = match driver.ask(Command::LoadMessages { contact, limit: 1 }) {
        Outcome::Messages { messages, .. } => messages[0].id.sequence(),
        other => panic!("{other:?}"),
    };
    let (earlier, complete) = texts(driver.ask(Command::LoadEarlier {
        contact,
        before,
        limit: 3,
    }));
    assert_eq!(earlier, ["message 8", "message 9", "message 10"]);
    assert!(!complete);
}

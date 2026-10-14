//! States of the window for screenshots and visual tests, built only
//! through what the bridge would say to the model. Built with the
//! `fixtures` feature, which the binary does not enable.

use monolith_ui_model::{
    Command, ContactView, ConversationView, Dialog, IdentitySnapshot, IdentityView, InvitationView,
    MessageView, Model, Outcome, ProfileOnDisk, RequestMode, RequestView, RuntimeEvent, Service,
    Status, TorCheck, UiContactId, UiIdentityId, UiInvitationId, UiMessageId, UiRequestId, Update,
};
use zeroize::Zeroizing;

use crate::app::{App, SettingsPage, View};

/// "Now" in every fixture: 2026-10-14 18:30 UTC, a Wednesday.
pub const NOW: u64 = 1_792_002_600;

/// The named states.
pub const NAMES: &[&str] = &[
    "welcome",
    "unlock",
    "unlock-error",
    "opening",
    "empty-identity",
    "conversation",
    "conversation-queued",
    "waiting",
    "tor-unavailable",
    "requests",
    "invitation",
    "identity-switcher",
    "contact-details",
    "add-contact",
    "confirm-block",
    "settings-appearance",
    "settings-identities",
    "settings-tor",
    "settings-data",
    "storage-failed",
];

fn identity(n: u8) -> UiIdentityId {
    UiIdentityId::from_bytes([n; 16])
}

fn contact(identity_n: u8, record: u8) -> UiContactId {
    UiContactId::new(identity(identity_n), [record; 16])
}

const ALICE_FP: &str = "AF3K 72PQ 9XZT M4LB D8EV 61RC";
const BOB_FP: &str = "B2NW 7HQA 4KTR 9MZE LC3D V8XP";
const CAROL_FP: &str = "C7TQ 2ZKW 8MPE 4NRD 6VBX 3HLA";
const DANA_FP: &str = "D4XM 9QTB 2KWE 7RLN 5PZC 8HVA";
const ERIN_FP: &str = "E9KR 3TWM 6ZQP 2BLD 7NXC 4VHE";
const FRANK_FP: &str = "F6QW 8NRT 3MKZ 5DPL 2XEC 9BVH";
const OWN_FP: &str = "M0N0 L1TH 7QZK 4WPE 9RTD 2XBC";

fn full(compact: &str) -> String {
    format!("{compact} 5TQN 2WRK 8MZP 3XEL 6VDB 9HCA Q4KM")
}

fn contact_view(
    id: UiContactId,
    name: Option<&str>,
    fingerprint: &str,
    accepted: bool,
) -> ContactView {
    ContactView {
        id,
        name: name.map(str::to_owned),
        fingerprint: fingerprint.to_owned(),
        full_fingerprint: full(fingerprint),
        accepted,
        verified: name == Some("Alice Martin"),
    }
}

fn message(
    contact: UiContactId,
    sequence: u64,
    status: Status,
    minutes_ago: u64,
    text: &str,
) -> MessageView {
    MessageView {
        id: UiMessageId::new(contact, sequence),
        status,
        time: NOW.saturating_sub(minutes_ago.saturating_mul(60)),
        text: text.to_owned(),
    }
}

fn alice_messages(queued_tail: bool) -> Vec<MessageView> {
    let alice = contact(1, 10);
    let mut list = vec![
        message(
            alice,
            1,
            Status::Received,
            1_500,
            "Hi! Are we still on for Thursday?",
        ),
        message(
            alice,
            2,
            Status::Received,
            1_499,
            "I booked the table for seven.",
        ),
        message(
            alice,
            3,
            Status::Delivered,
            1_490,
            "Yes, seven works for me.",
        ),
        message(
            alice,
            4,
            Status::Delivered,
            1_489,
            "Should I bring anything?\nI can pick up some wine on the way.",
        ),
        message(alice, 5, Status::Received, 95, "No need. Just yourself."),
        message(
            alice,
            6,
            Status::Received,
            40,
            "I went through the notes from the meeting again and I think we should write down the three decisions before we forget what we agreed on: the schedule, the budget and who talks to the supplier.",
        ),
        message(
            alice,
            7,
            Status::Delivered,
            12,
            "Agreed. I will send a summary tonight.",
        ),
    ];
    if queued_tail {
        list.push(message(
            alice,
            8,
            Status::Queued,
            2,
            "See you at the station at eight.",
        ));
    }
    list
}

struct Builder {
    model: Model,
}

impl Builder {
    fn new() -> Self {
        Self {
            model: Model::new(),
        }
    }

    fn answer(&mut self, mut answer: impl FnMut(&Command) -> Option<Outcome>) {
        for _ in 0..4 {
            for (id, command) in self.model.take_commands() {
                if let Some(outcome) = answer(&command) {
                    self.model.apply(Update::Done {
                        command: id,
                        outcome,
                    });
                }
            }
        }
    }

    fn started(mut self, exists: bool) -> Self {
        self.model.apply(Update::Started {
            profile: if exists {
                ProfileOnDisk::Exists
            } else {
                ProfileOnDisk::Absent
            },
            location: "/home/user/.local/share/monolith".to_owned(),
        });
        self
    }

    fn ready(mut self, tor: TorCheck, service: Service, queued_tail: bool) -> Self {
        self.model
            .unlock(Zeroizing::new("correct horse".to_owned()));
        let personal = identity(1);
        let work = identity(2);
        self.answer(|command| match command {
            Command::OpenProfile { .. } => Some(Outcome::ProfileOpen {
                ephemeral: false,
                message_store_missing: false,
            }),
            Command::LoadIdentities => Some(Outcome::Identities(vec![
                IdentityView {
                    id: personal,
                    name: Some("Personal".to_owned()),
                    fingerprint: OWN_FP.to_owned(),
                    full_fingerprint: full(OWN_FP),
                },
                IdentityView {
                    id: work,
                    name: Some("Work".to_owned()),
                    fingerprint: "W0RK 4ZQP 8TNM 2KLE 6XRD 9VBC".to_owned(),
                    full_fingerprint: full("W0RK 4ZQP 8TNM 2KLE 6XRD 9VBC"),
                },
            ])),
            Command::LoadIdentity(id) if *id == personal => {
                let alice = contact(1, 10);
                let bob = contact(1, 11);
                let carol = contact(1, 12);
                let dana = contact(1, 13);
                let erin = contact(1, 14);
                Some(Outcome::Identity(Box::new(IdentitySnapshot {
                    identity: personal,
                    contacts: vec![
                        contact_view(alice, Some("Alice Martin"), ALICE_FP, true),
                        contact_view(bob, Some("Bob at work"), BOB_FP, true),
                        contact_view(carol, None, CAROL_FP, false),
                        contact_view(erin, Some("Erin"), ERIN_FP, true),
                    ],
                    conversations: vec![
                        ConversationView {
                            contact: alice,
                            fingerprint: ALICE_FP.to_owned(),
                            active: true,
                            messages: 8,
                            queued: usize::from(queued_tail),
                            latest: alice_messages(queued_tail).last().cloned(),
                        },
                        ConversationView {
                            contact: bob,
                            fingerprint: BOB_FP.to_owned(),
                            active: true,
                            messages: 12,
                            queued: 0,
                            latest: Some(message(
                                bob,
                                12,
                                Status::Delivered,
                                300,
                                "Sounds good, sending the file over later today.",
                            )),
                        },
                        ConversationView {
                            contact: carol,
                            fingerprint: CAROL_FP.to_owned(),
                            active: true,
                            messages: 1,
                            queued: 1,
                            latest: Some(message(
                                carol,
                                1,
                                Status::Queued,
                                3_000,
                                "Hello, this is Sam from the workshop.",
                            )),
                        },
                        ConversationView {
                            contact: dana,
                            fingerprint: DANA_FP.to_owned(),
                            active: false,
                            messages: 3,
                            queued: 0,
                            latest: Some(message(dana, 3, Status::Received, 9_000, "Talk soon.")),
                        },
                    ],
                    requests: vec![RequestView {
                        id: UiRequestId::new(personal, 1),
                        fingerprint: FRANK_FP.to_owned(),
                        full_fingerprint: full(FRANK_FP),
                        display_name: "Frank".to_owned(),
                        introduction:
                            "Hi, it is Frank from the climbing gym. Dana gave me your invitation."
                                .to_owned(),
                        invitation: Some(UiInvitationId::new(personal, 1)),
                    }],
                    invitations: vec![
                        InvitationView {
                            id: UiInvitationId::new(personal, 1),
                            label: Some("Climbing gym".to_owned()),
                        },
                        InvitationView {
                            id: UiInvitationId::new(personal, 2),
                            label: Some("Laptop".to_owned()),
                        },
                    ],
                    service,
                    request_mode: RequestMode::Invitation,
                })))
            }
            Command::LoadIdentity(id) => Some(Outcome::Identity(Box::new(IdentitySnapshot {
                identity: *id,
                contacts: Vec::new(),
                conversations: Vec::new(),
                requests: Vec::new(),
                invitations: Vec::new(),
                service,
                request_mode: RequestMode::Invitation,
            }))),
            Command::CheckTor => Some(Outcome::Tor(tor.clone())),
            _ => None,
        });
        self.model.select_identity(personal);
        self
    }

    fn open(mut self, contact: UiContactId, queued_tail: bool) -> Self {
        self.model.select_conversation(contact);
        self.answer(|command| match command {
            Command::LoadMessages { contact: asked, .. } if *asked == contact => {
                Some(Outcome::Messages {
                    contact,
                    messages: if contact == self::contact(1, 10) {
                        alice_messages(queued_tail)
                    } else {
                        vec![message(
                            contact,
                            1,
                            Status::Queued,
                            3_000,
                            "Hello, this is Sam from the workshop.",
                        )]
                    },
                    complete: true,
                })
            }
            _ => None,
        });
        self
    }

    fn app(self) -> App {
        App::for_tests(self.model, NOW)
    }
}

fn ready_tor() -> TorCheck {
    TorCheck::Ready {
        version: "0.4.9.13".to_owned(),
        outdated: false,
    }
}

/// The window in the state `name` (one of [`NAMES`]).
pub fn fixture(name: &str) -> Option<App> {
    let standard = || {
        Builder::new()
            .started(true)
            .ready(ready_tor(), Service::Available, false)
    };
    let app = match name {
        "welcome" => Builder::new().started(false).app(),
        "unlock" => Builder::new().started(true).app(),
        "unlock-error" => {
            let mut builder = Builder::new().started(true);
            builder.model.unlock(Zeroizing::new("wrong".to_owned()));
            builder.answer(|command| match command {
                Command::OpenProfile { .. } => {
                    Some(Outcome::Failed(monolith_ui_model::Failure::WrongPassphrase))
                }
                _ => None,
            });
            builder.app()
        }
        "opening" => {
            let mut builder = Builder::new().started(true);
            builder
                .model
                .unlock(Zeroizing::new("correct horse".to_owned()));
            let _ = builder.model.take_commands();
            builder.app()
        }
        "empty-identity" => {
            let mut app = standard().app();
            app.model.select_identity(identity(2));
            app
        }
        "conversation" => standard().open(contact(1, 10), false).app(),
        "conversation-queued" => Builder::new()
            .started(true)
            .ready(ready_tor(), Service::Available, true)
            .open(contact(1, 10), true)
            .app(),
        "waiting" => standard().open(contact(1, 12), false).app(),
        "tor-unavailable" => Builder::new()
            .started(true)
            .ready(TorCheck::NotRunning, Service::Unavailable, true)
            .open(contact(1, 10), true)
            .app(),
        "requests" => {
            let mut app = standard().app();
            app.model.show_requests();
            app
        }
        "invitation" => {
            let mut app = standard().open(contact(1, 10), false).app();
            app.model.open(Dialog::NewInvitation {
                identity: identity(1),
                label: String::new(),
                created: None,
                busy: None,
                error: None,
            });
            app
        }
        "identity-switcher" => standard().open(contact(1, 10), false).app(),
        "contact-details" => {
            let mut app = standard().open(contact(1, 10), false).app();
            app.model.open(Dialog::ContactDetails {
                contact: contact(1, 10),
                name: "Alice Martin".to_owned(),
                busy: None,
                error: None,
            });
            app
        }
        "add-contact" => {
            let mut app = standard().open(contact(1, 10), false).app();
            app.model.open(Dialog::AddContact {
                identity: identity(1),
                card: String::new(),
                name: String::new(),
                busy: None,
                error: Some(monolith_ui_model::Failure::InvalidCard),
            });
            app
        }
        "confirm-block" => {
            let mut app = standard().open(contact(1, 11), false).app();
            app.model.open(Dialog::BlockContact {
                contact: contact(1, 11),
                busy: None,
                error: None,
            });
            app
        }
        "settings-appearance" => settings(standard().app(), SettingsPage::Appearance),
        "settings-identities" => settings(standard().app(), SettingsPage::Identities),
        "settings-tor" => settings(standard().app(), SettingsPage::Tor),
        "settings-data" => settings(standard().app(), SettingsPage::Data),
        "storage-failed" => {
            let mut app = standard().open(contact(1, 10), false).app();
            app.model.apply(Update::Event(RuntimeEvent::StorageFailed));
            app
        }
        _ => return None,
    };
    Some(app)
}

fn settings(mut app: App, page: SettingsPage) -> App {
    app.view = View::Settings(page);
    app
}

/// The conversation with Alice open and holding `count` messages, three
/// minutes apart and of varied length, for measuring long histories. The
/// window of a conversation holds at most
/// [`monolith_ui_model::model::MAX_WINDOW`] of them.
pub fn long_conversation(count: usize) -> App {
    const TEXTS: [&str; 4] = [
        "Yes.",
        "I will be there at seven, with the papers.",
        "We went through the list again this morning and agreed on the order: the \
         schedule first, then the budget, and the supplier last.",
        "Can you send me the address again? I lost the message.",
    ];
    let alice = contact(1, 10);
    let messages: Vec<MessageView> = (0..count)
        .map(|index| {
            let sequence = u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1);
            let status = match index.checked_rem(3).unwrap_or(0) {
                0 => Status::Received,
                1 => Status::Delivered,
                _ => Status::Queued,
            };
            let ago = u64::try_from(count.saturating_sub(index))
                .unwrap_or(0)
                .saturating_mul(3);
            let text = index
                .checked_rem(TEXTS.len())
                .and_then(|slot| TEXTS.get(slot))
                .copied()
                .unwrap_or_default();
            message(alice, sequence, status, ago, text)
        })
        .collect();
    let mut builder = Builder::new()
        .started(true)
        .ready(ready_tor(), Service::Available, false);
    builder.model.select_conversation(alice);
    builder.answer(|command| match command {
        Command::LoadMessages { contact: asked, .. } if *asked == alice => {
            Some(Outcome::Messages {
                contact: alice,
                messages: messages.clone(),
                complete: false,
            })
        }
        _ => None,
    });
    builder.app()
}

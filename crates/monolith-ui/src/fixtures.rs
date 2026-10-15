//! States of the window for screenshots and visual tests, built only
//! through what the bridge would say to the model. Built with the
//! `fixtures` feature, which the binary does not enable.

use monolith_ui_kit::toast::Kind;
use monolith_ui_model::{
    BlockedView, Command, ContactView, ConversationView, Deferral, Dialog, Failure,
    IdentitySnapshot, IdentityView, InvitationView, MessageView, Model, Outcome, ProfileOnDisk,
    RequestMode, RequestView, RuntimeEvent, Service, Status, TorCheck, UiBlockedId, UiContactId,
    UiIdentityId, UiInvitationId, UiMessageId, UiRequestId, Update,
};
use zeroize::Zeroizing;

use crate::app::{App, CardFile, SettingsPage, ThemeChoice, View};

/// "Now" in every fixture: 2026-10-14 18:30 UTC, a Wednesday.
pub const NOW: u64 = 1_792_002_600;

/// The named states.
pub const NAMES: &[&str] = &[
    "welcome",
    "unlock",
    "unlock-error",
    "opening",
    "no-identity",
    "empty-identity",
    "conversation",
    "conversation-queued",
    "conversation-deferred",
    "narrow-conversation",
    "waiting",
    "waiting-new",
    "waiting-card",
    "card-added",
    "tor-unavailable",
    "tor-connecting",
    "requests",
    "narrow-requests",
    "invitation",
    "own-card-open",
    "identity-switcher",
    "contact-details",
    "add-contact",
    "confirm-block",
    "settings-list",
    "settings-appearance",
    "settings-identities",
    "settings-nobody",
    "settings-blocked",
    "settings-tor",
    "settings-data",
    "about",
    "storage-failed",
    "no-profile-folder",
    "ephemeral",
    "close-ephemeral",
    "close-queued",
    "long-names",
    "long-names-requests",
    "ended",
    "requests-empty",
    "requests-without-name",
    "invitation-created",
    "toast-banners",
    "toast-dialog",
    "storage-failed-toast",
    // Every other dialog, over Alice's conversation unless said otherwise.
    "dialog-new-identity",
    "dialog-new-identity-no-tor",
    "dialog-add-contact-filled",
    "dialog-add-contact-read",
    "dialog-own-card",
    "dialog-show-invitation",
    "dialog-revoke",
    "dialog-contact-error",
    "dialog-contact-gone",
    "dialog-contact-waiting",
    "dialog-remove",
    "dialog-delete-messages",
    "dialog-delete-identity",
    "dialog-accept",
    "dialog-accept-gone",
    "dialog-block-request",
    "dialog-decline-request",
    "dialog-unblock",
    "dialog-tor",
    "dialog-tor-down",
];

/// A card as the field of "Add contact" takes it, holding an invitation.
const INVITATION_CARD: &str = "monolith:card:1:AJQXG43FMNZGK5BAONSWG4TFOQQGS3THEBUW4ZZAMVZXIZLTEBZG65LTMVZGKIDON5ZGOIDCMFWGS43IMFRGGZDFMZTWQ2LKNNWG23TPOBYXE43UOV3HO6DZPJTWC3DJNZSXE5DFPBPXG5LBNZSWCZB2GE3DENBQGIYTCMRRGMZDGNZYHA4DKNJWGU3TSMJZGQYDCOBWGM2TEMBTHE3DSOJTGAZGGMBRGQ3Q";

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
const GREG_FP: &str = "G3VD 8KTN 5QWZ 2MLE 7XRC 4PBH";
const OWN_FP: &str = "M0N0 L1TH 7QZK 4WPE 9RTD 2XBC";
const WORK_FP: &str = "W0RK 4ZQP 8TNM 2KLE 6XRD 9VBC";

/// The card of the personal identity, as the runtime gives it.
const OWN_CARD: &str = "MONOLITH1:AHLVVGABQKYQVN6VJP7NHSLEA45A5YLS6PNKMIZFV4BBU2HXA5IRVBJA6AEYSMFHKR2IW7O4WQ7POWQNX45A2JRYDL2OXJFJR2VJWTTKAAAAAAAAAAAACAMBHF3Q5KD5C5PVNI2UM3BUY7WMZOGYVENU5Y32EXPWB5NY7SNTSQAN4L47JWL3SCBZOLLAFXTZ6JK2RASOSQAL43SXXBA7ALY33GZ4R6QTXCN2RKXEPYRJ4PSJ34NEEL2XOAWXID2XQRJPDV3EHRZSDSKMAM";

/// Names and words long enough to find where the screens cut or wrap.
const LONG_IDENTITY: &str = "Personal identity for friends, family and the Thursday climbing group";
const LONG_CONTACT: &str =
    "Alexandra Konstantinopoulou-Vanderbilt from the Thursday evening climbing group";
const LONG_LINK: &str = "Here are the photos: https://photos.example.org/albums/\
                         2026-10-08-thursday-evening-climbing-session/\
                         IMG_20261008_193012_full_resolution_original.jpg?share=\
                         8f3a9c2e7b1d4f6a0c5e9b2d7f1a3c8e5b0d2f4a6c8e1b3d5f7a9c0e2b4d6f8a";
const LONG_SENDER: &str = "Frank from the climbing gym, who you met at the bouldering wall";
const LONG_INTRODUCTION: &str = "Hi, it is Frank from the climbing gym. Dana gave me your \
                                 invitation last week after the bouldering session, and said \
                                 you would not mind if I wrote to you. I am organising the trip \
                                 to the mountains in November and would like to ask whether \
                                 you want to join us. We leave on Friday evening and come back \
                                 on Sunday; there is room for two more in the car, and the hut \
                                 is already booked for everyone.";

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
        verified: id == contact(1, 10),
        invitation: true,
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

fn alice_messages(queued_tail: bool, long: bool) -> Vec<MessageView> {
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
    if long {
        list.push(message(alice, 8, Status::Received, 5, LONG_LINK));
    }
    if queued_tail {
        list.push(message(
            alice,
            9,
            Status::Queued,
            2,
            "See you at the station at eight.",
        ));
    }
    list
}

/// Bob's conversation: his questions, the answer, and then `new` messages
/// of his not read yet.
fn bob_messages(new: u8) -> Vec<MessageView> {
    let bob = contact(1, 11);
    let mut list = vec![
        message(
            bob,
            10,
            Status::Received,
            320,
            "The supplier moved our meeting to Friday morning.",
        ),
        message(
            bob,
            11,
            Status::Received,
            319,
            "Can you send me the new plan before then?",
        ),
        message(
            bob,
            12,
            Status::Delivered,
            300,
            "Sounds good, sending the file over later today.",
        ),
    ];
    for index in 0..new {
        list.push(message(
            bob,
            13_u64.saturating_add(u64::from(index)),
            Status::Received,
            30_u64.saturating_sub(u64::from(index)),
            "Got it, thanks. One more question about the budget.",
        ));
    }
    list
}

/// Dana's conversation, which ended: she was removed or blocked.
fn dana_messages() -> Vec<MessageView> {
    let dana = contact(1, 13);
    vec![
        message(
            dana,
            1,
            Status::Received,
            9_020,
            "Are you coming to the workshop on Friday?",
        ),
        message(dana, 2, Status::Delivered, 9_010, "Yes, I will be there."),
        message(dana, 3, Status::Received, 9_000, "Talk soon."),
    ]
}

/// How the bridge answers, for one state.
struct Builder {
    model: Model,
    ephemeral: bool,
    tor: TorCheck,
    service: Service,
    queued_tail: bool,
    mode: RequestMode,
    /// Messages of Bob not read yet.
    bob_new: u8,
    /// Long names and words everywhere.
    long: bool,
    /// Carol, who has not accepted, was added by an invitation, not by
    /// her card.
    carol_invited: bool,
    /// A message to Carol was written, which takes the request.
    carol_written: bool,
    /// The personal identity blocked Dana, a contact, and Greg, who sent a
    /// request.
    blocked: bool,
    /// No request waits.
    no_requests: bool,
    /// Frank's request carries no name and no introduction, as Monolith
    /// sends them.
    nameless_request: bool,
}

impl Builder {
    fn new() -> Self {
        Self {
            model: Model::new(),
            ephemeral: false,
            tor: ready_tor(),
            service: Service::Available,
            queued_tail: false,
            mode: RequestMode::Invitation,
            bob_new: 0,
            long: false,
            carol_invited: true,
            carol_written: true,
            blocked: false,
            no_requests: false,
            nameless_request: false,
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

    /// Answers what the model asked as the bridge of this state would.
    fn settle(&mut self) {
        for _ in 0..4 {
            for (id, command) in self.model.take_commands() {
                if let Some(outcome) = self.respond(&command) {
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

    fn respond(&self, command: &Command) -> Option<Outcome> {
        let personal = identity(1);
        match command {
            Command::OpenProfile { .. } | Command::StartWithoutSaving => {
                Some(Outcome::ProfileOpen {
                    ephemeral: self.ephemeral,
                    message_store_missing: false,
                })
            }
            Command::LoadIdentities => Some(Outcome::Identities(vec![
                IdentityView {
                    id: personal,
                    name: Some(if self.long { LONG_IDENTITY } else { "Personal" }.to_owned()),
                    fingerprint: OWN_FP.to_owned(),
                    full_fingerprint: full(OWN_FP),
                },
                IdentityView {
                    id: identity(2),
                    name: Some("Work".to_owned()),
                    fingerprint: WORK_FP.to_owned(),
                    full_fingerprint: full(WORK_FP),
                },
            ])),
            Command::LoadIdentity(id) if *id == personal => {
                Some(Outcome::Identity(Box::new(self.personal())))
            }
            Command::LoadIdentity(id) => Some(Outcome::Identity(Box::new(IdentitySnapshot {
                identity: *id,
                contacts: Vec::new(),
                conversations: Vec::new(),
                requests: Vec::new(),
                invitations: Vec::new(),
                blocked: Vec::new(),
                service: self.service,
                request_mode: RequestMode::Invitation,
            }))),
            Command::CheckTor => Some(Outcome::Tor(self.tor.clone())),
            Command::OwnCard(id) if *id == personal => {
                Some(Outcome::Card(Zeroizing::new(OWN_CARD.to_owned())))
            }
            Command::LoadMessages { contact: asked, .. } => {
                let messages = if *asked == contact(1, 10) {
                    alice_messages(self.queued_tail, self.long)
                } else if *asked == contact(1, 11) {
                    bob_messages(self.bob_new)
                } else if *asked == contact(1, 13) {
                    dana_messages()
                } else if *asked == contact(1, 12) && !self.carol_written {
                    Vec::new()
                } else {
                    vec![message(
                        *asked,
                        1,
                        Status::Queued,
                        3_000,
                        "Hello, this is Sam from the workshop.",
                    )]
                };
                Some(Outcome::Messages {
                    contact: *asked,
                    messages,
                    complete: true,
                })
            }
            _ => None,
        }
    }

    /// What the runtime says of the personal identity.
    fn personal(&self) -> IdentitySnapshot {
        let personal = identity(1);
        let alice = contact(1, 10);
        let bob = contact(1, 11);
        let carol = contact(1, 12);
        let dana = contact(1, 13);
        let erin = contact(1, 14);
        let alice_name = if self.long {
            LONG_CONTACT
        } else {
            "Alice Martin"
        };
        let (sender, introduction) = if self.long {
            (LONG_SENDER, LONG_INTRODUCTION)
        } else {
            (
                "Frank",
                "Hi, it is Frank from the climbing gym. Dana gave me your invitation.",
            )
        };
        let alice_all = alice_messages(self.queued_tail, self.long);
        let bob_all = bob_messages(self.bob_new);
        let (sender, introduction) = if self.nameless_request {
            ("", "")
        } else {
            (sender, introduction)
        };
        let mut conversations = vec![
            ConversationView {
                contact: alice,
                fingerprint: ALICE_FP.to_owned(),
                active: true,
                messages: alice_all.len(),
                queued: usize::from(self.queued_tail),
                latest: alice_all.last().cloned(),
            },
            ConversationView {
                contact: bob,
                fingerprint: BOB_FP.to_owned(),
                active: true,
                messages: bob_all.len(),
                queued: 0,
                latest: bob_all.last().cloned(),
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
                latest: dana_messages().last().cloned(),
            },
        ];
        if !self.carol_written {
            conversations.retain(|conversation| conversation.contact != carol);
        }
        let requests = if self.no_requests {
            Vec::new()
        } else {
            vec![RequestView {
                id: UiRequestId::new(personal, 1),
                fingerprint: FRANK_FP.to_owned(),
                full_fingerprint: full(FRANK_FP),
                display_name: sender.to_owned(),
                introduction: introduction.to_owned(),
                invitation: Some(UiInvitationId::new(personal, 1)),
            }]
        };
        IdentitySnapshot {
            identity: personal,
            contacts: vec![
                contact_view(alice, Some(alice_name), ALICE_FP, true),
                contact_view(bob, Some("Bob at work"), BOB_FP, true),
                ContactView {
                    invitation: self.carol_invited,
                    ..contact_view(carol, None, CAROL_FP, false)
                },
                contact_view(erin, Some("Erin"), ERIN_FP, true),
            ],
            conversations,
            requests,
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
            blocked: if self.blocked {
                [DANA_FP, GREG_FP]
                    .into_iter()
                    .zip(1..)
                    .map(|(fingerprint, handle)| BlockedView {
                        id: UiBlockedId::new(personal, handle),
                        fingerprint: fingerprint.to_owned(),
                        full_fingerprint: full(fingerprint),
                    })
                    .collect()
            } else {
                Vec::new()
            },
            service: self.service,
            request_mode: self.mode,
        }
    }

    /// The profile open with the personal identity shown.
    fn ready(mut self) -> Self {
        if self.ephemeral {
            self.model.start_without_saving();
        } else {
            self.model
                .unlock(Zeroizing::new("correct horse".to_owned()));
        }
        self.settle();
        self.model.select_identity(identity(1));
        self
    }

    fn open(mut self, contact: UiContactId) -> Self {
        self.model.select_conversation(contact);
        self.settle();
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
    let standard = || Builder::new().started(true).ready();
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
        "no-identity" => {
            let mut builder = Builder::new().started(true);
            builder
                .model
                .unlock(Zeroizing::new("correct horse".to_owned()));
            builder.answer(|command| match command {
                Command::OpenProfile { .. } => Some(Outcome::ProfileOpen {
                    ephemeral: false,
                    message_store_missing: false,
                }),
                Command::LoadIdentities => Some(Outcome::Identities(Vec::new())),
                Command::CheckTor => Some(Outcome::Tor(ready_tor())),
                _ => None,
            });
            builder.app()
        }
        "empty-identity" => {
            let mut app = standard().app();
            app.model.select_identity(identity(2));
            app
        }
        "conversation" => standard().open(contact(1, 10)).app(),
        "conversation-queued" => queued().open(contact(1, 10)).app(),
        "conversation-deferred" => {
            // The last attempt failed a minute ago; the next is due in four.
            let mut builder = queued().open(contact(1, 10));
            builder.model.set_time(NOW.saturating_sub(60));
            builder
                .model
                .apply(Update::Event(RuntimeEvent::DeliveryDeferred {
                    contact: contact(1, 10),
                    reason: Deferral::Unreachable,
                    retry_in: 300,
                }));
            builder.app()
        }
        "narrow-conversation" => {
            let mut app = queued().open(contact(1, 10)).app();
            app.narrow_conversation = true;
            app
        }
        "waiting" => standard().open(contact(1, 12)).app(),
        "waiting-new" => {
            let mut builder = Builder::new().started(true);
            builder.carol_written = false;
            builder.ready().open(contact(1, 12)).app()
        }
        "waiting-card" => {
            let mut app = card_only().open(contact(1, 12)).app();
            app.narrow_conversation = true;
            app
        }
        "card-added" => {
            let mut app = card_only().open(contact(1, 12)).app();
            app.model.open(Dialog::AddContact {
                identity: identity(1),
                card: String::new(),
                name: String::new(),
                busy: None,
                error: None,
                added: Some(contact(1, 12)),
            });
            app
        }
        "tor-unavailable" => {
            let mut builder = Builder::new().started(true);
            builder.tor = TorCheck::NotRunning;
            builder.service = Service::Unavailable;
            builder.queued_tail = true;
            builder.ready().open(contact(1, 10)).app()
        }
        "tor-connecting" => {
            let mut builder = Builder::new().started(true);
            builder.tor = TorCheck::Connecting { progress: Some(40) };
            builder.service = Service::Publishing;
            builder.ready().open(contact(1, 10)).app()
        }
        "requests" => {
            let mut app = standard().app();
            app.model.show_requests();
            app
        }
        "narrow-requests" => {
            let mut app = standard().app();
            app.model.show_requests();
            app.narrow_conversation = true;
            app
        }
        "invitation" => {
            let mut app = standard().open(contact(1, 10)).app();
            app.model.open(Dialog::NewInvitation {
                identity: identity(1),
                label: String::new(),
                created: None,
                busy: None,
                error: None,
            });
            app
        }
        "own-card-open" => {
            let mut builder = Builder::new().started(true);
            builder.mode = RequestMode::Open;
            let mut builder = builder.ready().open(contact(1, 10));
            builder.model.open(Dialog::OwnCard {
                identity: identity(1),
                card: None,
                busy: None,
                error: None,
            });
            builder.settle();
            builder.app()
        }
        "identity-switcher" => {
            let mut builder = Builder::new().started(true).ready().open(contact(1, 10));
            // Bob wrote while Alice's conversation was open.
            builder.bob_new = 3;
            for sequence in 13..16 {
                builder
                    .model
                    .apply(Update::Event(RuntimeEvent::MessageReceived(
                        UiMessageId::new(contact(1, 11), sequence),
                    )));
            }
            builder.settle();
            let mut app = builder.app();
            app.open_switcher = true;
            app
        }
        "contact-details" => {
            let mut app = standard().open(contact(1, 10)).app();
            app.model.open(Dialog::ContactDetails {
                contact: contact(1, 10),
                name: "Alice Martin".to_owned(),
                busy: None,
                error: None,
            });
            app
        }
        "add-contact" => {
            let mut app = standard().open(contact(1, 10)).app();
            app.model.open(Dialog::AddContact {
                identity: identity(1),
                card: String::new(),
                name: String::new(),
                busy: None,
                error: Some(monolith_ui_model::Failure::InvalidCard),
                added: None,
            });
            app
        }
        "confirm-block" => {
            let mut app = standard().open(contact(1, 11)).app();
            app.model.open(Dialog::BlockContact {
                contact: contact(1, 11),
                busy: None,
                error: None,
            });
            app
        }
        "settings-list" => {
            let mut app = settings(standard().app(), SettingsPage::Appearance);
            app.narrow_settings_page = false;
            app
        }
        "settings-appearance" => {
            // System, which is right in pictures of either theme.
            let mut app = settings(standard().app(), SettingsPage::Appearance);
            let mut preferences = app.preferences().clone();
            preferences.theme = ThemeChoice::System;
            app.set_preferences(preferences);
            app
        }
        "settings-identities" => settings(standard().app(), SettingsPage::Identities),
        "settings-nobody" => {
            let mut builder = Builder::new().started(true);
            builder.mode = RequestMode::Closed;
            settings(builder.ready().app(), SettingsPage::Identities)
        }
        "settings-blocked" => {
            let mut builder = Builder::new().started(true);
            builder.blocked = true;
            let mut app = settings(builder.ready().app(), SettingsPage::Identities);
            // The list is below the fold of the first window.
            app.reveal_blocked = true;
            app
        }
        "settings-tor" => settings(standard().app(), SettingsPage::Tor),
        "settings-data" => settings(standard().app(), SettingsPage::Data),
        "about" => settings(standard().app(), SettingsPage::About),
        "no-profile-folder" => {
            let mut builder = Builder::new();
            builder.model.apply(Update::Started {
                profile: ProfileOnDisk::Absent,
                location: String::new(),
            });
            builder.app()
        }
        "ephemeral" => {
            let mut builder = Builder::new().started(false);
            builder.ephemeral = true;
            builder.ready().open(contact(1, 10)).app()
        }
        "close-ephemeral" => {
            let mut builder = Builder::new().started(false);
            builder.ephemeral = true;
            let mut app = builder.ready().open(contact(1, 10)).app();
            app.close_requested();
            app
        }
        "close-queued" => {
            let mut app = queued().open(contact(1, 10)).app();
            app.close_requested();
            app
        }
        "storage-failed" => {
            let mut app = standard().open(contact(1, 10)).app();
            app.model.apply(Update::Event(RuntimeEvent::StorageFailed));
            app
        }
        "long-names" => long().open(contact(1, 10)).app(),
        "long-names-requests" => {
            let mut app = long().app();
            app.model.show_requests();
            app.narrow_conversation = true;
            app
        }
        // In a narrow window these show the conversation or the requests,
        // not the list.
        "ended" => {
            let mut app = standard().open(contact(1, 13)).app();
            app.narrow_conversation = true;
            app
        }
        "requests-empty" => {
            let mut builder = Builder::new().started(true);
            builder.no_requests = true;
            let mut app = builder.ready().app();
            app.model.show_requests();
            app.narrow_conversation = true;
            app
        }
        "requests-without-name" => {
            let mut builder = Builder::new().started(true);
            builder.nameless_request = true;
            let mut app = builder.ready().app();
            app.model.show_requests();
            app.narrow_conversation = true;
            app
        }
        "invitation-created" => with_dialog(Dialog::NewInvitation {
            identity: identity(1),
            label: "For Dana".to_owned(),
            created: Some((
                UiInvitationId::new(identity(1), 3),
                Zeroizing::new(INVITATION_CARD.to_owned()),
            )),
            busy: None,
            error: None,
        }),
        "toast-banners" => {
            // Over the notice that nothing is sent while Tor is down; in a
            // narrow window, the conversation.
            let mut app = fixture("tor-unavailable")?;
            app.toasts.push(Kind::Success, "Card copied");
            app.narrow_conversation = true;
            app
        }
        "toast-dialog" => {
            // Over a dialog as tall as the smallest window.
            let mut app = fixture("dialog-own-card")?;
            app.toasts.push(Kind::Success, "Card copied");
            app
        }
        "storage-failed-toast" => {
            // What fails after the store failed says so as well.
            let mut builder = standard().open(contact(1, 10));
            builder
                .model
                .apply(Update::Event(RuntimeEvent::StorageFailed));
            builder.model.set_verified(contact(1, 10), false);
            builder.answer(|command| match command {
                Command::SetVerified { .. } => Some(Outcome::Failed(Failure::Storage)),
                _ => None,
            });
            builder.app()
        }
        name => return dialog(name),
    };
    Some(app)
}

/// Alice's conversation with `dialog` open over it.
fn with_dialog(dialog: Dialog) -> App {
    let mut app = Builder::new()
        .started(true)
        .ready()
        .open(contact(1, 10))
        .app();
    app.model.open(dialog);
    app
}

/// The dialogs of [`NAMES`] that start with "dialog-".
fn dialog(name: &str) -> Option<App> {
    let personal = identity(1);
    let app = match name {
        "dialog-new-identity" => with_dialog(Dialog::NewIdentity {
            name: "Work".to_owned(),
            busy: None,
            error: None,
        }),
        "dialog-new-identity-no-tor" => {
            let mut app = fixture("tor-unavailable")?;
            app.model.open(Dialog::NewIdentity {
                name: "Work".to_owned(),
                busy: None,
                error: None,
            });
            app
        }
        "dialog-add-contact-filled" => with_dialog(Dialog::AddContact {
            identity: personal,
            card: INVITATION_CARD.to_owned(),
            name: "Dana".to_owned(),
            busy: None,
            error: None,
            added: None,
        }),
        "dialog-add-contact-read" => {
            let mut app = with_dialog(Dialog::AddContact {
                identity: personal,
                card: INVITATION_CARD.to_owned(),
                name: String::new(),
                busy: None,
                error: None,
                added: None,
            });
            app.card_file = Some(CardFile {
                name: "dana-card.txt".to_owned(),
                read: true,
            });
            app
        }
        "dialog-own-card" => {
            let mut builder = Builder::new().started(true).ready().open(contact(1, 10));
            builder.model.open(Dialog::OwnCard {
                identity: personal,
                card: None,
                busy: None,
                error: None,
            });
            builder.settle();
            builder.app()
        }
        "dialog-show-invitation" => {
            let mut app = with_dialog(Dialog::ShowInvitation {
                invitation: UiInvitationId::new(personal, 1),
                card: None,
                busy: None,
                error: None,
            });
            // The card the runtime gives back.
            if let Some(Dialog::ShowInvitation { card, busy, .. }) = app.model.dialog_mut() {
                *card = Some(Zeroizing::new(INVITATION_CARD.to_owned()));
                *busy = None;
            }
            app
        }
        "dialog-revoke" => with_dialog(Dialog::RevokeInvitation {
            invitation: UiInvitationId::new(personal, 1),
            discard_requests: true,
            busy: None,
            error: Some(Failure::Disk),
        }),
        "dialog-contact-error" => with_dialog(Dialog::ContactDetails {
            contact: contact(1, 10),
            name: "Alice Martin".to_owned(),
            busy: None,
            error: Some(Failure::InvalidText),
        }),
        "dialog-contact-gone" => with_dialog(Dialog::ContactDetails {
            contact: contact(1, 99),
            name: String::new(),
            busy: None,
            error: None,
        }),
        "dialog-contact-waiting" => with_dialog(Dialog::ContactDetails {
            contact: contact(1, 12),
            name: String::new(),
            busy: None,
            error: None,
        }),
        "dialog-remove" => with_dialog(Dialog::RemoveContact {
            contact: contact(1, 10),
            busy: None,
            error: None,
        }),
        "dialog-delete-messages" => with_dialog(Dialog::DeleteConversation {
            contact: contact(1, 12),
            busy: None,
            error: None,
        }),
        "dialog-delete-identity" => with_dialog(Dialog::DeleteIdentity {
            identity: personal,
            busy: None,
            error: None,
        }),
        "dialog-accept" => with_dialog(Dialog::AcceptRequest {
            request: UiRequestId::new(personal, 1),
            name: "Frank".to_owned(),
            busy: None,
            error: None,
        }),
        "dialog-accept-gone" => with_dialog(Dialog::AcceptRequest {
            request: UiRequestId::new(personal, 9),
            name: String::new(),
            busy: None,
            error: None,
        }),
        "dialog-block-request" => {
            let mut app = fixture("requests")?;
            app.block_request = Some(UiRequestId::new(personal, 1));
            app
        }
        "dialog-decline-request" => {
            let mut app = fixture("requests")?;
            app.decline_request = Some(UiRequestId::new(personal, 1));
            app
        }
        "dialog-unblock" => {
            let mut app = fixture("settings-blocked")?;
            app.unblock = Some(UiBlockedId::new(personal, 1));
            app
        }
        "dialog-tor" => with_dialog(Dialog::TorDetails),
        "dialog-tor-down" => {
            let mut app = fixture("tor-unavailable")?;
            app.model.open(Dialog::TorDetails);
            app
        }
        _ => return None,
    };
    Some(app)
}

/// The profile open with Alice's last message still queued.
fn queued() -> Builder {
    let mut builder = Builder::new().started(true);
    builder.queued_tail = true;
    builder.ready()
}

/// The profile open with Carol added by her card, not by an invitation.
fn card_only() -> Builder {
    let mut builder = Builder::new().started(true);
    builder.carol_invited = false;
    builder.ready()
}

/// The profile open with long names and words.
fn long() -> Builder {
    let mut builder = Builder::new().started(true);
    builder.long = true;
    builder.ready()
}

/// The settings at `page`; in a narrow window the page, not the list.
fn settings(mut app: App, page: SettingsPage) -> App {
    app.view = View::Settings(page);
    app.narrow_settings_page = true;
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
    conversation_of(1, count, |index| {
        index
            .checked_rem(TEXTS.len())
            .and_then(|slot| TEXTS.get(slot))
            .copied()
            .unwrap_or_default()
            .to_owned()
    })
}

/// The conversation with Alice open and holding `count` messages of about
/// `bytes` bytes each, every one different: long histories of long
/// messages.
pub fn long_conversation_of(count: usize, bytes: usize) -> App {
    conversation_window(1, count, bytes)
}

/// The conversation with Alice open and holding `count` messages of about
/// `bytes` bytes each, numbered from `first`, with older ones still to
/// read. Message `n` starts with "Message n." ([`long_text`]).
pub fn conversation_window(first: u64, count: usize, bytes: usize) -> App {
    conversation_of(first, count, |index| {
        let number = u64::try_from(index)
            .unwrap_or(u64::MAX)
            .saturating_add(first);
        long_text(usize::try_from(number).unwrap_or(usize::MAX), bytes)
    })
}

/// A text of about `bytes` bytes for message `index`, starting with
/// "Message index.": sentences, with a new paragraph about every 600
/// bytes, so that a message of the largest size is drawn whole.
pub fn long_text(index: usize, bytes: usize) -> String {
    const SENTENCES: [&str; 4] = [
        "We went through the list again this morning and agreed on the order.",
        "The schedule comes first, then the budget, and the supplier last.",
        "Can you send me the address again? I lost the message with the map.",
        "I will be there at seven, with the papers and the keys to the hut.",
    ];
    let mut text = format!("Message {index}.");
    let mut paragraph = text.len();
    let mut turn = index;
    while text.len() < bytes {
        let sentence = turn
            .checked_rem(SENTENCES.len())
            .and_then(|slot| SENTENCES.get(slot))
            .copied()
            .unwrap_or_default();
        if paragraph > 600 {
            text.push_str("\n\n");
            paragraph = 0;
        } else {
            text.push(' ');
        }
        text.push_str(sentence);
        paragraph = paragraph.saturating_add(sentence.len());
        turn = turn.saturating_add(1);
    }
    let mut end = bytes.min(text.len());
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text.truncate(end);
    text
}

/// The status of message `index` of a long conversation: received,
/// delivered and queued in turn.
fn status_of(index: usize) -> Status {
    match index.checked_rem(3).unwrap_or(0) {
        0 => Status::Received,
        1 => Status::Delivered,
        _ => Status::Queued,
    }
}

/// The conversation with Alice open and holding `count` messages three
/// minutes apart, numbered from `first`, message `index` of them reading
/// `text(index)`.
fn conversation_of(first: u64, count: usize, text: impl Fn(usize) -> String) -> App {
    let alice = contact(1, 10);
    let messages: Vec<MessageView> = (0..count)
        .map(|index| {
            let sequence = u64::try_from(index)
                .unwrap_or(u64::MAX)
                .saturating_add(first);
            let ago = u64::try_from(count.saturating_sub(index))
                .unwrap_or(0)
                .saturating_mul(3);
            message(alice, sequence, status_of(index), ago, &text(index))
        })
        .collect();
    let mut builder = Builder::new().started(true).ready();
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

/// The personal identity with `count` contacts, each with a conversation
/// whose latest message is about `bytes` bytes, and none of them open: a
/// long sidebar of long previews.
pub fn many_contacts(count: usize, bytes: usize) -> App {
    let personal = identity(1);
    let mut contacts = Vec::new();
    let mut conversations = Vec::new();
    for index in 0..count {
        let number = u64::try_from(index).unwrap_or(u64::MAX);
        let mut record = [0x80; 16];
        for (slot, byte) in record.iter_mut().zip(number.to_be_bytes()) {
            *slot = byte;
        }
        let id = UiContactId::new(personal, record);
        let fingerprint = format!("{:04X} 72PQ 9XZT M4LB D8EV 61RC", index & 0xFFFF);
        let name = format!("Person {index:04}");
        contacts.push(contact_view(id, Some(&name), &fingerprint, true));
        conversations.push(ConversationView {
            contact: id,
            fingerprint,
            active: true,
            messages: 1,
            queued: 0,
            latest: Some(message(
                id,
                1,
                status_of(index),
                number.saturating_mul(7),
                &long_text(index, bytes),
            )),
        });
    }
    let snapshot = IdentitySnapshot {
        identity: personal,
        contacts,
        conversations,
        requests: Vec::new(),
        invitations: Vec::new(),
        blocked: Vec::new(),
        service: Service::Available,
        request_mode: RequestMode::Invitation,
    };
    let mut builder = Builder::new().started(true);
    builder
        .model
        .unlock(Zeroizing::new("correct horse".to_owned()));
    let standard = Builder::new();
    builder.answer(|command| match command {
        Command::LoadIdentity(id) if *id == personal => {
            Some(Outcome::Identity(Box::new(snapshot.clone())))
        }
        other => standard.respond(other),
    });
    builder.model.select_identity(personal);
    builder.app()
}

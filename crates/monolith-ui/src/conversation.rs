//! A conversation: its header, its messages and the composer.

use core::time::Duration;
use std::sync::Arc;

use egui::{Align, Layout, Margin, Popup, Sense, TopBottomPanel, Ui, UiBuilder, Vec2};
use monolith_ui_kit::avatar::{Avatar, AvatarSize};
use monolith_ui_kit::banner::{Banner, InlineError, Tone};
use monolith_ui_kit::bubble::{DaySeparator, Direction, MessageBubble};
use monolith_ui_kit::button::{Button, Size};
use monolith_ui_kit::chrome::{self, TopBar};
use monolith_ui_kit::composer::{self, Composer};
use monolith_ui_kit::empty_state::EmptyState;
use monolith_ui_kit::icon_button::IconButton;
use monolith_ui_kit::menu::{MenuItem, context_menu, menu_below, menu_separator};
use monolith_ui_kit::message_status::Status as MarkStatus;
use monolith_ui_kit::text::Text;
use monolith_ui_kit::tor_status::TorStatus;
use monolith_ui_kit::{Theme, icons, theme};
use monolith_ui_model::present::Calendar;
use monolith_ui_model::{
    Deferral, Dialog, Failure, MessageView, Model, Status, Thread, TorIndicator, UiContactId,
    UiMessageId, Waiting, present,
};

use crate::app::App;
use crate::common;
use crate::message_list::{self, Key};

/// Messages of one sender within this many seconds form a group.
const GROUP_SECONDS: u64 = 300;

/// What an identity without contacts can do.
pub(crate) const NO_CONTACTS: &str =
    "Add someone using their Monolith card or invitation, or share your card with them.";

/// What the "more" menu asked for.
#[derive(Clone, Copy)]
enum MenuAction {
    Details,
    Block,
    Remove,
    Delete,
}

/// Runs `add` in a column narrower than `ui` by `pad` on both sides.
pub(crate) fn inset<R>(ui: &mut Ui, pad: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let mut rect = ui.available_rect_before_wrap();
    rect.min.x += pad;
    rect.max.x = (rect.max.x - pad).max(rect.min.x);
    let mut child = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    let result = add(&mut child);
    let used = child.min_rect();
    ui.allocate_rect(
        egui::Rect::from_min_max(
            egui::pos2(rect.min.x - pad, used.min.y),
            egui::pos2(rect.max.x + pad, used.max.y),
        ),
        Sense::hover(),
    );
    result
}

/// The words under the name in the header, if any: that the conversation
/// ended, what a contact that has not accepted waits for, as the list and
/// its details say it, or that it was verified.
fn status_line(ended: bool, waiting: Option<Waiting>, verified: bool) -> Option<&'static str> {
    if ended {
        Some("No longer a contact")
    } else if let Some(waiting) = waiting {
        Some(common::waiting_words(waiting))
    } else if verified {
        Some("Verified")
    } else {
        None
    }
}

/// Draws the conversation of `contact`.
pub(crate) fn show(ui: &mut Ui, app: &mut App, contact: UiContactId) {
    let theme = theme::current(ui.ctx());
    let narrow = ui.ctx().screen_rect().width() < theme.size.narrow_below;
    let pad = theme.space.xl;
    // An ended conversation takes no message: it has no composer, and its
    // notice says why.
    if app.model.contact(contact).is_some() {
        TopBottomPanel::bottom(egui::Id::new(("conversation-composer", contact)))
            .frame(chrome::surface_frame(
                ui.ctx(),
                Margin::from(Vec2::new(pad, theme.space.lg)),
            ))
            .show_separator_line(false)
            .show_inside(ui, |ui| composer(ui, app, contact));
    }
    header(ui, app, contact, narrow);
    if banners(ui, app, contact, pad) {
        app.model.open(Dialog::OwnCard {
            identity: contact.identity(),
            card: None,
            busy: None,
            error: None,
        });
    }
    messages(ui, app, contact, pad);
    app.focus_composer = false;
    app.last_conversation = Some(contact);
}

/// The bar at the top: who, in what state, Tor, and the menu.
fn header(ui: &mut Ui, app: &mut App, contact: UiContactId, narrow: bool) {
    let theme = theme::current(ui.ctx());
    let name = app.model.contact_name(contact);
    let view = app.model.contact(contact);
    let fingerprint = view
        .map(|view| view.fingerprint.clone())
        .or_else(|| {
            app.model
                .conversation(contact)
                .map(|conversation| conversation.fingerprint.clone())
        })
        .unwrap_or_default();
    let ended = view.is_none();
    let accepted = view.is_some_and(|view| view.accepted);
    let verified = view.is_some_and(|view| view.verified);
    let anonymous = !view.is_some_and(present::named);
    let waiting = app.model.waiting(contact);
    let line = status_line(ended, waiting, verified);
    let (tor, tor_label) = common::tor_state(&app.model);
    let mut back = false;
    let mut open_tor = false;
    let mut action = None;
    TopBar::new(("conversation-bar", contact)).show(
        ui,
        |ui| {
            if narrow {
                back = ui
                    .add(IconButton::new(icons::CHEVRON_LEFT, "Back").id_salt(("back", contact)))
                    .clicked();
            }
            ui.add(
                Avatar::new(&name, common::seed(&fingerprint))
                    .anonymous(anonymous)
                    .size(AvatarSize::Medium),
            );
            ui.add_space(theme.space.xs);
            let lines = theme.text.body_strong.line_height
                + if line.is_some() {
                    theme.text.caption.line_height
                } else {
                    0.0
                };
            let size = Vec2::new(ui.available_width(), lines);
            ui.allocate_ui_with_layout(size, Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add(Text::new(&name, &theme.text.body_strong, theme.color.text).truncate());
                if let Some(line) = line {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = theme.space.xs;
                        if verified && !ended && accepted {
                            let (rect, _) = ui.allocate_exact_size(
                                Vec2::splat(theme.size.icon_sm),
                                Sense::hover(),
                            );
                            icons::BADGE_CHECK.paint(
                                ui.painter(),
                                rect.center(),
                                theme.size.icon_sm,
                                theme.color.accent_text,
                            );
                        }
                        ui.add(
                            Text::new(line, &theme.text.caption, theme.color.text_secondary)
                                .truncate(),
                        );
                    });
                }
            });
        },
        |ui| {
            let more = ui
                .add(IconButton::new(icons::ELLIPSIS_VERTICAL, "More").id_salt(("more", contact)));
            menu_below(&more, |ui| {
                if !ended {
                    if ui
                        .add(MenuItem::new("Contact details").icon(icons::CONTACT))
                        .clicked()
                    {
                        action = Some(MenuAction::Details);
                    }
                    if ui
                        .add(MenuItem::new("Block contact").icon(icons::BAN))
                        .clicked()
                    {
                        action = Some(MenuAction::Block);
                    }
                    if ui
                        .add(MenuItem::new("Remove contact").icon(icons::USER_X))
                        .clicked()
                    {
                        action = Some(MenuAction::Remove);
                    }
                    menu_separator(ui);
                }
                if ui
                    .add(
                        MenuItem::new("Delete messages on this device")
                            .icon(icons::TRASH_2)
                            .danger(true),
                    )
                    .clicked()
                {
                    action = Some(MenuAction::Delete);
                }
            });
            open_tor = ui.add(TorStatus::new(tor, tor_label)).clicked();
        },
    );
    if back {
        app.narrow_conversation = false;
    }
    if open_tor {
        app.model.open(Dialog::TorDetails);
    }
    match action {
        Some(MenuAction::Details) => {
            let name = app
                .model
                .contact(contact)
                .and_then(|view| view.name.clone())
                .unwrap_or_default();
            app.model.open(Dialog::ContactDetails {
                contact,
                name,
                busy: None,
                error: None,
            });
        }
        Some(MenuAction::Block) => app.model.open(Dialog::BlockContact {
            contact,
            busy: None,
            error: None,
        }),
        Some(MenuAction::Remove) => app.model.open(Dialog::RemoveContact {
            contact,
            busy: None,
            error: None,
        }),
        Some(MenuAction::Delete) => app.model.open(Dialog::DeleteConversation {
            contact,
            busy: None,
            error: None,
        }),
        None => {}
    }
}

/// What every notice of queued messages ends with: the runtime dials only
/// while it runs (`DESIGN_QUESTIONS.md` P5-8).
const BOTH_OPEN: &str = "Messages can be delivered only while Monolith is open on both sides.";

/// The sentence for a delivery that waits, `None` for nothing to say.
/// While Tor is known not to work nothing can be delivered, whatever the
/// last attempt said; messages are still queued. Otherwise Tor not opening
/// a stream to the contact is what an offline contact looks like, and is
/// said as such. While the contact has not accepted, the notice of the
/// request says enough.
fn deferral_text(
    reason: Option<Deferral>,
    name: &str,
    tor_down: bool,
    waiting: bool,
) -> Option<String> {
    let text = match reason {
        _ if tor_down => "Tor is not available, so nothing can be delivered. Monolith tries \
                          again when Tor is back."
            .to_owned(),
        _ if waiting => return None,
        None => return None,
        Some(Deferral::Tor | Deferral::Unreachable) => format!(
            "{name} cannot be reached right now. Their Monolith may be closed or offline. \
             Monolith tries again from time to time; your messages stay queued on this device."
        ),
        Some(Deferral::NoAddress) => format!(
            "There is no address to send to {name} yet. Messages stay queued until there is."
        ),
        Some(Deferral::NoAcknowledgement) => {
            format!("{name} has not confirmed receipt yet. Monolith will send the messages again.")
        }
        Some(Deferral::Busy) => {
            "Monolith is busy with other conversations and tries this one again soon.".to_owned()
        }
    };
    Some(format!("{text} {BOTH_OPEN}"))
}

/// When delivery is tried next, `seconds` from now, said roughly, with the
/// seconds until that changes; `None` once it is due.
fn next_try(seconds: u64) -> Option<(String, u64)> {
    if seconds == 0 {
        return None;
    }
    if seconds < 60 {
        return Some(("Next try in less than a minute.".to_owned(), seconds));
    }
    // To the nearest minute, said until it rounds to one less.
    let minutes = seconds.saturating_add(30) / 60;
    let text = if minutes == 1 {
        "Next try in about a minute.".to_owned()
    } else {
        format!("Next try in about {minutes} minutes.")
    };
    let lowest = minutes.saturating_mul(60).saturating_sub(30).max(60);
    Some((text, seconds.saturating_sub(lowest).saturating_add(1)))
}

/// The notices under the bar. Returns true if the card of the identity was
/// asked for.
fn banners(ui: &mut Ui, app: &App, contact: UiContactId, pad: f32) -> bool {
    let theme = theme::current(ui.ctx());
    let name = present::prepare(&app.model.contact_name(contact), false).text;
    let view = app.model.contact(contact);
    let ended = view.is_none();
    let waiting = view.is_some_and(|view| !view.accepted);
    let invited = view.is_some_and(|view| view.invitation);
    let queued = app
        .model
        .conversation(contact)
        .is_some_and(|conversation| conversation.queued > 0);
    let deferral = app
        .model
        .current_slice()
        .and_then(|slice| slice.deferrals.get(&contact))
        .copied();
    let retry = app
        .model
        .current_slice()
        .and_then(|slice| slice.retries.get(&contact))
        .map(|at| at.saturating_sub(app.now()));
    let refused = app
        .model
        .thread(contact)
        .is_some_and(|thread| thread.refused);
    // A failed store is said once, for the whole window, not here.
    let mut notices: Vec<(&str, Tone, String, Option<&str>)> = Vec::new();
    if ended {
        notices.push((
            "ended",
            Tone::Info,
            "This contact was removed or blocked. You can read what is here, but not send messages."
                .to_owned(),
            None,
        ));
    }
    // Without an invitation the request is dropped, unless they take
    // requests from anyone (`PROTOCOL.md` section 12.2). With one, it is
    // seen only while the invitation is active and they take requests.
    if waiting && invited {
        let with = if app.model.waiting(contact) == Some(Waiting::FirstMessage) {
            "your first message"
        } else {
            "your messages"
        };
        notices.push((
            "waiting",
            Tone::Info,
            format!(
                "Your request goes out with {with}. {name} should see it through the invitation they gave you, with your fingerprint only, and can read your messages after accepting. Tell them it is you."
            ),
            None,
        ));
    } else if waiting {
        notices.push((
            "waiting-card",
            Tone::Info,
            format!(
                "You added their card, not an invitation. {name} must add your card too before your messages can arrive."
            ),
            Some("Show my card"),
        ));
    }
    if queued && !ended {
        let tor_down = app.model.tor_indicator() == TorIndicator::Unavailable;
        if let Some(mut text) = deferral_text(deferral, &name, tor_down, waiting) {
            // Not while Tor is down: the next try waits for Tor.
            if let Some((next, change)) = retry.filter(|_| !tor_down).and_then(next_try) {
                text = format!("{text} {next}");
                ui.ctx().request_repaint_after(Duration::from_secs(change));
            }
            notices.push(("deferral", Tone::Info, text, None));
        }
    }
    // Refused only for want of room: the conversation or the message store
    // is full (`RefusedReason::StoreFull` of the runtime).
    if refused {
        notices.push((
            "refused",
            Tone::Warning,
            format!(
                "There is no more room for messages on this device, so a message from {name} could not be saved. Their Monolith will send it again. To make room, delete the messages of this conversation or of others."
            ),
            None,
        ));
    }
    if notices.is_empty() {
        return false;
    }
    let mut asked = false;
    ui.add_space(theme.space.md);
    inset(ui, pad, |ui| {
        ui.spacing_mut().item_spacing.y = theme.space.sm;
        for (key, tone, text, action) in &notices {
            let mut banner = Banner::new(("conversation-banner", contact, *key), *tone, text);
            if let Some(label) = action {
                banner = banner.action(label);
            }
            asked |= banner.show(ui).action;
        }
    });
    ui.add_space(theme.space.sm);
    asked
}

/// Whether message `b` continues the group of `a`.
fn same_group(calendar: &dyn Calendar, a: &MessageView, b: &MessageView) -> bool {
    a.status.outgoing() == b.status.outgoing()
        && b.time.saturating_sub(a.time) <= GROUP_SECONDS
        && present::same_day(calendar, a.time, b.time)
}

/// The name of the scroll area of the messages of `contact`.
fn scroll_id(contact: UiContactId) -> egui::Id {
    egui::Id::new(("thread-scroll", contact))
}

/// The messages of one conversation, as the list draws them, and what the
/// user asked for while they were drawn.
struct Messages<'a> {
    model: &'a mut Model,
    calendar: &'a dyn Calendar,
    theme: Arc<Theme>,
    contact: UiContactId,
    /// "Load earlier messages" was chosen.
    load: bool,
    /// "Show more" was chosen on this message.
    expand: Option<UiMessageId>,
}

/// About how high a message of `bytes` bytes is at `width`, its spacing
/// included, before it is laid out: rows of characters half the font size
/// wide, in a bubble no wider than the theme's.
fn estimate(theme: &Theme, width: f32, bytes: usize, new_day: bool, first: bool) -> f32 {
    let bubble = theme.size.bubble_max.min(width) - theme.space.md * 2.0;
    let per_row = (bubble * 2.0 / theme.text.body.size).max(1.0);
    let rows = (bytes as f32 / per_row).ceil().max(1.0);
    let mut height = rows * theme.text.body.line_height + theme.space.lg + theme.space.sm;
    height += if first {
        theme.space.md
    } else {
        theme.space.xxs
    };
    if new_day {
        height += theme.space.md + theme.space.xl + theme.space.lg * 2.0 + theme.space.sm;
    }
    height
}

impl message_list::Content for Messages<'_> {
    fn revision(&self) -> u64 {
        self.model.thread(self.contact).map_or(0, Thread::revision)
    }

    fn entries(&self, width: f32) -> Vec<(Key, f32)> {
        let Some(thread) = self.model.thread(self.contact) else {
            return Vec::new();
        };
        let mut previous: Option<&MessageView> = None;
        let mut entries = Vec::with_capacity(thread.messages.len());
        for message in &thread.messages {
            let new_day = previous
                .is_none_or(|before| !present::same_day(self.calendar, before.time, message.time));
            let first_in_group =
                previous.is_none_or(|before| !same_group(self.calendar, before, message));
            let key = Key {
                message: message.id,
                expanded: thread.expanded.contains(&message.id),
                new_day,
                first_in_group,
            };
            let height = estimate(
                &self.theme,
                width,
                message.text.len(),
                new_day,
                first_in_group,
            );
            entries.push((key, height));
            previous = Some(message);
        }
        entries
    }

    fn header(&mut self, ui: &mut Ui) {
        let theme = theme::current(ui.ctx());
        let Some(thread) = self.model.thread(self.contact) else {
            return;
        };
        if !thread.complete && !thread.messages.is_empty() {
            ui.vertical_centered(|ui| {
                if ui
                    .add(
                        Button::ghost("Load earlier messages")
                            .size(Size::Small)
                            .busy(thread.loading),
                    )
                    .clicked()
                {
                    self.load = true;
                }
            });
            ui.add_space(theme.space.sm);
        }
        if thread.messages.is_empty() && !thread.loading {
            empty_note(ui);
        }
    }

    fn message(&mut self, ui: &mut Ui, index: usize) {
        let theme = theme::current(ui.ctx());
        // Its text is made ready once, the first time it is drawn.
        self.model
            .prepare_messages(self.contact, index..index.saturating_add(1));
        let Some(thread) = self.model.thread(self.contact) else {
            return;
        };
        let messages = &thread.messages;
        let Some(message) = messages.get(index) else {
            return;
        };
        let Some(prepared) = thread.prepared(message.id) else {
            return;
        };
        let previous = index.checked_sub(1).and_then(|before| messages.get(before));
        let next = messages.get(index.saturating_add(1));
        let calendar = self.calendar;
        let new_day =
            previous.is_none_or(|before| !present::same_day(calendar, before.time, message.time));
        if new_day {
            ui.add_space(theme.space.md);
            let heading = present::day_heading(calendar, message.time);
            ui.add(DaySeparator::new(&heading));
        }
        let first_in_group = previous.is_none_or(|before| !same_group(calendar, before, message));
        let last_in_group = next.is_none_or(|after| !same_group(calendar, message, after));
        let direction = if message.status.outgoing() {
            Direction::Outgoing
        } else {
            Direction::Incoming
        };
        let clock = present::clock_time(calendar, message.time);
        let mut bubble = MessageBubble::new(message.id, direction, &prepared.text, &clock)
            .first_in_group(first_in_group)
            .last_in_group(last_in_group);
        if let Some((mark, label)) = mark(message.status) {
            bubble = bubble.status(mark, label);
        }
        if prepared.collapsed {
            bubble = bubble.collapsed("Show more");
        }
        let shown = bubble.show(ui);
        if shown.show_more {
            self.expand = Some(message.id);
        }
        // The menu is built only for the message it opens on.
        let response = &shown.response;
        if response.secondary_clicked()
            || Popup::is_id_open(ui.ctx(), Popup::default_response_id(response))
        {
            context_menu(response, |ui| {
                if ui
                    .add(MenuItem::new("Copy text").icon(icons::COPY))
                    .clicked()
                {
                    ui.ctx().copy_text(message.text.clone());
                }
            });
        }
    }
}

/// The list of messages, with "Load earlier messages" above it. Only the
/// messages in or near view are laid out (`message_list`).
fn messages(ui: &mut Ui, app: &mut App, contact: UiContactId, pad: f32) {
    let theme = theme::current(ui.ctx());
    let mut shown = Messages {
        model: &mut app.model,
        calendar: &*app.calendar,
        theme: theme.clone(),
        contact,
        load: false,
        expand: None,
    };
    message_list::show(ui, scroll_id(contact), pad, theme.space.md, &mut shown);
    let (load, expand) = (shown.load, shown.expand);
    if let Some(id) = expand {
        app.model.expand(id);
    }
    if load {
        app.model.load_earlier(contact);
    }
}

/// Forgets what the window keeps of the conversation of `contact`: what
/// egui keeps of its composer and of its list of messages, and the error
/// of its last send. For a conversation whose messages were deleted, or
/// whose contact or identity went.
pub(crate) fn forget(ctx: &egui::Context, contact: UiContactId) {
    let mut none = String::new();
    composer::forget(
        ctx,
        Composer::new(("composer", contact), &mut none).edit_id(),
    );
    message_list::forget(ctx, scroll_id(contact));
    ctx.data_mut(|data| data.remove::<Failure>(egui::Id::new(("send-error", contact))));
}

/// The note of a conversation without messages. What a contact that has
/// not accepted waits for is said once, by the notice above.
fn empty_note(ui: &mut Ui) {
    let theme = theme::current(ui.ctx());
    ui.add_space(theme.space.huge);
    ui.vertical_centered(|ui| {
        ui.add(Text::new(
            "No messages yet.",
            &theme.text.body_strong,
            theme.color.text_secondary,
        ));
    });
}

fn make_composer(
    text: &mut String,
    contact: UiContactId,
    sendable: bool,
    busy: bool,
    focus: bool,
) -> Composer<'_> {
    Composer::new(("composer", contact), text)
        .max_bytes(present::MAX_MESSAGE_BYTES)
        .sendable(sendable)
        .busy(busy)
        .request_focus(focus)
}

/// The composer at the bottom of the conversation of a contact, with the
/// error of the last send above it.
fn composer(ui: &mut Ui, app: &mut App, contact: UiContactId) {
    let theme = theme::current(ui.ctx());
    let ctx = ui.ctx().clone();
    let error_id = egui::Id::new(("send-error", contact));
    // Reading the draft for editing clears the model's error, so it is
    // kept here until the draft changes.
    if let Some(failure) = app
        .model
        .thread(contact)
        .and_then(|thread| thread.send_error)
    {
        ctx.data_mut(|data| data.insert_temp::<Failure>(error_id, failure));
    }
    let failure: Option<Failure> = ctx.data(|data| data.get_temp(error_id));
    let sending = app
        .model
        .thread(contact)
        .is_some_and(|thread| thread.sending.is_some());
    // After a failed store nothing can be sent; the composer gives no
    // reason of its own, as the window says it once.
    let storage = app.model.storage_failed();
    let sendable = app.model.can_send(contact);
    let focus = !storage && (app.focus_composer || app.last_conversation != Some(contact));
    if let Some(failure) = failure.filter(|failure| failure.said_in_place()) {
        ui.add(InlineError::new(failure.message()));
        ui.add_space(theme.space.sm);
    }
    let (send, changed) = {
        // The draft is edited in the model, also while a message is being
        // queued; without a thread read there is none.
        let mut none = String::new();
        let output = match app.model.draft_mut(contact) {
            Some(draft) => make_composer(draft, contact, sendable, sending, focus).show(ui),
            None => make_composer(&mut none, contact, sendable, sending, focus).show(ui),
        };
        (output.send_requested, output.changed)
    };
    if changed {
        // Whether the draft still starts with a text being queued.
        app.model.draft_edited(contact);
    }
    if changed || send {
        ctx.data_mut(|data| data.remove::<Failure>(error_id));
    }
    if send {
        app.model.send(contact);
    }
}

/// Draws the content pane when nothing is selected.
pub(crate) fn show_nothing_selected(ui: &mut Ui, app: &mut App) {
    let (tor, label) = common::tor_state(&app.model);
    let mut open_tor = false;
    TopBar::new("nothing-selected-bar").show(
        ui,
        |_| {},
        |ui| {
            open_tor = ui.add(TorStatus::new(tor, label)).clicked();
        },
    );
    if open_tor {
        app.model.open(Dialog::TorDetails);
    }
    let identity = app.model.current();
    let loaded = app.model.current_slice().is_some_and(|slice| slice.loaded);
    let empty = app
        .model
        .current_slice()
        .is_some_and(|slice| slice.contacts.is_empty() && slice.conversations.is_empty());
    match identity {
        None => {
            let output = EmptyState::new(
                "nothing-no-identity",
                icons::USER_PLUS,
                "Create your first identity",
            )
            .body("An identity is what your contacts know you by.")
            .primary("Create identity")
            .show(ui);
            if output.primary {
                app.model.open(Dialog::NewIdentity {
                    name: String::new(),
                    busy: None,
                    error: None,
                });
            }
        }
        Some(identity) if loaded && empty => {
            // Invite is in the strip under the list.
            let output = EmptyState::new("nothing-no-contacts", icons::USERS, "No contacts yet")
                .body(NO_CONTACTS)
                .primary("Add contact")
                .secondary("Share my card")
                .show(ui);
            if output.primary {
                app.model.open(Dialog::AddContact {
                    identity,
                    card: String::new(),
                    name: String::new(),
                    busy: None,
                    error: None,
                    added: None,
                });
            }
            if output.secondary {
                app.model.open(Dialog::OwnCard {
                    identity,
                    card: None,
                    busy: None,
                    error: None,
                });
            }
        }
        Some(_) => {
            EmptyState::new(
                "nothing-selected",
                icons::MESSAGE_SQUARE,
                "Select a conversation",
            )
            .body("Choose a contact on the left to read and write messages.")
            .show(ui);
        }
    }
}

/// The mark of a message, and its words for a screen reader and the
/// tooltip: exactly the status the runtime gave it. A received message has
/// none.
fn mark(status: Status) -> Option<(MarkStatus, &'static str)> {
    match status {
        Status::Queued => Some((MarkStatus::Queued, "Queued: not delivered yet")),
        Status::Delivered => Some((MarkStatus::Delivered, "Delivered")),
        Status::Received => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_marked_with_the_status_the_runtime_gave() {
        assert!(matches!(
            mark(Status::Queued),
            Some((MarkStatus::Queued, "Queued: not delivered yet"))
        ));
        assert!(matches!(
            mark(Status::Delivered),
            Some((MarkStatus::Delivered, "Delivered"))
        ));
        assert!(mark(Status::Received).is_none());
    }

    #[test]
    fn while_tor_is_down_the_wait_is_put_on_tor_not_on_the_contact() {
        for reason in [None, Some(Deferral::Tor), Some(Deferral::Unreachable)] {
            let text = deferral_text(reason, "Alice", true, false).unwrap_or_default();
            assert!(text.starts_with("Tor is not available"), "{text}");
            assert!(!text.contains("Alice"), "{text}");
            // The composer still queues: what waits is delivery.
            assert!(text.contains("nothing can be delivered"), "{text}");
        }
        for reason in [Deferral::Tor, Deferral::Unreachable] {
            let text = deferral_text(Some(reason), "Alice", false, false).unwrap_or_default();
            assert!(text.starts_with("Alice cannot be reached"), "{text}");
            assert!(text.ends_with(BOTH_OPEN), "{text}");
        }
    }

    #[test]
    fn the_next_try_is_said_roughly_and_said_again_when_the_words_change() {
        let said = |text: &str, change: u64| Some((text.to_owned(), change));
        assert_eq!(next_try(0), None);
        assert_eq!(next_try(1), said("Next try in less than a minute.", 1));
        assert_eq!(next_try(59), said("Next try in less than a minute.", 59));
        assert_eq!(next_try(60), said("Next try in about a minute.", 1));
        assert_eq!(next_try(89), said("Next try in about a minute.", 30));
        assert_eq!(next_try(90), said("Next try in about 2 minutes.", 1));
        assert_eq!(next_try(240), said("Next try in about 4 minutes.", 31));
        assert_eq!(next_try(2_700), said("Next try in about 45 minutes.", 31));
        // The words hold until the change, and not past it.
        for seconds in 1..3_000 {
            let (text, change) = next_try(seconds).unwrap_or_default();
            assert!((1..=seconds).contains(&change), "{seconds}");
            let held = seconds.saturating_sub(change).saturating_add(1);
            assert_eq!(next_try(held).map(|said| said.0), Some(text.clone()));
            let changed = next_try(seconds.saturating_sub(change)).map(|said| said.0);
            assert_ne!(changed, Some(text), "{seconds}");
        }
    }

    #[test]
    fn a_contact_that_has_not_accepted_has_no_notice_of_delivery() {
        assert_eq!(
            deferral_text(Some(Deferral::Unreachable), "Carol", false, true),
            None
        );
        assert_eq!(deferral_text(None, "Carol", false, false), None);
    }
}

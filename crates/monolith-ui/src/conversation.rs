//! A conversation: its header, its messages and the composer.

use egui::{Align, Layout, Margin, Sense, TopBottomPanel, Ui, UiBuilder, Vec2};
use monolith_ui_kit::avatar::{Avatar, AvatarSize};
use monolith_ui_kit::banner::{Banner, InlineError, Tone};
use monolith_ui_kit::bubble::{DaySeparator, Direction, MessageBubble};
use monolith_ui_kit::button::{Button, Size};
use monolith_ui_kit::chrome::{self, TopBar, scroll_area};
use monolith_ui_kit::composer::Composer;
use monolith_ui_kit::empty_state::EmptyState;
use monolith_ui_kit::icon_button::IconButton;
use monolith_ui_kit::menu::{MenuItem, context_menu, menu_below, menu_separator};
use monolith_ui_kit::message_status::Status as MarkStatus;
use monolith_ui_kit::text::Text;
use monolith_ui_kit::tor_status::TorStatus;
use monolith_ui_kit::{icons, theme};
use monolith_ui_model::{Deferral, Dialog, Failure, Status, UiContactId, UiMessageId, present};

use crate::app::App;
use crate::common;

/// Messages of one sender within this many seconds form a group.
const GROUP_SECONDS: u64 = 300;

/// What the "more" menu asked for.
#[derive(Clone, Copy)]
enum MenuAction {
    Details,
    Block,
    Remove,
    Delete,
}

/// What is remembered of the scroll position of a thread between frames:
/// the first message shown, the height of the content, the offset.
type Remembered = (Option<UiMessageId>, f32, f32);

/// Runs `add` in a column narrower than `ui` by `pad` on both sides.
fn inset<R>(ui: &mut Ui, pad: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
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

/// The words under the name in the header, if any.
fn status_line(ended: bool, accepted: bool, verified: bool) -> Option<&'static str> {
    if ended {
        Some("No longer a contact")
    } else if !accepted {
        Some("Waiting for them to accept your request")
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
    TopBottomPanel::bottom(egui::Id::new(("conversation-composer", contact)))
        .frame(chrome::surface_frame(
            ui.ctx(),
            Margin::from(Vec2::new(pad, theme.space.lg)),
        ))
        .show_separator_line(false)
        .show_inside(ui, |ui| composer(ui, app, contact));
    header(ui, app, contact, narrow);
    banners(ui, app, contact, pad);
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
    let line = status_line(ended, accepted, verified);
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
            ui.add(Avatar::new(&name, common::seed(&fingerprint)).size(AvatarSize::Medium));
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

/// The sentence for a delivery that waits.
fn deferral_text(reason: Deferral, name: &str) -> String {
    match reason {
        Deferral::Tor => {
            "Waiting for Tor. Your messages will be delivered when Tor is available again."
                .to_owned()
        }
        Deferral::Unreachable => {
            format!("Waiting for {name} to come online. Messages stay queued until then.")
        }
        Deferral::NoAddress => format!(
            "There is no address to send to {name} yet. Messages stay queued until there is."
        ),
        Deferral::NoAcknowledgement => {
            format!("{name} has not confirmed receipt yet. Monolith will send the messages again.")
        }
        Deferral::Busy => {
            "Monolith is busy with other conversations. Your messages will be sent shortly."
                .to_owned()
        }
    }
}

/// The notices under the bar.
fn banners(ui: &mut Ui, app: &App, contact: UiContactId, pad: f32) {
    let theme = theme::current(ui.ctx());
    let name = present::prepare(&app.model.contact_name(contact), false).text;
    let view = app.model.contact(contact);
    let ended = view.is_none();
    let waiting = view.is_some_and(|view| !view.accepted);
    let queued = app
        .model
        .conversation(contact)
        .is_some_and(|conversation| conversation.queued > 0);
    let deferral = app
        .model
        .current_slice()
        .and_then(|slice| slice.deferrals.get(&contact))
        .copied();
    let refused = app
        .model
        .thread(contact)
        .is_some_and(|thread| thread.refused);
    let mut notices: Vec<(&str, Tone, String)> = Vec::new();
    if app.model.storage_failed() {
        notices.push((
            "storage",
            Tone::Danger,
            "Monolith can no longer save. Restart Monolith to continue.".to_owned(),
        ));
    }
    if ended {
        notices.push((
            "ended",
            Tone::Info,
            "This contact was removed or blocked. You can read what is here, but not send messages."
                .to_owned(),
        ));
    }
    if waiting {
        notices.push((
            "waiting",
            Tone::Info,
            "Your request is sent with your first message. They also need to add your card, or have given you an invitation."
                .to_owned(),
        ));
    }
    if let (true, Some(reason)) = (queued && !ended, deferral) {
        notices.push(("deferral", Tone::Info, deferral_text(reason, &name)));
    }
    if refused {
        notices.push((
            "refused",
            Tone::Warning,
            format!("A message from {name} could not be saved on this device."),
        ));
    }
    if notices.is_empty() {
        return;
    }
    ui.add_space(theme.space.md);
    inset(ui, pad, |ui| {
        ui.spacing_mut().item_spacing.y = theme.space.sm;
        for (key, tone, text) in &notices {
            Banner::new(("conversation-banner", contact, *key), *tone, text).show(ui);
        }
    });
    ui.add_space(theme.space.sm);
}

/// Whether message `b` continues the group of `a`.
fn same_group(
    app: &App,
    a: &monolith_ui_model::MessageView,
    b: &monolith_ui_model::MessageView,
) -> bool {
    a.status.outgoing() == b.status.outgoing()
        && b.time.saturating_sub(a.time) <= GROUP_SECONDS
        && present::same_day(&*app.calendar, a.time, b.time)
}

/// The list of messages, with "Load earlier messages" above it.
fn messages(ui: &mut Ui, app: &mut App, contact: UiContactId, pad: f32) {
    let theme = theme::current(ui.ctx());
    let ctx = ui.ctx().clone();
    let scroll_id = egui::Id::new(("thread-scroll", contact));
    let memory_id = scroll_id.with("remembered");
    let remembered: Option<Remembered> = ctx.data(|data| data.get_temp(memory_id));
    let name = app.model.contact_name(contact);
    let waiting = app
        .model
        .contact(contact)
        .is_some_and(|view| !view.accepted);
    let mut load = false;
    let mut expand = None;
    let mut first = None;
    let mut area = scroll_area(scroll_id).stick_to_bottom(true);
    if let Some((_, _, offset)) = remembered {
        area = area.vertical_scroll_offset(offset);
    }
    let output = {
        let app: &App = app;
        area.show(ui, |ui| {
            ui.add_space(theme.space.md);
            let Some(thread) = app.model.thread(contact) else {
                return;
            };
            first = thread.messages.first().map(|message| message.id);
            inset(ui, pad, |ui| {
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
                            load = true;
                        }
                    });
                    ui.add_space(theme.space.sm);
                }
                if thread.messages.is_empty() && !thread.loading {
                    empty_note(ui, waiting, &name);
                }
                let mut previous: Option<&monolith_ui_model::MessageView> = None;
                let mut iter = thread.messages.iter().peekable();
                while let Some(message) = iter.next() {
                    let new_day = previous.is_none_or(|before| {
                        !present::same_day(&*app.calendar, before.time, message.time)
                    });
                    if new_day {
                        ui.add_space(theme.space.md);
                        let heading = present::day_heading(&*app.calendar, message.time);
                        ui.add(DaySeparator::new(&heading));
                    }
                    let first_in_group =
                        previous.is_none_or(|before| !same_group(app, before, message));
                    let last_in_group = iter
                        .peek()
                        .is_none_or(|next| !same_group(app, message, next));
                    let outgoing = message.status.outgoing();
                    let direction = if outgoing {
                        Direction::Outgoing
                    } else {
                        Direction::Incoming
                    };
                    let expanded = thread.expanded.contains(&message.id);
                    let prepared = present::prepare(&message.text, expanded);
                    let clock = present::clock_time(&*app.calendar, message.time);
                    let mut bubble =
                        MessageBubble::new(message.id, direction, &prepared.text, &clock)
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
                        expand = Some(message.id);
                    }
                    context_menu(&shown.response, |ui| {
                        if ui
                            .add(MenuItem::new("Copy text").icon(icons::COPY))
                            .clicked()
                        {
                            ui.ctx().copy_text(message.text.clone());
                        }
                    });
                    previous = Some(message);
                }
            });
            ui.add_space(theme.space.md);
        })
    };
    if let Some(id) = expand {
        app.model.expand(id);
    }
    if load {
        app.model.load_earlier(contact);
    }
    // Keep the reader where they are when older messages are put above.
    let mut state = output.state;
    if let Some((was_first, was_height, was_offset)) = remembered {
        let older = match (was_first, first) {
            (Some(before), Some(now)) => now.sequence() < before.sequence(),
            _ => false,
        };
        if older {
            let was_bottom = was_offset + output.inner_rect.height() >= was_height - 1.0;
            if !was_bottom {
                let grown = output.content_size.y - was_height;
                state.offset.y = was_offset + grown;
                state.store(&ctx, output.id);
                ctx.request_discard("older messages were added");
            }
        }
    }
    ctx.data_mut(|data| {
        data.insert_temp::<Remembered>(memory_id, (first, output.content_size.y, state.offset.y));
    });
}

/// The note of a conversation without messages.
fn empty_note(ui: &mut Ui, waiting: bool, name: &str) {
    let theme = theme::current(ui.ctx());
    ui.add_space(theme.space.huge);
    ui.vertical_centered(|ui| {
        ui.spacing_mut().item_spacing.y = theme.space.xs;
        ui.add(Text::new(
            "No messages yet.",
            &theme.text.body_strong,
            theme.color.text_secondary,
        ));
        if waiting {
            let text = format!("Write a message to send your request to {name}.");
            ui.add(Text::new(
                &text,
                &theme.text.caption,
                theme.color.text_tertiary,
            ));
        }
    });
}

fn make_composer<'a>(
    text: &'a mut String,
    contact: UiContactId,
    (sendable, reason): (bool, Option<&'a str>),
    busy: bool,
    focus: bool,
) -> Composer<'a> {
    Composer::new(("composer", contact), text)
        .max_bytes(present::MAX_MESSAGE_BYTES)
        .sendable(sendable)
        .blocked_reason(reason)
        .busy(busy)
        .request_focus(focus)
}

/// The composer at the bottom, with the error of the last send above it.
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
    let ended = app.model.contact(contact).is_none();
    let storage = app.model.storage_failed();
    let reason = if storage {
        Some("Monolith can no longer save. Restart Monolith to continue.")
    } else if ended {
        Some("This contact was removed or blocked.")
    } else {
        None
    };
    let sendable = app.model.can_send(contact);
    let focus =
        !ended && !storage && (app.focus_composer || app.last_conversation != Some(contact));
    let fallback = app
        .model
        .thread(contact)
        .map(|thread| thread.draft.clone())
        .unwrap_or_default();
    if let Some(failure) = failure {
        ui.add(InlineError::new(failure.message()));
        ui.add_space(theme.space.sm);
    }
    let (send, changed) = {
        let output = match app.model.draft_mut(contact) {
            Some(draft) => {
                make_composer(draft, contact, (sendable, reason), sending, focus).show(ui)
            }
            None => {
                // Being queued: the draft stays as it is.
                let mut shown = fallback;
                make_composer(&mut shown, contact, (sendable, reason), sending, focus).show(ui)
            }
        };
        (output.send_requested, output.changed)
    };
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
            let output = EmptyState::new("nothing-no-contacts", icons::USERS, "No contacts yet")
                .body("Add someone using their Monolith card or invitation.")
                .primary("Add contact")
                .secondary("Invite")
                .show(ui);
            if output.primary {
                app.model.open(Dialog::AddContact {
                    identity,
                    card: String::new(),
                    name: String::new(),
                    busy: None,
                    error: None,
                });
            }
            if output.secondary {
                app.model.open(Dialog::NewInvitation {
                    identity,
                    label: String::new(),
                    created: None,
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

/// The mark of a message: exactly the status the runtime gave it. A
/// received message has none.
fn mark(status: Status) -> Option<(MarkStatus, &'static str)> {
    match status {
        Status::Queued => Some((MarkStatus::Queued, "Queued")),
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
            Some((MarkStatus::Queued, "Queued"))
        ));
        assert!(matches!(
            mark(Status::Delivered),
            Some((MarkStatus::Delivered, "Delivered"))
        ));
        assert!(mark(Status::Received).is_none());
    }
}

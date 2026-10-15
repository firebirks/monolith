//! The dialogs of the model, each acting on the handles it captured.
//!
//! Every dialog is drawn from the state of the open [`Dialog`] and acts
//! through the model on the handles that dialog holds, never on whatever is
//! selected. Cards can carry an invitation: they stay in the dialog and in
//! the QR code of the app, and nowhere in egui's memory. The questions
//! asked before blocking or declining the sender of a request and before
//! unblocking someone are the window's own: each holds what it acts on in
//! [`App`] from the moment it opens.

use std::hash::Hash;

use egui::epaint::CornerRadiusF32;
use egui::epaint::tessellator::path;
use egui::{
    Align, Context, CornerRadius, Direction, Frame, Id, Layout, Sense, Shape, Stroke, Ui,
    UiBuilder, Vec2, ViewportCommand, WidgetType,
};
use monolith_ui_kit::avatar::{Avatar, AvatarSize};
use monolith_ui_kit::badge::{Pill, Tone as PillTone};
use monolith_ui_kit::banner::{Banner, InlineError, Tone};
use monolith_ui_kit::button::{Button, Size};
use monolith_ui_kit::checkbox::Checkbox;
use monolith_ui_kit::divider::Divider;
use monolith_ui_kit::fingerprint::Fingerprint;
use monolith_ui_kit::icons;
use monolith_ui_kit::interact;
use monolith_ui_kit::modal::{Decision, Modal};
use monolith_ui_kit::qr::{self, QrCode, QrMatrix};
use monolith_ui_kit::request_card::RequestAction;
use monolith_ui_kit::spinner;
use monolith_ui_kit::text::Text;
use monolith_ui_kit::text_field::{self, TextField};
use monolith_ui_kit::theme::{self, Theme};
use monolith_ui_kit::toast::Kind;
use monolith_ui_kit::toggle::Toggle;
use monolith_ui_kit::tor_status::TorStatus;
use monolith_ui_model::{
    Dialog, Failure, RequestMode, TorCheck, TorIndicator, UiContactId, UiIdentityId,
    UiInvitationId, UiRequestId, Waiting, present,
};
use zeroize::Zeroizing;

use crate::app::{App, CardFile, CloseWarning, Effect, SettingsPage, ShownQr, View, close_warning};
use crate::common;

/// The longest name, in characters.
const NAME_LIMIT: usize = 64;

/// The longest name of a card file shown, in characters.
const FILE_NAME_LIMIT: usize = 40;

/// Failures that belong to a name field.
const NAME_ERRORS: &[Failure] = &[Failure::InvalidText];

/// Failures that belong to the card field of "Add contact".
const CARD_ERRORS: &[Failure] = &[Failure::InvalidCard, Failure::OwnCard, Failure::Blocked];

/// What the card field of "Add contact" says under it. Written as two
/// sentences, so that each can stand alone once the field tells a card
/// from an invitation.
const CARD_HELP: &str = "With their invitation, your request reaches them. With their card only, \
                         they must add your card too.";

/// What sharing an own card does, under the default request mode
/// (`PROTOCOL.md` section 12).
const OWN_CARD: &str = "Share this card with people you want to talk to. It contains no secret. \
                        Someone who has only your card can reach you once you have added their \
                        card too. To let someone reach you first, give them an invitation \
                        instead.";

/// The same, while the identity takes requests from anyone.
const OWN_CARD_OPEN: &str = "Share this card with people you want to talk to. It contains no \
                             secret. Anyone who has it can send you a request.";

/// The same, while the identity takes no request: two people who add each
/// other's cards still become contacts (`PROTOCOL.md` section 6.4).
const OWN_CARD_NOBODY: &str = "Share this card with people you want to talk to. It contains no \
                               secret. Requests are set to Nobody, so no request arrives at all. \
                               Someone who has your card can reach you once you have added their \
                               card too.";

/// A QR code is shown, never scanned (`DESIGN_QUESTIONS.md` P6-11).
const NO_SCAN: &str = "Monolith cannot scan QR codes; send the text.";

/// What identities share and what they do not.
const IDENTITIES: &str = "Each identity has its own contacts, card and address, and is never \
                          mentioned to the contacts of another. Identities in the same Monolith \
                          are online at the same times, so someone who talks to two of them \
                          could guess they are the same person.";

/// An invitation admits requests until it is revoked (P4-12).
const INVITATION: &str = "Anyone who has this invitation can send you a request until you revoke \
                          it. Make one for each person, and revoke it once they are your contact.";

/// What invitations do while the identity takes no request: nothing, until
/// that changes (`PROTOCOL.md` section 12).
pub(crate) const INVITATIONS_NOBODY: &str = "Requests are set to Nobody: no request arrives, not \
                                             even through an invitation, until you change who \
                                             can send you requests. Make one for each person, \
                                             and revoke it once they are your contact.";

/// What an invitation made or shown again does, under the request mode
/// of its identity.
const INVITATION_CARD: &str = "Give this invitation to one person. Anyone who has it can send \
                               you a request until you revoke it.";

/// The same, while the identity takes no request.
const INVITATION_CARD_NOBODY: &str = "Give this invitation to one person. Requests are set to \
                                      Nobody: no request arrives, not even through an \
                                      invitation, until you change who can send you requests.";

/// How to check a fingerprint before accepting.
const CHECK_BEFORE_ACCEPT: &str = "Before you accept, ask the person to read you the fingerprint \
                                   under their identity in Monolith (Settings > Identities), on a \
                                   call or in person. It must match this one.";

/// How to check the fingerprint of a contact.
const CHECK_CONTACT: &str = "Ask the person to read you the fingerprint under their identity in \
                             Monolith (Settings > Identities), on a call or in person. If it \
                             matches this one, mark them verified.";

/// What blocking the sender of a request does.
const BLOCK_REQUEST: &str = "They will not be able to send you a request or a message. You can \
                             unblock them later in Settings > Identities.";

/// What declining a request does: their later requests are dropped, and
/// nothing shows whom the identity declined (`PROTOCOL.md` section 12).
/// Adding their card takes them off the declined list (P4-13).
const DECLINE_REQUEST: &str = "Nothing is sent to them. Their later requests are dropped without \
                               being shown, and Monolith does not show whom you declined, so \
                               this cannot be undone here. To talk with them later, add their \
                               card.";

/// What unblocking does: the store keeps nothing of someone unblocked, so
/// they can be blocked again only from a request or as a contact
/// (`DESIGN_QUESTIONS.md` P6-21).
const UNBLOCK: &str = "They can send you requests again, as \"Who can send you requests\" allows. \
                       Monolith keeps no record of them after this, so you can block them again \
                       only from a request they send or once you have added their card.";

/// What closing a session that saves nothing does.
const SAVES_NOTHING: &str = "This session saves nothing. Closing Monolith deletes your \
                             identities, contacts and messages for good.";

/// Draws the open dialog, if any.
pub(crate) fn show(ctx: &Context, app: &mut App) {
    let add_contact = matches!(
        app.model.dialog(),
        Some(Dialog::AddContact { added: None, .. })
    );
    track_card_field(ctx, app, add_contact);
    if !add_contact {
        app.card_file = None;
    }
    if !matches!(
        app.model.dialog(),
        Some(Dialog::OwnCard { .. } | Dialog::ShowInvitation { .. } | Dialog::NewInvitation { .. })
    ) {
        // A card is shown only while its dialog is open.
        app.drop_qr();
    }
    if app.model.dialog().is_some() {
        // A dialog of the model replaces the questions of the window.
        app.block_request = None;
        app.decline_request = None;
        app.unblock = None;
    }
    if app.closing.is_some() {
        // In place of any other dialog, which is back if the window stays.
        close_question(ctx, app);
        return;
    }
    match app.model.dialog() {
        None => {
            block_request(ctx, app);
            decline_request(ctx, app);
            unblock_question(ctx, app);
        }
        Some(Dialog::NewIdentity { .. }) => new_identity(ctx, app),
        Some(Dialog::AddContact {
            added: Some(contact),
            ..
        }) => {
            let contact = *contact;
            card_added(ctx, app, contact);
        }
        Some(Dialog::AddContact { .. }) => add_contact_dialog(ctx, app),
        Some(Dialog::OwnCard { .. }) => card_dialog(ctx, app, "Your card"),
        Some(Dialog::ShowInvitation { .. }) => card_dialog(ctx, app, "Invitation"),
        Some(Dialog::NewInvitation { created, .. }) => {
            if created.is_some() {
                card_dialog(ctx, app, "New invitation");
            } else {
                new_invitation(ctx, app);
            }
        }
        Some(Dialog::RevokeInvitation { invitation, .. }) => {
            let invitation = *invitation;
            let message = revoke_text(app, invitation);
            danger(
                ctx,
                app,
                &Danger {
                    salt: ("revoke-invitation", invitation),
                    title: "Revoke invitation?",
                    message,
                    action: "Revoke",
                    checkbox: true,
                },
            );
        }
        Some(Dialog::ContactDetails { .. }) => contact_details(ctx, app),
        Some(Dialog::BlockContact { contact, .. }) => {
            let contact = *contact;
            let name = app.model.contact_name(contact);
            danger(
                ctx,
                app,
                &Danger {
                    salt: ("block", contact),
                    title: "Block contact?",
                    message: format!(
                        "{name} will not be able to reach you. Messages waiting to be sent to them will not be sent. Your conversation stays on this device, under their fingerprint, until you delete it. You can unblock them later in Settings > Identities."
                    ),
                    action: "Block",
                    checkbox: false,
                },
            );
        }
        Some(Dialog::RemoveContact { contact, .. }) => {
            let contact = *contact;
            let name = app.model.contact_name(contact);
            danger(
                ctx,
                app,
                &Danger {
                    salt: ("remove", contact),
                    title: "Remove contact?",
                    message: format!(
                        "{name} is removed from your contacts. Messages not yet delivered to them will not be sent. The conversation stays on this device as Former contact and their fingerprint until you delete it. To talk again, add their card again; ask them for it if you no longer have it."
                    ),
                    action: "Remove",
                    checkbox: false,
                },
            );
        }
        Some(Dialog::DeleteConversation { contact, .. }) => {
            let contact = *contact;
            let message = delete_messages_text(app, contact);
            danger(
                ctx,
                app,
                &Danger {
                    salt: ("delete-messages", contact),
                    title: "Delete messages?",
                    message,
                    action: "Delete",
                    checkbox: false,
                },
            );
        }
        Some(Dialog::DeleteIdentity { identity, .. }) => {
            let identity = *identity;
            let name = app
                .model
                .identities()
                .iter()
                .find(|view| view.id == identity)
                .map_or_else(|| "This identity".to_owned(), present::identity_name);
            danger(
                ctx,
                app,
                &Danger {
                    salt: ("delete-identity", identity),
                    title: "Delete identity?",
                    message: format!(
                        "{name}, its contacts and its messages are deleted from this device, and its address stops working. Your contacts are not told; what they send to it will never arrive. Deleted data may stay on the disk until it is overwritten. This cannot be undone."
                    ),
                    action: "Delete identity",
                    checkbox: false,
                },
            );
        }
        Some(Dialog::AcceptRequest { .. }) => accept_request(ctx, app),
        Some(Dialog::TorDetails) => tor_details(ctx, app),
    }
}

// ---- What the dialogs say ----

/// What revoking `invitation` does, naming it by its label.
fn revoke_text(app: &App, invitation: UiInvitationId) -> String {
    let label = app
        .model
        .slice(invitation.identity())
        .and_then(|slice| slice.invitations.iter().find(|view| view.id == invitation))
        .and_then(|view| view.label.as_deref())
        .map(|label| present::prepare(label, false).text)
        .filter(|label| !label.trim().is_empty());
    let named = match label {
        Some(label) => format!("the invitation \"{label}\""),
        None => "this invitation".to_owned(),
    };
    format!(
        "People who have {named} can no longer send you requests with it. Contacts you already accepted are not affected."
    )
}

/// What deleting the messages of `contact` does, counting those that were
/// never delivered: they go too, and are never sent. A contact that has
/// not accepted gets its request only with a message (P6-6), so with them
/// goes the request too.
fn delete_messages_text(app: &App, contact: UiContactId) -> String {
    let name = app.model.contact_name(contact);
    let queued = app
        .model
        .conversation(contact)
        .map_or(0, |conversation| conversation.queued);
    let undelivered = if queued == 0 {
        String::new()
    } else {
        format!(", including {queued} not yet delivered, which will not be sent")
    };
    let waiting = app.model.waiting(contact).is_some() && queued > 0;
    let request = if waiting {
        " Your request goes out only with a message, so it is not sent either until you write again."
    } else {
        ""
    };
    format!(
        "All messages with {name} are deleted from this device{undelivered}.{request} Their copies are not affected. Deleted data may stay on the disk until it is overwritten."
    )
}

/// What closing costs, a paragraph for each part. Queued messages are sent
/// only while Monolith runs; a profile on disk tries them again when it
/// opens (`DESIGN_QUESTIONS.md` P5-8), a session that saves nothing loses
/// them.
fn close_text(warning: CloseWarning) -> Vec<String> {
    let mut paragraphs = Vec::new();
    if warning.loses_everything {
        paragraphs.push(SAVES_NOTHING.to_owned());
    }
    let count = warning.undelivered;
    let undelivered = match (count, warning.loses_everything) {
        (0, _) => None,
        (1, true) => Some("1 message has not been delivered yet and never will be.".to_owned()),
        (_, true) => Some(format!(
            "{count} messages have not been delivered yet and never will be."
        )),
        (1, false) => Some(
            "1 message has not been delivered yet. Monolith tries to send it again the next \
             time it is open."
                .to_owned(),
        ),
        (_, false) => Some(format!(
            "{count} messages have not been delivered yet. Monolith tries to send them again \
             the next time it is open."
        )),
    };
    paragraphs.extend(undelivered);
    paragraphs
}

/// The error of a dialog, split: the sentence of a failure that belongs to
/// the field of `field`, drawn under that field, and any other failure,
/// drawn as a line under the form.
fn split_error(
    error: Option<Failure>,
    field: &[Failure],
) -> (Option<&'static str>, Option<Failure>) {
    match error {
        Some(failure) if field.contains(&failure) => (Some(failure.message()), None),
        other => (None, other),
    }
}

// ---- Small pieces ----

fn paragraph(ui: &mut Ui, theme: &Theme, text: &str) {
    ui.add(Text::new(text, &theme.text.body, theme.color.text_secondary).wrap());
}

fn caption(ui: &mut Ui, theme: &Theme, text: &str) {
    ui.add(Text::new(text, &theme.text.caption, theme.color.text_tertiary).wrap());
}

fn field_label(ui: &mut Ui, theme: &Theme, text: &str) {
    ui.add(Text::new(
        text,
        &theme.text.caption_strong,
        theme.color.text_secondary,
    ));
}

/// A sunken box across the width, with `add` inside.
fn sunken(ui: &mut Ui, theme: &Theme, add: impl FnOnce(&mut Ui)) {
    let width = ui.available_width();
    let margin = theme.space.md;
    Frame::new()
        .fill(theme.color.bg_sunken)
        .stroke(Stroke::new(
            theme.stroke.hairline,
            theme.color.border_subtle,
        ))
        .corner_radius(CornerRadius::same(theme.radius.md))
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width(width - margin * 2.0);
            add(ui);
        });
}

/// Mono text, wrapped and selectable, in a sunken box.
fn mono_box(ui: &mut Ui, theme: &Theme, text: &str) {
    sunken(ui, theme, |ui| {
        ui.add(
            Text::new(text, &theme.text.mono, theme.color.text)
                .selectable(true)
                .wrap(),
        );
    });
}

/// A fingerprint in a sunken box, in the rows it has everywhere.
fn fingerprint_box(ui: &mut Ui, theme: &Theme, fingerprint: &str) {
    sunken(ui, theme, |ui| {
        ui.add(Fingerprint::new(fingerprint));
    });
}

/// A progress mark with a line beside it.
fn loading(ui: &mut Ui, theme: &Theme, text: &str) {
    ui.horizontal(|ui| {
        let size = theme.size.icon_md;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        spinner::paint(ui, rect.center(), size, theme.color.accent);
        ui.add(Text::new(
            text,
            &theme.text.body,
            theme.color.text_secondary,
        ));
    });
}

/// A failure that belongs to no field, as a line under the form. A failed
/// store is not: the window says it, once.
fn error_line(ui: &mut Ui, error: Option<Failure>) {
    if let Some(failure) = error.filter(|failure| failure.said_in_place()) {
        ui.add(InlineError::new(failure.message()));
    }
}

/// The main action and a cancel in the footer, the main action at the
/// right; what was clicked.
fn actions(ui: &mut Ui, main: Button<'_>, cancel: &str, busy: bool) -> (bool, bool) {
    let main = ui.add(main).clicked();
    let cancel = ui.add(Button::secondary(cancel).enabled(!busy)).clicked();
    (main, cancel)
}

/// The state every dialog shares: whether its action runs, and its error.
fn status(dialog: &Dialog) -> (bool, Option<Failure>) {
    match dialog {
        Dialog::NewIdentity { busy, error, .. }
        | Dialog::AddContact { busy, error, .. }
        | Dialog::OwnCard { busy, error, .. }
        | Dialog::NewInvitation { busy, error, .. }
        | Dialog::ShowInvitation { busy, error, .. }
        | Dialog::RevokeInvitation { busy, error, .. }
        | Dialog::ContactDetails { busy, error, .. }
        | Dialog::BlockContact { busy, error, .. }
        | Dialog::RemoveContact { busy, error, .. }
        | Dialog::DeleteConversation { busy, error, .. }
        | Dialog::AcceptRequest { busy, error, .. }
        | Dialog::DeleteIdentity { busy, error, .. } => (busy.is_some(), *error),
        Dialog::TorDetails => (false, None),
    }
}

/// Closes the dialog (unless it waits for its action) and drops the card.
fn close(app: &mut App) {
    app.model.close();
    if app.model.dialog().is_none() {
        app.drop_qr();
    }
}

/// Nothing has the keyboard focus: a field of a dialog that just opened
/// takes it.
fn nothing_focused(ctx: &Context) -> bool {
    ctx.memory(|memory| memory.focused().is_none())
}

fn card_field_id(identity: UiIdentityId) -> Id {
    TextField::new(("add-contact-card", identity), &mut String::new()).edit_id()
}

fn path_field_id(identity: UiIdentityId) -> Id {
    TextField::new(("add-contact-path", identity), &mut String::new()).edit_id()
}

/// The card typed in "Add contact" can be an invitation: when the dialog is
/// gone, what egui kept of the field (cursor, undo history) goes too.
fn track_card_field(ctx: &Context, app: &mut App, open: bool) {
    let key = Id::new("monolith-dialog-card-field");
    let known = ctx.data(|data| data.get_temp::<Id>(key));
    if open {
        if let Some(Dialog::AddContact { identity, .. }) = app.model.dialog() {
            let id = card_field_id(*identity);
            if known != Some(id) {
                ctx.data_mut(|data| data.insert_temp(key, id));
            }
        }
    } else if let Some(id) = known {
        text_field::forget(ctx, id);
        ctx.data_mut(|data| data.remove::<Id>(key));
        app.card_path.clear();
    }
}

/// Where a card file can be dropped: a dashed frame that lights up while a
/// file is dragged over the window. Once a card was read from a file, the
/// frame holds the name of that file. There is no file picker
/// (`DESIGN_QUESTIONS.md` P6-15): a click on the frame goes to the field of
/// the path, below it, and is no stop of the Tab key. Returns whether it
/// was clicked; it takes no click while the dialog waits.
fn drop_zone(ui: &mut Ui, theme: &Theme, busy: bool, hovering: bool, read: Option<&str>) -> bool {
    const WORDS: &str = "Drop a card file here";
    let size = Vec2::new(ui.available_width(), theme.size.control_lg);
    let sense = if busy { Sense::hover() } else { Sense::CLICK };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let read = read.filter(|_| !hovering);
    let label = read.map_or_else(|| WORDS.to_owned(), |name| format!("Card read from {name}"));
    interact::describe(&response, WidgetType::Label, &label);
    if !ui.is_rect_visible(rect) {
        return response.clicked();
    }
    let state = interact::State::of(ui, &response, response.id, theme, false);
    let c = &theme.color;
    let (fill, edge, ink) = if hovering {
        (c.accent_subtle, c.accent, c.accent_text)
    } else {
        (c.bg_sunken, c.border_strong, c.text_secondary)
    };
    let radius = CornerRadius::same(theme.radius.md);
    let painter = ui.painter().clone();
    painter.rect_filled(rect, radius, fill);
    painter.rect_filled(rect, radius, c.hover.gamma_multiply(state.hover));
    let mut outline = Vec::new();
    path::rounded_rectangle(
        &mut outline,
        rect.shrink(theme.stroke.hairline / 2.0),
        CornerRadiusF32::from(radius),
    );
    if let Some(first) = outline.first().copied() {
        outline.push(first);
    }
    painter.extend(Shape::dashed_line(
        &outline,
        Stroke::new(theme.stroke.hairline, edge),
        theme.space.sm,
        theme.space.xs,
    ));
    if let Some(name) = read {
        let mut inside = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::centered_and_justified(Direction::LeftToRight)),
        );
        inside.add(Pill::new(PillTone::Success, name).icon(icons::CIRCLE_CHECK));
        return response.clicked();
    }
    let galley = painter.layout_no_wrap(WORDS.to_owned(), theme.text.body.font_id(), ink);
    let icon = theme.size.icon;
    let gap = theme.space.sm;
    let left = rect.center().x - (icon + gap + galley.size().x) / 2.0;
    icons::FILE_TEXT.paint(
        &painter,
        egui::pos2(left + icon / 2.0, rect.center().y),
        icon,
        ink,
    );
    let top = rect.center().y - galley.size().y / 2.0;
    painter.galley(egui::pos2(left + icon + gap, top), galley, ink);
    response.clicked()
}

// ---- Dialogs ----

fn new_identity(ctx: &Context, app: &mut App) {
    let theme = theme::current(ctx);
    let focus = nothing_focused(ctx);
    // Making an identity publishes its service, which takes Tor.
    let no_tor = app.model.tor_indicator() == TorIndicator::Unavailable;
    let Some(Dialog::NewIdentity { name, busy, error }) = app.model.dialog_mut() else {
        return;
    };
    let busy = busy.is_some();
    let (name_error, error) = split_error(*error, NAME_ERRORS);
    let (mut main, mut cancel, mut submitted, mut tor) = (false, false, false, false);
    let out = Modal::new("new-identity", "New identity")
        .dismissable(!busy)
        .show(
            ctx,
            |ui| {
                let output = TextField::new("new-identity-name", name)
                    .label("Name (only you see it)")
                    .char_limit(NAME_LIMIT)
                    .enabled(!busy)
                    .error(name_error)
                    .request_focus(focus)
                    .show(ui);
                submitted = output.submitted;
                ui.add_space(theme.space.xs);
                paragraph(ui, &theme, IDENTITIES);
                if no_tor && !busy {
                    ui.add_space(theme.space.xs);
                    tor = Banner::new(
                        "new-identity-tor",
                        Tone::Warning,
                        "Monolith needs Tor to make an identity, and Tor is not available now.",
                    )
                    .action("Tor details")
                    .show(ui)
                    .action;
                }
                error_line(ui, error);
            },
            |ui| {
                let create = Button::primary("Create").enabled(!no_tor).busy(busy);
                (main, cancel) = actions(ui, create, "Cancel", busy);
            },
        );
    if tor {
        app.model.open(Dialog::TorDetails);
    } else if (main || submitted) && !busy && !no_tor {
        app.model.confirm();
    } else if cancel || out.close_requested {
        close(app);
    }
}

fn add_contact_dialog(ctx: &Context, app: &mut App) {
    let theme = theme::current(ctx);
    let focus = nothing_focused(ctx);
    let Some(Dialog::AddContact { identity, .. }) = app.model.dialog() else {
        return;
    };
    let identity = *identity;
    let busy = status_busy(app);
    let mut open_path = None;
    if !busy {
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        if let Some(path) = dropped.into_iter().find_map(|file| file.path) {
            open_path = Some(path);
        }
    }
    let hovering = !busy && ctx.input(|input| !input.raw.hovered_files.is_empty());
    let App {
        model,
        card_path,
        card_file,
        ..
    } = app;
    let Some(Dialog::AddContact {
        card, name, error, ..
    }) = model.dialog_mut()
    else {
        return;
    };
    // The card asked for has arrived: the field was emptied when it was.
    if let Some(file) = card_file.as_mut() {
        if !file.read && !card.trim().is_empty() {
            file.read = true;
        }
    }
    let read = card_file
        .as_ref()
        .filter(|file| file.read)
        .map(|file| file.name.clone());
    let (card_error, error) = split_error(*error, CARD_ERRORS);
    let (name_error, error) = split_error(error, NAME_ERRORS);
    let has_card = !card.trim().is_empty();
    let (mut main, mut cancel, mut submitted, mut open_clicked) = (false, false, false, false);
    let mut typed = false;
    let out = Modal::new(("add-contact", identity), "Add contact")
        .dismissable(!busy)
        .show(
            ctx,
            |ui| {
                // An invitation in it is a capability: no copy of it stays
                // in an undo history.
                let output = TextField::new(("add-contact-card", identity), card)
                    .label("Card")
                    .hint("Paste their card or invitation")
                    .help(Some(CARD_HELP))
                    .error(card_error)
                    .enabled(!busy)
                    .no_history(true)
                    .request_focus(focus)
                    .show(ui);
                submitted |= output.submitted && has_card;
                typed = output.changed;
                ui.add_space(theme.space.xs);
                if drop_zone(ui, &theme, busy, hovering, read.as_deref()) {
                    ui.memory_mut(|memory| memory.request_focus(path_field_id(identity)));
                }
                ui.allocate_ui_with_layout(
                    Vec2::new(ui.available_width(), theme.size.control),
                    Layout::right_to_left(Align::Center),
                    |ui| {
                        let can_open = !busy && !card_path.trim().is_empty();
                        open_clicked = ui
                            .add(Button::secondary("Open").enabled(can_open))
                            .clicked();
                        let width = ui.available_width();
                        let output = TextField::new(("add-contact-path", identity), card_path)
                            .hint("or type its path")
                            .enabled(!busy)
                            .width(width)
                            .show(ui);
                        if output.submitted && can_open {
                            open_clicked = true;
                        }
                    },
                );
                ui.add_space(theme.space.xs);
                let output = TextField::new(("add-contact-name", identity), name)
                    .label("Name (only you see it)")
                    .char_limit(NAME_LIMIT)
                    .error(name_error)
                    .enabled(!busy)
                    .show(ui);
                submitted |= output.submitted && has_card;
                error_line(ui, error);
            },
            |ui| {
                (main, cancel) = actions(
                    ui,
                    Button::primary("Add contact").enabled(has_card).busy(busy),
                    "Cancel",
                    busy,
                );
            },
        );
    if typed {
        // The card in the field is no longer the one of the file.
        app.card_file = None;
    }
    let requested = open_clicked || open_path.is_some();
    let path = if open_clicked {
        Some(std::path::PathBuf::from(app.card_path.trim()))
    } else {
        open_path
    };
    if let Some(path) = path {
        app.card_path = path.display().to_string();
        // The result is for this dialog, and for no other opened later.
        if let Some(dialog) = app.model.dialog_instance() {
            app.effect(Effect::ReadCard {
                path,
                identity,
                dialog,
            });
        }
    }
    if requested {
        // The field is emptied so that the card of the file is told from
        // what was there; the file's name is shown once it has arrived.
        if let Some(Dialog::AddContact { card, .. }) = app.model.dialog_mut() {
            card.clear();
        }
        let name = std::path::Path::new(&app.card_path)
            .file_name()
            .map(|name| present::preview(&name.to_string_lossy(), FILE_NAME_LIMIT))
            .unwrap_or_default();
        app.card_file = Some(CardFile { name, read: false });
        return;
    }
    if (main || submitted) && has_card && !busy {
        app.model.confirm();
    } else if cancel || out.close_requested {
        close(app);
    }
}

/// What adding a card that is not an invitation leaves to the other side,
/// with the way to the card of this identity (`PROTOCOL.md` section 12.2).
fn card_added(ctx: &Context, app: &mut App, contact: UiContactId) {
    let theme = theme::current(ctx);
    // Named as listed once the identity is read again; until then by the
    // name typed, if any.
    let typed = match app.model.dialog() {
        Some(Dialog::AddContact { name, .. }) => name.trim().to_owned(),
        _ => String::new(),
    };
    let name = match app.model.contact(contact) {
        Some(view) => present::contact_name(view),
        None if !typed.is_empty() => typed,
        None => "They".to_owned(),
    };
    let name = present::prepare(&name, false).text;
    let text = format!(
        "This is their card, not an invitation. {name} must add your card too before your messages can arrive."
    );
    let (mut show_card, mut done) = (false, false);
    let out = Modal::new(("card-added", contact), "Contact added").show(
        ctx,
        |ui| paragraph(ui, &theme, &text),
        |ui| {
            show_card = ui
                .add(Button::primary("Show my card").icon(icons::ID_CARD))
                .clicked();
            done = ui.add(Button::secondary("Done")).clicked();
        },
    );
    if show_card {
        app.model.close();
        app.model.open(Dialog::OwnCard {
            identity: contact.identity(),
            card: None,
            busy: None,
            error: None,
        });
    } else if done || out.close_requested {
        close(app);
    }
}

fn status_busy(app: &App) -> bool {
    app.model.dialog().is_some_and(|dialog| status(dialog).0)
}

/// What a card dialog shows.
fn card_state(dialog: Option<&Dialog>) -> (Option<&str>, bool, Option<Failure>) {
    match dialog {
        Some(
            Dialog::OwnCard {
                card, busy, error, ..
            }
            | Dialog::ShowInvitation {
                card, busy, error, ..
            },
        ) => (
            card.as_ref().map(|text| text.as_str()),
            busy.is_some(),
            *error,
        ),
        Some(Dialog::NewInvitation {
            created,
            busy,
            error,
            ..
        }) => (
            created.as_ref().map(|(_, text)| text.as_str()),
            busy.is_some(),
            *error,
        ),
        _ => (None, false, None),
    }
}

/// Keeps `shown` in step with `text`: rebuilt only if the text differs.
fn sync_qr(shown: &mut Option<ShownQr>, text: Option<&str>) {
    let Some(text) = text else {
        *shown = None;
        return;
    };
    if shown.as_ref().is_some_and(|qr| qr.text.as_str() == text) {
        return;
    }
    *shown = if text.len() > qr::MAX_BYTES {
        None
    } else {
        QrMatrix::encode(text).ok().map(|matrix| ShownQr {
            text: Zeroizing::new(text.to_owned()),
            matrix,
        })
    };
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CardAction {
    None,
    Copy,
    Save,
}

/// An own card or an invitation, with its QR code.
fn card_dialog(ctx: &Context, app: &mut App, title: &str) {
    let theme = theme::current(ctx);
    let own = match app.model.dialog() {
        Some(Dialog::OwnCard { identity, .. }) => Some(*identity),
        _ => None,
    };
    let invitation = own.is_none();
    let mode = own.and_then(|identity| app.model.slice(identity)?.request_mode);
    // An invitation does nothing while its identity takes no request.
    let inviting = match app.model.dialog() {
        Some(Dialog::NewInvitation { identity, .. }) => Some(*identity),
        Some(Dialog::ShowInvitation { invitation, .. }) => Some(invitation.identity()),
        _ => None,
    };
    let refusing = inviting.and_then(|identity| app.model.slice(identity)?.request_mode)
        == Some(RequestMode::Closed);
    // The fingerprint of the identity, which those who add the card check.
    let fingerprint = own.and_then(|identity| {
        app.model
            .identities()
            .iter()
            .find(|view| view.id == identity)
            .map(|view| view.full_fingerprint.clone())
    });
    let salt = match app.model.dialog() {
        Some(Dialog::OwnCard { identity, .. }) => Id::new(("own-card", *identity)),
        Some(Dialog::ShowInvitation { invitation, .. }) => Id::new(("invitation", *invitation)),
        Some(Dialog::NewInvitation {
            created: Some((id, _)),
            ..
        }) => Id::new(("invitation", *id)),
        _ => Id::new("card"),
    };
    let (text, busy, error) = card_state(app.model.dialog());
    sync_qr(&mut app.qr, text);
    let matrix = app.qr.as_ref().map(|shown| &shown.matrix);
    let sentence = if invitation && refusing {
        INVITATION_CARD_NOBODY
    } else if invitation {
        INVITATION_CARD
    } else {
        match mode {
            Some(RequestMode::Open) => OWN_CARD_OPEN,
            Some(RequestMode::Closed) => OWN_CARD_NOBODY,
            Some(RequestMode::Invitation) | None => OWN_CARD,
        }
    };
    let qr_label = if invitation {
        "QR code of the invitation"
    } else {
        "QR code of your card"
    };
    let mut action = CardAction::None;
    let mut save = None;
    let mut done = false;
    let out = Modal::new(salt, title).wide(true).show(
        ctx,
        |ui| {
            let Some(text) = text else {
                if busy {
                    loading(ui, &theme, "Reading the card...");
                }
                error_line(ui, error);
                return;
            };
            paragraph(ui, &theme, sentence);
            ui.add_space(theme.space.xs);
            match (matrix, &fingerprint) {
                // The fingerprint beside the code, so that the dialog fits
                // the smallest window.
                (Some(matrix), Some(fingerprint)) => {
                    ui.horizontal_top(|ui| {
                        ui.add(QrCode::new(matrix, qr_label).max_side(theme.size.qr));
                        ui.add_space(theme.space.sm);
                        ui.vertical(|ui| {
                            field_label(ui, &theme, "Your fingerprint");
                            fingerprint_box(ui, &theme, fingerprint);
                            caption(ui, &theme, NO_SCAN);
                        });
                    });
                }
                (Some(matrix), None) => {
                    ui.vertical_centered(|ui| {
                        ui.add(QrCode::new(matrix, qr_label).max_side(theme.size.qr));
                        caption(ui, &theme, NO_SCAN);
                    });
                }
                (None, Some(fingerprint)) => {
                    field_label(ui, &theme, "Your fingerprint");
                    fingerprint_box(ui, &theme, fingerprint);
                }
                (None, None) => {}
            }
            ui.add_space(theme.space.xs);
            mono_box(ui, &theme, text);
            error_line(ui, error);
        },
        |ui| {
            if let Some(text) = text {
                if ui
                    .add(Button::primary("Copy card").icon(icons::COPY))
                    .clicked()
                {
                    ctx.copy_text(text.to_owned());
                    action = CardAction::Copy;
                }
                if ui
                    .add(Button::secondary("Save card").icon(icons::DOWNLOAD))
                    .clicked()
                {
                    save = Some(Zeroizing::new(text.to_owned()));
                    action = CardAction::Save;
                }
            }
            done = ui.add(Button::secondary("Close")).clicked();
        },
    );
    match action {
        CardAction::Copy => {
            app.toasts.push(Kind::Success, "Card copied.");
        }
        CardAction::Save => {
            if let Some(contents) = save {
                let name = if invitation {
                    "monolith-invitation.txt"
                } else {
                    "monolith-card.txt"
                };
                app.effect(Effect::SaveCard {
                    name: name.to_owned(),
                    contents,
                });
            }
        }
        CardAction::None => {}
    }
    if done || out.close_requested {
        close(app);
    }
}

fn new_invitation(ctx: &Context, app: &mut App) {
    let theme = theme::current(ctx);
    let focus = nothing_focused(ctx);
    // What an invitation does under the request mode of the identity.
    let said = match app.model.dialog() {
        Some(Dialog::NewInvitation { identity, .. })
            if app
                .model
                .slice(*identity)
                .and_then(|slice| slice.request_mode)
                == Some(RequestMode::Closed) =>
        {
            INVITATIONS_NOBODY
        }
        _ => INVITATION,
    };
    let Some(Dialog::NewInvitation {
        identity,
        label,
        busy,
        error,
        ..
    }) = app.model.dialog_mut()
    else {
        return;
    };
    let identity = *identity;
    let busy = busy.is_some();
    let (label_error, error) = split_error(*error, NAME_ERRORS);
    let (mut main, mut cancel, mut submitted) = (false, false, false);
    let out = Modal::new(("new-invitation", identity), "New invitation")
        .dismissable(!busy)
        .show(
            ctx,
            |ui| {
                paragraph(ui, &theme, said);
                ui.add_space(theme.space.xs);
                let output = TextField::new(("new-invitation-label", identity), label)
                    .label("Label (only you see it)")
                    .hint("For Dana")
                    .char_limit(NAME_LIMIT)
                    .error(label_error)
                    .enabled(!busy)
                    .request_focus(focus)
                    .show(ui);
                submitted = output.submitted;
                error_line(ui, error);
            },
            |ui| {
                (main, cancel) = actions(ui, Button::primary("Create").busy(busy), "Cancel", busy);
            },
        );
    if (main || submitted) && !busy {
        app.model.confirm();
    } else if cancel || out.close_requested {
        close(app);
    }
}

/// What a confirmation says and does.
struct Danger<S> {
    salt: S,
    title: &'static str,
    message: String,
    action: &'static str,
    /// Also shows the choice of revoking an invitation: drop its requests
    /// too.
    checkbox: bool,
}

fn danger<S: Hash>(ctx: &Context, app: &mut App, spec: &Danger<S>) {
    let theme = theme::current(ctx);
    let Some((busy, error)) = app.model.dialog().map(status) else {
        return;
    };
    let focus = nothing_focused(ctx);
    let (mut confirmed, mut cancelled) = (false, false);
    let model = &mut app.model;
    let out = Modal::new(&spec.salt, spec.title).dismissable(!busy).show(
        ctx,
        |ui| {
            paragraph(ui, &theme, &spec.message);
            if spec.checkbox {
                if let Some(Dialog::RevokeInvitation {
                    invitation,
                    discard_requests,
                    ..
                }) = model.dialog_mut()
                {
                    ui.add_space(theme.space.xs);
                    ui.add(
                        Checkbox::new(
                            discard_requests,
                            "Also drop the requests it brought that are still waiting",
                        )
                        .enabled(!busy)
                        .id_salt(*invitation),
                    );
                }
            }
            error_line(ui, error);
        },
        |ui| {
            confirmed = ui
                .add(
                    Button::danger(spec.action)
                        .busy(busy)
                        .min_width(theme.size.control_lg),
                )
                .clicked();
            let cancel = ui.add(Button::secondary("Cancel").enabled(!busy));
            if focus {
                // Enter must not destroy anything.
                cancel.request_focus();
            }
            cancelled = cancel.clicked();
        },
    );
    if confirmed && !busy {
        app.model.confirm();
    } else if cancelled || out.close_requested {
        close(app);
    }
}

/// Whether a question about `request` can be answered: it is waiting, and
/// no answer to it is under way.
fn open_to_answer(app: &App, request: UiRequestId) -> bool {
    let waiting = app
        .model
        .slice(request.identity())
        .is_some_and(|slice| slice.requests.iter().any(|view| view.id == request));
    waiting && !app.model.answering(request)
}

/// Asks before the sender of a request is blocked. The question holds the
/// request it was asked for, whatever the list shows when it is answered;
/// it goes away if that request is no longer waiting.
fn block_request(ctx: &Context, app: &mut App) {
    let Some(request) = app.block_request else {
        return;
    };
    if !open_to_answer(app, request) {
        app.block_request = None;
        return;
    }
    let decision = Modal::new(("block-request", request), "Block this person?").confirm(
        ctx,
        BLOCK_REQUEST,
        "Cancel",
        "Block",
        false,
    );
    match decision {
        Some(Decision::Confirm) => {
            app.block_request = None;
            app.request_answers.insert(request, RequestAction::Block);
            app.model.block_sender(request);
        }
        Some(Decision::Cancel) => app.block_request = None,
        None => {}
    }
}

/// Asks before a request is declined: the sender's later requests are
/// dropped, and nothing in the client shows or undoes a decline. Like the
/// question of a block, it holds the request it was asked for.
fn decline_request(ctx: &Context, app: &mut App) {
    let Some(request) = app.decline_request else {
        return;
    };
    if !open_to_answer(app, request) {
        app.decline_request = None;
        return;
    }
    let decision = Modal::new(("decline-request", request), "Decline this request?").confirm(
        ctx,
        DECLINE_REQUEST,
        "Cancel",
        "Decline",
        false,
    );
    match decision {
        Some(Decision::Confirm) => {
            app.decline_request = None;
            app.request_answers.insert(request, RequestAction::Decline);
            app.model.decline(request);
        }
        Some(Decision::Cancel) => app.decline_request = None,
        None => {}
    }
}

/// Asks before someone is unblocked: nothing is kept of them afterwards.
/// The question holds whom it was asked for, and goes away once they are
/// no longer listed or are being unblocked.
fn unblock_question(ctx: &Context, app: &mut App) {
    let Some(blocked) = app.unblock else {
        return;
    };
    let listed = app
        .model
        .slice(blocked.identity())
        .is_some_and(|slice| slice.blocked.iter().any(|view| view.id == blocked));
    if !listed || app.model.unblocking(blocked) {
        app.unblock = None;
        return;
    }
    let decision = Modal::new(("unblock", blocked), "Unblock this person?")
        .confirm(ctx, UNBLOCK, "Cancel", "Unblock", false);
    match decision {
        Some(Decision::Confirm) => {
            app.unblock = None;
            app.model.unblock(blocked);
        }
        Some(Decision::Cancel) => app.unblock = None,
        None => {}
    }
}

/// Asks before the window closes, while closing would lose a session that
/// saves nothing or leave messages undelivered. Close Monolith closes the
/// window, which asks no more; Cancel, Escape or a click beside the
/// question keeps it open.
fn close_question(ctx: &Context, app: &mut App) {
    let Some(asked) = app.closing else {
        return;
    };
    // What closing costs now; what it cost when asked, if nothing now.
    let warning = close_warning(&app.model).unwrap_or(asked);
    app.closing = Some(warning);
    let theme = theme::current(ctx);
    let focus = nothing_focused(ctx);
    let (mut confirmed, mut cancelled) = (false, false);
    let out = Modal::new("close-monolith", "Close Monolith?").show(
        ctx,
        |ui| {
            for text in close_text(warning) {
                paragraph(ui, &theme, &text);
            }
        },
        |ui| {
            let close = if warning.loses_everything {
                Button::danger("Close Monolith")
            } else {
                Button::primary("Close Monolith")
            };
            confirmed = ui.add(close.min_width(theme.size.control_lg)).clicked();
            let cancel = ui.add(Button::secondary("Cancel"));
            if focus {
                // Enter must not close.
                cancel.request_focus();
            }
            cancelled = cancel.clicked();
        },
    );
    if confirmed {
        app.allow_close();
        ctx.send_viewport_cmd(ViewportCommand::Close);
    } else if cancelled || out.close_requested {
        app.closing = None;
    }
}

/// What the details dialog shows of a contact, copied out of the model.
struct ContactInfo {
    shown_name: String,
    /// It has a name of its own, not one made of its fingerprint.
    named: bool,
    current_name: String,
    fingerprint: String,
    full_fingerprint: String,
    waiting: Option<Waiting>,
    verified: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Next {
    Stay,
    Block,
    Remove,
    DeleteMessages,
}

fn contact_info(app: &App, contact: UiContactId) -> Option<ContactInfo> {
    app.model.contact(contact).map(|view| ContactInfo {
        shown_name: present::contact_name(view),
        named: present::named(view),
        current_name: view.name.clone().unwrap_or_default(),
        fingerprint: view.fingerprint.clone(),
        full_fingerprint: view.full_fingerprint.clone(),
        waiting: app.model.waiting(contact),
        verified: view.verified,
    })
}

fn contact_details(ctx: &Context, app: &mut App) {
    let theme = theme::current(ctx);
    let Some(Dialog::ContactDetails { contact, .. }) = app.model.dialog() else {
        return;
    };
    let contact = *contact;
    let info = contact_info(app, contact);
    let Some(Dialog::ContactDetails {
        name, busy, error, ..
    }) = app.model.dialog_mut()
    else {
        return;
    };
    let busy = busy.is_some();
    let (name_error, error) = split_error(*error, NAME_ERRORS);
    let salt = ("contact", contact);

    let Some(info) = info else {
        let mut close_clicked = false;
        let out = Modal::new(salt, "Contact").dismissable(!busy).show(
            ctx,
            |ui| paragraph(ui, &theme, "This contact no longer exists."),
            |ui| {
                close_clicked = ui.add(Button::secondary("Close").enabled(!busy)).clicked();
            },
        );
        if close_clicked || out.close_requested {
            close(app);
        }
        return;
    };

    let changed = name.trim() != info.current_name.trim();
    let (mut save, mut close_clicked, mut submitted) = (false, false, false);
    let mut verified_change = None;
    let mut next = Next::Stay;
    let out = Modal::new(salt, "Contact").dismissable(!busy).show(
        ctx,
        |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    Avatar::new(&info.shown_name, common::seed(&info.fingerprint))
                        .anonymous(!info.named)
                        .size(AvatarSize::Extra)
                        .verified(info.verified),
                );
                ui.add_space(theme.space.sm);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = theme.space.xxs;
                    ui.add(
                        Text::new(&info.shown_name, &theme.text.heading, theme.color.text)
                            .truncate(),
                    );
                    let line = match info.waiting {
                        Some(waiting) => common::waiting_words(waiting),
                        None if info.verified => "Fingerprint verified",
                        None => "Fingerprint not verified yet",
                    };
                    caption(ui, &theme, line);
                });
            });
            ui.add_space(theme.space.md);
            let output = TextField::new(("contact-name", contact), name)
                .label("Name (only you see it)")
                .char_limit(NAME_LIMIT)
                .error(name_error)
                .enabled(!busy)
                .show(ui);
            submitted = output.submitted;

            ui.add_space(theme.space.sm);
            ui.horizontal(|ui| {
                field_label(ui, &theme, "Fingerprint");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let copy = Button::ghost("Copy fingerprint")
                        .icon(icons::COPY)
                        .size(Size::Small);
                    if ui.add(copy).clicked() {
                        ctx.copy_text(info.full_fingerprint.clone());
                    }
                });
            });
            fingerprint_box(ui, &theme, &info.full_fingerprint);
            caption(ui, &theme, CHECK_CONTACT);

            ui.add_space(theme.space.sm);
            let mut verified = info.verified;
            let response = ui.add(
                Toggle::new(&mut verified)
                    .label("Verified")
                    .full_width(true)
                    .id_salt(("verified", contact)),
            );
            if response.changed() {
                verified_change = Some(verified);
            }
            // The name waits for Save; the toggle does not.
            caption(
                ui,
                &theme,
                "Changes to Verified are saved at once; the name when you press Save.",
            );

            ui.add_space(theme.space.md);
            ui.add(Divider::horizontal());
            ui.add_space(theme.space.sm);
            ui.horizontal_wrapped(|ui| {
                let actions = [
                    (Next::Block, "Block", icons::BAN),
                    (Next::Remove, "Remove", icons::USER_X),
                    (Next::DeleteMessages, "Delete messages", icons::TRASH_2),
                ];
                // Each only asks; the question confirms with the solid one.
                for (choice, label, icon) in actions {
                    let button = Button::quiet_danger(label)
                        .icon(icon)
                        .size(Size::Small)
                        .enabled(!busy);
                    if ui.add(button).clicked() {
                        next = choice;
                    }
                }
            });
            error_line(ui, error);
        },
        |ui| {
            save = ui
                .add(Button::primary("Save").enabled(changed && !busy).busy(busy))
                .clicked();
            close_clicked = ui.add(Button::secondary("Close").enabled(!busy)).clicked();
        },
    );
    if let Some(value) = verified_change {
        app.model.set_verified(contact, value);
    }
    let dialog = match next {
        Next::Stay => None,
        Next::Block => Some(Dialog::BlockContact {
            contact,
            busy: None,
            error: None,
        }),
        Next::Remove => Some(Dialog::RemoveContact {
            contact,
            busy: None,
            error: None,
        }),
        Next::DeleteMessages => Some(Dialog::DeleteConversation {
            contact,
            busy: None,
            error: None,
        }),
    };
    if let Some(dialog) = dialog {
        app.model.close();
        app.model.open(dialog);
    } else if (save || (submitted && changed)) && !busy {
        app.model.confirm();
    } else if close_clicked || out.close_requested {
        close(app);
    }
}

fn accept_request(ctx: &Context, app: &mut App) {
    let theme = theme::current(ctx);
    let focus = nothing_focused(ctx);
    let Some(Dialog::AcceptRequest { request, .. }) = app.model.dialog() else {
        return;
    };
    let request = *request;
    let found = app
        .model
        .slice(request.identity())
        .and_then(|slice| slice.requests.iter().find(|view| view.id == request))
        .map(|view| {
            (
                present::prepare(&view.display_name, false).text,
                view.full_fingerprint.clone(),
            )
        });
    let Some(Dialog::AcceptRequest {
        name, busy, error, ..
    }) = app.model.dialog_mut()
    else {
        return;
    };
    let busy = busy.is_some();
    let (name_error, error) = split_error(*error, NAME_ERRORS);
    let salt = ("accept-request", request);

    let Some((display_name, fingerprint)) = found else {
        let mut close_clicked = false;
        let out = Modal::new(salt, "Accept request").dismissable(!busy).show(
            ctx,
            |ui| {
                if busy {
                    loading(ui, &theme, "Accepting...");
                } else {
                    paragraph(ui, &theme, "This request is no longer waiting.");
                }
                error_line(ui, error);
            },
            |ui| {
                close_clicked = ui.add(Button::secondary("Close").enabled(!busy)).clicked();
            },
        );
        if close_clicked || out.close_requested {
            close(app);
        }
        return;
    };

    let (mut main, mut cancel, mut submitted) = (false, false, false);
    let out = Modal::new(salt, "Accept request").dismissable(!busy).show(
        ctx,
        |ui| {
            let intro = if display_name.trim().is_empty() {
                "The request carries no name. Choose how you want to see them.".to_owned()
            } else {
                format!("They chose the name \"{display_name}\". Choose how you want to see them.")
            };
            paragraph(ui, &theme, &intro);
            ui.add_space(theme.space.sm);
            let output = TextField::new(("accept-name", request), name)
                .label("Name (only you see it)")
                .char_limit(NAME_LIMIT)
                .error(name_error)
                .enabled(!busy)
                .request_focus(focus)
                .show(ui);
            submitted = output.submitted;
            ui.add_space(theme.space.xs);
            field_label(ui, &theme, "Fingerprint");
            fingerprint_box(ui, &theme, &fingerprint);
            caption(ui, &theme, CHECK_BEFORE_ACCEPT);
            error_line(ui, error);
        },
        |ui| {
            (main, cancel) = actions(ui, Button::primary("Accept").busy(busy), "Cancel", busy);
        },
    );
    if (main || submitted) && !busy {
        app.model.confirm();
    } else if cancel || out.close_requested {
        close(app);
    }
}

fn tor_details(ctx: &Context, app: &mut App) {
    let theme = theme::current(ctx);
    let (state, label) = common::tor_state(&app.model);
    let checking = app.model.tor_checking();
    let found = app.model.tor_check();
    let ready = matches!(found, Some(TorCheck::Ready { .. }));
    let words = common::tor_words(found);
    let outdated = common::tor_outdated(found);
    let (mut check, mut settings, mut close_clicked) = (false, false, false);
    let out = Modal::new("tor-details", "Tor").show(
        ctx,
        |ui| {
            ui.add(TorStatus::new(state, label));
            ui.add_space(theme.space.xs);
            paragraph(ui, &theme, &words);
            if let Some(note) = &outdated {
                ui.add_space(theme.space.xs);
                Banner::new("tor-details-outdated", Tone::Warning, note).show(ui);
            }
            ui.add_space(theme.space.xs);
            caption(
                ui,
                &theme,
                "Monolith only connects through Tor. Nothing is delivered while Tor is unavailable; messages wait until it is back.",
            );
        },
        |ui| {
            // Tor works: nothing is left to do but close.
            if ready {
                close_clicked = ui.add(Button::primary("Close")).clicked();
            }
            let again = if ready {
                Button::secondary("Check again")
            } else {
                Button::primary("Check again")
            };
            check = ui.add(again.busy(checking)).clicked();
            settings = ui.add(Button::secondary("Connection settings")).clicked();
            if !ready {
                close_clicked = ui.add(Button::secondary("Close")).clicked();
            }
        },
    );
    if settings {
        app.model.close();
        app.view = View::Settings(SettingsPage::Tor);
    } else if check && !checking {
        app.model.check_tor();
    } else if close_clicked || out.close_requested {
        close(app);
    }
}

#[cfg(all(test, feature = "fixtures"))]
#[allow(clippy::expect_used)]
mod tests {
    use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect};
    use monolith_ui_kit::{Theme, fonts};
    use monolith_ui_model::{
        Command, IdentitySnapshot, Outcome, RequestMode, RequestView, RuntimeEvent, Service,
        UiInvitationId, UiRequestId, Update,
    };
    use monolith_ui_snapshot::{Options, render};

    use super::*;
    use crate::fixtures::{NAMES, fixture};

    const CARD: &str = "monolith:card:1:AJQXG43FMNZGK5BAONSWG4TFOQQGS3THEBUW4ZZAMVZXIZLTEBZG65LTMVZGKIDON5ZGOIDCMFWGS43IMFRGGZDFMZTWQ2LKNNWG23TPOBYXE43UOV3HO6DZPJTWC3DJNZSXE5DFPBPXG5LBNZSWCZB2GE3DENBQGIYTCMRRGMZDGNZYHA4DKNJWGU3TSMJZGQYDCOBWGM2TEMBTHE3DSOJTGAZGGMBRGQ3Q";

    fn identity() -> UiIdentityId {
        UiIdentityId::from_bytes([1; 16])
    }

    fn contact(record: u8) -> UiContactId {
        UiContactId::new(identity(), [record; 16])
    }

    fn base() -> App {
        fixture("conversation").expect("fixture")
    }

    /// Opens `dialog`; a card dialog gets its card at once.
    fn open(app: &mut App, dialog: Dialog) {
        app.model.open(dialog);
        if let Some(
            Dialog::OwnCard { card, busy, .. } | Dialog::ShowInvitation { card, busy, .. },
        ) = app.model.dialog_mut()
        {
            *card = Some(Zeroizing::new(CARD.to_owned()));
            *busy = None;
        }
    }

    /// A window that takes keys, for what the dialogs do.
    struct Window {
        ctx: Context,
        app: App,
        time: f64,
    }

    impl Window {
        fn new(app: App) -> Self {
            let ctx = Context::default();
            fonts::install(&ctx, &[]);
            theme::install(&ctx, Theme::light().with_reduced_motion(true));
            let mut window = Self {
                ctx,
                app,
                time: 0.0,
            };
            for _ in 0..3 {
                window.frame(Vec::new());
            }
            window
        }

        fn frame(&mut self, events: Vec<Event>) {
            self.time += 0.1;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(
                    Pos2::ZERO,
                    Theme::light().size.window_default,
                )),
                time: Some(self.time),
                focused: true,
                events,
                ..RawInput::default()
            };
            let app = &mut self.app;
            let _ = self.ctx.run(input, |ctx| crate::show(ctx, app));
        }

        fn press(&mut self, key: Key, modifiers: Modifiers) {
            self.frame(vec![Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }]);
            self.frame(Vec::new());
        }

        fn commands(&mut self) -> Vec<Command> {
            self.app
                .model
                .take_commands()
                .into_iter()
                .map(|(_, command)| command)
                .collect()
        }
    }

    fn request_view(number: u64) -> RequestView {
        RequestView {
            id: UiRequestId::new(identity(), number),
            fingerprint: "G2KW 8NRT 3MKZ 5DPL 2XEC 9BVH".to_owned(),
            full_fingerprint: "G2KW 8NRT 3MKZ 5DPL 2XEC 9BVH 5TQN 2WRK 8MZP 3XEL 6VDB 9HCA Q4KM"
                .to_owned(),
            display_name: String::new(),
            introduction: String::new(),
            invitation: None,
        }
    }

    /// The identity shown is read again, with `requests` waiting.
    fn requests_now(app: &mut App, requests: Vec<RequestView>) {
        app.model
            .apply(Update::Event(RuntimeEvent::RequestReceived {
                identity: identity(),
                queued: true,
            }));
        for (command, asked) in app.model.take_commands() {
            if asked == Command::LoadIdentity(identity()) {
                app.model.apply(Update::Done {
                    command,
                    outcome: Outcome::Identity(Box::new(IdentitySnapshot {
                        identity: identity(),
                        contacts: Vec::new(),
                        conversations: Vec::new(),
                        requests: requests.clone(),
                        invitations: Vec::new(),
                        blocked: Vec::new(),
                        service: Service::Available,
                        request_mode: RequestMode::Invitation,
                    })),
                });
            }
        }
    }

    #[test]
    fn every_dialog_fixture_shows_its_dialog() {
        // Pictured by the screens example with the other states.
        let names: Vec<&str> = NAMES
            .iter()
            .copied()
            .filter(|name| name.starts_with("dialog-"))
            .collect();
        assert!(names.len() >= 18, "{names:?}");
        for name in names {
            let mut window = Window::new(fixture(name).expect("fixture"));
            window.frame(Vec::new());
            let open = window.app.model.dialog().is_some()
                || window.app.block_request.is_some()
                || window.app.decline_request.is_some()
                || window.app.unblock.is_some();
            assert!(open, "{name} shows no dialog");
        }
    }

    #[test]
    fn the_qr_lives_with_its_dialog() {
        let mut app = base();
        open(
            &mut app,
            Dialog::OwnCard {
                identity: identity(),
                card: None,
                busy: None,
                error: None,
            },
        );
        let ctx = Context::default();
        fonts::install(&ctx, &[]);
        theme::install(&ctx, Theme::light());
        let options = Options::default();
        let _ = render(&ctx, &options, |ctx| crate::show(ctx, &mut app));
        assert!(app.qr.is_some());
        app.model.close();
        let _ = render(&ctx, &options, |ctx| crate::show(ctx, &mut app));
        assert!(app.qr.is_none());
    }

    fn new_identity_dialog() -> Dialog {
        Dialog::NewIdentity {
            name: "Work".to_owned(),
            busy: None,
            error: None,
        }
    }

    fn creates(commands: &[Command]) -> bool {
        commands
            .iter()
            .any(|command| matches!(command, Command::CreateIdentity { .. }))
    }

    /// Opens "New identity" in `app`, with the focus in its name field.
    fn naming(app: App) -> Window {
        let mut app = app;
        app.model.open(new_identity_dialog());
        let mut window = Window::new(app);
        let field = TextField::new("new-identity-name", &mut String::new()).edit_id();
        window.ctx.memory_mut(|memory| memory.request_focus(field));
        window.frame(Vec::new());
        let _ = window.commands();
        window
    }

    #[test]
    fn no_identity_is_made_while_tor_is_unavailable() {
        let mut window = naming(fixture("tor-unavailable").expect("fixture"));
        // Enter in the name field submits the dialog.
        window.press(Key::Enter, Modifiers::NONE);
        assert!(!creates(&window.commands()));
        assert!(matches!(
            window.app.model.dialog(),
            Some(Dialog::NewIdentity { busy: None, .. })
        ));

        // With Tor, the same key makes it.
        let mut window = naming(base());
        window.press(Key::Enter, Modifiers::NONE);
        assert!(creates(&window.commands()));
    }

    #[test]
    fn blocking_from_a_request_blocks_only_the_request_it_was_asked_for() {
        let mut app = fixture("requests").expect("fixture");
        let first = UiRequestId::new(identity(), 1);
        let second = UiRequestId::new(identity(), 2);
        requests_now(&mut app, vec![request_view(1)]);
        app.block_request = Some(first);
        let mut window = Window::new(app);
        let _ = window.commands();
        // Nothing is blocked by asking.
        assert!(window.app.model.dialog().is_none());
        assert_eq!(window.app.block_request, Some(first));
        // Another request arrives, listed first, while the question is open.
        requests_now(&mut window.app, vec![request_view(2), request_view(1)]);
        window.frame(Vec::new());
        // The question has Cancel focused; Block is before it.
        window.press(Key::Tab, Modifiers::SHIFT);
        window.press(Key::Enter, Modifiers::NONE);
        let commands = window.commands();
        assert!(
            commands.contains(&Command::BlockRequest(first)),
            "{commands:?}"
        );
        assert!(!commands.contains(&Command::BlockRequest(second)));
        assert_eq!(window.app.block_request, None);
    }

    #[test]
    fn the_shortcuts_of_the_window_wait_while_blocking_is_asked() {
        let mut app = fixture("requests").expect("fixture");
        app.block_request = Some(UiRequestId::new(identity(), 1));
        let mut window = Window::new(app);
        let _ = window.commands();
        let selected = window.app.model.selection();
        window.press(Key::ArrowDown, Modifiers::ALT);
        window.press(Key::ArrowUp, Modifiers::ALT);
        assert_eq!(window.app.model.selection(), selected);
        assert_eq!(
            window.app.block_request,
            Some(UiRequestId::new(identity(), 1))
        );
    }

    #[test]
    fn cancelling_the_question_blocks_nobody() {
        let mut app = fixture("requests").expect("fixture");
        app.block_request = Some(UiRequestId::new(identity(), 1));
        let mut window = Window::new(app);
        let _ = window.commands();
        window.press(Key::Enter, Modifiers::NONE);
        let commands = window.commands();
        assert!(
            !commands
                .iter()
                .any(|command| matches!(command, Command::BlockRequest(_))),
            "{commands:?}"
        );
        assert_eq!(window.app.block_request, None);
    }

    #[test]
    fn deleting_messages_names_those_not_yet_delivered() {
        let app = base();
        // Carol's conversation holds one queued message, Bob's none.
        let carol = delete_messages_text(&app, contact(12));
        assert!(
            carol.contains("including 1 not yet delivered, which will not be sent"),
            "{carol}"
        );
        let bob = delete_messages_text(&app, contact(11));
        assert!(!bob.contains("not yet delivered"), "{bob}");
        assert!(bob.starts_with("All messages with Bob at work are deleted"));
    }

    #[test]
    fn the_question_before_closing_says_what_goes_and_what_waits() {
        let warning = |loses_everything, undelivered| CloseWarning {
            loses_everything,
            undelivered,
        };
        assert_eq!(close_text(warning(true, 0)), [SAVES_NOTHING]);
        assert_eq!(
            close_text(warning(true, 3)),
            [
                SAVES_NOTHING,
                "3 messages have not been delivered yet and never will be."
            ]
        );
        assert_eq!(
            close_text(warning(false, 1)),
            [
                "1 message has not been delivered yet. Monolith tries to send it again the next \
                 time it is open."
            ]
        );
        assert_eq!(
            close_text(warning(false, 2)),
            [
                "2 messages have not been delivered yet. Monolith tries to send them again the \
                 next time it is open."
            ]
        );
    }

    #[test]
    fn an_error_goes_under_the_field_it_belongs_to() {
        let (card, rest) = split_error(Some(Failure::InvalidCard), CARD_ERRORS);
        assert_eq!(card, Some(Failure::InvalidCard.message()));
        assert_eq!(rest, None);
        let (name, rest) = split_error(rest, NAME_ERRORS);
        assert_eq!((name, rest), (None, None));
        // A failure of no field stays a line under the form.
        let (card, rest) = split_error(Some(Failure::Storage), CARD_ERRORS);
        assert_eq!((card, rest), (None, Some(Failure::Storage)));
        let (name, rest) = split_error(Some(Failure::InvalidText), CARD_ERRORS);
        assert_eq!(name, None);
        assert_eq!(
            split_error(rest, NAME_ERRORS),
            (Some(Failure::InvalidText.message()), None)
        );
    }

    /// The text of every row of the text drawn in `shapes` that starts with
    /// `start`.
    fn rows_of(shapes: &[egui::epaint::ClippedShape], start: &str) -> Vec<String> {
        fn walk(shape: &Shape, start: &str, into: &mut Vec<String>) {
            match shape {
                Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(shape, start, into);
                    }
                }
                Shape::Text(text) if text.galley.text().starts_with(start) => {
                    for row in &text.galley.rows {
                        let line: String = row.glyphs.iter().map(|glyph| glyph.chr).collect();
                        into.push(line.trim_end().to_owned());
                    }
                }
                _ => {}
            }
        }
        let mut found = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, start, &mut found);
        }
        found
    }

    #[test]
    fn a_fingerprint_is_laid_out_in_two_rows() {
        // In the smallest window too: a row of seven groups and one of six.
        let mut app = base();
        open(
            &mut app,
            Dialog::ContactDetails {
                contact: contact(10),
                name: "Alice Martin".to_owned(),
                busy: None,
                error: None,
            },
        );
        let ctx = Context::default();
        fonts::install(&ctx, &[]);
        theme::install(&ctx, Theme::light().with_reduced_motion(true));
        let mut rows = Vec::new();
        for frame in 0..3 {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(
                    Pos2::ZERO,
                    Theme::light().size.window_min,
                )),
                time: Some(f64::from(frame)),
                ..RawInput::default()
            };
            let output = ctx.run(input, |ctx| crate::show(ctx, &mut app));
            rows = rows_of(&output.shapes, "AF3K");
        }
        assert_eq!(
            rows,
            [
                "AF3K 72PQ 9XZT M4LB D8EV 61RC 5TQN",
                "2WRK 8MZP 3XEL 6VDB 9HCA Q4KM"
            ]
        );
    }

    #[test]
    fn a_revoked_invitation_is_named_by_its_label() {
        let app = base();
        let text = revoke_text(&app, UiInvitationId::new(identity(), 1));
        assert!(
            text.starts_with("People who have the invitation \"Climbing gym\""),
            "{text}"
        );
    }
}

//! The dialogs of the model, each acting on the handles it captured.
//!
//! Every dialog is drawn from the state of the open [`Dialog`] and acts
//! through the model on the handles that dialog holds, never on whatever is
//! selected. Cards can carry an invitation: they stay in the dialog and in
//! the QR code of the app, and nowhere in egui's memory.

use std::hash::Hash;

use egui::{Align, Context, CornerRadius, Frame, Id, Layout, Sense, Stroke, Ui, Vec2};
use monolith_ui_kit::avatar::{Avatar, AvatarSize};
use monolith_ui_kit::banner::InlineError;
use monolith_ui_kit::button::{Button, Size};
use monolith_ui_kit::checkbox::Checkbox;
use monolith_ui_kit::divider::Divider;
use monolith_ui_kit::icons;
use monolith_ui_kit::modal::Modal;
use monolith_ui_kit::qr::{self, QrCode, QrMatrix};
use monolith_ui_kit::spinner;
use monolith_ui_kit::text::Text;
use monolith_ui_kit::text_field::{self, TextField};
use monolith_ui_kit::theme::{self, Theme};
use monolith_ui_kit::toast::Kind;
use monolith_ui_kit::toggle::Toggle;
use monolith_ui_kit::tor_status::TorStatus;
use monolith_ui_model::{Dialog, Failure, TorCheck, UiContactId, UiIdentityId, present};
use zeroize::Zeroizing;

use crate::app::{App, Effect, SettingsPage, ShownQr, View};
use crate::common;

/// The longest name, in characters.
const NAME_LIMIT: usize = 64;

/// Draws the open dialog, if any.
pub(crate) fn show(ctx: &Context, app: &mut App) {
    let add_contact = matches!(app.model.dialog(), Some(Dialog::AddContact { .. }));
    track_card_field(ctx, app, add_contact);
    if !matches!(
        app.model.dialog(),
        Some(Dialog::OwnCard { .. } | Dialog::ShowInvitation { .. } | Dialog::NewInvitation { .. })
    ) {
        // A card is shown only while its dialog is open.
        app.drop_qr();
    }
    match app.model.dialog() {
        None => {}
        Some(Dialog::NewIdentity { .. }) => new_identity(ctx, app),
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
            let salt = ("revoke-invitation", *invitation);
            danger(
                ctx,
                app,
                &Danger {
                    salt,
                    title: "Revoke invitation?",
                    message: "Revoke this invitation? People who have it can no longer send you requests with it. Contacts you already accepted are not affected.".to_owned(),
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
                        "{name} will not be able to reach you. Messages waiting to be sent to them will not be sent."
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
                        "{name} is removed from your contacts. Your conversation stays on this device until you delete it. To talk again, add their card again."
                    ),
                    action: "Remove",
                    checkbox: false,
                },
            );
        }
        Some(Dialog::DeleteConversation { contact, .. }) => {
            let contact = *contact;
            let name = app.model.contact_name(contact);
            danger(
                ctx,
                app,
                &Danger {
                    salt: ("delete-messages", contact),
                    title: "Delete messages?",
                    message: format!(
                        "All messages with {name} are deleted from this device. Their copies are not affected. Deleted data may stay on the disk until it is overwritten."
                    ),
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
                        "{name}, its contacts and its messages are deleted from this device, and its address stops working. This cannot be undone."
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

/// Mono text, wrapped and selectable, in a sunken box.
fn mono_box(ui: &mut Ui, theme: &Theme, text: &str) {
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
            ui.add(
                Text::new(text, &theme.text.mono, theme.color.text)
                    .selectable(true)
                    .wrap(),
            );
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

fn error_line(ui: &mut Ui, error: Option<Failure>) {
    if let Some(failure) = error {
        ui.add(InlineError::new(failure.message()));
    }
}

/// The main action and a cancel in the footer; what was clicked.
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

// ---- Dialogs ----

fn new_identity(ctx: &Context, app: &mut App) {
    let theme = theme::current(ctx);
    let focus = nothing_focused(ctx);
    let Some(Dialog::NewIdentity { name, busy, error }) = app.model.dialog_mut() else {
        return;
    };
    let busy = busy.is_some();
    let error = *error;
    let (mut main, mut cancel, mut submitted) = (false, false, false);
    let out = Modal::new("new-identity", "New identity")
        .dismissable(!busy)
        .show(
            ctx,
            |ui| {
                let output = TextField::new("new-identity-name", name)
                    .label("Name (only you see it)")
                    .char_limit(NAME_LIMIT)
                    .enabled(!busy)
                    .request_focus(focus)
                    .show(ui);
                submitted = output.submitted;
                ui.add_space(theme.space.xs);
                paragraph(
                    ui,
                    &theme,
                    "Each identity has its own contacts, card and address. People you talk to with one identity cannot see your others.",
                );
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
    let App {
        model, card_path, ..
    } = app;
    let Some(Dialog::AddContact {
        card, name, error, ..
    }) = model.dialog_mut()
    else {
        return;
    };
    let error = *error;
    let has_card = !card.trim().is_empty();
    let (mut main, mut cancel, mut submitted, mut open_clicked) = (false, false, false, false);
    let out = Modal::new(("add-contact", identity), "Add contact")
        .dismissable(!busy)
        .show(
            ctx,
            |ui| {
                let output = TextField::new(("add-contact-card", identity), card)
                    .label("Card")
                    .hint("Paste their Monolith card")
                    .enabled(!busy)
                    .request_focus(focus)
                    .show(ui);
                submitted |= output.submitted && has_card;
                ui.add_space(theme.space.xs);
                field_label(ui, &theme, "Card file");
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
                            .hint("Path to a card file")
                            .enabled(!busy)
                            .width(width)
                            .show(ui);
                        if output.submitted && can_open {
                            open_clicked = true;
                        }
                    },
                );
                caption(ui, &theme, "or drop the file on this window");
                ui.add_space(theme.space.xs);
                let output = TextField::new(("add-contact-name", identity), name)
                    .label("Name (only you see it)")
                    .char_limit(NAME_LIMIT)
                    .enabled(!busy)
                    .show(ui);
                submitted |= output.submitted && has_card;
                ui.add_space(theme.space.xs);
                paragraph(
                    ui,
                    &theme,
                    "If they gave you an invitation, they will see your request when you send your first message. Otherwise they also need to add your card.",
                );
                error_line(ui, error);
            },
            |ui| {
                (main, cancel) = actions(
                    ui,
                    Button::primary("Add contact")
                        .enabled(has_card)
                        .busy(busy),
                    "Cancel",
                    busy,
                );
            },
        );
    let path = if open_clicked {
        Some(std::path::PathBuf::from(app.card_path.trim()))
    } else {
        open_path
    };
    if let Some(path) = path {
        app.card_path = path.display().to_string();
        app.effect(Effect::ReadCard { path });
    }
    if (main || submitted) && has_card && !busy {
        app.model.confirm();
    } else if cancel || out.close_requested {
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
    let invitation = !matches!(app.model.dialog(), Some(Dialog::OwnCard { .. }));
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
    let sentence = if invitation {
        "Give this invitation to one person. Anyone who has it can send you a request until you revoke it."
    } else {
        "Share this card with people you want to talk to. It contains no secret."
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
            if let Some(matrix) = matrix {
                ui.vertical_centered(|ui| {
                    ui.add(QrCode::new(matrix, qr_label).max_side(theme.size.qr));
                });
                ui.add_space(theme.space.xs);
            }
            mono_box(ui, &theme, text);
            ui.add_space(theme.space.xs);
            ui.horizontal(|ui| {
                if ui
                    .add(Button::secondary("Copy card").icon(icons::COPY))
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
            });
            error_line(ui, error);
        },
        |ui| {
            done = ui.add(Button::primary("Done")).clicked();
        },
    );
    match action {
        CardAction::Copy => {
            app.toasts.push(Kind::Success, "Card copied");
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
    let error = *error;
    let (mut main, mut cancel, mut submitted) = (false, false, false);
    let out = Modal::new(("new-invitation", identity), "New invitation")
        .dismissable(!busy)
        .show(
            ctx,
            |ui| {
                paragraph(
                    ui,
                    &theme,
                    "An invitation lets one person send you a request. You can revoke it at any time.",
                );
                ui.add_space(theme.space.xs);
                let output = TextField::new(("new-invitation-label", identity), label)
                    .label("Label (only you see it)")
                    .hint("For Dana")
                    .char_limit(NAME_LIMIT)
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
    /// Also shows the choice of revoking an invitation: decline its
    /// requests too.
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
                            "Also decline the requests it brought that are still waiting",
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

/// What the details dialog shows of a contact, copied out of the model.
struct ContactInfo {
    shown_name: String,
    current_name: String,
    fingerprint: String,
    full_fingerprint: String,
    accepted: bool,
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
        current_name: view.name.clone().unwrap_or_default(),
        fingerprint: view.fingerprint.clone(),
        full_fingerprint: view.full_fingerprint.clone(),
        accepted: view.accepted,
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
    let error = *error;
    let salt = ("contact", contact);

    let Some(info) = info else {
        let mut close_clicked = false;
        let out = Modal::new(salt, "Contact").dismissable(!busy).show(
            ctx,
            |ui| paragraph(ui, &theme, "This contact no longer exists."),
            |ui| {
                close_clicked = ui.add(Button::primary("Close").enabled(!busy)).clicked();
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
                        .size(AvatarSize::Extra)
                        .verified(info.verified),
                );
                ui.add_space(theme.space.sm);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = theme.space.xxs;
                    ui.add(
                        Text::new(&info.shown_name, &theme.text.heading, theme.color.text).truncate(),
                    );
                    let line = if !info.accepted {
                        "Waiting for them to accept"
                    } else if info.verified {
                        "Fingerprint verified"
                    } else {
                        "Fingerprint not verified yet"
                    };
                    caption(ui, &theme, line);
                });
            });
            ui.add_space(theme.space.md);
            let output = TextField::new(("contact-name", contact), name)
                .label("Name (only you see it)")
                .char_limit(NAME_LIMIT)
                .enabled(!busy)
                .show(ui);
            submitted = output.submitted;
            error_line(ui, error);

            ui.add_space(theme.space.md);
            field_label(ui, &theme, "Fingerprint");
            mono_box(ui, &theme, &info.full_fingerprint);
            if ui
                .add(
                    Button::ghost("Copy fingerprint")
                        .icon(icons::COPY)
                        .size(Size::Small),
                )
                .clicked()
            {
                ctx.copy_text(info.full_fingerprint.clone());
            }

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
            caption(
                ui,
                &theme,
                "Compare this fingerprint with the one your contact sees, in person or on a call you trust, then mark them verified.",
            );

            ui.add_space(theme.space.md);
            ui.add(Divider::horizontal());
            ui.add_space(theme.space.sm);
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(Button::ghost("Block").icon(icons::BAN).size(Size::Small))
                    .clicked()
                {
                    next = Next::Block;
                }
                if ui
                    .add(
                        Button::ghost("Remove")
                            .icon(icons::USER_X)
                            .size(Size::Small),
                    )
                    .clicked()
                {
                    next = Next::Remove;
                }
                if ui
                    .add(
                        Button::ghost("Delete messages")
                            .icon(icons::TRASH_2)
                            .size(Size::Small),
                    )
                    .clicked()
                {
                    next = Next::DeleteMessages;
                }
            });
        },
        |ui| {
            save = ui
                .add(
                    Button::primary("Save")
                        .enabled(changed && !busy)
                        .busy(busy),
                )
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
        .map(|view| (view.display_name.clone(), view.full_fingerprint.clone()));
    let Some(Dialog::AcceptRequest {
        name, busy, error, ..
    }) = app.model.dialog_mut()
    else {
        return;
    };
    let busy = busy.is_some();
    let error = *error;
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
                close_clicked = ui.add(Button::primary("Close").enabled(!busy)).clicked();
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
                "They did not choose a name. Choose how you want to see them.".to_owned()
            } else {
                format!("They chose the name '{display_name}'. Choose how you want to see them.")
            };
            paragraph(ui, &theme, &intro);
            ui.add_space(theme.space.sm);
            let output = TextField::new(("accept-name", request), name)
                .label("Name (only you see it)")
                .char_limit(NAME_LIMIT)
                .enabled(!busy)
                .request_focus(focus)
                .show(ui);
            submitted = output.submitted;
            ui.add_space(theme.space.xs);
            field_label(ui, &theme, "Fingerprint");
            mono_box(ui, &theme, &fingerprint);
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
    let words = match app.model.tor_check() {
        None => "Tor has not been checked yet.".to_owned(),
        Some(TorCheck::Ready { version, outdated }) => {
            if *outdated {
                format!(
                    "Tor {version} is running. It is older than recommended; update it when you can."
                )
            } else {
                format!("Tor {version} is running.")
            }
        }
        Some(TorCheck::Connecting { progress }) => match progress {
            Some(percent) => format!("Tor is starting up ({percent}%)."),
            None => "Tor is starting up.".to_owned(),
        },
        Some(TorCheck::NotRunning) => {
            "Tor is not running, or Monolith cannot find it. Start Tor and check again.".to_owned()
        }
        Some(TorCheck::SocksUnavailable) => {
            "Monolith cannot use the SOCKS port of Tor. Check the connection settings.".to_owned()
        }
        Some(TorCheck::ControlUnavailable) => {
            "Monolith cannot reach the control port of Tor. Check the connection settings."
                .to_owned()
        }
        Some(TorCheck::AuthenticationFailed) => {
            "Tor refused Monolith access to its control port. Check the cookie file in the connection settings."
                .to_owned()
        }
        Some(TorCheck::TooOld { version }) => {
            format!("Tor {version} is too old. Install a newer version of Tor.")
        }
    };
    let (mut check, mut settings, mut close_clicked) = (false, false, false);
    let out = Modal::new("tor-details", "Tor").show(
        ctx,
        |ui| {
            ui.add(TorStatus::new(state, label));
            ui.add_space(theme.space.xs);
            paragraph(ui, &theme, &words);
            ui.add_space(theme.space.xs);
            caption(
                ui,
                &theme,
                "Monolith only connects through Tor. Nothing is sent while Tor is unavailable; messages wait until it is back.",
            );
        },
        |ui| {
            check = ui
                .add(Button::primary("Check again").busy(checking))
                .clicked();
            settings = ui.add(Button::secondary("Connection settings")).clicked();
            close_clicked = ui.add(Button::ghost("Close")).clicked();
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
    use egui::Vec2;
    use monolith_ui_kit::{Theme, fonts};
    use monolith_ui_model::{UiInvitationId, UiRequestId};
    use monolith_ui_snapshot::{Options, render};

    use super::*;
    use crate::fixtures::fixture;

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

    /// Renders `app` light and dark into `MONOLITH_DIALOG_SHOTS`, if set.
    fn shot(name: &str, mut app: App) {
        let Ok(dir) = std::env::var("MONOLITH_DIALOG_SHOTS") else {
            return;
        };
        for (suffix, chosen) in [("light", Theme::light()), ("dark", Theme::dark())] {
            let ctx = Context::default();
            fonts::install(&ctx, &[]);
            let background = chosen.color.bg_app;
            theme::install(&ctx, chosen.with_reduced_motion(true));
            let options = Options {
                size: Vec2::new(900.0, 760.0),
                background,
                ..Options::default()
            };
            let image = render(&ctx, &options, |ctx| crate::show(ctx, &mut app));
            let path = std::path::Path::new(&dir).join(format!("{name}-{suffix}.png"));
            image.save_png(&path).expect("write the PNG");
        }
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

    #[test]
    fn every_dialog() {
        let none = || (None, None);
        let (busy, error) = none();
        let mut cases: Vec<(&str, Dialog)> = vec![
            (
                "new-identity",
                Dialog::NewIdentity {
                    name: "Work".to_owned(),
                    busy,
                    error,
                },
            ),
            (
                "add-contact-filled",
                Dialog::AddContact {
                    identity: identity(),
                    card: CARD.to_owned(),
                    name: "Dana".to_owned(),
                    busy,
                    error,
                },
            ),
            (
                "own-card",
                Dialog::OwnCard {
                    identity: identity(),
                    card: None,
                    busy,
                    error,
                },
            ),
            (
                "show-invitation",
                Dialog::ShowInvitation {
                    invitation: UiInvitationId::new(identity(), 1),
                    card: None,
                    busy,
                    error,
                },
            ),
            (
                "invitation-created",
                Dialog::NewInvitation {
                    identity: identity(),
                    label: "For Dana".to_owned(),
                    created: Some((
                        UiInvitationId::new(identity(), 3),
                        Zeroizing::new(CARD.to_owned()),
                    )),
                    busy,
                    error,
                },
            ),
            (
                "revoke",
                Dialog::RevokeInvitation {
                    invitation: UiInvitationId::new(identity(), 1),
                    discard_requests: true,
                    busy,
                    error: Some(Failure::Disk),
                },
            ),
            (
                "contact-gone",
                Dialog::ContactDetails {
                    contact: contact(99),
                    name: String::new(),
                    busy,
                    error,
                },
            ),
            (
                "contact-waiting",
                Dialog::ContactDetails {
                    contact: contact(12),
                    name: String::new(),
                    busy,
                    error,
                },
            ),
            (
                "remove",
                Dialog::RemoveContact {
                    contact: contact(10),
                    busy,
                    error,
                },
            ),
            (
                "delete-messages",
                Dialog::DeleteConversation {
                    contact: contact(10),
                    busy,
                    error,
                },
            ),
            (
                "delete-identity",
                Dialog::DeleteIdentity {
                    identity: identity(),
                    busy,
                    error,
                },
            ),
            (
                "accept",
                Dialog::AcceptRequest {
                    request: UiRequestId::new(identity(), 1),
                    name: "Frank".to_owned(),
                    busy,
                    error,
                },
            ),
            (
                "accept-gone",
                Dialog::AcceptRequest {
                    request: UiRequestId::new(identity(), 9),
                    name: String::new(),
                    busy,
                    error,
                },
            ),
            ("tor", Dialog::TorDetails),
        ];
        for (name, dialog) in cases.drain(..) {
            let mut app = base();
            open(&mut app, dialog);
            shot(name, app);
        }
        let mut app = fixture("tor-unavailable").expect("fixture");
        app.model.open(Dialog::TorDetails);
        shot("tor-down", app);
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
}

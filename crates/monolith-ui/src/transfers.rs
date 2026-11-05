//! The file transfers of a conversation: a strip of cards above the
//! composer, one per transfer the runtime holds, per file being prepared
//! and per file that could not be sent. Each card says where its transfer
//! stands from the model alone (`TransferPhase`) and offers the actions
//! that apply then; what they do is the model's and the runtime's.

use egui::{Ui, Vec2};
use monolith_ui_kit::chrome;
use monolith_ui_kit::icons;
use monolith_ui_kit::line::Line;
use monolith_ui_kit::progress::Fill;
use monolith_ui_kit::theme;
use monolith_ui_kit::transfer_card::{ActionKind, CardAction, Tone, TransferCard};
use monolith_ui_model::{
    Dialog, TransferItem, TransferKey, TransferPhase, TransferReason, UiContactId, present,
};

use crate::app::App;

/// The note of a transfer on its way.
const THROUGH_TOR: &str = "Encrypted connection through Tor";

/// The note of a file being prepared.
const ON_THIS_DEVICE: &str = "Encrypted on this device before it is offered";

/// What the user did with a card.
enum Asked {
    /// An action on the card of `key`.
    Card(TransferKey, CardAction),
}

/// What a card shows of `item`, in the conversation with `contact` named
/// `name`.
struct Shown {
    status: Line,
    tone: Tone,
    fill: Option<Fill>,
    amounts: Option<(String, String)>,
    note: Option<&'static str>,
    actions: Vec<(CardAction, &'static str, ActionKind)>,
}

fn failure_words(reason: Option<TransferReason>) -> &'static str {
    match reason {
        Some(TransferReason::Storage) => "Failed: it could not be stored",
        Some(TransferReason::Digest) => "Failed: what arrived is not the file offered",
        Some(TransferReason::Stalled) => "Failed: no progress for too long",
        Some(TransferReason::OffererGone) => "Failed: it is no longer offered",
        Some(TransferReason::ContactEnded) => "Stopped: the contact was blocked or removed",
        Some(TransferReason::ByUs | TransferReason::ByPeer) | None => "Failed",
    }
}

/// "64 MB \u{b7} " before the words of a card, once the size is known.
fn sized(size: Option<u64>) -> Line {
    match size {
        Some(size) => Line::said(format!("{} \u{b7} ", present::size(size))),
        None => Line::default(),
    }
}

fn shown(item: &TransferItem, name: &str) -> Shown {
    let mut out = Shown {
        status: sized(item.size),
        tone: Tone::Neutral,
        fill: None,
        amounts: None,
        note: None,
        actions: Vec::new(),
    };
    let size = item.size.unwrap_or(0);
    let cancel = (CardAction::Cancel, "Stop", ActionKind::Secondary);
    let remove = (CardAction::Remove, "Remove", ActionKind::Quiet);
    match item.phase {
        TransferPhase::Preparing => {
            out.status = Line::said("Preparing: reading and encrypting the file");
            out.fill = Some(Fill::Unknown);
            out.note = Some(ON_THIS_DEVICE);
        }
        TransferPhase::Unsent(failure) => {
            out.status = Line::said(format!("Not sent. {}", failure.message()));
            out.tone = Tone::Ended;
            out.actions
                .push((CardAction::Dismiss, "Dismiss", ActionKind::Quiet));
        }
        TransferPhase::Waiting => {
            out.status = out.status.and("Waiting for ").name(name).and(" to accept");
            out.note = Some(THROUGH_TOR);
            out.actions.push(cancel);
        }
        TransferPhase::Offered => {
            out.status = out.status.name(name).and(" offers you this file");
            out.tone = Tone::Attention;
            out.actions
                .push((CardAction::Accept, "Accept", ActionKind::Primary));
            out.actions
                .push((CardAction::Reject, "Reject", ActionKind::Secondary));
        }
        TransferPhase::Transferring { bytes } => {
            let words = if item.incoming {
                "Receiving"
            } else {
                "Sending"
            };
            out.status = out.status.and(words);
            out.fill = Some(Fill::Share {
                done: bytes,
                total: size,
            });
            out.amounts = Some((
                present::progress(bytes, size),
                format!("{}%", present::percent(bytes, size)),
            ));
            out.note = Some(THROUGH_TOR);
            out.actions.push(cancel);
        }
        TransferPhase::Restoring => {
            // How far it got is found again once it goes on: no share is
            // shown until then, and none is made up.
            out.status = out.status.and("Restoring transfer\u{2026}");
            out.fill = Some(Fill::Unknown);
            out.note = Some(THROUGH_TOR);
            out.actions.push(cancel);
        }
        TransferPhase::Paused => {
            out.status = out
                .status
                .and("Paused: the disk is full or the file is in use");
            out.tone = Tone::Attention;
            out.actions
                .push((CardAction::Resume, "Resume", ActionKind::Primary));
            out.actions.push(cancel);
        }
        TransferPhase::Completed => {
            out.status = out.status.and("Sent: ").name(name).and(" has the file");
            out.tone = Tone::Success;
            out.actions.push(remove);
        }
        TransferPhase::Received => {
            out.status = out.status.and("Received: ready to save");
            out.tone = Tone::Attention;
            out.actions
                .push((CardAction::Save, "Save as\u{2026}", ActionKind::Primary));
            out.actions
                .push((CardAction::Remove, "Remove", ActionKind::Danger));
        }
        TransferPhase::Saved => {
            out.status = out.status.and("Saved");
            out.tone = Tone::Success;
            out.actions.push(remove);
        }
        TransferPhase::Rejected => {
            out.status = if item.incoming {
                out.status.and("You rejected it")
            } else {
                out.status.and("Rejected by ").name(name)
            };
            out.tone = Tone::Ended;
            out.actions.push(remove);
        }
        TransferPhase::Expired => {
            out.status = out.status.and("Not answered in time");
            out.tone = Tone::Ended;
            out.actions.push(remove);
        }
        TransferPhase::Cancelled { by_us } => {
            out.status = if by_us {
                out.status.and("You stopped it")
            } else {
                out.status.name(name).and(" stopped it")
            };
            out.tone = Tone::Ended;
            out.actions.push(remove);
        }
        TransferPhase::Failed(reason) => {
            out.status = out.status.and(failure_words(reason));
            out.tone = Tone::Ended;
            out.actions.push(remove);
        }
    }
    out
}

/// Whether the conversation of `contact` has a card to show.
pub(crate) fn any(app: &App, contact: UiContactId) -> bool {
    !app.model.transfers_of(contact).is_empty()
}

/// The most the strip takes: half the window but its bar, and room for two
/// cards at least.
pub(crate) fn max_height(ctx: &egui::Context) -> f32 {
    let theme = theme::current(ctx);
    (ctx.screen_rect().height() * 0.5 - theme.size.topbar).max(theme.size.control_lg * 2.0)
}

fn measured(contact: UiContactId) -> egui::Id {
    egui::Id::new(("transfers-height", contact))
}

/// The height the cards of `contact` took when last laid out, at most
/// `max`; `max` before they were.
pub(crate) fn height(ctx: &egui::Context, contact: UiContactId, max: f32) -> f32 {
    ctx.data(|data| data.get_temp::<f32>(measured(contact)))
        .map_or(max, |height| height.min(max))
}

/// Draws the cards of the conversation of `contact`, the newest at the
/// bottom, in a strip that scrolls past half the window.
pub(crate) fn strip(ui: &mut Ui, app: &mut App, contact: UiContactId) {
    let theme = theme::current(ui.ctx());
    let items = app.model.transfers_of(contact);
    if items.is_empty() {
        return;
    }
    let name = app.model.contact_name(contact);
    let max = max_height(ui.ctx());
    let mut asked = None;
    let shown = chrome::scroll_area(("transfers", contact))
        .stick_to_bottom(true)
        .max_height(max)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(0.0, theme.space.sm);
            for item in &items {
                let shown = shown(item, &name);
                let label = format!("Progress of {}", item.name);
                let mut card = TransferCard::new((contact, item.key), &item.name)
                    .incoming(item.incoming)
                    .status(shown.status, shown.tone)
                    .busy(item.busy);
                if let Some(fill) = shown.fill {
                    card = card.progress(fill, &label);
                }
                if let Some((done, share)) = &shown.amounts {
                    card = card.amounts(done, share);
                }
                if let Some(note) = shown.note {
                    card = card.note(icons::LOCK, note);
                }
                for (action, words, kind) in shown.actions {
                    card = card.action(action, words, kind);
                }
                if let Some(action) = card.show(ui).action {
                    asked = Some(Asked::Card(item.key, action));
                }
            }
        });
    // The panel takes the height the cards took, from the next frame on.
    let took = shown.content_size.y;
    let before = ui
        .ctx()
        .data(|data| data.get_temp::<f32>(measured(contact)));
    if before.is_none_or(|before| (before - took).abs() > 0.5) {
        ui.ctx()
            .data_mut(|data| data.insert_temp(measured(contact), took));
        ui.ctx().request_repaint();
    }
    if let Some(Asked::Card(key, action)) = asked {
        act(app, contact, key, action);
    }
}

/// Carries out what a card asked for, through the model.
fn act(app: &mut App, contact: UiContactId, key: TransferKey, action: CardAction) {
    let model = &mut app.model;
    match (key, action) {
        (TransferKey::Unsent(number), CardAction::Dismiss) => {
            model.dismiss_unsent(contact, number);
        }
        (TransferKey::Transfer(transfer), CardAction::Accept) => model.accept_transfer(transfer),
        (TransferKey::Transfer(transfer), CardAction::Reject) => model.reject_transfer(transfer),
        (TransferKey::Transfer(transfer), CardAction::Resume) => model.resume_transfer(transfer),
        (TransferKey::Transfer(transfer), CardAction::Save) => model.open_save(transfer),
        (TransferKey::Transfer(transfer), CardAction::Cancel) => {
            model.open(Dialog::CancelTransfer {
                transfer,
                busy: None,
                error: None,
            });
        }
        (TransferKey::Transfer(transfer), CardAction::Remove) => {
            // A received file not saved goes with its card: the user is
            // asked first. The phase its card shows is the one gone by (a
            // file just saved is saved before the next read).
            let unsaved = matches!(
                model.transfer_phase(transfer),
                Some(monolith_ui_model::TransferPhase::Received)
            );
            if unsaved {
                model.open(Dialog::RemoveFile {
                    transfer,
                    busy: None,
                    error: None,
                });
            } else {
                model.remove_transfer(transfer);
            }
        }
        _ => {}
    }
}

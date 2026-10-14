//! The contact requests waiting for an answer.

use egui::{Align, Id, Layout, Rect, Sense, Ui, UiBuilder};
use monolith_ui_kit::chrome::{TopBar, scroll_area};
use monolith_ui_kit::empty_state::EmptyState;
use monolith_ui_kit::icon_button::IconButton;
use monolith_ui_kit::request_card::{RequestAction, RequestCard};
use monolith_ui_kit::text::Text;
use monolith_ui_kit::tor_status::TorStatus;
use monolith_ui_kit::{icons, theme};
use monolith_ui_model::{Dialog, RequestView, UiRequestId, present};

use crate::app::App;
use crate::common;

/// What a card shows, made ready to draw: the sender's words have their
/// invisible characters made visible.
struct Entry {
    id: UiRequestId,
    title: String,
    name: String,
    introduction: String,
    fingerprint: String,
    seed: String,
    caption: String,
}

fn entry(request: &RequestView, label: Option<Option<&str>>) -> Entry {
    // The name is the sender's own words, not who they are: it is quoted
    // as what they say, never made the title.
    let name = present::prepare(&request.display_name, false).text;
    let title = "Someone wants to connect".to_owned();
    let said = (!name.trim().is_empty()).then(|| format!("Calls themselves \"{name}\""));
    let through = match label {
        Some(Some(label)) if !label.trim().is_empty() => Some(format!(
            "Through your invitation {}",
            present::prepare(label, false).text
        )),
        Some(_) => Some("Through one of your invitations".to_owned()),
        None => None,
    };
    let caption = said
        .into_iter()
        .chain(through)
        .collect::<Vec<_>>()
        .join(". ");
    Entry {
        id: request.id,
        title,
        name,
        introduction: present::prepare(&request.introduction, false).text,
        fingerprint: request.full_fingerprint.clone(),
        seed: request.fingerprint.clone(),
        caption,
    }
}

/// Draws the requests of the identity shown.
pub(crate) fn show(ui: &mut Ui, app: &mut App) {
    let theme = theme::current(ui.ctx());
    let narrow = ui.ctx().screen_rect().width() < theme.size.narrow_below;
    let (tor, tor_label) = common::tor_state(&app.model);
    let mut back = false;
    let mut open_tor = false;
    TopBar::new("requests-bar").show(
        ui,
        |ui| {
            if narrow {
                back = ui
                    .add(IconButton::new(icons::CHEVRON_LEFT, "Back").id_salt("requests-back"))
                    .clicked();
            }
            ui.add(Text::new("Requests", &theme.text.heading, theme.color.text));
        },
        |ui| {
            open_tor = ui.add(TorStatus::new(tor, tor_label)).clicked();
        },
    );
    if back {
        app.narrow_conversation = false;
    }
    if open_tor {
        app.model.open(Dialog::TorDetails);
    }
    let entries: Vec<Entry> = match app.model.current_slice() {
        Some(slice) => slice
            .requests
            .iter()
            .map(|request| {
                let label = request
                    .invitation
                    .and_then(|id| {
                        slice
                            .invitations
                            .iter()
                            .find(|invitation| invitation.id == id)
                    })
                    .map(|invitation| invitation.label.as_deref());
                entry(request, label)
            })
            .collect(),
        None => Vec::new(),
    };
    if entries.is_empty() {
        EmptyState::new("requests-empty", icons::INBOX, "No requests waiting.").show(ui);
        return;
    }
    let mut accept = None;
    let mut decline = None;
    let mut block = None;
    scroll_area("requests-list").show(ui, |ui| {
        let width = ui
            .available_width()
            .min(theme.size.modal_wide + theme.space.xxl * 2.0);
        let side = ((ui.available_width() - width) / 2.0).max(0.0) + theme.space.xl;
        ui.add_space(theme.space.xl);
        let mut rect = ui.available_rect_before_wrap();
        rect.min.x += side;
        rect.max.x = (rect.max.x - side).max(rect.min.x);
        let mut column = ui.new_child(
            UiBuilder::new()
                .max_rect(rect)
                .layout(Layout::top_down(Align::Min)),
        );
        column.spacing_mut().item_spacing.y = theme.space.md;
        for item in &entries {
            let busy = app.model.answering(item.id).then(|| {
                column
                    .ctx()
                    .data(|data| data.get_temp::<RequestAction>(busy_id(item.id)))
                    .unwrap_or(RequestAction::Decline)
            });
            let output = RequestCard::new(item.id, &item.title, "Accept", "Decline", "Block")
                .sender(&item.name, common::seed(&item.seed))
                .introduction("Their message", &item.introduction)
                .fingerprint(&item.fingerprint)
                .caption(&item.caption)
                .busy(busy)
                .show(&mut column);
            if output.accept {
                accept = Some((item.id, item.name.clone()));
            }
            if output.decline {
                decline = Some(item.id);
            }
            if output.block {
                block = Some(item.id);
            }
        }
        column.add_space(theme.space.xs);
        column.add(
            Text::new("Compare the fingerprint with the person through another channel before you accept. Accepting adds them to your contacts.", &theme.text.caption, theme.color.text_tertiary),
        );
        let used = column.min_rect();
        ui.allocate_rect(
            Rect::from_min_max(
                egui::pos2(rect.min.x - side, used.min.y),
                egui::pos2(rect.max.x + side, used.max.y + theme.space.xl),
            ),
            Sense::hover(),
        );
    });
    if let Some((request, name)) = accept {
        app.model.open(Dialog::AcceptRequest {
            request,
            name,
            busy: None,
            error: None,
        });
    }
    if let Some(request) = decline {
        remember(ui, request, RequestAction::Decline);
        app.model.decline(request);
    }
    if let Some(request) = block {
        remember(ui, request, RequestAction::Block);
        app.model.block_sender(request);
    }
}

/// The key of the answer last given to a request, so that the card shows
/// progress on the right button.
fn busy_id(request: UiRequestId) -> Id {
    Id::new(("request-answer", request))
}

fn remember(ui: &Ui, request: UiRequestId, action: RequestAction) {
    ui.ctx()
        .data_mut(|data| data.insert_temp(busy_id(request), action));
}

#[cfg(test)]
mod tests {
    use monolith_ui_model::{UiIdentityId, UiInvitationId};

    use super::*;

    #[test]
    fn the_name_a_sender_chose_is_quoted_never_made_the_title() {
        let identity = UiIdentityId::from_bytes([1; 16]);
        let request = RequestView {
            id: UiRequestId::new(identity, 1),
            fingerprint: "F6QW 8NRT".to_owned(),
            full_fingerprint: "F6QW 8NRT 3MKZ".to_owned(),
            display_name: "Your Bank".to_owned(),
            introduction: String::new(),
            invitation: Some(UiInvitationId::new(identity, 2)),
        };
        let shown = entry(&request, Some(Some("Climbing gym")));
        assert!(!shown.title.contains("Your Bank"), "{}", shown.title);
        assert!(shown.caption.contains("\"Your Bank\""), "{}", shown.caption);
        assert!(shown.caption.contains("Climbing gym"), "{}", shown.caption);
    }
}

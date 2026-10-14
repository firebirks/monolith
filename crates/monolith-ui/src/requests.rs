//! The contact requests waiting for an answer.

use egui::{Align, Layout, Rect, Sense, Ui, UiBuilder};
use monolith_ui_kit::chrome::{TopBar, scroll_area};
use monolith_ui_kit::empty_state::EmptyState;
use monolith_ui_kit::icon_button::IconButton;
use monolith_ui_kit::request_card::{RequestAction, RequestCard};
use monolith_ui_kit::text::Text;
use monolith_ui_kit::tor_status::TorStatus;
use monolith_ui_kit::{icons, theme};
use monolith_ui_model::{Dialog, RequestView, UiRequestId, present};

use crate::app::App;
use crate::{common, dialogs};

/// What a request without a name or an introduction says: Monolith sends
/// none (`DESIGN_QUESTIONS.md` P6-5).
const NOTHING_SENT: &str = "No name or message comes with a request. Check the fingerprint with \
                            the person before you accept.";

/// How to check who sent a request.
const CHECK: &str = "Before you accept, ask the person to read you the fingerprint under their \
                     identity in Monolith (Settings > Identities), on a call or in person. It \
                     must match the one on the request.";

/// The queue of requests is in memory (`DESIGN_QUESTIONS.md` P4-12).
const KEPT: &str = "Requests are kept only while Monolith is open. One you have not answered \
                    comes back when the sender's Monolith tries again.";

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
    let introduction = present::prepare(&request.introduction, false).text;
    let mut sentences = Vec::new();
    if !name.trim().is_empty() {
        sentences.push(format!("Calls themselves \"{name}\"."));
    }
    match label {
        Some(Some(label)) if !label.trim().is_empty() => sentences.push(format!(
            "Through your invitation \"{}\".",
            present::prepare(label, false).text
        )),
        Some(_) => sentences.push("Through one of your invitations.".to_owned()),
        None => {}
    }
    if name.trim().is_empty() && introduction.trim().is_empty() {
        sentences.push(NOTHING_SENT.to_owned());
    }
    Entry {
        id: request.id,
        title,
        name,
        introduction,
        fingerprint: dialogs::fingerprint_rows(&request.full_fingerprint).join("\n"),
        seed: request.fingerprint.clone(),
        caption: sentences.join(" "),
    }
}

/// Forgets the answers that are no longer under way: answered, refused,
/// or of a request that is gone.
pub(crate) fn forget_answers(app: &mut App) {
    let model = &app.model;
    app.request_answers
        .retain(|request, _| model.answering(*request));
}

/// Draws the requests of the identity shown.
pub(crate) fn show(ui: &mut Ui, app: &mut App) {
    forget_answers(app);
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
        EmptyState::new("requests-empty", icons::INBOX, "No requests waiting.")
            .body(KEPT)
            .show(ui);
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
                app.request_answers
                    .get(&item.id)
                    .copied()
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
        for text in [CHECK, KEPT] {
            column.add(Text::new(text, &theme.text.caption, theme.color.text_tertiary).wrap());
        }
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
        app.request_answers.insert(request, RequestAction::Decline);
        app.model.decline(request);
    }
    if let Some(request) = block {
        // Asked first: the dialog holds this request, whatever the list
        // shows when it is answered.
        app.block_request = Some(request);
    }
}

#[cfg(test)]
mod tests {
    use monolith_ui_model::{UiIdentityId, UiInvitationId};

    use super::*;

    fn request(display_name: &str, introduction: &str) -> RequestView {
        let identity = UiIdentityId::from_bytes([1; 16]);
        RequestView {
            id: UiRequestId::new(identity, 1),
            fingerprint: "F6QW 8NRT".to_owned(),
            full_fingerprint: "F6QW 8NRT 3MKZ".to_owned(),
            display_name: display_name.to_owned(),
            introduction: introduction.to_owned(),
            invitation: Some(UiInvitationId::new(identity, 2)),
        }
    }

    #[test]
    fn the_name_a_sender_chose_is_quoted_never_made_the_title() {
        let shown = entry(&request("Your Bank", ""), Some(Some("Climbing gym")));
        assert!(!shown.title.contains("Your Bank"), "{}", shown.title);
        assert!(shown.caption.contains("\"Your Bank\""), "{}", shown.caption);
        assert!(
            shown.caption.contains("\"Climbing gym\""),
            "{}",
            shown.caption
        );
    }

    #[test]
    fn a_request_without_a_name_says_that_none_comes_with_it() {
        let shown = entry(&request("", ""), Some(None));
        assert!(shown.caption.contains(NOTHING_SENT), "{}", shown.caption);
        let named = entry(&request("Frank", ""), Some(None));
        assert!(!named.caption.contains(NOTHING_SENT), "{}", named.caption);
    }
}

#[cfg(all(test, feature = "fixtures"))]
#[allow(clippy::expect_used)]
mod answers {
    use monolith_ui_model::{Outcome, UiIdentityId, Update};

    use super::*;
    use crate::fixtures::fixture;

    #[test]
    fn the_answer_given_to_a_request_is_forgotten_once_it_is_done() {
        let mut app = fixture("requests").expect("fixture");
        let request = UiRequestId::new(UiIdentityId::from_bytes([1; 16]), 1);
        let gone = UiRequestId::new(UiIdentityId::from_bytes([1; 16]), 7);
        app.request_answers.insert(request, RequestAction::Decline);
        app.request_answers.insert(gone, RequestAction::Block);
        app.model.decline(request);
        forget_answers(&mut app);
        assert_eq!(
            app.request_answers.keys().copied().collect::<Vec<_>>(),
            [request]
        );
        for (command, _) in app.model.take_commands() {
            app.model.apply(Update::Done {
                command,
                outcome: Outcome::Done,
            });
        }
        forget_answers(&mut app);
        assert!(app.request_answers.is_empty());
    }
}

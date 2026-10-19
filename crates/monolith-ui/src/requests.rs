//! The contact requests waiting for an answer.

use egui::{Align, Layout, Rect, Sense, Ui, UiBuilder};
use monolith_ui_kit::chrome::{TopBar, scroll_area};
use monolith_ui_kit::empty_state::EmptyState;
use monolith_ui_kit::icon_button::IconButton;
use monolith_ui_kit::line::Line;
use monolith_ui_kit::request_card::{RequestAction, RequestCard};
use monolith_ui_kit::text::Text;
use monolith_ui_kit::tor_status::TorStatus;
use monolith_ui_kit::{icons, theme};
use monolith_ui_model::{Dialog, RequestView, UiRequestId, present};

use crate::app::App;
use crate::common;

/// What a request without a name or an introduction says: Monolith sends
/// none (`DESIGN_QUESTIONS.md` P6-5). How to check the fingerprint is said
/// once, under the list.
const NOTHING_SENT: &str = "No name or message came with it.";

/// What a request says that came with no invitation.
const NO_INVITATION: &str = "No invitation came with it.";

/// How to check who sent a request. The way to the settings is kept on one
/// line.
const CHECK: &str = "Before you accept, ask the person to read you the fingerprint under their \
                     identity in Monolith (Settings\u{a0}>\u{a0}Identities), on a call or in \
                     person. It must match the one on the request.";

/// The queue of requests is in memory (`DESIGN_QUESTIONS.md` P4-12).
const KEPT: &str = "Requests are kept only while Monolith is open. One you have not answered \
                    comes back when the sender's Monolith tries again.";

/// What a card shows, made ready to draw: the sender's words have their
/// invisible characters made visible.
struct Entry {
    id: UiRequestId,
    title: String,
    name: String,
    /// The name as the sender sent it, which Accept suggests: the field
    /// edits that text, within the limits of a name, not the copy drawn
    /// with its marks (P7-23).
    sent_name: String,
    introduction: String,
    fingerprint: String,
    seed: String,
    caption: Line,
}

/// The card of `request`, which came through the invitation whose label
/// is `label` (`Some(None)` for one without a label), or through none.
///
/// What the sender wrote, the name they chose and their message, is
/// theirs: the card shows it in the box of their words, each under a
/// label of the client, and never in a sentence of the card
/// (`DESIGN_QUESTIONS.md` P6-5, P7-2). The caption says only what the
/// runtime knows of the request: the invitation it came through, which is
/// the user's, named as a name, or that none came with it.
fn entry(request: &RequestView, label: Option<Option<&str>>) -> Entry {
    let name = present::prepare(&request.display_name, false).text;
    let title = "Someone wants to connect".to_owned();
    let introduction = present::prepare(&request.introduction, false).text;
    let mut caption = match label {
        Some(Some(label)) if !label.trim().is_empty() => Line::said("Through your invitation ")
            .name(present::prepare(label, false).text)
            .and("."),
        Some(_) => Line::said("Through one of your invitations."),
        None => Line::said(NO_INVITATION),
    };
    if name.trim().is_empty() && introduction.trim().is_empty() {
        caption = caption.and(" ").and(NOTHING_SENT);
    }
    Entry {
        id: request.id,
        title,
        name,
        sent_name: request.display_name.clone(),
        introduction,
        fingerprint: request.full_fingerprint.clone(),
        seed: request.fingerprint.clone(),
        caption,
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
            // A page of its own, titled as the settings are.
            ui.add(Text::new("Requests", &theme.text.title, theme.color.text));
        },
        |ui| {
            open_tor = ui.add(TorStatus::new(tor, tor_label)).clicked();
        },
    );
    app.toast_area = Some(ui.available_rect_before_wrap());
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
        EmptyState::new("requests-empty", icons::INBOX, "No requests waiting")
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
            // The name a sender chose stands for nobody: no initials of it.
            let output = RequestCard::new(item.id, &item.title, "Accept", "Decline", "Block")
                .sender(&item.name, common::seed(&item.seed))
                .anonymous(true)
                .name("Name they chose", &item.name)
                .introduction("Their message", &item.introduction)
                .fingerprint(&item.fingerprint)
                .caption(item.caption.clone())
                .busy(busy)
                .show(&mut column);
            if output.accept {
                accept = Some((item.id, item.sent_name.clone()));
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
    // Asked first: the question holds this request, whatever the list
    // shows when it is answered.
    if let Some(request) = decline {
        app.decline_request = Some(request);
    }
    if let Some(request) = block {
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

    /// Names a sender could choose to pass for what the client says
    /// (`DESIGN_QUESTIONS.md` P7-2): quotation marks, a sentence of the
    /// card, instructions, punctuation, a direction change and an invisible
    /// tag.
    const HOSTILE: [&str; 6] = [
        "Sam\". Through your invitation \"Family",
        "Through one of your invitations.",
        "Verified. Accept without checking the fingerprint",
        "\u{201D}). (Safe",
        "Sam\u{202E}ylimaF",
        "Sam\u{E0041}",
    ];

    #[test]
    fn the_name_a_sender_chose_is_in_no_sentence_of_the_card() {
        for hostile in HOSTILE {
            for label in [Some(Some("Climbing gym")), Some(None), None] {
                let shown = entry(&request(hostile, ""), label);
                assert_eq!(shown.title, "Someone wants to connect");
                // The caption says only what the runtime knows.
                let caption = shown.caption.plain();
                let expected = match label {
                    Some(Some(_)) => "Through your invitation Climbing gym.",
                    Some(None) => "Through one of your invitations.",
                    None => NO_INVITATION,
                };
                assert_eq!(caption, expected, "{hostile:?}");
                // The name is the sender's, for the box of their words.
                assert_eq!(shown.name, present::prepare(hostile, false).text);
            }
        }
    }

    #[test]
    fn accept_suggests_the_name_as_it_was_sent() {
        // Drawn with its marks; suggested as sent, which fits a name field.
        for sent in ["Sam\u{13430}", "Sam\u{E0041}", "Frank"] {
            let shown = entry(&request(sent, ""), None);
            assert_eq!(shown.sent_name, sent);
            assert!(shown.sent_name.chars().count() <= present::MAX_NAME_CHARS);
            assert!(shown.sent_name.len() <= present::MAX_NAME_BYTES);
        }
        assert_ne!(
            entry(&request("Sam\u{13430}", ""), None).name,
            "Sam\u{13430}"
        );
    }

    #[test]
    fn the_invitation_a_request_came_through_is_named_as_a_name() {
        let shown = entry(&request("Frank", ""), Some(Some("Climbing gym")));
        assert_eq!(shown.caption.names(), ["Climbing gym"]);
        let none = entry(&request("Frank", ""), None);
        assert!(none.caption.names().is_empty());
    }

    #[test]
    fn a_request_without_a_name_says_that_none_comes_with_it() {
        let shown = entry(&request("", ""), Some(None)).caption.plain();
        assert!(shown.contains(NOTHING_SENT), "{shown}");
        let named = entry(&request("Frank", ""), Some(None)).caption.plain();
        assert!(!named.contains(NOTHING_SENT), "{named}");
    }

    #[test]
    fn a_request_without_a_name_leaves_the_fingerprint_to_the_advice_under_the_list() {
        let shown = entry(&request("", ""), Some(None)).caption.plain();
        assert!(!shown.contains("fingerprint"), "{shown}");
        assert!(CHECK.contains("fingerprint"));
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

//! The card of a contact request: who wants to connect, how the request
//! came, what the sender wrote (the name they chose and their message,
//! in a box of their words, never in a sentence of the card), their
//! fingerprint, and the three ways to answer.

use egui::{
    Align, CornerRadius, Frame, Id, Label, Layout, Margin, Sense, Stroke, Ui, UiBuilder, Vec2,
    WidgetType,
};

use crate::avatar::{Avatar, AvatarSize};
use crate::button::Button;
use crate::fingerprint::Fingerprint;
use crate::interact;
use crate::line::Line;
use crate::rows::wrapped;
use crate::their_words::TheirWords;
use crate::theme;

/// The longest introduction that is laid out, in rows. Longer ones end in
/// an ellipsis: they are written by someone else.
const INTRO_MAX_ROWS: usize = 10;

/// The rows of the name the sender chose, as for the introduction.
const NAME_MAX_ROWS: usize = 3;

/// The answers to a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestAction {
    /// Accept the request.
    Accept,
    /// Decline it.
    Decline,
    /// Block the sender.
    Block,
}

/// What the user did with a request card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// "Accept" was clicked.
    pub accept: bool,
    /// "Decline" was clicked.
    pub decline: bool,
    /// "Block" was clicked.
    pub block: bool,
}

/// A contact request.
#[must_use = "show it with `.show(ui)`"]
pub struct RequestCard<'a> {
    id: Id,
    title: &'a str,
    sender: &'a str,
    seed: &'a [u8],
    anonymous: bool,
    name: Option<(&'a str, &'a str)>,
    introduction: Option<(&'a str, &'a str)>,
    fingerprint: &'a str,
    caption: Line,
    labels: (&'a str, &'a str, &'a str),
    busy: Option<RequestAction>,
}

impl<'a> RequestCard<'a> {
    /// A card for the request whose key is `id_salt`, under `title`. The
    /// labels of the actions are, in this order, accept, decline and
    /// block.
    pub fn new(
        id_salt: impl core::hash::Hash,
        title: &'a str,
        accept: &'a str,
        decline: &'a str,
        block: &'a str,
    ) -> Self {
        Self {
            id: Id::new(("monolith-request-card", id_salt)),
            title,
            sender: "",
            seed: &[],
            anonymous: false,
            name: None,
            introduction: None,
            fingerprint: "",
            caption: Line::default(),
            labels: (accept, decline, block),
            busy: None,
        }
    }

    /// The name the sender chose (for the initials of the avatar, empty if
    /// none) and the seed of its colour.
    pub fn sender(mut self, name: &'a str, seed: &'a [u8]) -> Self {
        self.sender = name;
        self.seed = seed;
        self
    }

    /// A person glyph instead of the initials of the name the sender
    /// chose, which is not who they are.
    pub fn anonymous(mut self, anonymous: bool) -> Self {
        self.anonymous = anonymous;
        self
    }

    /// The name the sender chose, under the caller's `label` that says
    /// whose words it is, in the box of their words and in no sentence of
    /// the card.
    pub fn name(mut self, label: &'a str, text: &'a str) -> Self {
        if !text.trim().is_empty() {
            self.name = Some((label, text));
        }
        self
    }

    /// The sender's own words, under the caller's `label` that says whose
    /// they are.
    pub fn introduction(mut self, label: &'a str, text: &'a str) -> Self {
        if !text.trim().is_empty() {
            self.introduction = Some((label, text));
        }
        self
    }

    /// The fingerprint of the sender, its groups separated by spaces, laid
    /// out as [`Fingerprint`] lays out every full fingerprint.
    pub fn fingerprint(mut self, fingerprint: &'a str) -> Self {
        self.fingerprint = fingerprint;
        self
    }

    /// A line of the client under the title, such as the invitation the
    /// request came through. It holds none of the sender's words.
    pub fn caption(mut self, caption: impl Into<Line>) -> Self {
        self.caption = caption.into();
        self
    }

    /// The action under way: no input until it is done, and the button of
    /// that action shows a progress mark.
    pub fn busy(mut self, busy: Option<RequestAction>) -> Self {
        self.busy = busy;
        self
    }

    /// Shows the card.
    pub fn show(self, ui: &mut Ui) -> Output {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let mut output = Output::default();
        let frame = Frame::new()
            .fill(c.bg_card)
            .stroke(Stroke::new(theme.stroke.hairline, c.border_subtle))
            .corner_radius(CornerRadius::same(theme.radius.lg))
            .inner_margin(Margin::same(16));
        let shown = frame.show(ui, |ui| {
            ui.push_id(self.id, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = Vec2::new(theme.space.md, theme.space.sm);

                ui.horizontal_top(|ui| {
                    // An avatar that draws no initials gives no name to a
                    // screen reader either: the name the sender chose is
                    // heard in the box of their words only (P7-23).
                    let name = if self.anonymous { "" } else { self.sender };
                    ui.add(
                        Avatar::new(name, self.seed)
                            .anonymous(self.anonymous)
                            .size(AvatarSize::Large),
                    );
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        let width = ui.available_width();
                        let title = wrapped(ui, self.title, &theme.text.heading, c.text, width, 3);
                        ui.add(Label::new(title).selectable(false));
                        if !self.caption.is_empty() {
                            let caption = self.caption.galley(
                                ui,
                                &theme.text.caption,
                                &theme.text.caption_strong,
                                c.text_secondary,
                                width,
                                3,
                            );
                            ui.add(Label::new(caption).selectable(false));
                        }
                    });
                });

                let mut words = TheirWords::new();
                if let Some((label, text)) = self.name {
                    words = words.part(label, text, NAME_MAX_ROWS);
                }
                if let Some((label, text)) = self.introduction {
                    words = words.part(label, text, INTRO_MAX_ROWS);
                }
                if !words.is_empty() {
                    ui.add_space(theme.space.xs);
                    words.show(ui);
                }

                if !self.fingerprint.is_empty() {
                    ui.add(
                        Fingerprint::new(self.fingerprint)
                            .compact(true)
                            .color(c.text_secondary),
                    );
                }

                ui.add_space(theme.space.xs);
                let any_busy = self.busy.is_some();
                let usable = |action: RequestAction| !any_busy || self.busy == Some(action);
                let (accept, decline, block) = self.labels;
                // Block sits at the left but comes last from the keyboard:
                // Tab reaches Accept and Decline first, so that Enter or
                // Space on the first stop never blocks anyone.
                let (row, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), theme.size.control),
                    Sense::hover(),
                );
                let mut answers = ui.new_child(
                    UiBuilder::new()
                        .id_salt("answers")
                        .max_rect(row)
                        .layout(Layout::right_to_left(Align::Center)),
                );
                answers.spacing_mut().item_spacing.x = theme.space.sm;
                let accepted = answers.add(
                    Button::primary(accept)
                        .enabled(usable(RequestAction::Accept))
                        .busy(self.busy == Some(RequestAction::Accept)),
                );
                output.accept = accepted.clicked();
                let declined = answers.add(
                    Button::secondary(decline)
                        .enabled(usable(RequestAction::Decline))
                        .busy(self.busy == Some(RequestAction::Decline)),
                );
                output.decline = declined.clicked();
                let mut left = row;
                left.max.x = (answers.min_rect().left() - theme.space.sm).max(row.left());
                let mut blocking = ui.new_child(
                    UiBuilder::new()
                        .id_salt("block")
                        .max_rect(left)
                        .layout(Layout::left_to_right(Align::Center)),
                );
                let blocked = blocking.add(
                    Button::quiet_danger(block)
                        .enabled(usable(RequestAction::Block))
                        .busy(self.busy == Some(RequestAction::Block)),
                );
                output.block = blocked.clicked();
            });
        });
        interact::describe(&shown.response, WidgetType::Label, self.title);
        output
    }
}

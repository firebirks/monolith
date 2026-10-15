//! The card of a contact request: who wants to connect, what they wrote
//! (their words, marked as such), their fingerprint, and the three ways to
//! answer.

use egui::{
    Align, CornerRadius, Frame, Id, Label, Layout, Margin, Sense, Stroke, Ui, UiBuilder, Vec2,
    WidgetType,
};

use crate::avatar::{Avatar, AvatarSize};
use crate::button::Button;
use crate::fingerprint::Fingerprint;
use crate::interact;
use crate::rows::wrapped;
use crate::theme;

/// The longest introduction that is laid out, in rows. Longer ones end in
/// an ellipsis: they are written by someone else.
const INTRO_MAX_ROWS: usize = 10;

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
    introduction: Option<(&'a str, &'a str)>,
    fingerprint: &'a str,
    caption: &'a str,
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
            introduction: None,
            fingerprint: "",
            caption: "",
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

    /// A line under the title, such as the invitation it came through.
    pub fn caption(mut self, caption: &'a str) -> Self {
        self.caption = caption;
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
                    ui.add(
                        Avatar::new(self.sender, self.seed)
                            .anonymous(self.anonymous)
                            .size(AvatarSize::Large),
                    );
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        let width = ui.available_width();
                        let title = wrapped(ui, self.title, &theme.text.heading, c.text, width, 3);
                        ui.add(Label::new(title).selectable(false));
                        if !self.caption.is_empty() {
                            let caption = wrapped(
                                ui,
                                self.caption,
                                &theme.text.caption,
                                c.text_secondary,
                                width,
                                3,
                            );
                            ui.add(Label::new(caption).selectable(false));
                        }
                    });
                });

                if let Some((label, text)) = self.introduction {
                    ui.add_space(theme.space.xs);
                    // A hairline as well as the fill: on the card of the dark
                    // theme the fill alone hardly shows.
                    Frame::new()
                        .fill(c.bg_sunken)
                        .stroke(Stroke::new(theme.stroke.hairline, c.border_subtle))
                        .corner_radius(CornerRadius::same(theme.radius.md))
                        .inner_margin(Margin::same(12))
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.spacing_mut().item_spacing.y = theme.space.xs;
                            let width = ui.available_width();
                            ui.add(
                                Label::new(wrapped(
                                    ui,
                                    label,
                                    &theme.text.caption_strong,
                                    c.text_tertiary,
                                    width,
                                    1,
                                ))
                                .selectable(false),
                            );
                            let quote = Frame::new().inner_margin(Margin {
                                left: 11,
                                right: 0,
                                top: 0,
                                bottom: 0,
                            });
                            let shown = quote.show(ui, |ui| {
                                let width = ui.available_width();
                                ui.add(
                                    Label::new(wrapped(
                                        ui,
                                        text,
                                        &theme.text.body,
                                        c.text,
                                        width,
                                        INTRO_MAX_ROWS,
                                    ))
                                    .selectable(true),
                                );
                            });
                            let rect = shown.response.rect;
                            let bar = egui::Rect::from_min_max(
                                rect.min + Vec2::new(0.0, 1.0),
                                egui::pos2(rect.min.x + 3.0, rect.max.y - 1.0),
                            );
                            ui.painter().rect_filled(
                                bar,
                                CornerRadius::same(theme.radius.xs),
                                c.border_strong,
                            );
                        });
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

//! The card of a file transfer in a conversation: the filename on a line
//! of its own, never in a sentence of the card (for a file the contact
//! offers, it is their text); a line of the client that says its size and
//! where it stands; its progress, from bytes the caller was told; a note;
//! and the actions that apply now.

use egui::{
    Align, CornerRadius, Frame, Id, Label, Layout, Margin, Sense, Stroke, Ui, Vec2, WidgetType,
};

use crate::button::{Button, Size};
use crate::icons::{self, Icon};
use crate::interact;
use crate::line::Line;
use crate::progress::{Fill, Progress};
use crate::rows::{join_label, single_line, single_line_middle};
use crate::theme;

/// What a card can ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CardAction {
    /// Accept the file offered.
    Accept,
    /// Reject it.
    Reject,
    /// Cancel the transfer.
    Cancel,
    /// Save the received file.
    Save,
    /// Go on with a transfer held after a failed write.
    Resume,
    /// Take it off the list.
    Remove,
    /// Take a file that could not be sent off the list.
    Dismiss,
}

/// How an action looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    /// The main action.
    Primary,
    /// Another action.
    Secondary,
    /// A quiet one.
    Quiet,
    /// One that cannot be undone.
    Danger,
}

/// The tone of the line that says where the transfer stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Under way, or waiting.
    Neutral,
    /// Done.
    Success,
    /// It needs the user: a decision, or a pause to resume.
    Attention,
    /// It failed or was cancelled.
    Ended,
}

/// What the user did with a card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// The action clicked, if any.
    pub action: Option<CardAction>,
}

/// A transfer card.
#[must_use = "show it with `.show(ui)`"]
pub struct TransferCard<'a> {
    id: Id,
    name: &'a str,
    incoming: bool,
    status: Line,
    tone: Tone,
    progress: Option<(Fill, &'a str)>,
    amounts: Option<(&'a str, &'a str)>,
    note: Option<(Icon, &'a str)>,
    actions: Vec<(CardAction, &'a str, ActionKind)>,
    busy: bool,
}

impl<'a> TransferCard<'a> {
    /// A card for the transfer whose key is `id_salt`, of the file named
    /// `name`. A name the contact gave is drawn as a name on a line of its
    /// own, cut to the width with an ellipsis, its control characters as
    /// spaces: the caller makes it ready to draw (marks for invisible
    /// characters, a bounded length).
    pub fn new(id_salt: impl core::hash::Hash, name: &'a str) -> Self {
        Self {
            id: Id::new(("monolith-transfer-card", id_salt)),
            name,
            incoming: false,
            status: Line::default(),
            tone: Tone::Neutral,
            progress: None,
            amounts: None,
            note: None,
            actions: Vec::new(),
            busy: false,
        }
    }

    /// The contact offers the file.
    pub fn incoming(mut self, incoming: bool) -> Self {
        self.incoming = incoming;
        self
    }

    /// The client's line of its size and where it stands, such as "64 MB
    /// and Transferring", and its tone.
    pub fn status(mut self, status: impl Into<Line>, tone: Tone) -> Self {
        self.status = status.into();
        self.tone = tone;
        self
    }

    /// A progress bar at `fill`, named `label` for screen readers.
    pub fn progress(mut self, fill: Fill, label: &'a str) -> Self {
        self.progress = Some((fill, label));
        self
    }

    /// Under the bar: what is done of what (such as "41 MB of 64 MB") and
    /// its share (such as "64%").
    pub fn amounts(mut self, done: &'a str, share: &'a str) -> Self {
        self.amounts = Some((done, share));
        self
    }

    /// A quiet note with an icon, such as how the file travels.
    pub fn note(mut self, icon: Icon, text: &'a str) -> Self {
        self.note = Some((icon, text));
        self
    }

    /// An action, in the order given from the right: the first is the
    /// rightmost, and the first reached from the keyboard.
    pub fn action(mut self, action: CardAction, label: &'a str, kind: ActionKind) -> Self {
        self.actions.push((action, label, kind));
        self
    }

    /// A command on the transfer is under way: no action takes input.
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    /// Shows the card across the available width.
    pub fn show(self, ui: &mut Ui) -> Output {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let mut output = Output::default();
        let frame = Frame::new()
            .fill(c.bg_card)
            .stroke(Stroke::new(theme.stroke.hairline, c.border_subtle))
            .corner_radius(CornerRadius::same(theme.radius.lg))
            .inner_margin(Margin::same(12));
        let status_plain = self.status.plain();
        let shown = frame.show(ui, |ui| {
            ui.push_id(self.id, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing = Vec2::new(theme.space.md, theme.space.sm);
                // The actions at the right of the row of the name, the
                // first rightmost and first from the keyboard; the tile,
                // the name and the status in what is left.
                ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = theme.space.sm;
                    for (action, label, kind) in &self.actions {
                        let button = match kind {
                            ActionKind::Primary => Button::primary(label),
                            ActionKind::Secondary => Button::secondary(label),
                            ActionKind::Quiet => Button::ghost(label),
                            ActionKind::Danger => Button::quiet_danger(label),
                        }
                        .size(Size::Small)
                        .enabled(!self.busy);
                        if ui.push_id(action, |ui| ui.add(button)).inner.clicked() {
                            output.action = Some(*action);
                        }
                    }
                    ui.with_layout(Layout::left_to_right(Align::Min), |ui| {
                        ui.spacing_mut().item_spacing.x = theme.space.md;
                        // The tile: what kind of thing this is, in the accent.
                        let side = theme.size.control;
                        let (tile, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
                        ui.painter().rect_filled(
                            tile,
                            CornerRadius::same(theme.radius.md),
                            c.accent_subtle,
                        );
                        let glyph = if self.incoming {
                            icons::DOWNLOAD
                        } else {
                            icons::FILE
                        };
                        glyph.paint(
                            ui.painter(),
                            tile.center(),
                            theme.size.icon_md,
                            c.accent_text,
                        );
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            let width = ui.available_width();
                            let name =
                                single_line_middle(ui, self.name, &theme.text.body_strong, width);
                            let (rect, _) = ui.allocate_exact_size(name.size(), Sense::hover());
                            ui.painter().galley(rect.min, name, c.text);
                            if !self.status.is_empty() {
                                let ink = match self.tone {
                                    Tone::Neutral => c.text_secondary,
                                    Tone::Success => c.success,
                                    Tone::Attention => c.accent_text,
                                    Tone::Ended => c.text_tertiary,
                                };
                                let status = self.status.galley(
                                    ui,
                                    &theme.text.caption,
                                    &theme.text.caption_strong,
                                    ink,
                                    width,
                                    2,
                                );
                                ui.add(Label::new(status).selectable(false));
                            }
                        });
                    });
                });

                if let Some((fill, label)) = self.progress {
                    ui.add(Progress::new(fill, label));
                }
                if let Some((done, share)) = self.amounts {
                    let (row, _) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), theme.text.caption.line_height),
                        Sense::hover(),
                    );
                    let left = single_line(ui, done, &theme.text.caption, row.width() * 0.7);
                    ui.painter().galley(row.left_top(), left, c.text_secondary);
                    let right =
                        single_line(ui, share, &theme.text.caption_strong, row.width() * 0.3);
                    let at = egui::pos2(row.right() - right.size().x, row.top());
                    ui.painter().galley(at, right, c.text);
                }
                if let Some((icon, text)) = self.note {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = theme.space.xs;
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::splat(theme.size.icon_sm), Sense::hover());
                        icon.paint(
                            ui.painter(),
                            rect.center(),
                            theme.size.icon_sm,
                            c.text_tertiary,
                        );
                        let width = ui.available_width();
                        let note = single_line(ui, text, &theme.text.caption, width);
                        let (rect, _) = ui.allocate_exact_size(note.size(), Sense::hover());
                        ui.painter().galley(rect.min, note, c.text_tertiary);
                    });
                }
            });
        });
        let label = join_label(&[self.name, &status_plain]);
        interact::describe(&shown.response, WidgetType::Label, &label);
        output
    }
}

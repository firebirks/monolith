//! The delivery status of an outgoing message. There are two states and
//! nothing else: queued (stored durably, not yet acknowledged by the
//! contact) and delivered (the contact's Monolith acknowledged it). The
//! two glyphs differ in shape, not only in colour, and `Delivered` is
//! drawn only when the caller says so.

use egui::{Color32, Id, Pos2, Response, Sense, Ui, Vec2, Widget, WidgetType};

use crate::icons;
use crate::interact;
use crate::theme;

/// The status of an outgoing message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// Stored durably, not yet acknowledged by the contact.
    Queued,
    /// The contact's Monolith acknowledged it.
    Delivered,
}

/// The egui id under which the crossfade of the message `id_salt` runs.
fn fade_id(id_salt: Id) -> Id {
    id_salt.with("monolith-message-status")
}

/// Paints the glyph of `status` centred on `center`, `size` points across,
/// in `color`. A message whose status changes from queued to delivered
/// crossfades between the two glyphs; `id_salt` names the message, so the
/// fade belongs to it and to nothing else.
pub fn paint(ui: &Ui, id_salt: Id, center: Pos2, size: f32, status: Status, color: Color32) {
    let theme = theme::current(ui.ctx());
    let delivered = ui.ctx().animate_bool_with_time(
        fade_id(id_salt),
        status == Status::Delivered,
        theme.motion.duration(theme.motion.normal),
    );
    let painter = ui.painter();
    if delivered < 1.0 {
        icons::CLOCK.paint(painter, center, size, color.gamma_multiply(1.0 - delivered));
    }
    if delivered > 0.0 {
        icons::CHECK_CHECK.paint(painter, center, size, color.gamma_multiply(delivered));
    }
}

/// A small inline status indicator: the glyph, and optionally the label
/// after it. The label is also the accessible text.
#[derive(Clone, Debug)]
#[must_use = "add it with `ui.add(...)`"]
pub struct MessageStatus<'a> {
    id_salt: Id,
    status: Status,
    label: &'a str,
    show_label: bool,
}

impl<'a> MessageStatus<'a> {
    /// The status of the message named by `id_salt`; `label` says it in
    /// words ("Queued", "Delivered").
    pub fn new(id_salt: impl core::hash::Hash, status: Status, label: &'a str) -> Self {
        Self {
            id_salt: Id::new(("monolith-message-status", id_salt)),
            status,
            label,
            show_label: false,
        }
    }

    /// Whether the label is shown after the glyph. It is the accessible
    /// text either way.
    pub fn show_label(mut self, show: bool) -> Self {
        self.show_label = show;
        self
    }
}

impl Widget for MessageStatus<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let icon_size = theme.size.icon_sm;
        let color = match self.status {
            Status::Queued => theme.color.text_tertiary,
            Status::Delivered => theme.color.text_secondary,
        };
        let galley = self.show_label.then(|| {
            ui.painter().layout_no_wrap(
                self.label.to_owned(),
                theme.text.micro.font_id(),
                Color32::PLACEHOLDER,
            )
        });
        let label_width = galley
            .as_ref()
            .map_or(0.0, |galley| galley.size().x + theme.space.xs);
        let height = icon_size.max(theme.text.micro.line_height);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(icon_size + label_width, height), Sense::hover());
        interact::describe(&response, WidgetType::Label, self.label);
        if ui.is_rect_visible(rect) {
            paint(
                ui,
                self.id_salt,
                egui::pos2(rect.left() + icon_size / 2.0, rect.center().y),
                icon_size,
                self.status,
                color,
            );
            if let Some(galley) = galley {
                let pos = egui::pos2(
                    rect.left() + icon_size + theme.space.xs,
                    rect.center().y - galley.size().y / 2.0,
                );
                ui.painter().galley(pos, galley, color);
            }
        }
        response
    }
}

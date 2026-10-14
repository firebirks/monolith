//! Hairline dividers: horizontal, vertical, and with a label centred
//! between two lines (for "or").

use egui::{Color32, Response, Sense, Stroke, Ui, Vec2, Widget, WidgetType, pos2};

use crate::interact;
use crate::theme;

/// A hairline in `border_subtle`.
#[derive(Clone, Copy, Debug)]
#[must_use = "add it with `ui.add(...)`"]
pub struct Divider<'a> {
    vertical: bool,
    inset: f32,
    length: Option<f32>,
    label: Option<&'a str>,
}

impl<'a> Divider<'a> {
    /// A line across the available width.
    pub fn horizontal() -> Self {
        Self {
            vertical: false,
            inset: 0.0,
            length: None,
            label: None,
        }
    }

    /// A line down the available height (or the [`length`](Self::length)).
    pub fn vertical() -> Self {
        Self {
            vertical: true,
            ..Self::horizontal()
        }
    }

    /// A horizontal line with `text` centred between two lines.
    pub fn labelled(text: &'a str) -> Self {
        Self {
            label: Some(text),
            ..Self::horizontal()
        }
    }

    /// Leaves this much empty at each end.
    pub fn inset(mut self, inset: f32) -> Self {
        self.inset = inset;
        self
    }

    /// A fixed length instead of the available one.
    pub fn length(mut self, length: f32) -> Self {
        self.length = Some(length);
        self
    }
}

impl Widget for Divider<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let stroke = Stroke::new(theme.stroke.hairline, c.border_subtle);

        if let Some(text) = self.label {
            let galley = ui.painter().layout_no_wrap(
                text.to_owned(),
                theme.text.caption.font_id(),
                Color32::PLACEHOLDER,
            );
            let width = self.length.unwrap_or_else(|| ui.available_width());
            let height = theme.text.caption.line_height;
            let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
            interact::describe(&response, WidgetType::Label, text);
            if ui.is_rect_visible(rect) {
                let painter = ui.painter();
                let gap = theme.space.md;
                let y = painter.round_to_pixel_center(rect.center().y);
                let text_w = galley.size().x;
                let left = rect.left() + self.inset;
                let right = rect.right() - self.inset;
                let text_left = rect.center().x - text_w / 2.0;
                painter.line_segment([pos2(left, y), pos2(text_left - gap, y)], stroke);
                painter.line_segment([pos2(text_left + text_w + gap, y), pos2(right, y)], stroke);
                painter.galley(
                    pos2(text_left, rect.center().y - galley.size().y / 2.0),
                    galley,
                    c.text_tertiary,
                );
            }
            return response;
        }

        let thickness = theme.stroke.hairline;
        let size = if self.vertical {
            Vec2::new(
                thickness,
                self.length.unwrap_or_else(|| ui.available_height()),
            )
        } else {
            Vec2::new(
                self.length.unwrap_or_else(|| ui.available_width()),
                thickness,
            )
        };
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            if self.vertical {
                let x = painter.round_to_pixel_center(rect.center().x);
                painter.line_segment(
                    [
                        pos2(x, rect.top() + self.inset),
                        pos2(x, rect.bottom() - self.inset),
                    ],
                    stroke,
                );
            } else {
                let y = painter.round_to_pixel_center(rect.center().y);
                painter.line_segment(
                    [
                        pos2(rect.left() + self.inset, y),
                        pos2(rect.right() - self.inset, y),
                    ],
                    stroke,
                );
            }
        }
        response
    }
}

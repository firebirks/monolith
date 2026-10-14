//! Message bubbles and the separators between days.
//!
//! Message text is plain text, from the contact or from the user: it is
//! never interpreted, a string that looks like a link is just text, and the
//! user can select and copy it.

use egui::emath::GuiRounding;
use egui::{
    Align, Align2, Color32, CornerRadius, Id, Label, Layout, Rect, Response, RichText, Sense,
    Stroke, StrokeKind, Ui, UiBuilder, Vec2, Widget, WidgetType, text::LayoutJob,
};

use crate::interact::{self, State, mix, over};
use crate::message_status::{self, Status};
use crate::theme;

/// Which side a message came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// From the contact: on the left, no status.
    Incoming,
    /// From the user: on the right, with a delivery status.
    Outgoing,
}

/// A bubble is at most this share of the width it is shown in.
const MAX_SHARE: f32 = 0.72;

/// What came of showing a bubble.
#[derive(Debug)]
pub struct BubbleOutput {
    /// The response of the bubble, for a context menu.
    pub response: Response,
    /// The user chose "Show more" in this frame.
    pub show_more: bool,
}

/// One message.
#[derive(Clone, Debug)]
#[must_use = "show it with `.show(ui)`"]
pub struct MessageBubble<'a> {
    id: Id,
    direction: Direction,
    text: &'a str,
    time: &'a str,
    status: Option<(Status, &'a str)>,
    first_in_group: bool,
    last_in_group: bool,
    show_more: Option<&'a str>,
}

impl<'a> MessageBubble<'a> {
    /// A bubble for the message named by `id_salt` (a handle of the message,
    /// never its position), reading `text`, sent at `time` (already
    /// formatted).
    pub fn new(
        id_salt: impl core::hash::Hash,
        direction: Direction,
        text: &'a str,
        time: &'a str,
    ) -> Self {
        Self {
            id: Id::new(("monolith-bubble", id_salt)),
            direction,
            text,
            time,
            status: None,
            first_in_group: true,
            last_in_group: true,
            show_more: None,
        }
    }

    /// The delivery status of an outgoing message, and `label` for it in
    /// words (the accessible text of the glyph). Incoming messages have no
    /// status; it is ignored for them.
    pub fn status(mut self, status: Status, label: &'a str) -> Self {
        self.status = Some((status, label));
        self
    }

    /// Whether this is the first bubble of a run from the same sender.
    pub fn first_in_group(mut self, first: bool) -> Self {
        self.first_in_group = first;
        self
    }

    /// Whether this is the last bubble of a run from the same sender. Only
    /// the last one has the pointed corner.
    pub fn last_in_group(mut self, last: bool) -> Self {
        self.last_in_group = last;
        self
    }

    /// Marks the text as cut: the caller gives only its first lines as the
    /// text, and `label` reads on the control that asks for the rest
    /// ([`BubbleOutput::show_more`]).
    pub fn collapsed(mut self, label: &'a str) -> Self {
        self.show_more = Some(label);
        self
    }

    /// Shows the bubble in the width of `ui`.
    pub fn show(self, ui: &mut Ui) -> BubbleOutput {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let outgoing = self.direction == Direction::Outgoing;
        let (fill, text_color, meta_color) = if outgoing {
            (c.bubble_out, c.bubble_out_text, c.bubble_out_meta)
        } else {
            (c.bubble_in, c.bubble_in_text, c.bubble_in_meta)
        };
        let status = if outgoing { self.status } else { None };

        let available = ui.available_width();
        let share = if available.is_finite() {
            available * MAX_SHARE
        } else {
            theme.size.bubble_max
        };
        let max_width = theme.size.bubble_max.min(share).max(theme.size.control);
        let pad_x = theme.space.md;
        let pad_y = theme.space.sm;
        let content_max = (max_width - pad_x * 2.0).max(theme.size.control_sm);

        // The text is measured here and drawn by a label of the same font
        // and width below, so that the two wrap alike. Long words break at
        // the width instead of overflowing.
        let mut job = LayoutJob::single_section(
            self.text.to_owned(),
            theme.text.body.format(Color32::PLACEHOLDER),
        );
        job.wrap.max_width = content_max;
        let galley = ui.painter().layout_job(job);
        let text_size = galley.size();
        let last_row_right = galley.rows.last().map_or(0.0, |row| row.rect().right());
        let line = theme.text.body.line_height;

        let time_galley = ui.painter().layout_no_wrap(
            self.time.to_owned(),
            theme.text.micro.font_id(),
            Color32::PLACEHOLDER,
        );
        let glyph = theme.size.icon_sm;
        let meta_width = time_galley.size().x
            + if status.is_some() {
                theme.space.xs + glyph
            } else {
                0.0
            };
        let meta_gap = theme.space.md;

        let show_more_galley = self.show_more.map(|label| {
            ui.painter().layout_no_wrap(
                label.to_owned(),
                theme.text.caption_strong.font_id(),
                Color32::PLACEHOLDER,
            )
        });
        let row_height = theme.space.xl + theme.space.xs;
        let more_padding = theme.space.sm;

        // What goes under the text, and how wide the content is.
        enum Meta {
            Inline,
            Below,
            WithMore,
        }
        let (meta, content_width, content_height) = if let Some(more) = &show_more_galley {
            let width = text_size
                .x
                .max(more.size().x + more_padding * 2.0 + meta_gap + meta_width);
            (
                Meta::WithMore,
                width,
                text_size.y + theme.space.xs + row_height,
            )
        } else if last_row_right + meta_gap + meta_width <= content_max {
            (
                Meta::Inline,
                text_size.x.max(last_row_right + meta_gap + meta_width),
                text_size.y,
            )
        } else {
            (
                Meta::Below,
                text_size.x.max(meta_width),
                text_size.y + glyph,
            )
        };
        let size = Vec2::new(content_width + pad_x * 2.0, content_height + pad_y * 2.0);

        ui.add_space(if self.first_in_group {
            theme.space.md
        } else {
            theme.space.xxs
        });
        let row_width = if available.is_finite() {
            available.max(size.x)
        } else {
            size.x
        };
        let (row, _) = ui.allocate_exact_size(Vec2::new(row_width, size.y), Sense::hover());
        let left = if outgoing {
            row.right() - size.x
        } else {
            row.left()
        };
        let rect = Rect::from_min_size(egui::pos2(left, row.top()), size)
            .round_to_pixels(ui.pixels_per_point());

        let full = theme.radius.bubble;
        let tail = if self.last_in_group {
            theme.radius.bubble_tail
        } else {
            full
        };
        let radius = CornerRadius {
            nw: full,
            ne: full,
            sw: if outgoing { full } else { tail },
            se: if outgoing { tail } else { full },
        };
        if ui.is_rect_visible(rect) {
            ui.painter().rect_filled(rect, radius, fill);
        }

        // The text: a selectable label in a child ui, as wide as it was
        // measured.
        let text_origin = egui::pos2(rect.left() + pad_x, rect.top() + pad_y);
        let mut text_ui = ui.new_child(
            UiBuilder::new()
                .id_salt(self.id.with("text"))
                .max_rect(Rect::from_min_size(
                    text_origin,
                    Vec2::new(content_max, text_size.y + theme.space.xs),
                ))
                .layout(Layout::top_down(Align::Min)),
        );
        if outgoing {
            // The default selection colour is the accent, which is the
            // colour of this bubble.
            text_ui.visuals_mut().selection.bg_fill = c.bubble_out_text.gamma_multiply(0.30);
        }
        text_ui.add(
            Label::new(
                RichText::new(self.text)
                    .font(theme.text.body.font_id())
                    .line_height(Some(line))
                    .color(text_color),
            )
            .selectable(true)
            .wrap(),
        );

        // The bubble itself, above the text in the order of widgets: a
        // click goes to it (a context menu), a drag to the text.
        let response = ui.interact(rect, self.id, Sense::click());

        // Time and status.
        let baseline = theme.stroke.hairline;
        let meta_center_y = match meta {
            Meta::Inline => rect.bottom() - pad_y - line / 2.0,
            Meta::Below => rect.bottom() - pad_y - glyph / 2.0,
            Meta::WithMore => rect.bottom() - pad_y - row_height / 2.0,
        };
        let right = rect.right() - pad_x;
        if ui.is_rect_visible(rect) {
            let mut x = right;
            if let Some((state, label)) = status {
                let center = egui::pos2(x - glyph / 2.0, meta_center_y);
                message_status::paint(ui, self.id, center, glyph, state, meta_color);
                let area = Rect::from_center_size(center, Vec2::splat(glyph));
                let node = ui.interact(area, self.id.with("status"), Sense::hover());
                interact::describe(&node, WidgetType::Label, label);
                x -= glyph + theme.space.xs;
            }
            let time_pos = egui::pos2(
                x - time_galley.size().x,
                meta_center_y - time_galley.size().y / 2.0 + baseline,
            );
            ui.painter().galley(time_pos, time_galley, meta_color);
        }

        // "Show more": a quiet control at the left of the meta row.
        let mut show_more = false;
        if let (Some(galley), Some(label)) = (show_more_galley, self.show_more) {
            let button = Rect::from_min_size(
                egui::pos2(
                    rect.left() + pad_x - more_padding,
                    meta_center_y - row_height / 2.0,
                ),
                Vec2::new(galley.size().x + more_padding * 2.0, row_height),
            );
            let more = ui.interact(button, self.id.with("more"), Sense::click());
            interact::describe(&more, WidgetType::Button, label);
            let state = State::of(ui, &more, more.id, &theme, false);
            let (rest, active, wash) = if outgoing {
                (c.bubble_out_meta, c.bubble_out_text, c.bubble_out_text)
            } else {
                (c.text_secondary, c.text, c.text)
            };
            let tint = if state.pressed {
                0.20
            } else {
                0.12 * state.hover
            };
            let small = CornerRadius::same(theme.radius.sm);
            let painter = ui.painter();
            painter.rect(
                button,
                small,
                over(fill, wash.gamma_multiply(tint)),
                Stroke::NONE,
                StrokeKind::Inside,
            );
            let pos = Align2::CENTER_CENTER
                .anchor_size(button.center(), galley.size())
                .min;
            painter.galley(pos, galley, mix(rest, active, state.hover));
            if state.focused {
                interact::focus_ring(painter, button, small, &theme);
            }
            show_more = more.clicked();
        }

        BubbleOutput {
            response,
            show_more,
        }
    }
}

/// Removes the vertical space egui puts between widgets, so that the
/// bubbles of a list are spaced by their own grouping. Call it on the ui of
/// the list.
pub fn tighten_list(ui: &mut Ui) {
    ui.spacing_mut().item_spacing.y = 0.0;
}

/// A date between two days of messages: a small pill between hairlines.
#[derive(Clone, Debug)]
#[must_use = "add it with `ui.add(...)`"]
pub struct DaySeparator<'a> {
    text: &'a str,
}

impl<'a> DaySeparator<'a> {
    /// A separator that reads `text` (already formatted, for example
    /// "Today").
    pub fn new(text: &'a str) -> Self {
        Self { text }
    }
}

impl Widget for DaySeparator<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let galley = ui.painter().layout_no_wrap(
            self.text.to_owned(),
            theme.text.micro.font_id(),
            Color32::PLACEHOLDER,
        );
        let pill_height = theme.space.xl + theme.space.xxs;
        let height = pill_height + theme.space.lg * 2.0;
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        interact::describe(&response, WidgetType::Label, self.text);
        if ui.is_rect_visible(rect) {
            let pill = Rect::from_center_size(
                rect.center(),
                Vec2::new(galley.size().x + theme.space.md * 2.0, pill_height),
            );
            let painter = ui.painter();
            let stroke = Stroke::new(theme.stroke.hairline, c.border_subtle);
            let gap = theme.space.sm;
            let y = rect.center().y;
            painter.hline(
                rect.left()..=(pill.left() - gap).max(rect.left()),
                y,
                stroke,
            );
            painter.hline(
                (pill.right() + gap).min(rect.right())..=rect.right(),
                y,
                stroke,
            );
            painter.rect_filled(pill, CornerRadius::same(theme.radius.pill), c.bg_sunken);
            let pos = Align2::CENTER_CENTER
                .anchor_size(pill.center(), galley.size())
                .min;
            painter.galley(pos, galley, c.text_secondary);
        }
        response
    }
}

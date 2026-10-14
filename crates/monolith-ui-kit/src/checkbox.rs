//! The checkbox: a rounded square with a check mark and a label to its
//! right, which wraps.

use std::hash::Hash;

use egui::{
    Color32, CornerRadius, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget,
    WidgetType, pos2,
};

use crate::icons;
use crate::interact::{self, State, mix, over};
use crate::theme;

/// A checkbox bound to a `bool`.
#[must_use = "add it with `ui.add(...)`"]
pub struct Checkbox<'a> {
    value: &'a mut bool,
    label: &'a str,
    enabled: bool,
    error: bool,
    id_salt: Option<Id>,
}

impl<'a> Checkbox<'a> {
    /// A checkbox editing `value`, reading `label`.
    pub fn new(value: &'a mut bool, label: &'a str) -> Self {
        Self {
            value,
            label,
            enabled: true,
            error: false,
            id_salt: None,
        }
    }

    /// Whether it accepts input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Draws it as wrong, for a box that has to be ticked.
    pub fn error(mut self, error: bool) -> Self {
        self.error = error;
        self
    }

    /// Keys its animation state to `salt` instead of its place in the
    /// layout.
    pub fn id_salt(mut self, salt: impl Hash) -> Self {
        self.id_salt = Some(Id::new(("monolith-checkbox", salt)));
        self
    }
}

impl Widget for Checkbox<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let line = theme.text.body.line_height;
        let box_side = theme.size.icon_md;
        let gap = theme.space.sm + 2.0;
        let pad = theme.space.xxs;

        let wrap = (ui.available_width() - box_side - gap).max(box_side);
        let galley = ui.painter().layout(
            self.label.to_owned(),
            theme.text.body.font_id(),
            Color32::PLACEHOLDER,
            wrap,
        );
        let size = Vec2::new(
            box_side + gap + galley.size().x,
            galley.size().y.max(line) + pad * 2.0,
        );
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (auto_id, rect) = ui.allocate_space(size);
        let id = self.id_salt.map_or(auto_id, |salt| ui.id().with(salt));
        let mut response = ui.interact(rect, id, sense);
        if response.clicked() {
            *self.value = !*self.value;
            response.mark_changed();
        }
        let checked = *self.value;
        interact::describe_selectable(&response, WidgetType::Checkbox, self.label, checked);

        if ui.is_rect_visible(rect) {
            let state = if self.enabled {
                State::of(ui, &response, id, &theme, checked)
            } else {
                State {
                    enabled: false,
                    hover: 0.0,
                    pressed: false,
                    focused: false,
                    selected: checked,
                }
            };
            let t = ui.ctx().animate_bool_with_time(
                id.with("checked"),
                checked,
                theme.motion.duration(theme.motion.fast),
            );
            // The box sits on the centre line of the first text line.
            let first_line = galley.size().y.min(line);
            let box_rect = Rect::from_center_size(
                pos2(
                    rect.left() + box_side / 2.0,
                    rect.top() + pad + first_line / 2.0,
                ),
                Vec2::splat(box_side),
            );
            let radius = CornerRadius::same(theme.radius.xs);

            let (unchecked_fill, checked_fill, border) = if !state.enabled {
                (
                    c.bg_sunken,
                    mix(c.bg_sunken, c.accent, 0.4),
                    c.border_subtle,
                )
            } else {
                let base = if self.error {
                    c.danger
                } else {
                    mix(c.border_strong, c.text_tertiary, state.hover)
                };
                let on = if state.pressed {
                    c.accent_pressed
                } else {
                    mix(c.accent, c.accent_hover, state.hover)
                };
                let off_fill = if state.pressed {
                    over(c.bg_sunken, c.pressed)
                } else {
                    c.bg_sunken
                };
                (off_fill, on, base)
            };
            let fill = mix(unchecked_fill, checked_fill, t);
            let edge = mix(border, checked_fill, t);
            let painter = ui.painter();
            painter.rect(
                box_rect,
                radius,
                fill,
                Stroke::new(theme.stroke.hairline, edge),
                StrokeKind::Inside,
            );
            if t > 0.0 {
                let mark = if state.enabled {
                    c.text_on_accent
                } else {
                    c.bg_surface
                };
                icons::CHECK.paint(
                    painter,
                    box_rect.center(),
                    theme.size.icon_sm,
                    mark.gamma_multiply(t),
                );
            }
            let text_color = if !state.enabled {
                c.text_disabled
            } else {
                c.text
            };
            painter.galley(
                pos2(box_rect.right() + gap, rect.top() + pad),
                galley,
                text_color,
            );
            if state.focused {
                interact::focus_ring(painter, box_rect, radius, &theme);
            }
        }
        response
    }
}

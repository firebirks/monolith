//! The toggle: an animated switch, with an optional label to its left that
//! is part of the same click target.

use std::hash::Hash;

use egui::{
    Align2, Color32, CornerRadius, Id, Response, Sense, Ui, Vec2, Widget, WidgetType, pos2,
};

use egui::emath::GuiRounding;

use crate::interact::{self, State, mix};
use crate::theme::{self, Theme};

/// A switch bound to a `bool`.
#[must_use = "add it with `ui.add(...)`"]
pub struct Toggle<'a> {
    value: &'a mut bool,
    label: Option<&'a str>,
    name: &'a str,
    enabled: bool,
    full_width: bool,
    id_salt: Option<Id>,
}

impl<'a> Toggle<'a> {
    /// A switch editing `value`.
    pub fn new(value: &'a mut bool) -> Self {
        Self {
            value,
            label: None,
            name: "",
            enabled: true,
            full_width: false,
            id_salt: None,
        }
    }

    /// A label to the left of the switch; also its accessible name.
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// The accessible name of a switch without a visible label.
    pub fn name(mut self, name: &'a str) -> Self {
        self.name = name;
        self
    }

    /// Whether it accepts input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// With a label: takes the full width, the switch at the right edge.
    pub fn full_width(mut self, full: bool) -> Self {
        self.full_width = full;
        self
    }

    /// Keys its animation state to `salt` instead of its place in the
    /// layout.
    pub fn id_salt(mut self, salt: impl Hash) -> Self {
        self.id_salt = Some(Id::new(("monolith-toggle", salt)));
        self
    }
}

struct Colors {
    track: Color32,
    knob: Color32,
}

fn colors(theme: &Theme, state: State, on: f32) -> Colors {
    let c = &theme.color;
    if !state.enabled {
        return Colors {
            track: mix(c.border, c.accent, 0.45 * on),
            knob: c.text_on_accent.gamma_multiply(0.75),
        };
    }
    let off = if state.pressed {
        c.text_tertiary
    } else {
        mix(c.border_strong, c.text_tertiary, state.hover)
    };
    let on_color = if state.pressed {
        c.accent_pressed
    } else {
        mix(c.accent, c.accent_hover, state.hover)
    };
    Colors {
        track: mix(off, on_color, on),
        knob: c.text_on_accent,
    }
}

impl Widget for Toggle<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let track_size = Vec2::new(theme.size.control, theme.size.control_sm - theme.space.sm);
        let height = track_size.y + theme.space.xs;
        let name = self.label.unwrap_or(self.name);

        let galley = self.label.map(|label| {
            ui.painter().layout_no_wrap(
                label.to_owned(),
                theme.text.body.font_id(),
                Color32::PLACEHOLDER,
            )
        });
        let natural = galley
            .as_ref()
            .map_or(track_size.x, |g| g.size().x + theme.space.md + track_size.x);
        let width = if self.full_width && galley.is_some() {
            natural.max(ui.available_width())
        } else {
            natural
        };
        let height = galley.as_ref().map_or(height, |g| height.max(g.size().y));

        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (auto_id, rect) = ui.allocate_space(Vec2::new(width, height));
        let id = self.id_salt.map_or(auto_id, |salt| ui.id().with(salt));
        let mut response = ui.interact(rect, id, sense);
        if response.clicked() {
            *self.value = !*self.value;
            response.mark_changed();
        }
        let on = *self.value;
        interact::describe_selectable(&response, WidgetType::Checkbox, name, on);

        if ui.is_rect_visible(rect) {
            let state = if self.enabled {
                State::of(ui, &response, id, &theme, on)
            } else {
                State {
                    enabled: false,
                    hover: 0.0,
                    pressed: false,
                    focused: false,
                    selected: on,
                }
            };
            let t = ui.ctx().animate_bool_with_time(
                id.with("on"),
                on,
                theme.motion.duration(theme.motion.normal),
            );
            let colors = colors(&theme, state, t);

            let track = egui::Align2::RIGHT_CENTER
                .align_size_within_rect(track_size, rect)
                .round_to_pixels(ui.pixels_per_point());
            let radius = CornerRadius::same(theme.radius.pill);
            let painter = ui.painter();
            painter.rect_filled(track, radius, colors.track);

            let inset = theme.space.xxs + 1.0;
            let knob_r = track_size.y / 2.0 - inset;
            let travel = track_size.x - track_size.y;
            let knob_center = pos2(
                track.left() + track_size.y / 2.0 + travel * t,
                track.center().y,
            );
            if state.enabled {
                let knob_rect =
                    egui::Rect::from_center_size(knob_center, Vec2::splat(knob_r * 2.0));
                painter.add(
                    theme
                        .shadow
                        .sm
                        .as_shape(knob_rect, CornerRadius::same(theme.radius.pill)),
                );
            }
            painter.circle_filled(knob_center, knob_r, colors.knob);

            if let Some(galley) = galley {
                let text_color = if state.enabled {
                    c.text
                } else {
                    c.text_disabled
                };
                let pos = Align2::LEFT_CENTER
                    .anchor_size(pos2(rect.left(), rect.center().y), galley.size())
                    .min;
                painter.galley(pos, galley, text_color);
            }
            if state.focused {
                interact::focus_ring(painter, track, radius, &theme);
            }
        }
        response
    }
}

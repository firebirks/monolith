//! The segmented control: one choice out of a few labelled options, with a
//! highlight that slides to the chosen one.

use std::hash::Hash;

use egui::{
    Align2, Color32, CornerRadius, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget,
    WidgetType, pos2,
};

use crate::interact::{self, State, mix, over};
use crate::theme::{self, Mode};

/// A choice among `options`, each a value and its label.
#[must_use = "add it with `ui.add(...)`"]
pub struct Segmented<'a, T> {
    id: Id,
    value: &'a mut T,
    options: &'a [(T, &'a str)],
    enabled: bool,
    full_width: bool,
}

impl<'a, T: Copy + PartialEq + Hash> Segmented<'a, T> {
    /// A control editing `value`. `id_salt` keys its animation state; the
    /// option values key the segments.
    pub fn new(id_salt: impl Hash, value: &'a mut T, options: &'a [(T, &'a str)]) -> Self {
        Self {
            id: Id::new(("monolith-segmented", id_salt)),
            value,
            options,
            enabled: true,
            full_width: false,
        }
    }

    /// Whether it accepts input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Takes the full width, the segments sharing it equally.
    pub fn full_width(mut self, full: bool) -> Self {
        self.full_width = full;
        self
    }
}

impl<T: Copy + PartialEq + Hash> Widget for Segmented<'_, T> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let inset = theme.space.xxs;
        let token = &theme.text.body_strong;

        let galleys: Vec<_> = self
            .options
            .iter()
            .map(|(value, label)| {
                let galley = ui.painter().layout_no_wrap(
                    (*label).to_owned(),
                    token.font_id(),
                    Color32::PLACEHOLDER,
                );
                (*value, *label, galley)
            })
            .collect();
        let count = u16::try_from(galleys.len()).map_or(1.0, f32::from).max(1.0);
        let widest = galleys
            .iter()
            .map(|(_, _, galley)| galley.size().x)
            .fold(0.0_f32, f32::max);
        let mut segment_w = widest + theme.space.lg * 2.0;
        if self.full_width {
            segment_w = segment_w.max((ui.available_width() - inset * 2.0) / count);
        }
        let seg_h = theme.size.control_sm;
        let size = Vec2::new(segment_w * count + inset * 2.0, seg_h + inset * 2.0);
        let (track, mut response) = ui.allocate_exact_size(size, Sense::hover());

        let track_radius = CornerRadius::same(theme.radius.md);
        let seg_radius = CornerRadius::same(theme.radius.sm);

        // The segments, laid out left to right inside the track.
        let mut segments = Vec::with_capacity(galleys.len());
        let mut x = track.left() + inset;
        let mut selected_x = None;
        for (value, label, galley) in galleys {
            let rect =
                Rect::from_min_size(pos2(x, track.top() + inset), Vec2::new(segment_w, seg_h));
            let id = self.id.with(("segment", value));
            let sense = if self.enabled {
                Sense::click()
            } else {
                Sense::hover()
            };
            let segment = ui.interact(rect, id, sense);
            let chosen = *self.value == value;
            interact::describe_selectable(&segment, WidgetType::RadioButton, label, chosen);
            if chosen {
                selected_x = Some(x);
            }
            if segment.clicked() && !chosen {
                *self.value = value;
                response.mark_changed();
            }
            segments.push((rect, id, segment, galley, chosen));
            x += segment_w;
        }

        if ui.is_rect_visible(track) {
            ui.painter().rect(
                track,
                track_radius,
                c.bg_sunken,
                Stroke::new(theme.stroke.hairline, c.border_subtle),
                StrokeKind::Inside,
            );
            let duration = theme.motion.duration(theme.motion.normal);
            if let Some(target) = selected_x {
                let at = ui
                    .ctx()
                    .animate_value_with_time(self.id.with("slide"), target, duration);
                let thumb =
                    Rect::from_min_size(pos2(at, track.top() + inset), Vec2::new(segment_w, seg_h));
                // Raised above the track: white on the light one; on the dark
                // one the sunken track is lighter than the surface, so the
                // thumb is the raised colour with a wash of light over it.
                let dark = theme.mode == Mode::Dark;
                let raised = if dark {
                    over(c.bg_raised, c.hover)
                } else {
                    c.bg_surface
                };
                let edge = if dark { c.border } else { c.border_subtle };
                let painter = ui.painter();
                painter.add(theme.shadow.sm.as_shape(thumb, seg_radius));
                painter.rect(
                    thumb,
                    seg_radius,
                    if self.enabled {
                        raised
                    } else {
                        mix(c.bg_sunken, raised, 0.5)
                    },
                    Stroke::new(theme.stroke.hairline, edge),
                    StrokeKind::Inside,
                );
            }
            for (rect, id, segment, galley, chosen) in segments {
                let state = if self.enabled {
                    State::of(ui, &segment, id, &theme, chosen)
                } else {
                    State {
                        enabled: false,
                        hover: 0.0,
                        pressed: false,
                        focused: false,
                        selected: chosen,
                    }
                };
                let painter = ui.painter();
                if state.enabled && !chosen && (state.hover > 0.0 || state.pressed) {
                    let wash = if state.pressed {
                        c.pressed
                    } else {
                        c.hover.gamma_multiply(state.hover)
                    };
                    painter.rect_filled(rect, seg_radius, wash);
                }
                let ink = if !state.enabled {
                    c.text_disabled
                } else if chosen {
                    c.text
                } else {
                    mix(c.text_secondary, c.text, state.hover)
                };
                let pos = Align2::CENTER_CENTER
                    .anchor_size(rect.center(), galley.size())
                    .min;
                painter.galley(pos, galley, ink);
                if state.focused {
                    interact::focus_ring(painter, rect, seg_radius, &theme);
                }
            }
        }
        response
    }
}

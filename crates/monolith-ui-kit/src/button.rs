//! The button: the reference component. Every other component follows its
//! pattern: a builder, `egui::Widget`, the theme from the context, every
//! interaction state from [`crate::interact::State`], a focus ring, and an
//! accessible description.

use egui::{
    Align2, CornerRadius, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget, WidgetType,
};

use crate::icons::Icon;
use crate::interact::{self, State, mix, over};
use crate::theme::{self, Theme};

/// What the button is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The main action of a view or a modal. One per view.
    Primary,
    /// Any other action.
    Secondary,
    /// A quiet action, without a frame until hovered.
    Ghost,
    /// An action that destroys something.
    Danger,
}

/// How large.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// Inside rows and dense places.
    Small,
    /// The default.
    Medium,
    /// Onboarding and other single actions.
    Large,
}

/// A button with a label and an optional leading icon.
#[derive(Clone, Debug)]
#[must_use = "add it with `ui.add(...)`"]
pub struct Button<'a> {
    label: &'a str,
    icon: Option<Icon>,
    kind: Kind,
    size: Size,
    enabled: bool,
    busy: bool,
    full_width: bool,
    min_width: f32,
}

impl<'a> Button<'a> {
    /// A button of `kind` that reads `label`.
    pub fn new(kind: Kind, label: &'a str) -> Self {
        Self {
            label,
            icon: None,
            kind,
            size: Size::Medium,
            enabled: true,
            busy: false,
            full_width: false,
            min_width: 0.0,
        }
    }

    /// The main action.
    pub fn primary(label: &'a str) -> Self {
        Self::new(Kind::Primary, label)
    }

    /// Another action.
    pub fn secondary(label: &'a str) -> Self {
        Self::new(Kind::Secondary, label)
    }

    /// A quiet action.
    pub fn ghost(label: &'a str) -> Self {
        Self::new(Kind::Ghost, label)
    }

    /// A destructive action.
    pub fn danger(label: &'a str) -> Self {
        Self::new(Kind::Danger, label)
    }

    /// A leading icon.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The size.
    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }

    /// Whether it accepts input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Whether its action is under way: it shows a progress mark and takes
    /// no input until the action is done.
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    /// Takes the full width of the layout.
    pub fn full_width(mut self, full: bool) -> Self {
        self.full_width = full;
        self
    }

    /// At least this wide.
    pub fn min_width(mut self, width: f32) -> Self {
        self.min_width = width;
        self
    }
}

struct Colors {
    fill: egui::Color32,
    stroke: egui::Color32,
    text: egui::Color32,
}

fn colors(theme: &Theme, kind: Kind, state: State) -> Colors {
    let c = &theme.color;
    if !state.enabled {
        return match kind {
            Kind::Ghost => Colors {
                fill: egui::Color32::TRANSPARENT,
                stroke: egui::Color32::TRANSPARENT,
                text: c.text_disabled,
            },
            _ => Colors {
                fill: c.bg_sunken,
                stroke: c.border_subtle,
                text: c.text_disabled,
            },
        };
    }
    match kind {
        Kind::Primary | Kind::Danger => {
            let (base, hover, pressed) = if kind == Kind::Primary {
                (c.accent, c.accent_hover, c.accent_pressed)
            } else {
                (c.danger, c.danger_hover, c.danger_pressed)
            };
            let fill = if state.pressed {
                pressed
            } else {
                mix(base, hover, state.hover)
            };
            Colors {
                fill,
                stroke: fill,
                text: c.text_on_accent,
            }
        }
        Kind::Secondary => {
            let tint = if state.pressed {
                c.pressed
            } else {
                c.hover.gamma_multiply(state.hover)
            };
            Colors {
                fill: over(c.bg_surface, tint),
                stroke: mix(c.border, c.border_strong, state.hover),
                text: c.text,
            }
        }
        Kind::Ghost => {
            let tint = if state.pressed {
                c.pressed
            } else {
                c.hover.gamma_multiply(state.hover)
            };
            Colors {
                fill: tint,
                stroke: egui::Color32::TRANSPARENT,
                text: mix(c.text_secondary, c.text, state.hover),
            }
        }
    }
}

impl Widget for Button<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let (height, padding, token) = match self.size {
            Size::Small => (
                theme.size.control_sm,
                theme.space.sm,
                &theme.text.caption_strong,
            ),
            Size::Medium => (
                theme.size.control,
                theme.space.md + 2.0,
                &theme.text.body_strong,
            ),
            Size::Large => (
                theme.size.control_lg,
                theme.space.lg + 4.0,
                &theme.text.body_strong,
            ),
        };
        let icon_size = theme.size.icon;
        let gap = theme.space.sm;

        // Laid out in the placeholder colour, painted in the colour of the
        // state below.
        let galley = ui.painter().layout_no_wrap(
            self.label.to_owned(),
            token.font_id(),
            egui::Color32::PLACEHOLDER,
        );
        let content_width = galley.size().x
            + if self.icon.is_some() || self.busy {
                icon_size + gap
            } else {
                0.0
            };
        let mut width = (content_width + padding * 2.0).max(self.min_width);
        if self.full_width {
            width = width.max(ui.available_width());
        }
        let interactive = self.enabled && !self.busy;
        let sense = if interactive {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), sense);
        interact::describe(&response, WidgetType::Button, self.label);

        if ui.is_rect_visible(rect) {
            let state = State::of(ui, &response, response.id, &theme, false);
            let state = State {
                enabled: self.enabled,
                ..state
            };
            let colors = colors(&theme, self.kind, state);
            let radius = CornerRadius::same(theme.radius.md);
            let painter = ui.painter();
            painter.rect(
                rect,
                radius,
                colors.fill,
                Stroke::new(theme.stroke.hairline, colors.stroke),
                StrokeKind::Inside,
            );
            let mut x = rect.center().x - content_width / 2.0;
            if self.busy {
                crate::spinner::paint(
                    ui,
                    egui::pos2(x + icon_size / 2.0, rect.center().y),
                    icon_size,
                    colors.text,
                );
                x += icon_size + gap;
            } else if let Some(icon) = self.icon {
                icon.paint(
                    painter,
                    egui::pos2(x + icon_size / 2.0, rect.center().y),
                    icon_size,
                    colors.text,
                );
                x += icon_size + gap;
            }
            let text_pos = Align2::LEFT_CENTER
                .anchor_size(egui::pos2(x, rect.center().y), galley.size())
                .min;
            painter.galley(text_pos, galley, colors.text);
            if state.focused {
                interact::focus_ring(painter, rect, radius, &theme);
            }
        }
        response
    }
}

//! A square button with an icon and no label; its tooltip and accessible
//! name say what it does.

use std::hash::Hash;

use egui::{CornerRadius, Id, Response, Sense, Ui, Vec2, Widget, WidgetType};

use crate::icons::Icon;
use crate::interact::{self, State, mix, over};
use crate::theme::{self, Theme};
use crate::tooltip;

/// What the button looks like at rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// No frame until hovered. The default.
    Ghost,
    /// The accent colour: the one action of a bar, such as send.
    Filled,
    /// An action that destroys something.
    Danger,
}

/// How large.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Size {
    /// Inside a field or a row (4 less than the small control).
    Tiny,
    /// In dense bars (the small control height).
    Small,
    /// The default (the control height).
    Medium,
}

/// A square icon button.
#[derive(Clone, Debug)]
#[must_use = "add it with `ui.add(...)`"]
pub struct IconButton<'a> {
    icon: Icon,
    label: &'a str,
    kind: Kind,
    size: Size,
    selected: bool,
    enabled: bool,
    busy: bool,
    tooltip: bool,
    id_salt: Option<Id>,
}

impl<'a> IconButton<'a> {
    /// A ghost button with `icon`. `label` is its tooltip and its
    /// accessible name.
    pub fn new(icon: Icon, label: &'a str) -> Self {
        Self {
            icon,
            label,
            kind: Kind::Ghost,
            size: Size::Medium,
            selected: false,
            enabled: true,
            busy: false,
            tooltip: true,
            id_salt: None,
        }
    }

    /// An accent-filled button.
    pub fn filled(icon: Icon, label: &'a str) -> Self {
        Self::new(icon, label).kind(Kind::Filled)
    }

    /// A button for an action that destroys something.
    pub fn danger(icon: Icon, label: &'a str) -> Self {
        Self::new(icon, label).kind(Kind::Danger)
    }

    /// The look at rest.
    pub fn kind(mut self, kind: Kind) -> Self {
        self.kind = kind;
        self
    }

    /// The size.
    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }

    /// The small size.
    pub fn small(self) -> Self {
        self.size(Size::Small)
    }

    /// The tiny size, for inside a field.
    pub fn tiny(self) -> Self {
        self.size(Size::Tiny)
    }

    /// Whether it is toggled on (a ghost button then keeps a wash).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Whether it accepts input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Whether its action is under way: a progress mark replaces the icon
    /// and it takes no input.
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    /// Whether the label shows as a tooltip (it is the accessible name in
    /// either case).
    pub fn tooltip(mut self, tooltip: bool) -> Self {
        self.tooltip = tooltip;
        self
    }

    /// Keys its animation state to `salt` instead of its place in the
    /// layout.
    pub fn id_salt(mut self, salt: impl Hash) -> Self {
        self.id_salt = Some(Id::new(("monolith-icon-button", salt)));
        self
    }
}

struct Colors {
    fill: egui::Color32,
    icon: egui::Color32,
}

fn colors(theme: &Theme, kind: Kind, state: State) -> Colors {
    let c = &theme.color;
    let clear = egui::Color32::TRANSPARENT;
    if !state.enabled {
        return match kind {
            Kind::Filled => Colors {
                fill: c.bg_sunken,
                icon: c.text_disabled,
            },
            _ => Colors {
                fill: if state.selected { c.selected } else { clear },
                icon: c.text_disabled,
            },
        };
    }
    match kind {
        Kind::Filled => Colors {
            fill: if state.pressed {
                c.accent_pressed
            } else {
                mix(c.accent, c.accent_hover, state.hover)
            },
            icon: c.text_on_accent,
        },
        Kind::Ghost => {
            let tint = if state.pressed {
                c.pressed
            } else {
                c.hover.gamma_multiply(state.hover)
            };
            if state.selected {
                Colors {
                    fill: over(c.selected, tint),
                    icon: c.accent_text,
                }
            } else {
                Colors {
                    fill: tint,
                    icon: mix(c.text_secondary, c.text, state.hover),
                }
            }
        }
        Kind::Danger => {
            let wash = if state.pressed {
                over(c.danger_subtle, c.pressed)
            } else {
                c.danger_subtle.gamma_multiply(state.hover)
            };
            Colors {
                fill: if state.selected {
                    over(c.danger_subtle, wash)
                } else {
                    wash
                },
                icon: c.danger,
            }
        }
    }
}

impl Widget for IconButton<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let (side, icon_size, radius) = match self.size {
            Size::Tiny => (
                theme.size.control_sm - theme.space.xs,
                theme.size.icon_sm,
                theme.radius.sm,
            ),
            Size::Small => (theme.size.control_sm, theme.size.icon, theme.radius.sm),
            Size::Medium => (theme.size.control, theme.size.icon_md, theme.radius.md),
        };
        let interactive = self.enabled && !self.busy;
        let sense = if interactive {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (auto_id, rect) = ui.allocate_space(Vec2::splat(side));
        let id = self.id_salt.map_or(auto_id, |salt| ui.id().with(salt));
        let mut response = ui.interact(rect, id, sense);
        interact::describe_selectable(&response, WidgetType::Button, self.label, self.selected);

        if ui.is_rect_visible(rect) {
            // A button that takes no input does not react to the pointer,
            // so it does not go through `State::of` (which sets the cursor).
            let state = if interactive {
                State::of(ui, &response, id, &theme, self.selected)
            } else {
                State {
                    enabled: self.enabled,
                    hover: 0.0,
                    pressed: false,
                    focused: false,
                    selected: self.selected,
                }
            };
            let colors = colors(&theme, self.kind, state);
            let radius = CornerRadius::same(radius);
            ui.painter().rect_filled(rect, radius, colors.fill);
            if self.busy {
                crate::spinner::paint(ui, rect.center(), icon_size, colors.icon);
            } else {
                self.icon
                    .paint(ui.painter(), rect.center(), icon_size, colors.icon);
            }
            if state.focused {
                interact::focus_ring(ui.painter(), rect, radius, &theme);
            }
        }
        if self.tooltip {
            response = tooltip::attach(response, self.label);
        }
        response
    }
}

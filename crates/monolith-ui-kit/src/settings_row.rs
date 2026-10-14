//! Settings: a group is a rounded card with hairlines between its rows; a
//! row has an optional icon, a title, a description, and a control at the
//! right, or navigates to another view, or is a destructive entry.

use egui::{
    Align, Color32, CornerRadius, Frame, Id, Layout, Margin, Rect, Response, Sense, Shape, Stroke,
    Ui, UiBuilder, Vec2, WidgetType, layers::ShapeIdx,
};

use crate::icons::{self, Icon};
use crate::interact::{self, State, mix};
use crate::rows::{join_label, single_line, wrapped};
use crate::theme::{self, Theme};

/// What kind of row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// Shows a control, or only text.
    Static,
    /// Opens another view; shows a chevron.
    Navigate,
    /// A destructive entry.
    Danger,
}

/// A row of a [`SettingsGroup`].
#[must_use = "add it with `SettingsRows::row`"]
pub struct SettingsRow<'a> {
    id: Id,
    title: &'a str,
    description: &'a str,
    icon: Option<Icon>,
    trailing: Option<crate::rows::Trailing<'a>>,
    kind: Kind,
    enabled: bool,
}

impl<'a> SettingsRow<'a> {
    /// A row for the setting whose key is `id_salt`.
    pub fn new(id_salt: impl core::hash::Hash, title: &'a str) -> Self {
        Self {
            id: Id::new(("monolith-settings-row", id_salt)),
            title,
            description: "",
            icon: None,
            trailing: None,
            kind: Kind::Static,
            enabled: true,
        }
    }

    /// A line under the title, wrapped.
    pub fn description(mut self, description: &'a str) -> Self {
        self.description = description;
        self
    }

    /// An icon at the left.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The control at the right, laid out right to left and centred (a
    /// toggle, a segmented control, a value). In a row that navigates or is
    /// a danger entry it should only display: the row takes the clicks.
    pub fn trailing(mut self, add: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.trailing = Some(Box::new(add));
        self
    }

    /// The row opens another view: it is clickable and shows a chevron.
    pub fn navigates(mut self) -> Self {
        self.kind = Kind::Navigate;
        self
    }

    /// The row is a destructive entry: clickable, in the danger colour.
    pub fn danger(mut self) -> Self {
        self.kind = Kind::Danger;
        self
    }

    /// Whether it accepts input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

struct Pending {
    slot: ShapeIdx,
    rect: Rect,
    color: Color32,
    index: usize,
}

/// The rows of a group, as the closure of [`SettingsGroup::show`] adds
/// them.
pub struct SettingsRows<'u> {
    ui: &'u mut Ui,
    theme: std::sync::Arc<Theme>,
    count: usize,
    pending: Vec<Pending>,
}

impl SettingsRows<'_> {
    /// Adds a row. The response is clicked when a row that navigates or a
    /// danger entry is activated.
    pub fn row(&mut self, row: SettingsRow<'_>) -> Response {
        let theme = self.theme.clone();
        let c = &theme.color;
        let ui = &mut *self.ui;
        let index = self.count;
        self.count = self.count.saturating_add(1);

        let origin = ui.cursor().min;
        let width = ui.available_width();
        let min_height = theme.size.control_lg + theme.space.sm;
        let pad = theme.space.lg;

        let clickable = row.kind != Kind::Static && row.enabled;
        if index > 0 {
            // The hairline between rows, inset from the edges.
            ui.painter().hline(
                (origin.x + pad)..=(origin.x + width - pad),
                origin.y,
                Stroke::new(theme.stroke.hairline, c.border_subtle),
            );
        }
        let slot = ui.painter().add(Shape::Noop);

        let (title_color, secondary_color, tertiary_color, icon_color) = if !row.enabled {
            (
                c.text_disabled,
                c.text_disabled,
                c.text_disabled,
                c.text_disabled,
            )
        } else if row.kind == Kind::Danger {
            (c.danger, c.text_secondary, c.text_tertiary, c.danger)
        } else {
            (c.text, c.text_secondary, c.text_tertiary, c.text_secondary)
        };

        // The control at the right first: its width limits the text.
        let mut right = origin.x + width - pad;
        if row.kind == Kind::Navigate {
            right -= theme.size.icon_md;
            right -= theme.space.sm;
        }
        if let Some(add) = row.trailing {
            let slot_rect = Rect::from_min_max(
                egui::pos2(origin.x + width * 0.35, origin.y),
                egui::pos2(right, origin.y + min_height),
            );
            let mut child = ui.new_child(
                UiBuilder::new()
                    .id_salt(("monolith-settings-row-trailing", row.id))
                    .max_rect(slot_rect)
                    .layout(Layout::right_to_left(Align::Center)),
            );
            child.add_enabled_ui(row.enabled, |ui| add(ui));
            let used = child.min_rect();
            if used.width() > 0.0 {
                right = used.left() - theme.space.md;
            }
        }

        let mut left = origin.x + pad;
        let icon_slot = theme.size.icon_md;
        let icon_left = left;
        if row.icon.is_some() {
            left += icon_slot + theme.space.md;
        }
        let text_width = (right - left).max(1.0);
        let title = single_line(ui, row.title, &theme.text.body_strong, text_width);
        let description = (!row.description.is_empty()).then(|| {
            wrapped(
                ui,
                row.description,
                &theme.text.caption,
                Color32::PLACEHOLDER,
                text_width,
                6,
            )
        });
        let text_height = title.size().y
            + description
                .as_ref()
                .map_or(0.0, |g| g.size().y + theme.space.xxs);
        let height = (text_height + theme.space.md * 2.0).max(min_height);
        let (_, rect) = ui.allocate_space(Vec2::new(width, height));

        let sense = if clickable {
            Sense::click()
        } else {
            Sense::hover()
        };
        let response = ui.interact(rect, row.id, sense);
        let label = join_label(&[row.title, row.description]);
        interact::describe_enabled(&response, WidgetType::Button, &label, row.enabled);
        let state = State {
            enabled: row.enabled,
            ..State::of(ui, &response, row.id, &theme, false)
        };

        if ui.is_rect_visible(rect) {
            if clickable {
                let tint = if state.pressed {
                    c.pressed
                } else if row.kind == Kind::Danger {
                    c.danger_subtle.gamma_multiply(state.hover)
                } else {
                    c.hover.gamma_multiply(state.hover)
                };
                if tint.a() > 0 {
                    self.pending.push(Pending {
                        slot,
                        rect,
                        color: tint,
                        index,
                    });
                }
            }
            let top = rect.center().y - text_height / 2.0;
            let title_height = title.size().y;
            let painter = ui.painter();
            if let Some(icon) = row.icon {
                icon.paint(
                    painter,
                    egui::pos2(icon_left + icon_slot / 2.0, rect.center().y),
                    icon_slot,
                    icon_color,
                );
            }
            painter.galley(egui::pos2(left, top), title, title_color);
            if let Some(galley) = description {
                painter.galley(
                    egui::pos2(left, top + title_height + theme.space.xxs),
                    galley,
                    secondary_color,
                );
            }
            if row.kind == Kind::Navigate {
                icons::CHEVRON_RIGHT.paint(
                    painter,
                    egui::pos2(
                        origin.x + width - pad - theme.size.icon_md / 2.0,
                        rect.center().y,
                    ),
                    theme.size.icon_md,
                    mix(tertiary_color, secondary_color, state.hover),
                );
            }
            if state.focused {
                interact::focus_ring(
                    painter,
                    rect.shrink(theme.stroke.focus_gap + theme.stroke.focus),
                    CornerRadius::same(theme.radius.md),
                    &theme,
                );
            }
        }
        response
    }

    /// Paints the tints of the rows with the corners of the group on its
    /// first and last row.
    fn finish(self) {
        let theme = self.theme;
        let last = self.count.saturating_sub(1);
        let big = theme.radius.lg.saturating_sub(1);
        for pending in self.pending {
            let top = if pending.index == 0 { big } else { 0 };
            let bottom = if pending.index == last { big } else { 0 };
            let radius = CornerRadius {
                nw: top,
                ne: top,
                sw: bottom,
                se: bottom,
            };
            self.ui.painter().set(
                pending.slot,
                Shape::rect_filled(pending.rect.shrink(1.0), radius, pending.color),
            );
        }
    }
}

/// A card of settings rows.
#[derive(Clone, Copy, Debug)]
#[must_use = "show it with `.show(ui, ...)`"]
pub struct SettingsGroup {
    id: Id,
}

impl SettingsGroup {
    /// A group identified by `id_salt`.
    pub fn new(id_salt: impl core::hash::Hash) -> Self {
        Self {
            id: Id::new(("monolith-settings-group", id_salt)),
        }
    }

    /// Shows the group; `add` adds its rows.
    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut SettingsRows<'_>) -> R) -> R {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let frame = Frame::new()
            .fill(c.bg_card)
            .stroke(Stroke::new(theme.stroke.hairline, c.border_subtle))
            .corner_radius(CornerRadius::same(theme.radius.lg))
            .inner_margin(Margin::ZERO);
        frame
            .show(ui, |ui| {
                ui.push_id(self.id, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing = Vec2::ZERO;
                    let mut rows = SettingsRows {
                        ui,
                        theme: theme.clone(),
                        count: 0,
                        pending: Vec::new(),
                    };
                    let result = add(&mut rows);
                    rows.finish();
                    result
                })
                .inner
            })
            .inner
    }
}

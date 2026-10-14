//! The empty state: what a view shows when there is nothing in it yet. An
//! icon in a soft circle, a title, a sentence, and up to two actions,
//! centred in the space the view has.

use egui::epaint::text::{LayoutJob, TextWrapping};
use egui::{Align, Id, Layout, Rect, Sense, Ui, UiBuilder, Vec2, WidgetType};

use crate::button::Button;
use crate::icons::Icon;
use crate::interact;
use crate::theme;

/// The widest the text gets.
const TEXT_WIDTH: f32 = 360.0;

/// Which action was clicked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// The primary action was clicked.
    pub primary: bool,
    /// The secondary action was clicked.
    pub secondary: bool,
}

/// An empty view.
#[must_use = "show it with `.show(ui)`"]
pub struct EmptyState<'a> {
    id: Id,
    icon: Icon,
    title: &'a str,
    body: &'a str,
    primary: Option<&'a str>,
    secondary: Option<&'a str>,
}

impl<'a> EmptyState<'a> {
    /// An empty state identified by `id_salt`, with its icon and title.
    pub fn new(id_salt: impl core::hash::Hash, icon: Icon, title: &'a str) -> Self {
        Self {
            id: Id::new(("monolith-empty-state", id_salt)),
            icon,
            title,
            body: "",
            primary: None,
            secondary: None,
        }
    }

    /// The sentence under the title.
    pub fn body(mut self, body: &'a str) -> Self {
        self.body = body;
        self
    }

    /// The main action.
    pub fn primary(mut self, label: &'a str) -> Self {
        self.primary = Some(label);
        self
    }

    /// Another action.
    pub fn secondary(mut self, label: &'a str) -> Self {
        self.secondary = Some(label);
        self
    }

    /// Shows the state, centred in the available space.
    pub fn show(self, ui: &mut Ui) -> Output {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let area = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(area, Sense::hover());
        interact::describe(&response, WidgetType::Label, self.title);
        let mut output = Output::default();
        if !ui.is_rect_visible(area) {
            return output;
        }

        let width = TEXT_WIDTH.min((area.width() - theme.space.xxl * 2.0).max(120.0));
        let disc = 64.0;
        let title = centered(ui, self.title, &theme.text.title, width, 3);
        let body =
            (!self.body.is_empty()).then(|| centered(ui, self.body, &theme.text.body, width, 8));
        let has_actions = self.primary.is_some() || self.secondary.is_some();
        let mut height = disc + theme.space.xl + title.size().y;
        if let Some(body) = &body {
            height += theme.space.sm + body.size().y;
        }
        if has_actions {
            height += theme.space.xxl + theme.size.control;
        }

        let center_x = area.center().x;
        let mut y = area.center().y - height / 2.0;
        // Never above the top of the area, when it is too small.
        y = y.max(area.top());
        let painter = ui.painter();
        let disc_center = egui::pos2(center_x, y + disc / 2.0);
        painter.circle_filled(disc_center, disc / 2.0, c.accent_subtle);
        self.icon
            .paint(painter, disc_center, theme.size.icon_lg, c.accent_text);
        y += disc + theme.space.xl;

        let title_height = title.size().y;
        ui.painter().galley(egui::pos2(center_x, y), title, c.text);
        y += title_height;
        if let Some(body) = body {
            y += theme.space.sm;
            let body_height = body.size().y;
            ui.painter()
                .galley(egui::pos2(center_x, y), body, c.text_secondary);
            y += body_height;
        }

        if has_actions {
            y += theme.space.xxl;
            // The width of the buttons is known after they are laid out;
            // the pass before gives it for centring them, and a pass that
            // did not know it is laid out again before it is shown.
            let measured_id = self.id.with("actions-width");
            let known: f32 = ui
                .ctx()
                .data(|data| data.get_temp(measured_id))
                .unwrap_or(0.0);
            let half = if known > 0.0 {
                known / 2.0
            } else {
                width / 2.0
            };
            let slot = Rect::from_min_size(
                egui::pos2(center_x - half, y),
                Vec2::new(half * 2.0 + 1.0, theme.size.control),
            );
            let mut child = ui.new_child(
                UiBuilder::new()
                    .id_salt(self.id)
                    .max_rect(slot)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            child.spacing_mut().item_spacing.x = theme.space.sm;
            if let Some(label) = self.primary {
                output.primary = child.add(Button::primary(label)).clicked();
            }
            if let Some(label) = self.secondary {
                output.secondary = child.add(Button::secondary(label)).clicked();
            }
            let used = child.min_rect().width();
            if (used - known).abs() > 0.5 {
                ui.ctx()
                    .data_mut(|data| data.insert_temp(measured_id, used));
                ui.ctx()
                    .request_discard("the actions of an empty state were measured");
            }
        }
        output
    }
}

/// Text centred on the x of the position it is painted at, wrapped.
fn centered(
    ui: &Ui,
    text: &str,
    token: &theme::TextToken,
    width: f32,
    max_rows: usize,
) -> std::sync::Arc<egui::Galley> {
    let mut job =
        LayoutJob::single_section(text.to_owned(), token.format(egui::Color32::PLACEHOLDER));
    job.wrap = TextWrapping {
        max_width: width,
        max_rows,
        break_anywhere: false,
        overflow_character: Some('\u{2026}'),
    };
    job.halign = Align::Center;
    ui.painter().layout_job(job)
}

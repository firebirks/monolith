//! Notices inside a view: the banner, which spans the width and can carry
//! an action and a dismiss button, and the inline error under a field.

use egui::{
    Align, Color32, CornerRadius, Id, Layout, Rect, Response, Sense, Stroke, StrokeKind, Ui,
    UiBuilder, Vec2, Widget, WidgetType,
};

use crate::button::{Button, Size};
use crate::icons::{self, Icon};
use crate::interact::{self, State};
use crate::line::Line;
use crate::rows::{join_label, wrapped};
use crate::theme::{self, Theme};

/// What a banner is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Something to know.
    Info,
    /// Something went well.
    Success,
    /// Something needs attention.
    Warning,
    /// Something is wrong.
    Danger,
}

impl Tone {
    fn icon(self) -> Icon {
        match self {
            Self::Info => icons::INFO,
            Self::Success => icons::CIRCLE_CHECK,
            Self::Warning => icons::TRIANGLE_ALERT,
            Self::Danger => icons::CIRCLE_ALERT,
        }
    }

    /// The background and the colour of the icon.
    fn colors(self, theme: &Theme) -> (Color32, Color32) {
        let c = &theme.color;
        match self {
            Self::Info => (c.accent_subtle, c.accent_text),
            Self::Success => (c.success_subtle, c.success),
            Self::Warning => (c.warning_subtle, c.warning),
            Self::Danger => (c.danger_subtle, c.danger),
        }
    }
}

/// What the user did with a banner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// The action was clicked.
    pub action: bool,
    /// The dismiss button was clicked.
    pub dismiss: bool,
}

/// An inline notice.
#[must_use = "show it with `.show(ui)`"]
pub struct Banner<'a> {
    id: Id,
    tone: Tone,
    text: Line,
    title: &'a str,
    action: Option<&'a str>,
    dismiss: Option<&'a str>,
}

impl<'a> Banner<'a> {
    /// A banner identified by `id_salt`.
    pub fn new(id_salt: impl core::hash::Hash, tone: Tone, text: impl Into<Line>) -> Self {
        Self {
            id: Id::new(("monolith-banner", id_salt)),
            tone,
            text: text.into(),
            title: "",
            action: None,
            dismiss: None,
        }
    }

    /// A bold line above the text.
    pub fn title(mut self, title: &'a str) -> Self {
        self.title = title;
        self
    }

    /// An action button with this label.
    pub fn action(mut self, label: &'a str) -> Self {
        self.action = Some(label);
        self
    }

    /// A dismiss button; `label` is its accessible name.
    pub fn dismissible(mut self, label: &'a str) -> Self {
        self.dismiss = Some(label);
        self
    }

    /// Shows the banner across the available width.
    pub fn show(self, ui: &mut Ui) -> Output {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let (background, accent) = self.tone.colors(&theme);
        let width = ui.available_width();
        let origin = ui.cursor().min;
        let pad_x = theme.space.md + 2.0;
        let pad_y = theme.space.md - 2.0;
        let min_height = theme.size.control_lg;
        let slot = ui.painter().add(egui::Shape::Noop);
        let mut output = Output::default();

        // The buttons at the right first: their width limits the text.
        let mut right = origin.x + width - pad_x + 4.0;
        if self.action.is_some() || self.dismiss.is_some() {
            let area = Rect::from_min_max(
                egui::pos2(origin.x + width * 0.4, origin.y),
                egui::pos2(right, origin.y + min_height),
            );
            let mut child = ui.new_child(
                UiBuilder::new()
                    .id_salt(self.id)
                    .max_rect(area)
                    .layout(Layout::right_to_left(Align::Center)),
            );
            child.spacing_mut().item_spacing.x = theme.space.xs;
            if let Some(label) = self.dismiss {
                output.dismiss = dismiss_button(&mut child, &theme, self.id, label).clicked();
            }
            if let Some(label) = self.action {
                output.action = child
                    .add(Button::secondary(label).size(Size::Small))
                    .clicked();
            }
            let used = child.min_rect();
            if used.width() > 0.0 {
                right = used.left() - theme.space.md;
            } else {
                right = origin.x + width - pad_x;
            }
        } else {
            right = origin.x + width - pad_x;
        }

        let icon_size = theme.size.icon_md;
        let text_left = origin.x + pad_x + icon_size + theme.space.md;
        let text_width = (right - text_left).max(1.0);
        let title = (!self.title.is_empty()).then(|| {
            wrapped(
                ui,
                self.title,
                &theme.text.body_strong,
                Color32::PLACEHOLDER,
                text_width,
                3,
            )
        });
        let body = self.text.galley(
            ui,
            &theme.text.body,
            &theme.text.body_strong,
            Color32::PLACEHOLDER,
            text_width,
            12,
        );
        let title_height = title.as_ref().map_or(0.0, |g| g.size().y);
        let text_height = title_height + body.size().y;
        let height = (text_height + pad_y * 2.0).max(min_height);
        let (_, rect) = ui.allocate_space(Vec2::new(width, height));
        let info = ui.interact(rect, self.id.with("banner"), Sense::hover());
        interact::describe(
            &info,
            WidgetType::Label,
            &join_label(&[self.title, &self.text.plain()]),
        );

        if ui.is_rect_visible(rect) {
            let radius = CornerRadius::same(theme.radius.lg);
            ui.painter().set(
                slot,
                egui::Shape::Rect(egui::epaint::RectShape::new(
                    rect,
                    radius,
                    background,
                    Stroke::new(theme.stroke.hairline, accent.gamma_multiply(0.22)),
                    StrokeKind::Inside,
                )),
            );
            let top = rect.center().y - text_height / 2.0;
            let first_line = title
                .as_ref()
                .map_or(theme.text.body.line_height, |g| g.size().y);
            let painter = ui.painter();
            self.tone.icon().paint(
                painter,
                egui::pos2(origin.x + pad_x + icon_size / 2.0, top + first_line / 2.0),
                icon_size,
                accent,
            );
            if let Some(galley) = title {
                painter.galley(egui::pos2(text_left, top), galley, c.text);
                painter.galley(
                    egui::pos2(text_left, top + title_height),
                    body,
                    c.text_secondary,
                );
            } else {
                painter.galley(egui::pos2(text_left, top), body, c.text);
            }
        }
        output
    }
}

/// The small cross at the right of a banner.
fn dismiss_button(ui: &mut Ui, theme: &Theme, banner: Id, label: &str) -> Response {
    let c = &theme.color;
    let edge = theme.size.control_sm - 4.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(edge), Sense::click());
    interact::describe(&response, WidgetType::Button, label);
    let state = State::of(ui, &response, banner.with("dismiss"), theme, false);
    let tint = if state.pressed {
        c.pressed
    } else {
        c.hover.gamma_multiply(state.hover)
    };
    let radius = CornerRadius::same(theme.radius.sm);
    let painter = ui.painter();
    if tint.a() > 0 {
        painter.rect_filled(rect, radius, tint);
    }
    icons::X.paint(
        painter,
        rect.center(),
        theme.size.icon_sm,
        interact::mix(c.text_secondary, c.text, state.hover),
    );
    if state.focused {
        interact::focus_ring(painter, rect, radius, theme);
    }
    response
}

/// An error under a field or a form: a small alert icon and the text, in
/// the danger colour, wrapped.
#[derive(Clone, Copy, Debug)]
#[must_use = "add it with `ui.add(...)`"]
pub struct InlineError<'a> {
    text: &'a str,
}

impl<'a> InlineError<'a> {
    /// An error that reads `text`.
    pub fn new(text: &'a str) -> Self {
        Self { text }
    }
}

impl Widget for InlineError<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let icon_size = theme.size.icon_sm;
        let gap = theme.space.xs + 2.0;
        let width = ui.available_width();
        let galley = wrapped(
            ui,
            self.text,
            &theme.text.caption,
            Color32::PLACEHOLDER,
            width - icon_size - gap,
            6,
        );
        let height = galley.size().y.max(icon_size);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        interact::describe(&response, WidgetType::Label, self.text);
        if ui.is_rect_visible(rect) {
            let line = theme.text.caption.line_height;
            let painter = ui.painter();
            icons::CIRCLE_ALERT.paint(
                painter,
                egui::pos2(rect.left() + icon_size / 2.0, rect.top() + line / 2.0),
                icon_size,
                c.danger,
            );
            painter.galley(
                egui::pos2(rect.left() + icon_size + gap, rect.top()),
                galley,
                c.danger,
            );
        }
        response
    }
}

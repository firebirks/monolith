//! The badge (an unread count) and the pill (a short state label).

use egui::{
    Color32, CornerRadius, FontId, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget,
    WidgetType, pos2,
};

use crate::icons::Icon;
use crate::interact;
use crate::theme::{self, Theme};

/// The count above which a badge reads "99+".
const CAP: u32 = 99;

/// An unread count in a pill. A count of zero draws nothing.
#[derive(Clone, Copy, Debug)]
#[must_use = "add it with `ui.add(...)`"]
pub struct Badge {
    count: u32,
    muted: bool,
}

impl Badge {
    /// A badge for `count` unread items.
    pub fn new(count: u32) -> Self {
        Self {
            count,
            muted: false,
        }
    }

    /// The quiet variant, for a conversation whose notifications are off.
    pub fn muted(mut self, muted: bool) -> Self {
        self.muted = muted;
        self
    }

    /// The text the badge shows.
    pub fn text(&self) -> String {
        if self.count > CAP {
            format!("{CAP}+")
        } else {
            self.count.to_string()
        }
    }
}

/// The font of the numbers: the weight of the strong caption at the size
/// of the micro text.
fn count_font(theme: &Theme) -> FontId {
    FontId::new(
        theme.text.micro.size,
        theme.text.caption_strong.family.clone(),
    )
}

impl Widget for Badge {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        if self.count == 0 {
            return ui.allocate_response(Vec2::ZERO, Sense::hover());
        }
        let text = self.text();
        let galley =
            ui.painter()
                .layout_no_wrap(text.clone(), count_font(&theme), Color32::PLACEHOLDER);
        let height = theme.text.micro.line_height + theme.space.xs;
        let width = (galley.size().x + theme.space.sm).max(height);
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        interact::describe(&response, WidgetType::Label, &format!("{text} unread"));
        if ui.is_rect_visible(rect) {
            let (fill, ink) = if self.muted {
                (c.border_subtle, c.text_secondary)
            } else {
                (c.accent, c.text_on_accent)
            };
            let painter = ui.painter();
            painter.rect_filled(rect, CornerRadius::same(theme.radius.pill), fill);
            let pos = pos2(
                rect.center().x - galley.size().x / 2.0,
                rect.center().y - galley.size().y / 2.0,
            );
            painter.galley(pos, galley, ink);
        }
        response
    }
}

/// The meaning of a pill.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// No particular meaning.
    Neutral,
    /// The accent.
    Accent,
    /// Done, working, connected.
    Success,
    /// Needs attention.
    Warning,
    /// Wrong, or blocked.
    Danger,
}

/// A short state label, with an optional leading dot or icon.
#[derive(Clone, Debug)]
#[must_use = "add it with `ui.add(...)`"]
pub struct Pill<'a> {
    tone: Tone,
    text: &'a str,
    dot: bool,
    icon: Option<Icon>,
}

impl<'a> Pill<'a> {
    /// A pill of `tone` reading `text`.
    pub fn new(tone: Tone, text: &'a str) -> Self {
        Self {
            tone,
            text,
            dot: false,
            icon: None,
        }
    }

    /// A leading dot in the colour of the tone.
    pub fn dot(mut self, dot: bool) -> Self {
        self.dot = dot;
        self
    }

    /// A leading icon (instead of a dot).
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
}

fn tone_colors(theme: &Theme, tone: Tone) -> (Color32, Color32, Color32) {
    let c = &theme.color;
    match tone {
        Tone::Neutral => (c.bg_sunken, c.text_secondary, c.border_subtle),
        Tone::Accent => (c.accent_subtle, c.accent_text, Color32::TRANSPARENT),
        Tone::Success => (c.success_subtle, c.success, Color32::TRANSPARENT),
        Tone::Warning => (c.warning_subtle, c.warning, Color32::TRANSPARENT),
        Tone::Danger => (c.danger_subtle, c.danger, Color32::TRANSPARENT),
    }
}

impl Widget for Pill<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let (fill, ink, edge) = tone_colors(&theme, self.tone);
        let token = &theme.text.caption_strong;
        let galley = ui.painter().layout_no_wrap(
            self.text.to_owned(),
            token.font_id(),
            Color32::PLACEHOLDER,
        );
        let height = token.line_height + theme.space.xs;
        let pad = theme.space.sm;
        let mark = if self.icon.is_some() {
            token.size
        } else if self.dot {
            theme.space.sm - theme.space.xxs
        } else {
            0.0
        };
        let gap = if mark > 0.0 {
            theme.space.xs + 1.0
        } else {
            0.0
        };
        let width = pad * 2.0 + mark + gap + galley.size().x;
        let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
        interact::describe(&response, WidgetType::Label, self.text);
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let radius = CornerRadius::same(theme.radius.pill);
            painter.rect(
                rect,
                radius,
                fill,
                Stroke::new(theme.stroke.hairline, edge),
                StrokeKind::Inside,
            );
            let mut x = rect.left() + pad;
            let mid = rect.center().y;
            if let Some(icon) = self.icon {
                icon.paint(painter, pos2(x + mark / 2.0, mid), mark, ink);
            } else if self.dot {
                painter.circle_filled(pos2(x + mark / 2.0, mid), mark / 2.0, ink);
            }
            x += mark + gap;
            painter.galley(pos2(x, mid - galley.size().y / 2.0), galley, ink);
        }
        response
    }
}

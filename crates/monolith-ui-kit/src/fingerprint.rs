//! A fingerprint, its groups in rows of seven in the monospace face: the 13
//! groups of a full fingerprint make a row of seven and a row of six,
//! wherever it is shown, so that two people comparing it keep their place.
//! It can be selected and copied as it is written: groups separated by
//! single spaces.

use egui::text::LayoutJob;
use egui::text_selection::LabelSelectionState;
use egui::{Color32, Response, Sense, Stroke, Ui, Vec2, Widget, WidgetInfo, WidgetType};

use crate::theme::{self, TextToken};

/// Groups on a row when there is room.
const PER_ROW: usize = 7;

/// A fingerprint laid out in rows of groups.
#[derive(Clone, Debug)]
#[must_use = "add it with `ui.add(...)`"]
pub struct Fingerprint<'a> {
    text: &'a str,
    color: Option<Color32>,
    compact: bool,
    per_row: usize,
}

impl<'a> Fingerprint<'a> {
    /// The fingerprint `text`, its groups separated by spaces.
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            color: None,
            compact: false,
            per_row: PER_ROW,
        }
    }

    /// The colour of the text (main text otherwise).
    pub fn color(mut self, color: Color32) -> Self {
        self.color = Some(color);
        self
    }

    /// The size of a caption, for cards and rows; the monospace size of the
    /// theme otherwise.
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    /// At most this many groups on a row (seven by default). Fewer are put
    /// on a row that has no room for them.
    pub fn per_row(mut self, groups: usize) -> Self {
        self.per_row = groups.max(1);
        self
    }
}

impl Widget for Fingerprint<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let token = if self.compact {
            TextToken {
                size: theme.text.caption.size,
                line_height: theme.text.caption.line_height + 2.0,
                ..theme.text.mono.clone()
            }
        } else {
            theme.text.mono.clone()
        };
        let color = self.color.unwrap_or(theme.color.text);
        let groups: Vec<&str> = self.text.split_whitespace().collect();
        let text = groups.join(" ");

        // The width of a row of `count` groups, with the space after it or
        // without: the text wraps after the space that ends a full row, but
        // that space takes no room on screen.
        let row_width = |count: usize, space: bool| {
            let mut row = groups
                .iter()
                .take(count)
                .copied()
                .collect::<Vec<_>>()
                .join(" ");
            if space {
                row.push(' ');
            }
            let job = LayoutJob::single_section(row, token.format(color));
            ui.painter().layout_job(job).size().x
        };
        let available = ui.available_width();
        let mut count = self.per_row.min(groups.len()).max(1);
        while count > 1 && row_width(count, false) > available {
            count = count.saturating_sub(1);
        }

        let mut job = LayoutJob::single_section(text, token.format(color));
        job.wrap.max_width = row_width(count, true);
        let galley = ui.painter().layout_job(job);
        let size = Vec2::new(
            galley.size().x.min(row_width(count, false)),
            galley.size().y,
        );
        // Selectable as a label is: no stop of the Tab key.
        let (rect, response) = ui.allocate_exact_size(size, Sense::CLICK | Sense::DRAG);
        response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, galley.text()));
        if ui.is_rect_visible(rect) {
            LabelSelectionState::label_text_selection(
                ui,
                &response,
                rect.min,
                galley,
                color,
                Stroke::NONE,
            );
        }
        response
    }
}

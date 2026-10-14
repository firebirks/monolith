//! A fingerprint, its groups in rows of six in the monospace face, so that
//! two people comparing it keep their place. It can be selected and copied
//! as it is written: groups separated by single spaces.

use egui::text::LayoutJob;
use egui::{Color32, Label, Response, Ui, Widget};

use crate::theme::{self, TextToken};

/// Groups on a row when there is room.
const PER_ROW: usize = 6;

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

    /// At most this many groups on a row (six by default). Fewer are put on
    /// a row that has no room for them.
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

        // The width of a row of `count` groups and the space after it: the
        // text wraps after the space that ends a full row.
        let row_width = |count: usize| {
            let mut row = groups
                .iter()
                .take(count)
                .copied()
                .collect::<Vec<_>>()
                .join(" ");
            row.push(' ');
            let job = LayoutJob::single_section(row, token.format(color));
            ui.painter().layout_job(job).size().x
        };
        let available = ui.available_width();
        let mut count = self.per_row.min(groups.len()).max(1);
        while count > 1 && row_width(count) > available {
            count = count.saturating_sub(1);
        }

        let mut job = LayoutJob::single_section(text, token.format(color));
        job.wrap.max_width = row_width(count);
        let galley = ui.painter().layout_job(job);
        ui.add(Label::new(galley).selectable(true))
    }
}

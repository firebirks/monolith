//! The section header: a small, letter-spaced caption above a group of
//! rows, with an optional count or an optional icon action at the end.

use std::hash::Hash;

use egui::text::LayoutJob;
use egui::{Align2, Color32, Rect, Response, Sense, Ui, Vec2, WidgetType, pos2};

use crate::icon_button::IconButton;
use crate::icons::Icon;
use crate::interact;
use crate::theme;

/// What came of showing a section header.
#[derive(Debug)]
pub struct Output {
    /// The response of the header row.
    pub response: Response,
    /// The action button was clicked.
    pub action_clicked: bool,
}

/// A header above a group, such as "Requests".
#[derive(Clone, Debug)]
#[must_use = "show it with `.show(ui)`"]
pub struct SectionHeader<'a> {
    text: &'a str,
    count: Option<usize>,
    action: Option<(Icon, &'a str)>,
    action_salt: Option<egui::Id>,
}

impl<'a> SectionHeader<'a> {
    /// A header reading `text`.
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            count: None,
            action: None,
            action_salt: None,
        }
    }

    /// A count at the end of the row.
    pub fn count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }

    /// An icon button at the end of the row; `label` is its tooltip and
    /// accessible name; `salt` keys its animation state.
    pub fn action(mut self, icon: Icon, label: &'a str, salt: impl Hash) -> Self {
        self.action = Some((icon, label));
        self.action_salt = Some(egui::Id::new(("monolith-section-action", salt)));
        self
    }

    /// Shows the header across the available width.
    pub fn show(self, ui: &mut Ui) -> Output {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let height = theme.size.control_sm;
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        interact::describe(&response, WidgetType::Label, self.text);

        let mut clicked = false;
        let mut right = rect.right();
        if let Some((icon, label)) = self.action {
            let side = theme.size.control_sm - theme.space.xs;
            let slot = Rect::from_min_size(
                pos2(right - side, rect.center().y - side / 2.0),
                Vec2::splat(side),
            );
            let mut button = IconButton::new(icon, label).tiny();
            if let Some(salt) = self.action_salt {
                button = button.id_salt(salt);
            }
            clicked = ui.put(slot, button).clicked();
            right -= side + theme.space.xs;
        }
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            if let Some(count) = self.count {
                let galley = painter.layout_no_wrap(
                    count.to_string(),
                    theme.text.caption.font_id(),
                    Color32::PLACEHOLDER,
                );
                let pos = Align2::RIGHT_CENTER
                    .anchor_size(pos2(right, rect.center().y), galley.size())
                    .min;
                painter.galley(pos, galley, c.text_tertiary);
            }
            // Letter-spaced a little, which keeps small text readable.
            let mut format = theme.text.caption_strong.format(Color32::PLACEHOLDER);
            format.extra_letter_spacing = 0.4;
            let mut job = LayoutJob::single_section(self.text.to_owned(), format);
            job.wrap.max_rows = 1;
            job.wrap.max_width = (right - rect.left()).max(0.0);
            let galley = painter.layout_job(job);
            let pos = Align2::LEFT_CENTER
                .anchor_size(pos2(rect.left(), rect.center().y), galley.size())
                .min;
            painter.galley(pos, galley, c.text_tertiary);
        }
        Output {
            response,
            action_clicked: clicked,
        }
    }
}

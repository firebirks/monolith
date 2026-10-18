//! Words someone else wrote, shown as theirs (`DESIGN_QUESTIONS.md` P6-5
//! and P7-2): in a sunken box, each under a label the client gives and
//! behind a bar that marks a quotation, and never inside a sentence of the
//! client. Whatever the words hold, quotation marks included, they stay in
//! their box: nothing they say can read as something the client says.

use egui::{CornerRadius, Frame, Label, Margin, Response, Stroke, Ui, Vec2};

use crate::rows::wrapped;
use crate::theme;

/// The words of someone else, each under its label.
#[derive(Debug, Default)]
#[must_use = "show it with `.show(ui)`"]
pub struct TheirWords<'a> {
    parts: Vec<(&'a str, &'a str, usize)>,
}

impl<'a> TheirWords<'a> {
    /// No words yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// `text`, under the caller's `label` that says whose words they are,
    /// laid out in at most `max_rows` rows. Empty text is left out.
    pub fn part(mut self, label: &'a str, text: &'a str, max_rows: usize) -> Self {
        if !text.trim().is_empty() {
            self.parts.push((label, text, max_rows));
        }
        self
    }

    /// Whether there are no words to show.
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Shows the box across the available width. The words can be
    /// selected; the labels cannot.
    pub fn show(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        // A hairline as well as the fill: on a card of the dark theme the
        // fill alone hardly shows.
        Frame::new()
            .fill(c.bg_sunken)
            .stroke(Stroke::new(theme.stroke.hairline, c.border_subtle))
            .corner_radius(CornerRadius::same(theme.radius.md))
            .inner_margin(Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = theme.space.xs;
                for (index, (label, text, max_rows)) in self.parts.into_iter().enumerate() {
                    if index > 0 {
                        ui.add_space(theme.space.xs);
                    }
                    let width = ui.available_width();
                    ui.add(
                        Label::new(wrapped(
                            ui,
                            label,
                            &theme.text.caption_strong,
                            c.text_tertiary,
                            width,
                            1,
                        ))
                        .selectable(false),
                    );
                    let quote = Frame::new().inner_margin(Margin {
                        left: 11,
                        right: 0,
                        top: 0,
                        bottom: 0,
                    });
                    let shown = quote.show(ui, |ui| {
                        let width = ui.available_width();
                        ui.add(
                            Label::new(wrapped(
                                ui,
                                text,
                                &theme.text.body,
                                c.text,
                                width,
                                max_rows,
                            ))
                            .selectable(true),
                        );
                    });
                    let rect = shown.response.rect;
                    let bar = egui::Rect::from_min_max(
                        rect.min + Vec2::new(0.0, 1.0),
                        egui::pos2(rect.min.x + 3.0, rect.max.y - 1.0),
                    );
                    ui.painter().rect_filled(
                        bar,
                        CornerRadius::same(theme.radius.xs),
                        c.border_strong,
                    );
                }
            })
            .response
    }
}

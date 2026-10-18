//! A line the client says that names someone (`DESIGN_QUESTIONS.md`
//! P7-2).
//!
//! A name in a sentence of the client (a contact, an identity, an
//! invitation, a file) is not the client's own words: a contact's name can
//! hold what a sender wrote, kept when the user accepted the name a request
//! suggested. A [`Line`] keeps each name as a part of its own, and draws it
//! in the strong style of the sentence, without quotation marks around it.
//! Whatever a name holds, quotation marks and full stops included, then
//! reads as part of the name and never as more of the client's sentence.

use core::fmt;
use std::sync::Arc;

use egui::text::{LayoutJob, TextWrapping};
use egui::{Color32, Galley, Label, Response, Ui, Widget};

use crate::interact;
use crate::theme::TextToken;

/// A part of a line.
#[derive(Clone, PartialEq, Eq)]
enum Part {
    /// The client's own words.
    Said(String),
    /// A name, drawn apart from them.
    Name(String),
}

/// A line of the client's words and the names in it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Line {
    parts: Vec<Part>,
}

impl Line {
    /// A line that begins with the client's words `text`.
    pub fn said(text: impl Into<String>) -> Self {
        Self::default().and(text)
    }

    /// The line with more of the client's words.
    pub fn and(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        if !text.is_empty() {
            self.parts.push(Part::Said(text));
        }
        self
    }

    /// The line with a name, drawn as one.
    pub fn name(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        if !name.is_empty() {
            self.parts.push(Part::Name(name));
        }
        self
    }

    /// Whether the line says nothing.
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// The names in the line, in order.
    pub fn names(&self) -> Vec<&str> {
        self.parts
            .iter()
            .filter_map(|part| match part {
                Part::Name(name) => Some(name.as_str()),
                Part::Said(_) => None,
            })
            .collect()
    }

    /// The line as one text, for what a screen reader is told.
    pub fn plain(&self) -> String {
        self.parts
            .iter()
            .map(|part| match part {
                Part::Said(text) | Part::Name(text) => text.as_str(),
            })
            .collect()
    }

    /// The layout of the line: the client's words in `token`, each name in
    /// `strong`, everything in `color`. It wraps at `max_width` and ends in
    /// an ellipsis after `max_rows`.
    pub fn job(
        &self,
        token: &TextToken,
        strong: &TextToken,
        color: Color32,
        max_width: f32,
        max_rows: usize,
    ) -> LayoutJob {
        let mut job = LayoutJob::default();
        for part in &self.parts {
            match part {
                Part::Said(text) => job.append(text, 0.0, token.format(color)),
                Part::Name(name) => job.append(name, 0.0, strong.format(color)),
            }
        }
        job.wrap = TextWrapping {
            max_width: max_width.max(1.0),
            max_rows,
            break_anywhere: false,
            overflow_character: Some('\u{2026}'),
        };
        job
    }

    /// [`Self::job`] laid out.
    pub fn galley(
        &self,
        ui: &Ui,
        token: &TextToken,
        strong: &TextToken,
        color: Color32,
        max_width: f32,
        max_rows: usize,
    ) -> Arc<Galley> {
        ui.painter()
            .layout_job(self.job(token, strong, color, max_width, max_rows))
    }
}

impl From<&str> for Line {
    fn from(text: &str) -> Self {
        Self::said(text)
    }
}

impl From<String> for Line {
    fn from(text: String) -> Self {
        Self::said(text)
    }
}

impl From<&String> for Line {
    fn from(text: &String) -> Self {
        Self::said(text.as_str())
    }
}

impl fmt::Debug for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A line can name a contact.
        f.debug_struct("Line").finish_non_exhaustive()
    }
}

/// A [`Line`] as a paragraph: it wraps at the width it has.
#[must_use = "add it with `ui.add(...)`"]
pub struct Sentence<'a> {
    line: &'a Line,
    token: &'a TextToken,
    strong: &'a TextToken,
    color: Color32,
}

impl<'a> Sentence<'a> {
    /// `line` in the style `token`, its names in `strong`, in `color`.
    pub fn new(
        line: &'a Line,
        token: &'a TextToken,
        strong: &'a TextToken,
        color: Color32,
    ) -> Self {
        Self {
            line,
            token,
            strong,
            color,
        }
    }
}

impl Widget for Sentence<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let job = self.line.job(
            self.token,
            self.strong,
            self.color,
            ui.available_width(),
            usize::MAX,
        );
        let response = Label::new(job).selectable(false).wrap().ui(ui);
        interact::describe(&response, egui::WidgetType::Label, &self.line.plain());
        response
    }
}

//! Text in one style of the theme and one of its colours. Screens draw
//! every line of text with this, or with a component that holds it.

use egui::{Color32, Label, Response, RichText, Ui, Widget};

use crate::theme::TextToken;

/// What text does when it is wider than the room it has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Overflow {
    /// As the layout says: it wraps in a column and extends in a row.
    Layout,
    /// It goes on to the next line.
    Wrap,
    /// It is cut with an ellipsis.
    Truncate,
}

/// A run of text.
#[derive(Debug)]
pub struct Text<'a> {
    content: &'a str,
    token: &'a TextToken,
    color: Color32,
    overflow: Overflow,
    selectable: bool,
}

impl<'a> Text<'a> {
    /// `content` in the style `token` and `color`, not selectable. It
    /// wraps in a column and extends in a row, as the layout says.
    pub fn new(content: &'a str, token: &'a TextToken, color: Color32) -> Self {
        Self {
            content,
            token,
            color,
            overflow: Overflow::Layout,
            selectable: false,
        }
    }

    /// Goes on to the next line where it does not fit, in any layout.
    pub fn wrap(mut self) -> Self {
        self.overflow = Overflow::Wrap;
        self
    }

    /// Cut with an ellipsis where it does not fit, on one line.
    pub fn truncate(mut self) -> Self {
        self.overflow = Overflow::Truncate;
        self
    }

    /// Lets the user select the text and copy it with the keyboard or the
    /// context menu of the system.
    pub fn selectable(mut self, selectable: bool) -> Self {
        self.selectable = selectable;
        self
    }
}

impl Widget for Text<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let text = RichText::new(self.content)
            .font(self.token.font_id())
            .line_height(Some(self.token.line_height))
            .extra_letter_spacing(self.token.letter_spacing)
            .color(self.color);
        let label = Label::new(text).selectable(self.selectable);
        match self.overflow {
            Overflow::Layout => label,
            Overflow::Wrap => label.wrap(),
            Overflow::Truncate => label.truncate(),
        }
        .ui(ui)
    }
}

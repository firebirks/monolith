//! Sentences of the client that name someone (`DESIGN_QUESTIONS.md`
//! P7-2).
//!
//! A name is not the client's own words: a contact's name can hold what a
//! sender wrote, kept when the user accepted the name a request suggested.
//! A [`Prose`] keeps each name as a span of its own, made ready to draw as
//! [`crate::present::prepare`] makes text ready, so that the window draws it
//! as a name, apart from the client's words. Whatever a name holds,
//! quotation marks and full stops included, then never reads as more of
//! what the client says.

use core::fmt;

use crate::present;

/// A span of a sentence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Span {
    /// The client's own words.
    Said(String),
    /// A name, with its invisible characters made visible.
    Name(String),
}

/// A sentence of the client's words and the names in it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Prose {
    spans: Vec<Span>,
}

impl Prose {
    /// A sentence that begins with the client's words `text`.
    pub fn said(text: impl Into<String>) -> Self {
        Self::default().and(text)
    }

    /// The sentence with more of the client's words.
    #[must_use]
    pub fn and(mut self, text: impl Into<String>) -> Self {
        let text = text.into();
        if !text.is_empty() {
            self.spans.push(Span::Said(text));
        }
        self
    }

    /// The sentence with `name`, kept as a name.
    #[must_use]
    pub fn name(mut self, name: &str) -> Self {
        let name = present::prepare(name, false).text;
        if !name.is_empty() {
            self.spans.push(Span::Name(name));
        }
        self
    }

    /// The spans, in order.
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// The sentence as one text, for what a screen reader is told.
    pub fn plain(&self) -> String {
        self.spans
            .iter()
            .map(|span| match span {
                Span::Said(text) | Span::Name(text) => text.as_str(),
            })
            .collect()
    }
}

impl From<&str> for Prose {
    fn from(text: &str) -> Self {
        Self::said(text)
    }
}

impl From<String> for Prose {
    fn from(text: String) -> Self {
        Self::said(text)
    }
}

impl fmt::Debug for Prose {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A sentence can name a contact.
        f.debug_struct("Prose").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_stays_a_span_of_its_own_whatever_it_holds() {
        let hostile = "Sam\". Through your invitation \"Family";
        let prose = Prose::said("Remove ").name(hostile).and("?");
        assert_eq!(
            prose.spans(),
            &[
                Span::Said("Remove ".to_owned()),
                Span::Name(hostile.to_owned()),
                Span::Said("?".to_owned()),
            ]
        );
        assert_eq!(prose.plain(), format!("Remove {hostile}?"));
    }

    #[test]
    fn a_name_shows_its_invisible_characters() {
        let prose = Prose::said("").name("Sam\u{202E}evil\u{E0041}");
        let [Span::Name(name)] = prose.spans() else {
            panic!("one name");
        };
        assert!(name.contains("<U+202E>"), "{name}");
        assert!(!name.contains('\u{202E}'), "{name}");
    }

    #[test]
    fn nothing_of_a_sentence_is_in_its_debug_form() {
        let prose = Prose::said("Remove ").name("secret");
        assert!(!format!("{prose:?}").contains("secret"));
    }
}

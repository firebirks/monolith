//! Validated text fields.
//!
//! The rules are those of `docs/PROTOCOL.md` section 9. A value of one of
//! the types below has passed them; there is no other way to build one. The
//! same constructors are used for text that arrives from a peer and for text
//! the local user typed, so a conforming peer never sends text that the
//! other side rejects.
//!
//! Every rule here is a byte length, a count of scalar values, or
//! membership in a fixed list of code points. None depends on Unicode
//! tables, so two builds never disagree about whether a text is valid, and
//! accepted text is kept byte for byte as it was sent. In particular no
//! normalization form is required or applied.
//!
//! None of these types implements `Display`, and their `Debug` output is a
//! fixed label: text from a peer does not reach a log by accident.

use core::fmt;

use monolith_identity::redact::REDACTED;

use crate::ProtocolError;
use crate::limits::{
    MAX_CHAT_TEXT_LEN, MAX_DISPLAY_NAME_LEN, MAX_DISPLAY_NAME_SCALARS, MAX_FILENAME_LEN,
    MAX_INTRODUCTION_TEXT_LEN, MAX_PROFILE_TEXT_LEN,
};

/// Characters that no text field accepts: C0 and C1 controls other than
/// tab, line feed and carriage return (those three are decided per field),
/// DEL, the line and paragraph separators, and noncharacters.
fn is_forbidden_everywhere(character: char) -> bool {
    let value = u32::from(character);
    matches!(
        value,
        0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F | 0x7F..=0x9F | 0x2028 | 0x2029 | 0xFDD0..=0xFDEF
    ) || (value & 0xFFFE) == 0xFFFE
}

/// Characters that display names and filenames reject in addition:
/// tab, line feed and carriage return; bidirectional controls; the code
/// points that are ignorable by default, variation selectors excepted; a few
/// more that are drawn as nothing; and every whitespace character other than
/// U+0020.
fn is_forbidden_in_names(character: char) -> bool {
    matches!(
        u32::from(character),
        // Tab, line feed, carriage return.
        0x09 | 0x0A | 0x0D
        // Bidirectional controls.
        | 0x061C | 0x200E | 0x200F | 0x202A..=0x202E | 0x2066..=0x2069
        // Default_Ignorable_Code_Point, except the bidirectional controls
        // above and the variation selectors (U+180B to U+180D, U+180F,
        // U+FE00 to U+FE0F, U+E0100 to U+E01EF), which stay allowed.
        | 0x00AD | 0x034F | 0x115F | 0x1160 | 0x17B4 | 0x17B5 | 0x180E | 0x200B..=0x200D
        | 0x2060..=0x2065 | 0x206A..=0x206F | 0x3164 | 0xFEFF | 0xFFA0 | 0xFFF0..=0xFFF8
        | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A | 0xE0000..=0xE00FF | 0xE01F0..=0xE0FFF
        // Not ignorable by default, but drawn as nothing or as a blank: the
        // blank braille pattern, the interlinear annotation controls and
        // the object replacement character.
        | 0x2800 | 0xFFF9..=0xFFFC
        // White_Space other than U+0020. The remaining members of the
        // property are rejected by is_forbidden_everywhere.
        | 0x00A0 | 0x1680 | 0x2000..=0x200A | 0x202F | 0x205F | 0x3000
    )
}

fn decode_utf8(bytes: &[u8], min: usize, max: usize) -> Result<&str, ProtocolError> {
    if bytes.len() > max {
        return Err(ProtocolError::FieldTooLong);
    }
    if bytes.len() < min {
        return Err(ProtocolError::InvalidValue);
    }
    core::str::from_utf8(bytes).map_err(|_| ProtocolError::InvalidUtf8)
}

/// Rules for chat text, introductions and profile text: tab and line feed
/// are allowed, carriage return is not.
fn check_free_text(bytes: &[u8], min: usize, max: usize) -> Result<&str, ProtocolError> {
    let text = decode_utf8(bytes, min, max)?;
    if text
        .chars()
        .any(|c| c == '\r' || is_forbidden_everywhere(c))
    {
        return Err(ProtocolError::ForbiddenCharacter);
    }
    Ok(text)
}

/// Rules shared by display names and filenames.
fn check_name(bytes: &[u8], min: usize, max: usize) -> Result<&str, ProtocolError> {
    let text = decode_utf8(bytes, min, max)?;
    if text
        .chars()
        .any(|c| is_forbidden_everywhere(c) || is_forbidden_in_names(c))
    {
        return Err(ProtocolError::ForbiddenCharacter);
    }
    if text.starts_with(' ') || text.ends_with(' ') {
        return Err(ProtocolError::ForbiddenCharacter);
    }
    Ok(text)
}

macro_rules! text_type {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, Hash)]
        pub struct $name(String);

        impl $name {
            /// Validates text the local user supplied.
            pub fn new(text: &str) -> Result<Self, ProtocolError> {
                Self::from_bytes(text.as_bytes())
            }

            /// Returns the validated text. Callers must not log it.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            /// Returns the validated text as bytes.
            pub fn as_bytes(&self) -> &[u8] {
                self.0.as_bytes()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "({})"), REDACTED)
            }
        }
    };
}

text_type!(
    /// The text of a chat message: 1 to [`MAX_CHAT_TEXT_LEN`] bytes.
    ChatText
);

impl ChatText {
    /// Validates bytes received from a peer.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        check_free_text(bytes, 1, MAX_CHAT_TEXT_LEN).map(|text| Self(text.to_owned()))
    }
}

text_type!(
    /// The introduction in a contact request: up to
    /// [`MAX_INTRODUCTION_TEXT_LEN`] bytes, possibly empty.
    IntroductionText
);

impl IntroductionText {
    /// Validates bytes received from a peer.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        check_free_text(bytes, 0, MAX_INTRODUCTION_TEXT_LEN).map(|text| Self(text.to_owned()))
    }
}

text_type!(
    /// The profile text of a contact: up to [`MAX_PROFILE_TEXT_LEN`] bytes,
    /// possibly empty.
    ProfileText
);

impl ProfileText {
    /// Validates bytes received from a peer.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        check_free_text(bytes, 0, MAX_PROFILE_TEXT_LEN).map(|text| Self(text.to_owned()))
    }
}

text_type!(
    /// A display name: up to [`MAX_DISPLAY_NAME_LEN`] bytes and
    /// [`MAX_DISPLAY_NAME_SCALARS`] scalar values, possibly empty.
    ///
    /// A display name is never an identifier. Two contacts may have the same
    /// one, and the same name may arrive in different byte sequences that
    /// are drawn alike, because no normalization is required. Nothing that
    /// decides identity, authentication, contact equality, authorization,
    /// duplicate detection or protocol state may look at a display name.
    /// A front end may normalize a copy for drawing or searching; the bytes
    /// held here stay as they were sent.
    DisplayName
);

impl DisplayName {
    /// Validates bytes received from a peer.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let text = check_name(bytes, 0, MAX_DISPLAY_NAME_LEN)?;
        if text.chars().count() > MAX_DISPLAY_NAME_SCALARS {
            return Err(ProtocolError::FieldTooLong);
        }
        Ok(Self(text.to_owned()))
    }
}

text_type!(
    /// The filename in a file offer: 1 to [`MAX_FILENAME_LEN`] bytes.
    ///
    /// It is a display string. It is never used as a path; see
    /// [`Filename::save_name`].
    Filename
);

/// Name used when nothing usable is left of a received filename.
const FALLBACK_SAVE_NAME: &str = "file";

/// Characters that are reserved on common filesystems.
const RESERVED_IN_SAVE_NAMES: [char; 7] = ['<', '>', ':', '"', '|', '?', '*'];

/// Device names that some systems treat specially, with or without an
/// extension. `COM` and `LPT` followed by one digit are handled separately.
const RESERVED_DEVICE_NAMES: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];

/// Returns true if `stem`, the part of a name before its first dot, is a
/// reserved device name. Letter case does not matter.
fn is_reserved_device_name(stem: &str) -> bool {
    if RESERVED_DEVICE_NAMES
        .iter()
        .any(|device| stem.eq_ignore_ascii_case(device))
    {
        return true;
    }
    // COM0 to COM9 and LPT0 to LPT9, and the same with a superscript one,
    // two or three, which the systems in question accept as digits here.
    let Some((port, unit)) = stem.split_at_checked(3) else {
        return false;
    };
    let mut unit = unit.chars();
    (port.eq_ignore_ascii_case("COM") || port.eq_ignore_ascii_case("LPT"))
        && matches!(
            (unit.next(), unit.next()),
            (Some('0'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'), None)
        )
}

impl Filename {
    /// Validates bytes received from a peer.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let text = check_name(bytes, 1, MAX_FILENAME_LEN)?;
        if text.contains(['/', '\\']) {
            return Err(ProtocolError::ForbiddenCharacter);
        }
        if text == "." || text == ".." {
            return Err(ProtocolError::InvalidValue);
        }
        Ok(Self(text.to_owned()))
    }

    /// Returns a name that is safe to suggest when the user saves the file.
    ///
    /// The result is a single path component: it contains no separator, is
    /// never empty, never `.` or `..`, has no leading or trailing dot or
    /// space, contains none of the characters that common filesystems
    /// reserve, is not a reserved device name, and is at most
    /// [`MAX_FILENAME_LEN`] bytes long. It is a suggestion for a save dialog
    /// and nothing else; the caller still creates the file with
    /// exclusive-create semantics in a directory the user chose.
    pub fn save_name(&self) -> String {
        let replaced: String = self
            .0
            .chars()
            .map(|c| {
                if RESERVED_IN_SAVE_NAMES.contains(&c) {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        let trimmed = replaced.trim_matches(|c| c == '.' || c == ' ');
        if trimmed.is_empty() {
            return FALLBACK_SAVE_NAME.to_owned();
        }
        let stem = trimmed.split('.').next().unwrap_or(trimmed);
        if !is_reserved_device_name(stem.trim_end_matches(' ')) {
            // Not longer than the validated name it was cut from.
            return trimmed.to_owned();
        }

        // The prefix makes the name one byte longer. Cut it back to the
        // limit, on a character boundary, and trim again what the cut may
        // have exposed. The prefix itself always remains.
        let mut name = format!("_{trimmed}");
        if name.len() > MAX_FILENAME_LEN {
            let mut end = MAX_FILENAME_LEN;
            while !name.is_char_boundary(end) {
                end = end.saturating_sub(1);
            }
            name.truncate(end);
            let kept = name.trim_end_matches(['.', ' ']).len();
            name.truncate(kept);
        }
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(text: &str) -> Result<ChatText, ProtocolError> {
        ChatText::new(text)
    }

    #[test]
    fn chat_text_accepts_ordinary_text_with_tabs_and_line_feeds() {
        assert!(chat("hello").is_ok());
        assert!(chat("line one\nline two\tindented").is_ok());
        assert!(chat("\u{e9}\u{4e2d}\u{6587} \u{1f600}").is_ok());
        // Bidirectional and zero-width characters are allowed in chat text.
        assert!(chat("a\u{202e}b\u{200d}c").is_ok());
    }

    #[test]
    fn chat_text_length_bounds() {
        assert_eq!(chat(""), Err(ProtocolError::InvalidValue));
        assert!(chat(&"a".repeat(MAX_CHAT_TEXT_LEN)).is_ok());
        assert_eq!(
            chat(&"a".repeat(MAX_CHAT_TEXT_LEN + 1)),
            Err(ProtocolError::FieldTooLong)
        );
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        for bytes in [
            &[0xff_u8][..],
            &[0xc3][..],
            &[0xe2, 0x82][..],
            &[0xc0, 0xaf][..],
            &[0xed, 0xa0, 0x80][..],
            &[b'a', 0x80, b'b'][..],
        ] {
            assert_eq!(ChatText::from_bytes(bytes), Err(ProtocolError::InvalidUtf8));
            assert_eq!(
                DisplayName::from_bytes(bytes),
                Err(ProtocolError::InvalidUtf8)
            );
            assert_eq!(Filename::from_bytes(bytes), Err(ProtocolError::InvalidUtf8));
        }
    }

    #[test]
    fn control_characters_are_rejected_everywhere() {
        let forbidden = [
            '\u{0}',
            '\u{1}',
            '\u{8}',
            '\u{b}',
            '\u{c}',
            '\u{e}',
            '\u{1b}',
            '\u{1f}',
            '\u{7f}',
            '\u{80}',
            '\u{85}',
            '\u{9f}',
            '\u{2028}',
            '\u{2029}',
            '\u{fdd0}',
            '\u{fdef}',
            '\u{fffe}',
            '\u{ffff}',
            '\u{1fffe}',
            '\u{10ffff}',
        ];
        for character in forbidden {
            let text = format!("a{character}b");
            assert_eq!(
                chat(&text),
                Err(ProtocolError::ForbiddenCharacter),
                "chat {:x}",
                u32::from(character)
            );
            assert_eq!(
                DisplayName::new(&text),
                Err(ProtocolError::ForbiddenCharacter),
                "name {:x}",
                u32::from(character)
            );
            assert_eq!(
                Filename::new(&text),
                Err(ProtocolError::ForbiddenCharacter),
                "filename {:x}",
                u32::from(character)
            );
        }
    }

    #[test]
    fn carriage_return_is_rejected_in_free_text() {
        assert_eq!(chat("a\r\nb"), Err(ProtocolError::ForbiddenCharacter));
        assert_eq!(
            IntroductionText::new("a\rb"),
            Err(ProtocolError::ForbiddenCharacter)
        );
        assert_eq!(
            ProfileText::new("a\rb"),
            Err(ProtocolError::ForbiddenCharacter)
        );
    }

    #[test]
    fn introduction_and_profile_may_be_empty_and_are_bounded() {
        assert!(IntroductionText::new("").is_ok());
        assert!(IntroductionText::new(&"a".repeat(MAX_INTRODUCTION_TEXT_LEN)).is_ok());
        assert_eq!(
            IntroductionText::new(&"a".repeat(MAX_INTRODUCTION_TEXT_LEN + 1)),
            Err(ProtocolError::FieldTooLong)
        );
        assert!(ProfileText::new("").is_ok());
        assert!(ProfileText::new(&"a".repeat(MAX_PROFILE_TEXT_LEN)).is_ok());
        assert_eq!(
            ProfileText::new(&"a".repeat(MAX_PROFILE_TEXT_LEN + 1)),
            Err(ProtocolError::FieldTooLong)
        );
    }

    #[test]
    fn display_name_accepts_ordinary_names() {
        for name in [
            "",
            "Alice",
            "Jos\u{e9} Mar\u{ed}a",
            "\u{5c71}\u{7530} \u{592a}\u{90ce}",
        ] {
            assert!(DisplayName::new(name).is_ok(), "{name:?}");
        }
    }

    #[test]
    fn display_name_rejects_line_breaks_and_tabs() {
        for name in ["a\nb", "a\tb", "a\rb"] {
            assert_eq!(
                DisplayName::new(name),
                Err(ProtocolError::ForbiddenCharacter)
            );
        }
    }

    #[test]
    fn display_name_rejects_bidirectional_controls() {
        for character in [
            '\u{61c}', '\u{200e}', '\u{200f}', '\u{202a}', '\u{202b}', '\u{202c}', '\u{202d}',
            '\u{202e}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        ] {
            assert_eq!(
                DisplayName::new(&format!("a{character}b")),
                Err(ProtocolError::ForbiddenCharacter),
                "{:x}",
                u32::from(character)
            );
        }
    }

    #[test]
    fn display_name_rejects_invisible_characters() {
        for character in [
            '\u{ad}',
            '\u{34f}',
            '\u{180e}',
            '\u{200b}',
            '\u{200c}',
            '\u{200d}',
            '\u{2060}',
            '\u{2064}',
            '\u{feff}',
            '\u{fff9}',
            '\u{fffb}',
            '\u{e0001}',
            '\u{e007f}',
        ] {
            assert_eq!(
                DisplayName::new(&format!("a{character}b")),
                Err(ProtocolError::ForbiddenCharacter),
                "{:x}",
                u32::from(character)
            );
        }
    }

    #[test]
    fn display_name_allows_only_the_ordinary_space_as_whitespace() {
        assert!(DisplayName::new("a b").is_ok());
        for character in [
            '\u{a0}', '\u{1680}', '\u{2000}', '\u{2003}', '\u{200a}', '\u{202f}', '\u{205f}',
            '\u{3000}',
        ] {
            assert_eq!(
                DisplayName::new(&format!("a{character}b")),
                Err(ProtocolError::ForbiddenCharacter),
                "{:x}",
                u32::from(character)
            );
        }
    }

    #[test]
    fn display_name_rejects_leading_and_trailing_spaces() {
        for name in [" a", "a ", " ", "  a  "] {
            assert_eq!(
                DisplayName::new(name),
                Err(ProtocolError::ForbiddenCharacter)
            );
        }
    }

    #[test]
    fn display_name_length_is_bounded_in_bytes_and_in_scalars() {
        assert!(DisplayName::new(&"a".repeat(MAX_DISPLAY_NAME_SCALARS)).is_ok());
        assert_eq!(
            DisplayName::new(&"a".repeat(MAX_DISPLAY_NAME_SCALARS + 1)),
            Err(ProtocolError::FieldTooLong)
        );
        // 43 three-byte characters are 129 bytes: within the scalar limit,
        // over the byte limit.
        let wide = "\u{4e2d}".repeat(43);
        assert!(wide.len() > MAX_DISPLAY_NAME_LEN);
        assert_eq!(DisplayName::new(&wide), Err(ProtocolError::FieldTooLong));
        assert!(DisplayName::new(&"\u{4e2d}".repeat(42)).is_ok());
    }

    #[test]
    fn display_names_are_kept_as_sent_whatever_their_normalization() {
        // "e" followed by a combining acute accent, and the composed
        // character. The two are drawn alike. Both are accepted, neither
        // is changed, and they stay two different names: normalization is
        // not part of the protocol, so that builds with different Unicode
        // tables cannot disagree about a name.
        let decomposed = DisplayName::new("Jose\u{301}").unwrap();
        let composed = DisplayName::new("Jos\u{e9}").unwrap();
        assert_eq!(decomposed.as_bytes(), "Jose\u{301}".as_bytes());
        assert_eq!(composed.as_bytes(), "Jos\u{e9}".as_bytes());
        assert_ne!(decomposed, composed);

        // Other forms that normalization would change: a compatibility
        // character, Hangul written as jamo, marks in non-canonical order.
        for name in ["\u{fb01}", "\u{1112}\u{1161}\u{11ab}", "q\u{301}\u{323}"] {
            assert_eq!(
                DisplayName::new(name).unwrap().as_bytes(),
                name.as_bytes(),
                "{name:?}"
            );
        }
    }

    #[test]
    fn filename_accepts_ordinary_names() {
        for name in [
            "a",
            "report.pdf",
            "archive.tar.gz",
            "\u{6587}\u{4ef6}.txt",
            "a b.txt",
        ] {
            assert!(Filename::new(name).is_ok(), "{name:?}");
        }
    }

    #[test]
    fn filename_rejects_separators_and_traversal() {
        for name in [
            "a/b",
            "/etc/passwd",
            "..\\..\\x",
            "a\\b",
            "../x",
            "x/..",
            "\\\\server\\share",
        ] {
            assert_eq!(
                Filename::new(name),
                Err(ProtocolError::ForbiddenCharacter),
                "{name:?}"
            );
        }
        assert_eq!(Filename::new("."), Err(ProtocolError::InvalidValue));
        assert_eq!(Filename::new(".."), Err(ProtocolError::InvalidValue));
    }

    #[test]
    fn filename_rejects_the_right_to_left_override_trick() {
        assert_eq!(
            Filename::new("photo\u{202e}gpj.exe"),
            Err(ProtocolError::ForbiddenCharacter)
        );
    }

    #[test]
    fn filename_length_bounds() {
        assert_eq!(Filename::new(""), Err(ProtocolError::InvalidValue));
        assert!(Filename::new(&"a".repeat(MAX_FILENAME_LEN)).is_ok());
        assert_eq!(
            Filename::new(&"a".repeat(MAX_FILENAME_LEN + 1)),
            Err(ProtocolError::FieldTooLong)
        );
    }

    #[test]
    fn save_name_keeps_ordinary_names() {
        assert_eq!(
            Filename::new("report.pdf").unwrap().save_name(),
            "report.pdf"
        );
    }

    #[test]
    fn save_name_strips_leading_and_trailing_dots() {
        assert_eq!(Filename::new(".bashrc").unwrap().save_name(), "bashrc");
        assert_eq!(Filename::new("name.").unwrap().save_name(), "name");
        assert_eq!(Filename::new("...").unwrap().save_name(), "file");
        assert_eq!(Filename::new(". .").unwrap().save_name(), "file");
    }

    #[test]
    fn save_name_replaces_reserved_characters() {
        assert_eq!(
            Filename::new("C:evil.txt").unwrap().save_name(),
            "C_evil.txt"
        );
        assert_eq!(
            Filename::new("a<b>c|d?e*f\"g").unwrap().save_name(),
            "a_b_c_d_e_f_g"
        );
    }

    #[test]
    fn save_name_defuses_reserved_device_names() {
        assert_eq!(Filename::new("CON").unwrap().save_name(), "_CON");
        assert_eq!(Filename::new("nul.txt").unwrap().save_name(), "_nul.txt");
        assert_eq!(
            Filename::new("Com1.tar.gz").unwrap().save_name(),
            "_Com1.tar.gz"
        );
        assert_eq!(
            Filename::new("LPT9 .txt").unwrap().save_name(),
            "_LPT9 .txt"
        );
        for (name, expected) in [
            ("COM0", "_COM0"),
            ("lpt0.txt", "_lpt0.txt"),
            ("COM\u{b9}", "_COM\u{b9}"),
            ("com\u{b2}.log", "_com\u{b2}.log"),
            ("LPT\u{b3}.tar.gz", "_LPT\u{b3}.tar.gz"),
            ("CONIN$", "_CONIN$"),
            ("conout$.txt", "_conout$.txt"),
            // A reserved character is replaced first; what is left is not a
            // device name.
            ("CON:", "CON_"),
        ] {
            assert_eq!(Filename::new(name).unwrap().save_name(), expected);
        }
        // Names that merely start with a device name are left alone.
        for name in [
            "CONSOLE.txt",
            "COM",
            "COM10",
            "COM1a",
            "LPT",
            "LPTX",
            "COM\u{2074}",
            "CONIN",
            "NULL",
        ] {
            assert_eq!(Filename::new(name).unwrap().save_name(), name);
        }
    }

    #[test]
    fn save_name_is_never_longer_than_a_filename() {
        // The longest filename that is a device name with an extension.
        let longest = format!("CON.{}", "a".repeat(MAX_FILENAME_LEN - 4));
        let saved = Filename::new(&longest).unwrap().save_name();
        assert_eq!(saved.len(), MAX_FILENAME_LEN);
        assert!(saved.starts_with("_CON.a"));

        // The cut falls inside a three-byte character and is moved back to
        // its start; the dot that the cut exposes is trimmed.
        let wide = format!("NUL.{}.\u{4e2d}", "a".repeat(MAX_FILENAME_LEN - 8));
        assert_eq!(wide.len(), MAX_FILENAME_LEN);
        let saved = Filename::new(&wide).unwrap().save_name();
        assert_eq!(saved, format!("_NUL.{}", "a".repeat(MAX_FILENAME_LEN - 8)));
        assert!(saved.len() <= MAX_FILENAME_LEN);
    }

    #[test]
    fn names_reject_characters_that_draw_as_nothing() {
        for character in [
            '\u{115f}',
            '\u{1160}',
            '\u{17b4}',
            '\u{17b5}',
            '\u{2065}',
            '\u{206a}',
            '\u{206f}',
            '\u{2800}',
            '\u{3164}',
            '\u{ffa0}',
            '\u{fffc}',
            '\u{1d173}',
            '\u{1d17a}',
            '\u{fff0}',
            '\u{fff8}',
            '\u{1bca0}',
            '\u{1bca3}',
            '\u{e0080}',
            '\u{e00ff}',
            '\u{e01f0}',
            '\u{e0fff}',
        ] {
            let text = format!("a{character}b");
            assert_eq!(
                DisplayName::new(&text),
                Err(ProtocolError::ForbiddenCharacter),
                "name {:x}",
                u32::from(character)
            );
            assert_eq!(
                Filename::new(&text),
                Err(ProtocolError::ForbiddenCharacter),
                "filename {:x}",
                u32::from(character)
            );
            // Free text is delivered as sent.
            assert!(ChatText::new(&text).is_ok());
        }
        // Variation selectors stay allowed: emoji and some scripts need
        // them.
        assert!(DisplayName::new("\u{2764}\u{fe0f}").is_ok());
        for selector in [
            '\u{180b}',
            '\u{180d}',
            '\u{180f}',
            '\u{fe00}',
            '\u{fe0f}',
            '\u{e0100}',
            '\u{e01ef}',
        ] {
            assert!(
                Filename::new(&format!("a{selector}b")).is_ok(),
                "{:x}",
                u32::from(selector)
            );
        }
    }

    #[test]
    fn debug_output_shows_no_text() {
        assert_eq!(
            format!("{:?}", chat("secret words").unwrap()),
            "ChatText([redacted])"
        );
        assert_eq!(
            format!("{:?}", DisplayName::new("Alice").unwrap()),
            "DisplayName([redacted])"
        );
        assert_eq!(
            format!("{:?}", Filename::new("plan.txt").unwrap()),
            "Filename([redacted])"
        );
    }
}

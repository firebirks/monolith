//! Validated text fields.
//!
//! The rules are those of `docs/PROTOCOL.md` section 9. A value of one of
//! the types below has passed them; there is no other way to build one. The
//! same constructors are used for text that arrives from a peer and for text
//! the local user typed, so a conforming peer never sends text that the
//! other side rejects.
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
/// tab, line feed and carriage return; bidirectional controls; zero-width
/// and other invisible format characters; and every whitespace character
/// other than U+0020.
fn is_forbidden_in_names(character: char) -> bool {
    matches!(
        u32::from(character),
        // Tab, line feed, carriage return.
        0x09 | 0x0A | 0x0D
        // Bidirectional controls.
        | 0x061C | 0x200E | 0x200F | 0x202A..=0x202E | 0x2066..=0x2069
        // Zero-width and invisible format characters.
        | 0x00AD | 0x034F | 0x180E | 0x200B..=0x200D | 0x2060..=0x2064 | 0xFEFF
        | 0xFFF9..=0xFFFB | 0xE0000..=0xE007F
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
    /// [`MAX_DISPLAY_NAME_SCALARS`] scalar values, in Normalization Form C,
    /// possibly empty.
    ///
    /// A display name is never an identifier. Two contacts may have the same
    /// one.
    DisplayName
);

impl DisplayName {
    /// Validates bytes received from a peer.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let text = check_name(bytes, 0, MAX_DISPLAY_NAME_LEN)?;
        if text.chars().count() > MAX_DISPLAY_NAME_SCALARS {
            return Err(ProtocolError::FieldTooLong);
        }
        if !unicode_normalization::is_nfc(text) {
            return Err(ProtocolError::InvalidValue);
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
/// extension.
const RESERVED_DEVICE_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

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
    /// reserve, and is not a reserved device name. It is a suggestion for a
    /// save dialog and nothing else; the caller still creates the file with
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
        if RESERVED_DEVICE_NAMES
            .iter()
            .any(|device| stem.trim_end_matches(' ').eq_ignore_ascii_case(device))
        {
            return format!("_{trimmed}");
        }
        trimmed.to_owned()
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
    fn display_name_must_be_in_nfc() {
        // "e" followed by a combining acute accent is not NFC; the composed
        // character is.
        assert_eq!(
            DisplayName::new("Jose\u{301}"),
            Err(ProtocolError::InvalidValue)
        );
        assert!(DisplayName::new("Jos\u{e9}").is_ok());
        // Combining marks that have no composed form are fine.
        assert!(DisplayName::new("q\u{301}").is_ok());
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
        // Names that merely start with a device name are left alone.
        assert_eq!(
            Filename::new("CONSOLE.txt").unwrap().save_name(),
            "CONSOLE.txt"
        );
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

//! Text validators: arbitrary bytes are accepted exactly when the rules of
//! `docs/PROTOCOL.md` section 9 say so. The rules are written out here a
//! second time, as tables, independently of the validators. Includes the
//! save-name suggestion for filenames, which must always be a single safe
//! path component.
//!
//! No rule depends on Unicode tables: accepted text is the input, byte for
//! byte.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_protocol::limits::{
    MAX_CHAT_TEXT_LEN, MAX_DISPLAY_NAME_LEN, MAX_DISPLAY_NAME_SCALARS, MAX_FILENAME_LEN,
    MAX_INTRODUCTION_TEXT_LEN, MAX_PROFILE_TEXT_LEN,
};
use monolith_protocol::text::{ChatText, DisplayName, Filename, IntroductionText, ProfileText};

/// Rejected in every text field. Tab, line feed and carriage return are
/// decided per field.
const REJECTED_EVERYWHERE: [(u32, u32); 7] = [
    (0x0000, 0x0008),
    (0x000B, 0x000C),
    (0x000E, 0x001F),
    (0x007F, 0x009F),
    (0x2028, 0x2029),
    (0xFDD0, 0xFDEF),
    // The last two code points of the basic plane. The other planes are
    // handled in `rejected_everywhere`.
    (0xFFFE, 0xFFFF),
];

/// Rejected in display names and filenames in addition.
const REJECTED_IN_NAMES: [(u32, u32); 31] = [
    // Tab, line feed, carriage return.
    (0x0009, 0x000A),
    (0x000D, 0x000D),
    // Bidirectional controls.
    (0x061C, 0x061C),
    (0x200E, 0x200F),
    (0x202A, 0x202E),
    (0x2066, 0x2069),
    // Code points that are ignorable by default, without the variation
    // selectors, and a few more that are drawn as nothing.
    (0x00AD, 0x00AD),
    (0x034F, 0x034F),
    (0x115F, 0x1160),
    (0x17B4, 0x17B5),
    (0x180E, 0x180E),
    (0x200B, 0x200D),
    (0x2060, 0x2065),
    (0x206A, 0x206F),
    (0x2800, 0x2800),
    (0x3164, 0x3164),
    (0xFEFF, 0xFEFF),
    (0xFFA0, 0xFFA0),
    (0xFFF0, 0xFFFC),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0000, 0xE00FF),
    (0xE01F0, 0xE0FFF),
    // Whitespace other than U+0020.
    (0x00A0, 0x00A0),
    (0x1680, 0x1680),
    (0x2000, 0x200A),
    (0x202F, 0x202F),
    (0x205F, 0x205F),
    (0x3000, 0x3000),
    // Listed twice on purpose: these two are whitespace and are already
    // rejected everywhere.
    (0x0085, 0x0085),
    (0x2028, 0x2029),
];

fn in_table(table: &[(u32, u32)], character: char) -> bool {
    let value = u32::from(character);
    table
        .iter()
        .any(|(first, last)| (*first..=*last).contains(&value))
}

fn rejected_everywhere(character: char) -> bool {
    // Noncharacters: the last two code points of every plane.
    in_table(&REJECTED_EVERYWHERE, character) || u32::from(character) % 0x1_0000 >= 0xFFFE
}

fn rejected_in_names(character: char) -> bool {
    rejected_everywhere(character) || in_table(&REJECTED_IN_NAMES, character)
}

fn free_text_is_valid(data: &[u8], min: usize, max: usize) -> bool {
    let Ok(text) = core::str::from_utf8(data) else {
        return false;
    };
    (min..=max).contains(&data.len()) && !text.chars().any(|c| c == '\r' || rejected_everywhere(c))
}

fn name_is_valid(data: &[u8], min: usize, max: usize) -> bool {
    let Ok(text) = core::str::from_utf8(data) else {
        return false;
    };
    (min..=max).contains(&data.len())
        && !text.chars().any(rejected_in_names)
        && !text.starts_with(' ')
        && !text.ends_with(' ')
}

fn filename_is_valid(data: &[u8]) -> bool {
    name_is_valid(data, 1, MAX_FILENAME_LEN)
        && !data.contains(&b'/')
        && !data.contains(&b'\\')
        && data != b"."
        && data != b".."
}

/// Every reserved device name, written out.
fn is_device_name(stem: &str) -> bool {
    let fixed = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
    if fixed.iter().any(|name| stem.eq_ignore_ascii_case(name)) {
        return true;
    }
    ["COM", "LPT"].iter().any(|port| {
        "0123456789\u{b9}\u{b2}\u{b3}"
            .chars()
            .any(|unit| stem.eq_ignore_ascii_case(&format!("{port}{unit}")))
    })
}

fuzz_target!(|data: &[u8]| {
    let chat = ChatText::from_bytes(data);
    assert_eq!(chat.is_ok(), free_text_is_valid(data, 1, MAX_CHAT_TEXT_LEN));
    if let Ok(text) = chat {
        assert_eq!(text.as_bytes(), data);
    }
    assert_eq!(
        IntroductionText::from_bytes(data).is_ok(),
        free_text_is_valid(data, 0, MAX_INTRODUCTION_TEXT_LEN)
    );
    assert_eq!(
        ProfileText::from_bytes(data).is_ok(),
        free_text_is_valid(data, 0, MAX_PROFILE_TEXT_LEN)
    );

    let name = DisplayName::from_bytes(data);
    let name_expected = name_is_valid(data, 0, MAX_DISPLAY_NAME_LEN)
        && core::str::from_utf8(data)
            .is_ok_and(|text| text.chars().count() <= MAX_DISPLAY_NAME_SCALARS);
    assert_eq!(name.is_ok(), name_expected);
    if let Ok(name) = name {
        assert_eq!(name.as_bytes(), data);
    }

    let filename = Filename::from_bytes(data);
    assert_eq!(filename.is_ok(), filename_is_valid(data));
    if let Ok(filename) = filename {
        assert_eq!(filename.as_bytes(), data);

        let saved = filename.save_name();
        assert!(!saved.is_empty() && saved.len() <= MAX_FILENAME_LEN);
        assert!(saved != "." && saved != "..");
        assert!(!saved.contains(['/', '\\', '<', '>', ':', '"', '|', '?', '*']));
        assert!(!saved.starts_with(['.', ' ']) && !saved.ends_with(['.', ' ']));
        assert!(!saved.chars().any(rejected_in_names));
        let stem = saved.split('.').next().unwrap_or(&saved);
        assert!(!is_device_name(stem.trim_end_matches(' ')));
    }
});

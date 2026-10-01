//! Text validators: arbitrary bytes are accepted or rejected without a
//! panic, and whatever is accepted holds the invariants of its type.
//! Includes the save-name suggestion for filenames, which must always be a
//! single safe path component.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_protocol::limits::{
    MAX_CHAT_TEXT_LEN, MAX_DISPLAY_NAME_LEN, MAX_DISPLAY_NAME_SCALARS, MAX_FILENAME_LEN,
    MAX_INTRODUCTION_TEXT_LEN, MAX_PROFILE_TEXT_LEN,
};
use monolith_protocol::text::{ChatText, DisplayName, Filename, IntroductionText, ProfileText};

fn no_forbidden_controls(text: &str, allow_tab_and_line_feed: bool) -> bool {
    text.chars().all(|c| {
        let allowed_control = allow_tab_and_line_feed && (c == '\t' || c == '\n');
        allowed_control || !c.is_control()
    })
}

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = ChatText::from_bytes(data) {
        assert!(!text.as_str().is_empty());
        assert!(text.as_bytes().len() <= MAX_CHAT_TEXT_LEN);
        assert!(no_forbidden_controls(text.as_str(), true));
        assert_eq!(text.as_bytes(), data);
    }
    if let Ok(text) = IntroductionText::from_bytes(data) {
        assert!(text.as_bytes().len() <= MAX_INTRODUCTION_TEXT_LEN);
        assert!(no_forbidden_controls(text.as_str(), true));
    }
    if let Ok(text) = ProfileText::from_bytes(data) {
        assert!(text.as_bytes().len() <= MAX_PROFILE_TEXT_LEN);
        assert!(no_forbidden_controls(text.as_str(), true));
    }
    if let Ok(name) = DisplayName::from_bytes(data) {
        let name = name.as_str();
        assert!(name.len() <= MAX_DISPLAY_NAME_LEN);
        assert!(name.chars().count() <= MAX_DISPLAY_NAME_SCALARS);
        assert!(no_forbidden_controls(name, false));
        assert!(!name.starts_with(' ') && !name.ends_with(' '));
    }
    if let Ok(filename) = Filename::from_bytes(data) {
        let shown = filename.as_str();
        assert!(!shown.is_empty() && shown.len() <= MAX_FILENAME_LEN);
        assert!(no_forbidden_controls(shown, false));
        assert!(!shown.contains(['/', '\\']));

        let saved = filename.save_name();
        assert!(!saved.is_empty());
        assert!(saved != "." && saved != "..");
        assert!(!saved.contains(['/', '\\', '<', '>', ':', '"', '|', '?', '*']));
        assert!(!saved.starts_with(['.', ' ']) && !saved.ends_with(['.', ' ']));
        assert!(no_forbidden_controls(&saved, false));
    }
});

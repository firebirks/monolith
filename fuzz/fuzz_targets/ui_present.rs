//! The presentation functions of the desktop client under arbitrary text:
//! `prepare`, `preview`, `sendable` and the name helpers. Nothing panics;
//! what is drawn is bounded by the rendering limits of
//! `RESOURCE_LIMITS.md` section 10; no invisible or direction-changing
//! character reaches the output as itself, only as its `<U+XXXX>` mark;
//! the text of an unexpanded message is a prefix of the expanded one;
//! a preview is one line cut at the character limit; a draft is sendable
//! exactly when it holds a character that is not white space and is not
//! longer than a message may be.
//!
//! The first byte selects: bit 0 expands the message, bits 1 to 3 the
//! character limit of the preview, bit 4 removes the local name. The rest
//! is the text, taken lossily as UTF-8. The name helpers get the first half
//! of the text (cut at a character) as the name and the second half as the
//! fingerprint.
//!
//! The classes of characters and of white space are written out here from
//! `PROTOCOL.md` section 9, independently of `present.rs`. The ignorable
//! characters of the planes above the basic one, U+2800 and U+FFFC are in
//! that list but not in the set `present.rs` shows, so they are not
//! checked.

#![no_main]

use libfuzzer_sys::fuzz_target;
use monolith_ui_model::data::{ContactView, IdentityView};
use monolith_ui_model::ids::{UiContactId, UiIdentityId};
use monolith_ui_model::present::{
    MAX_MESSAGE_BYTES, MAX_RENDERED_MESSAGE_LINES, contact_name, ended_contact_name,
    fallback_contact_name, first_group, identity_name, prepare, preview, sendable,
};

/// Drawn as nothing or changing the direction of text: shown as a mark.
const INVISIBLE: [(u32, u32); 14] = [
    (0x00AD, 0x00AD),
    (0x034F, 0x034F),
    (0x061C, 0x061C),
    (0x115F, 0x1160),
    (0x17B4, 0x17B5),
    (0x180E, 0x180E),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x2064),
    (0x2066, 0x206F),
    (0x3164, 0x3164),
    (0xFEFF, 0xFEFF),
    (0xFFA0, 0xFFA0),
    (0xFFF9, 0xFFFB),
];

/// The characters with the Unicode property White_Space.
const WHITE_SPACE: [(u32, u32); 10] = [
    (0x0009, 0x000D),
    (0x0020, 0x0020),
    (0x0085, 0x0085),
    (0x00A0, 0x00A0),
    (0x1680, 0x1680),
    (0x2000, 0x200A),
    (0x2028, 0x2029),
    (0x202F, 0x202F),
    (0x205F, 0x205F),
    (0x3000, 0x3000),
];

const PREVIEW_LIMITS: [usize; 8] = [0, 1, 2, 5, 12, 40, 80, 300];

fn in_table(table: &[(u32, u32)], character: char) -> bool {
    let value = u32::from(character);
    table
        .iter()
        .any(|(low, high)| (*low..=*high).contains(&value))
}

fn invisible(character: char) -> bool {
    in_table(&INVISIBLE, character)
}

fn white_space(character: char) -> bool {
    in_table(&WHITE_SPACE, character)
}

fn check_names(name: &str, fingerprint: &str, no_name: bool) {
    let group = first_group(fingerprint);
    assert!(fingerprint.starts_with(group));
    assert!(!group.contains(' '));
    assert!(group.len() == fingerprint.len() || fingerprint[group.len()..].starts_with(' '));

    let local = (!no_name).then(|| name.to_owned());
    let expected_fallback = format!("Contact {group}");
    assert_eq!(fallback_contact_name(fingerprint), expected_fallback);
    assert_eq!(
        ended_contact_name(fingerprint),
        format!("Former contact {group}")
    );

    let contact = ContactView {
        id: UiContactId::new(UiIdentityId::from_bytes([1; 16]), [2; 16]),
        name: local.clone(),
        fingerprint: fingerprint.to_owned(),
        full_fingerprint: String::new(),
        accepted: true,
        verified: false,
        invitation: false,
    };
    let shown = contact_name(&contact);
    match &local {
        Some(given) if !given.is_empty() => assert_eq!(&shown, given),
        _ => assert_eq!(shown, expected_fallback),
    }

    let identity = IdentityView {
        id: UiIdentityId::from_bytes([1; 16]),
        name: local.clone(),
        fingerprint: fingerprint.to_owned(),
        full_fingerprint: String::new(),
    };
    let shown = identity_name(&identity);
    match &local {
        Some(given) if !given.is_empty() => assert_eq!(&shown, given),
        _ => assert_eq!(shown, format!("Identity {group}")),
    }
}

fn check_prepare(text: &str, expanded: bool) {
    let prepared = prepare(text, expanded);
    let shown = &prepared.text;

    // No invisible character is drawn as itself.
    assert!(!shown.chars().any(invisible), "an invisible character");
    // Each character becomes at most a mark of eight characters.
    assert!(shown.chars().count() <= text.chars().count().saturating_mul(8));
    assert!(shown.len() <= text.len().saturating_mul(8));

    // A text too short to hold an over-long cluster comes out exactly as
    // the rules say.
    if text.chars().count() <= 16 {
        let mut expected = String::new();
        for character in text.chars() {
            if invisible(character) {
                expected.push_str(&format!("<U+{:04X}>", u32::from(character)));
            } else {
                expected.push(character);
            }
        }
        assert_eq!(shown, &expected);
        assert!(!prepared.collapsed);
    }

    let newlines = shown.matches('\n').count();
    let input_newlines = text.matches('\n').count();
    assert!(newlines <= input_newlines);
    if expanded {
        assert!(!prepared.collapsed);
    } else {
        assert!(newlines < MAX_RENDERED_MESSAGE_LINES);
        // Collapsed exactly when a line past the limit was left out.
        assert_eq!(
            prepared.collapsed,
            input_newlines >= MAX_RENDERED_MESSAGE_LINES
        );
        if prepared.collapsed {
            assert_eq!(newlines, MAX_RENDERED_MESSAGE_LINES - 1);
        }
        // The unexpanded text is a prefix of the expanded one.
        let full = prepare(text, true);
        assert!(full.text.starts_with(shown.as_str()));
        if !prepared.collapsed {
            assert_eq!(&full.text, shown);
        }
    }
}

fn check_preview(text: &str, limit: usize) {
    let shown = preview(text, limit);
    assert!(!shown.chars().any(invisible), "an invisible character");
    assert!(shown.chars().count() <= limit.saturating_add(1));
    assert!(!shown.starts_with(' '));
    assert!(shown.chars().all(|c| c == ' ' || !white_space(c)));

    // What the preview shows is the words of the prepared text, joined
    // by single spaces, up to the limit.
    let prepared = prepare(text, false);
    let words: Vec<&str> = prepared
        .text
        .split(white_space)
        .filter(|word| !word.is_empty())
        .collect();
    let full = words.join(" ");
    let full_len = full.chars().count();
    if full_len <= limit && !prepared.collapsed {
        // Nothing was cut: the line as it is, with no ellipsis added.
        assert_eq!(shown, full);
        return;
    }
    // The text before the ellipsis is a prefix of the full line, within the
    // limit. The text may end in an ellipsis of its own, so a missing one
    // is only noticed where the line is longer than the limit.
    let kept = shown.strip_suffix('\u{2026}').unwrap_or(&shown);
    assert!(kept.chars().count() <= limit);
    assert!(
        full.starts_with(kept.trim_end_matches(' ')),
        "the preview is not a prefix of the line"
    );
    if full_len > limit {
        assert!(
            shown.ends_with('\u{2026}'),
            "no ellipsis where the line was cut"
        );
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&selector, rest)) = data.split_first() else {
        return;
    };
    let text = String::from_utf8_lossy(rest);
    let text: &str = &text;

    check_prepare(text, selector & 1 != 0);
    check_prepare(text, selector & 1 == 0);
    check_preview(text, PREVIEW_LIMITS[usize::from((selector >> 1) & 7)]);

    let expected = text.len() <= MAX_MESSAGE_BYTES && text.chars().any(|c| !white_space(c));
    assert_eq!(sendable(text), expected);

    let mut middle = text.len() / 2;
    while !text.is_char_boundary(middle) {
        middle -= 1;
    }
    let (name, fingerprint) = text.split_at(middle);
    check_names(name, fingerprint, selector & 0x10 != 0);
});

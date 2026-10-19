//! How things are put into words and text is made safe to draw: names,
//! previews, times, and the rendering limits of `RESOURCE_LIMITS.md`
//! section 10 (`PROTOCOL.md` section 9.1).

use unicode_segmentation::UnicodeSegmentation;

use crate::data::{ContactView, IdentityView};

/// A grapheme cluster with more scalar values than this is drawn as a
/// placeholder (`MAX_RENDERED_GRAPHEME_SCALARS`).
pub const MAX_RENDERED_GRAPHEME_SCALARS: usize = 16;

/// A message with more lines than this is collapsed behind "Show more"
/// (`MAX_RENDERED_MESSAGE_LINES`).
pub const MAX_RENDERED_MESSAGE_LINES: usize = 200;

/// The largest message, in bytes (`MAX_CHAT_TEXT_LEN`).
pub const MAX_MESSAGE_BYTES: usize = 16_384;

/// The longest local name of a contact, an identity or an invitation, in
/// bytes of UTF-8 (`MAX_DISPLAY_NAME_LEN`); the runtime takes no longer.
pub const MAX_NAME_BYTES: usize = 128;

/// The longest local name, in characters (`MAX_DISPLAY_NAME_SCALARS`).
pub const MAX_NAME_CHARS: usize = 64;

/// The most characters a passphrase field takes: as UTF-8 they are at most
/// 1024 bytes, the most the vault takes (`STORAGE.md` section 3.3).
pub const MAX_PASSPHRASE_CHARS: usize = 256;

/// The longest preview of the newest message of a conversation, in
/// characters; the list cuts it to its width.
pub const PREVIEW_CHARS: usize = 120;

/// A preview reads at most this many bytes of a text for each character
/// it can show: room for any script, marks and runs of white space, and
/// never the whole of a long message.
pub const PREVIEW_BYTES_PER_CHAR: usize = 16;

const ELLIPSIS: char = '\u{2026}';
const REPLACEMENT: char = '\u{FFFD}';

/// The first group of a fingerprint shown in groups.
pub fn first_group(fingerprint: &str) -> &str {
    fingerprint.split(' ').next().unwrap_or(fingerprint)
}

/// The name of a contact: the one the user gave it, or "Contact" and the
/// first group of its fingerprint.
pub fn contact_name(contact: &ContactView) -> String {
    match &contact.name {
        Some(name) if !name.is_empty() => name.clone(),
        _ => fallback_contact_name(&contact.fingerprint),
    }
}

/// Whether the user gave `contact` a name; without one it is named by
/// its fingerprint ([`contact_name`]).
pub fn named(contact: &ContactView) -> bool {
    contact.name.as_ref().is_some_and(|name| !name.is_empty())
}

/// The name of a contact that has no name, from its fingerprint.
pub fn fallback_contact_name(fingerprint: &str) -> String {
    format!("Contact {}", first_group(fingerprint))
}

/// The name of a conversation whose contact was blocked or removed.
pub fn ended_contact_name(fingerprint: &str) -> String {
    format!("Former contact {}", first_group(fingerprint))
}

/// The name of a local identity: its local name, or "Identity" and the
/// first group of its fingerprint.
pub fn identity_name(identity: &IdentityView) -> String {
    match &identity.name {
        Some(name) if !name.is_empty() => name.clone(),
        _ => format!("Identity {}", first_group(&identity.fingerprint)),
    }
}

/// Whether `character` is drawn as nothing or changes the direction of
/// text around it: such characters are shown as a visible mark instead.
/// These are the characters a display name rejects as such (`PROTOCOL.md`
/// section 9): the bidirectional controls, the code points ignorable by
/// default other than variation selectors, tag characters included, and
/// the interlinear annotation controls; and the format controls of
/// Egyptian hieroglyphs, which nothing here shapes. Variation selectors
/// stay as they are, since emoji need them and they hide no text.
fn is_invisible(character: char) -> bool {
    matches!(
        character,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'
            | '\u{1160}'
            | '\u{17B4}'
            | '\u{17B5}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2065}'
            | '\u{2066}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF0}'..='\u{FFF8}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{13430}'..='\u{1343F}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E00FF}'
            | '\u{E01F0}'..='\u{E0FFF}'
    )
}

/// Text made ready to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    /// What to draw.
    pub text: String,
    /// Lines were left out: the message is collapsed.
    pub collapsed: bool,
}

/// Prepares `text` for drawing: an over-long grapheme cluster becomes a
/// replacement character, an invisible or direction-changing character a
/// visible mark such as `<U+202E>`, and unless `expanded` only the first
/// [`MAX_RENDERED_MESSAGE_LINES`] lines are kept. A run without a break
/// opportunity is broken where it meets the edge by the text layout of
/// the toolkit, which breaks a word that does not fit.
pub fn prepare(text: &str, expanded: bool) -> Prepared {
    let mut out = String::with_capacity(text.len());
    let mut lines = 1_usize;
    let mut collapsed = false;
    for cluster in text.graphemes(true) {
        // A carriage return before a line feed makes one cluster of the two.
        if cluster == "\n" || cluster == "\r\n" {
            if !expanded && lines >= MAX_RENDERED_MESSAGE_LINES {
                collapsed = true;
                break;
            }
            lines = lines.saturating_add(1);
            out.push_str(cluster);
            continue;
        }
        if cluster.chars().count() > MAX_RENDERED_GRAPHEME_SCALARS {
            out.push(REPLACEMENT);
            continue;
        }
        for character in cluster.chars() {
            if is_invisible(character) {
                out.push_str(&format!("<U+{:04X}>", u32::from(character)));
            } else {
                out.push(character);
            }
        }
    }
    Prepared {
        text: out,
        collapsed,
    }
}

/// One line of preview of `text`, at most `max_chars` characters: lines
/// joined by spaces, runs of white space made one, and an ellipsis where
/// it is cut. Only the start of the text is read
/// ([`PREVIEW_BYTES_PER_CHAR`] for each character), and a text longer
/// than that counts as cut.
pub fn preview(text: &str, max_chars: usize) -> String {
    let mut end = text.len().min(
        max_chars
            .saturating_add(1)
            .saturating_mul(PREVIEW_BYTES_PER_CHAR),
    );
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let unread = end < text.len();
    let prepared = prepare(text.get(..end).unwrap_or_default(), false);
    let mut out = String::new();
    let mut count = 0_usize;
    let mut space = false;
    for character in prepared.text.chars() {
        if character.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            // A space and the next character both have to fit.
            if count.saturating_add(1) >= max_chars {
                out.push(ELLIPSIS);
                return out;
            }
            out.push(' ');
            count = count.saturating_add(1);
            space = false;
        }
        if count >= max_chars {
            out.push(ELLIPSIS);
            return out;
        }
        out.push(character);
        count = count.saturating_add(1);
    }
    // Lines past the limit were left out by `prepare`, or the rest of the
    // text was not read: that is a cut too. White space at the end of the
    // text is not.
    if prepared.collapsed || unread {
        out.push(ELLIPSIS);
    }
    out
}

/// Whether a draft can be sent: not only white space, and not longer than
/// a message may be. The runtime checks the text again.
pub fn sendable(draft: &str) -> bool {
    !draft.trim().is_empty() && draft.len() <= MAX_MESSAGE_BYTES
}

/// A day and a time of day in local time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    /// The year.
    pub year: i32,
    /// 1 to 12.
    pub month: u8,
    /// 1 to 31.
    pub day: u8,
    /// 0 to 23.
    pub hour: u8,
    /// 0 to 59.
    pub minute: u8,
    /// 0 for Monday to 6 for Sunday.
    pub weekday: u8,
}

impl LocalTime {
    fn same_day(&self, other: &Self) -> bool {
        self.year == other.year && self.month == other.month && self.day == other.day
    }
}

/// The local calendar: given by the desktop from the system's time zone,
/// fixed in tests.
pub trait Calendar {
    /// Seconds since the epoch now.
    fn now(&self) -> u64;
    /// The local time of `seconds` since the epoch.
    fn local(&self, seconds: u64) -> LocalTime;
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const WEEKDAYS: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

fn month_name(month: u8) -> &'static str {
    MONTHS
        .get(usize::from(month.saturating_sub(1)))
        .copied()
        .unwrap_or("")
}

fn weekday_name(weekday: u8) -> &'static str {
    WEEKDAYS.get(usize::from(weekday)).copied().unwrap_or("")
}

/// The time of day, as "14:05".
pub fn clock_time(calendar: &dyn Calendar, seconds: u64) -> String {
    let time = calendar.local(seconds);
    format!("{:02}:{:02}", time.hour, time.minute)
}

/// When something happened, as short as the sidebar wants it: the time
/// today, "Yesterday", a weekday within the week, or a date.
pub fn short_time(calendar: &dyn Calendar, seconds: u64) -> String {
    let now = calendar.now();
    let today = calendar.local(now);
    let then = calendar.local(seconds);
    if then.same_day(&today) {
        return clock_time(calendar, seconds);
    }
    let yesterday = calendar.local(now.saturating_sub(86_400));
    if then.same_day(&yesterday) {
        return "Yesterday".to_owned();
    }
    let age = now.saturating_sub(seconds);
    if age < 6 * 86_400 {
        return weekday_name(then.weekday).get(..3).unwrap_or("").to_owned();
    }
    let month = month_name(then.month).get(..3).unwrap_or("");
    if then.year == today.year {
        format!("{} {month}", then.day)
    } else {
        format!("{} {month} {}", then.day, then.year)
    }
}

/// The heading of a day in a conversation: "Today", "Yesterday", or the
/// weekday and date.
pub fn day_heading(calendar: &dyn Calendar, seconds: u64) -> String {
    let now = calendar.now();
    let then = calendar.local(seconds);
    if then.same_day(&calendar.local(now)) {
        return "Today".to_owned();
    }
    if then.same_day(&calendar.local(now.saturating_sub(86_400))) {
        return "Yesterday".to_owned();
    }
    let today = calendar.local(now);
    let date = format!(
        "{} {} {}",
        weekday_name(then.weekday),
        then.day,
        month_name(then.month)
    );
    if then.year == today.year {
        date
    } else {
        format!("{date} {}", then.year)
    }
}

/// Whether two times fall on the same local day.
pub fn same_day(calendar: &dyn Calendar, a: u64, b: u64) -> bool {
    calendar.local(a).same_day(&calendar.local(b))
}

/// A calendar in UTC, with a fixed "now": for tests and as a fallback
/// when the system's time zone cannot be read.
#[derive(Clone, Copy, Debug)]
pub struct UtcCalendar {
    /// Seconds since the epoch taken as now.
    pub now: u64,
}

impl Calendar for UtcCalendar {
    fn now(&self) -> u64 {
        self.now
    }

    fn local(&self, seconds: u64) -> LocalTime {
        utc(seconds)
    }
}

/// The UTC time of `seconds` since the epoch (the civil-from-days method).
// The days of a u64 of seconds are below 2^48, so every intermediate value
// of the method fits an i64 with room to spare; no step can overflow.
#[allow(clippy::arithmetic_side_effects)]
pub fn utc(seconds: u64) -> LocalTime {
    let days = seconds / 86_400;
    let rest = seconds % 86_400;
    let hour = rest / 3_600;
    let minute = (rest % 3_600) / 60;
    // 1970-01-01 was a Thursday: weekday 3 with Monday 0.
    let weekday = (days.saturating_add(3)) % 7;
    let z = i64::try_from(days).unwrap_or(0).saturating_add(719_468);
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    if month <= 2 {
        year += 1;
    }
    let byte = |value: i64| u8::try_from(value).unwrap_or(0);
    let small = |value: u64| u8::try_from(value).unwrap_or(0);
    LocalTime {
        year: i32::try_from(year).unwrap_or(1970),
        month: byte(month),
        day: byte(day),
        hour: small(hour),
        minute: small(minute),
        weekday: small(weekday),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invisible_and_direction_marks_are_shown() {
        let prepared = prepare("abc\u{202E}def\u{200B}", false);
        assert_eq!(prepared.text, "abc<U+202E>def<U+200B>");
        assert!(!prepared.collapsed);
    }

    #[test]
    fn tag_and_other_format_characters_are_shown() {
        // A tag sequence spells hidden words; each tag shows as its mark.
        let prepared = prepare("hi\u{E0001}\u{E0041}\u{E007F}!", false);
        assert_eq!(prepared.text, "hi<U+E0001><U+E0041><U+E007F>!");
        for hidden in [
            '\u{2065}',
            '\u{FFF0}',
            '\u{13430}',
            '\u{1343F}',
            '\u{1BCA0}',
            '\u{1D173}',
            '\u{E0000}',
            '\u{E00FF}',
            '\u{E01F0}',
            '\u{E0FFF}',
        ] {
            assert_eq!(
                prepare(&format!("a{hidden}b"), false).text,
                format!("a<U+{:04X}>b", u32::from(hidden))
            );
        }
        // Variation selectors are left as they are, and so are characters
        // just outside the ranges.
        for kept in [
            '\u{180B}',
            '\u{180F}',
            '\u{FE0F}',
            '\u{E0100}',
            '\u{E01EF}',
            '\u{1342F}',
            '\u{13440}',
            '\u{1D172}',
            '\u{1D17B}',
            '\u{2800}',
            '\u{FFFC}',
        ] {
            let text = format!("a{kept}b");
            assert_eq!(prepare(&text, false).text, text);
        }
    }

    #[test]
    fn an_over_long_cluster_is_replaced() {
        let long: String = core::iter::once('e')
            .chain(core::iter::repeat_n('\u{0301}', 20))
            .collect();
        let prepared = prepare(&format!("a{long}b"), false);
        assert_eq!(prepared.text, "a\u{FFFD}b");
        let short: String = core::iter::once('e')
            .chain(core::iter::repeat_n('\u{0301}', 3))
            .collect();
        assert_eq!(prepare(&short, false).text, short);
    }

    #[test]
    fn more_lines_than_the_limit_collapse_unless_expanded() {
        let text = vec!["line"; MAX_RENDERED_MESSAGE_LINES + 5].join("\n");
        let collapsed = prepare(&text, false);
        assert!(collapsed.collapsed);
        assert_eq!(collapsed.text.lines().count(), MAX_RENDERED_MESSAGE_LINES);
        let expanded = prepare(&text, true);
        assert!(!expanded.collapsed);
        assert_eq!(expanded.text, text);
    }

    #[test]
    fn a_carriage_return_and_line_feed_count_as_one_line_break() {
        let text = vec!["line"; MAX_RENDERED_MESSAGE_LINES + 5].join("\r\n");
        let collapsed = prepare(&text, false);
        assert!(collapsed.collapsed);
        assert_eq!(
            collapsed.text.matches('\n').count(),
            MAX_RENDERED_MESSAGE_LINES - 1
        );
        assert_eq!(prepare(&text, true).text, text);
    }

    #[test]
    fn a_preview_is_one_line_and_cut_with_an_ellipsis() {
        assert_eq!(preview("hello\n\n  world", 40), "hello world");
        assert_eq!(preview("abcdefghij", 4), "abcd\u{2026}");
        assert_eq!(preview("abcd", 4), "abcd");
    }

    #[test]
    fn a_preview_has_an_ellipsis_only_where_it_cuts() {
        // White space after the last word is not text that was cut.
        assert_eq!(preview("abcde \n  ", 5), "abcde");
        assert_eq!(preview("abcde", 5), "abcde");
        assert_eq!(preview("abcdef", 5), "abcde\u{2026}");
        // A cut at a space shows the ellipsis too, and no space before it.
        assert_eq!(preview("l at 06:05", 1), "l\u{2026}");
        assert_eq!(preview("abc def", 3), "abc\u{2026}");
        assert_eq!(preview("abc def", 4), "abc\u{2026}");
        assert_eq!(preview("abc def", 5), "abc d\u{2026}");
        assert_eq!(preview("abc de", 6), "abc de");
        // Lines left out by the line limit are.
        let text = vec!["a"; MAX_RENDERED_MESSAGE_LINES + 5].join("\n");
        assert!(preview(&text, 1_000).ends_with('\u{2026}'));
    }

    #[test]
    fn a_preview_reads_only_the_start_of_a_long_message() {
        // A direction mark far into a message of 64 KB is never read: the
        // preview is the start, cut where the reading stopped.
        let late = format!("Hello{}\u{202E}olleh", " ".repeat(65_536));
        assert_eq!(preview(&late, PREVIEW_CHARS), "Hello\u{2026}");
        // Within what is read, it is shown as its mark.
        let early = format!("Hello \u{202E}olleh{}", "a".repeat(65_536));
        let shown = preview(&early, PREVIEW_CHARS);
        assert!(shown.starts_with("Hello <U+202E>olleh"), "{shown}");
        assert!(shown.ends_with('\u{2026}'));
        assert_eq!(shown.chars().count(), PREVIEW_CHARS + 1);
        // What is read is a bounded start, cut at a character.
        let wide = "\u{20AC}".repeat(32_768);
        let shown = preview(&wide, 3);
        assert_eq!(shown, "\u{20AC}\u{20AC}\u{20AC}\u{2026}");
    }

    #[test]
    fn a_draft_of_white_space_or_over_the_limit_is_not_sendable() {
        assert!(!sendable("  \n "));
        assert!(sendable("hi"));
        assert!(sendable(&"a".repeat(MAX_MESSAGE_BYTES)));
        assert!(!sendable(&"a".repeat(MAX_MESSAGE_BYTES + 1)));
    }

    #[test]
    fn utc_dates_are_right() {
        // 2026-10-14 12:30:00 UTC, a Wednesday.
        let time = utc(1_791_981_000);
        assert_eq!(
            (time.year, time.month, time.day, time.hour, time.minute),
            (2026, 10, 14, 12, 30)
        );
        assert_eq!(time.weekday, 2);
        let epoch = utc(0);
        assert_eq!((epoch.year, epoch.month, epoch.day), (1970, 1, 1));
        assert_eq!(epoch.weekday, 3);
        let leap = utc(951_782_400);
        assert_eq!((leap.year, leap.month, leap.day), (2000, 2, 29));
    }

    #[test]
    fn times_are_short_and_relative() {
        let now = 1_791_981_000;
        let calendar = UtcCalendar { now };
        assert_eq!(short_time(&calendar, now - 3_600), "11:30");
        assert_eq!(short_time(&calendar, now - 86_400), "Yesterday");
        assert_eq!(short_time(&calendar, now - 3 * 86_400), "Sun");
        assert_eq!(short_time(&calendar, now - 40 * 86_400), "4 Sep");
        assert_eq!(short_time(&calendar, now - 400 * 86_400), "9 Sep 2025");
        assert_eq!(day_heading(&calendar, now), "Today");
        assert_eq!(day_heading(&calendar, now - 86_400), "Yesterday");
        assert_eq!(
            day_heading(&calendar, now - 2 * 86_400),
            "Monday 12 October"
        );
    }

    #[test]
    fn names_fall_back_to_the_fingerprint() {
        let contact = ContactView {
            id: crate::ids::UiContactId::new(
                crate::ids::UiIdentityId::from_bytes([1; 16]),
                [2; 16],
            ),
            name: None,
            fingerprint: "ABCD EFGH IJKL".to_owned(),
            full_fingerprint: String::new(),
            accepted: true,
            verified: false,
            invitation: false,
        };
        assert_eq!(contact_name(&contact), "Contact ABCD");
        let named = ContactView {
            name: Some("Bob".to_owned()),
            ..contact
        };
        assert_eq!(contact_name(&named), "Bob");
    }
}

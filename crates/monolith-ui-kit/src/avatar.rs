//! Avatars. A contact is a circle with up to two initials; one of the
//! user's own identities is a rounded square, so that the two are never
//! confused. The background comes from a stable hash of a caller seed (the
//! fingerprint, say), so a contact keeps its colour across restarts.

use egui::{
    Align2, CornerRadius, Painter, Rect, Response, Sense, Stroke, Ui, Vec2, Widget, WidgetInfo,
    WidgetType,
};

use crate::icons;
use crate::theme::{self, Theme};

/// FNV-1a, 64 bits. Stable across platforms and releases, which is why it
/// is written out here instead of using a hasher of the standard library.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The index into the palette of avatar colours that `seed` selects.
pub fn color_index(seed: &[u8]) -> usize {
    // Eight colours: the low three bits of the hash.
    usize::try_from(fnv1a(seed) & 7).unwrap_or(0)
}

/// Up to two initials of `name`: the first letter or digit of each of the
/// first two words, uppercased. Empty when the name has no such character.
pub fn initials(name: &str) -> String {
    let mut out = String::new();
    for word in name.split_whitespace().take(2) {
        if let Some(first) = word.chars().find(|c| c.is_alphanumeric()) {
            out.extend(first.to_uppercase());
        }
    }
    if out.is_empty() {
        if let Some(first) = name.chars().find(|c| !c.is_whitespace()) {
            out.extend(first.to_uppercase());
        }
    }
    out
}

/// The sizes of an avatar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvatarSize {
    /// Compact lists and menus.
    Small,
    /// Rows.
    Medium,
    /// Headers.
    Large,
    /// Details views.
    Extra,
}

impl AvatarSize {
    /// The edge in points.
    pub fn points(self, theme: &Theme) -> f32 {
        match self {
            Self::Small => theme.size.avatar_sm,
            Self::Medium => theme.size.avatar,
            Self::Large => theme.size.avatar_lg,
            Self::Extra => theme.size.avatar_xl,
        }
    }
}

/// The outline of an avatar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outline {
    /// A contact.
    Circle,
    /// One of the user's own identities.
    RoundedSquare,
}

/// What an avatar shows.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Face<'a> {
    pub(crate) name: &'a str,
    pub(crate) seed: &'a [u8],
    pub(crate) outline: Outline,
    pub(crate) verified: bool,
    /// Drawn faded, for a disabled row.
    pub(crate) dim: bool,
}

/// Paints an avatar filling `rect` (square).
pub(crate) fn paint(painter: &Painter, theme: &Theme, rect: Rect, face: Face<'_>) {
    let Face {
        name,
        seed,
        outline,
        verified,
        dim,
    } = face;
    let c = &theme.color;
    let size = rect.width().min(rect.height());
    let background = c.avatar.get(color_index(seed)).copied().unwrap_or(c.accent);
    let (fill, ink) = if dim {
        (
            background.gamma_multiply(0.45),
            c.avatar_text.gamma_multiply(0.7),
        )
    } else {
        (background, c.avatar_text)
    };
    match outline {
        Outline::Circle => {
            painter.circle_filled(rect.center(), size / 2.0, fill);
        }
        Outline::RoundedSquare => {
            // Between 0 and 255 by the clamp.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let radius = (size * 0.3).round().clamp(0.0, 255.0) as u8;
            painter.rect_filled(rect, CornerRadius::same(radius), fill);
        }
    }
    let letters = initials(name);
    if letters.is_empty() {
        icons::USER.paint(painter, rect.center(), size * 0.5, ink);
    } else {
        let font = egui::FontId::new(size * 0.37, theme.text.heading.family.clone());
        // The glyphs of a line sit above the middle of its box; a small
        // shift gives optical centring.
        painter.text(
            rect.center() + Vec2::new(0.0, size * 0.01),
            Align2::CENTER_CENTER,
            letters,
            font,
            ink,
        );
    }
    if verified {
        let mark = (size * 0.38).max(13.0);
        let offset = match outline {
            Outline::Circle => size / 2.0 * core::f32::consts::FRAC_1_SQRT_2,
            Outline::RoundedSquare => size / 2.0 - mark / 2.0 + 2.0,
        };
        let center = rect.center() + Vec2::splat(offset);
        painter.circle(
            center,
            mark / 2.0 + 0.5,
            c.bg_surface,
            Stroke::new(theme.stroke.hairline, c.bg_surface),
        );
        icons::BADGE_CHECK.paint(painter, center, mark, c.accent);
    }
}

/// The avatar of a contact: a circle.
#[derive(Clone, Debug)]
#[must_use = "add it with `ui.add(...)`"]
pub struct Avatar<'a> {
    name: &'a str,
    seed: &'a [u8],
    size: AvatarSize,
    verified: bool,
}

impl<'a> Avatar<'a> {
    /// An avatar for the contact called `name`; the colour follows `seed`.
    pub fn new(name: &'a str, seed: &'a [u8]) -> Self {
        Self {
            name,
            seed,
            size: AvatarSize::Medium,
            verified: false,
        }
    }

    /// The size.
    pub fn size(mut self, size: AvatarSize) -> Self {
        self.size = size;
        self
    }

    /// Shows the small mark of a contact whose fingerprint was verified.
    pub fn verified(mut self, verified: bool) -> Self {
        self.verified = verified;
        self
    }
}

fn show(
    ui: &mut Ui,
    name: &str,
    seed: &[u8],
    size: AvatarSize,
    outline: Outline,
    verified: bool,
) -> Response {
    let theme = theme::current(ui.ctx());
    let edge = size.points(&theme);
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(edge), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, name));
    if ui.is_rect_visible(rect) {
        let face = Face {
            name,
            seed,
            outline,
            verified,
            dim: false,
        };
        paint(ui.painter(), &theme, rect, face);
    }
    response
}

impl Widget for Avatar<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        show(
            ui,
            self.name,
            self.seed,
            self.size,
            Outline::Circle,
            self.verified,
        )
    }
}

/// The avatar of one of the user's own identities: a rounded square.
#[derive(Clone, Debug)]
#[must_use = "add it with `ui.add(...)`"]
pub struct IdentityAvatar<'a> {
    name: &'a str,
    seed: &'a [u8],
    size: AvatarSize,
}

impl<'a> IdentityAvatar<'a> {
    /// An avatar for the identity called `name`; the colour follows `seed`.
    pub fn new(name: &'a str, seed: &'a [u8]) -> Self {
        Self {
            name,
            seed,
            size: AvatarSize::Medium,
        }
    }

    /// The size.
    pub fn size(mut self, size: AvatarSize) -> Self {
        self.size = size;
        self
    }
}

impl Widget for IdentityAvatar<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        show(
            ui,
            self.name,
            self.seed,
            self.size,
            Outline::RoundedSquare,
            false,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_of_two_words() {
        assert_eq!(initials("alice smith"), "AS");
        assert_eq!(initials("  Bob   the builder "), "BT");
    }

    #[test]
    fn initials_of_one_word_and_empty() {
        assert_eq!(initials("zoe"), "Z");
        assert_eq!(initials(""), "");
        assert_eq!(initials("   "), "");
    }

    #[test]
    fn initials_work_for_any_script() {
        assert_eq!(initials("\u{e9}lise dupont"), "\u{c9}D");
        assert_eq!(initials("\u{5f20}\u{4f1f}"), "\u{5f20}");
        assert_eq!(initials("\u{10d}ech"), "\u{10c}");
        // Uppercasing can produce two characters.
        assert_eq!(initials("\u{df}"), "SS");
    }

    #[test]
    fn initials_skip_punctuation() {
        assert_eq!(initials("(work) bob"), "WB");
        assert_eq!(initials("... "), ".");
    }

    #[test]
    fn color_index_is_stable_and_in_range() {
        assert_eq!(color_index(b"abc"), color_index(b"abc"));
        for seed in [&b""[..], b"a", b"abc", b"0123456789abcdef"] {
            assert!(color_index(seed) < 8);
        }
    }

    #[test]
    fn fnv1a_matches_the_reference_vectors() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
    }
}

//! The fonts of the design system, compiled into the binary
//! (`assets/README.md`), and the fallbacks a caller may add.

use std::collections::BTreeMap;
use std::sync::Arc;

use egui::{Context, FontData, FontDefinitions, FontFamily};

/// Inter Regular.
pub const REGULAR: &str = "inter";
/// Inter Medium.
pub const MEDIUM: &str = "inter-medium";
/// Inter SemiBold.
pub const SEMIBOLD: &str = "inter-semibold";
/// JetBrains Mono Regular.
pub const MONO: &str = "mono";
/// The icon font.
pub const ICONS: &str = "icons";

const EMOJI: &str = "noto-emoji";

const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
const JETBRAINS_MONO: &[u8] = include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf");
const LUCIDE: &[u8] = include_bytes!("../assets/fonts/LucideMonolith.ttf");
const NOTO_EMOJI: &[u8] = include_bytes!("../assets/fonts/NotoEmoji-Regular.ttf");

/// A font the caller adds after the bundled ones, for scripts they lack.
/// Its file is borrowed for the life of the process, as the bundled ones
/// are: egui keeps a copy of a font it is given to own, and a font of
/// Chinese characters is 20 MiB.
#[derive(Clone)]
pub struct Fallback {
    /// A name of the caller's choice, unique among fallbacks.
    pub name: String,
    /// The font file.
    pub data: &'static [u8],
    /// The face within a collection, 0 for a single font.
    pub index: u32,
}

impl core::fmt::Debug for Fallback {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Fallback")
            .field("name", &self.name)
            .field("bytes", &self.data.len())
            .finish()
    }
}

/// The font definitions: every text family is its Inter face (or JetBrains
/// Mono), then Noto Emoji, then `fallbacks` in order. The icon family holds
/// the icon font alone, so that an icon never falls back to a letter.
pub fn definitions(fallbacks: &[Fallback]) -> FontDefinitions {
    let mut data: BTreeMap<String, Arc<FontData>> = BTreeMap::new();
    let mut add = |name: &str, font: FontData| {
        data.insert(name.to_owned(), Arc::new(font));
    };
    add(REGULAR, FontData::from_static(INTER_REGULAR));
    add(MEDIUM, FontData::from_static(INTER_MEDIUM));
    add(SEMIBOLD, FontData::from_static(INTER_SEMIBOLD));
    add(MONO, FontData::from_static(JETBRAINS_MONO));
    add(ICONS, FontData::from_static(LUCIDE));
    add(EMOJI, FontData::from_static(NOTO_EMOJI));
    for fallback in fallbacks {
        let mut font = FontData::from_static(fallback.data);
        font.index = fallback.index;
        add(&fallback.name, font);
    }

    let chain = |first: &str| -> Vec<String> {
        let mut names = vec![first.to_owned(), EMOJI.to_owned()];
        names.extend(fallbacks.iter().map(|fallback| fallback.name.clone()));
        names
    };
    let mut families = BTreeMap::new();
    families.insert(FontFamily::Proportional, chain(REGULAR));
    families.insert(FontFamily::Monospace, chain(MONO));
    for name in [REGULAR, MEDIUM, SEMIBOLD, MONO] {
        families.insert(FontFamily::Name(name.into()), chain(name));
    }
    families.insert(FontFamily::Name(ICONS.into()), vec![ICONS.to_owned()]);
    FontDefinitions {
        font_data: data,
        families,
    }
}

/// Installs the fonts in `ctx`. Called once, at start.
pub fn install(ctx: &Context, fallbacks: &[Fallback]) {
    ctx.set_fonts(definitions(fallbacks));
}

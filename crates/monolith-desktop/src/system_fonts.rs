//! Fonts the system already has, for the scripts the bundled fonts lack
//! (`docs/adr/0012-design-system.md`): from a fixed list of paths, when
//! they exist and are not larger than [`MAX_FONT`].

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;

use egui::{Context, FontData, FontDefinitions, FontFamily, RawInput};
use monolith_ui_kit::fonts::Fallback;

/// The largest font file loaded.
pub const MAX_FONT: u64 = 32 * 1024 * 1024;

/// The paths tried, each group in order until one exists: the first of a
/// group that loads is used.
fn candidates() -> Vec<(&'static str, Vec<PathBuf>)> {
    #[cfg(windows)]
    {
        let fonts = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"))
            .join("Fonts");
        vec![
            // Greek, Cyrillic, Arabic, Hebrew and more.
            ("segoe-ui", vec![fonts.join("segoeui.ttf")]),
            // Chinese and Japanese kana.
            ("yahei", vec![fonts.join("msyh.ttc")]),
            // Korean.
            ("malgun", vec![fonts.join("malgun.ttf")]),
        ]
    }
    #[cfg(not(windows))]
    {
        let path = PathBuf::from;
        vec![
            (
                "dejavu-sans",
                vec![
                    path("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
                    path("/usr/share/fonts/dejavu/DejaVuSans.ttf"),
                    path("/usr/share/fonts/TTF/DejaVuSans.ttf"),
                ],
            ),
            (
                "noto-sans-cjk",
                vec![
                    path("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
                    path("/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc"),
                    path("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc"),
                ],
            ),
        ]
    }
}

/// Whether egui can read the face `index` of `data`. egui stops the
/// process on a font it cannot read, when it first lays out text; a system
/// font that is damaged is left out instead. The check borrows the file as
/// egui will, so that it makes no copy of it.
fn readable(data: &'static [u8], index: u32) -> bool {
    let mut font = FontData::from_static(data);
    font.index = index;
    let mut definitions = FontDefinitions::empty();
    definitions
        .font_data
        .insert("candidate".to_owned(), Arc::new(font));
    definitions
        .families
        .insert(FontFamily::Proportional, vec!["candidate".to_owned()]);
    let ctx = Context::default();
    ctx.set_fonts(definitions);
    // The fonts are made at the start of a frame.
    std::panic::catch_unwind(AssertUnwindSafe(|| {
        let _ = ctx.run(RawInput::default(), |_| {});
    }))
    .is_ok()
}

/// The fallback fonts found. Each file is kept for the life of the process,
/// once: egui borrows it from there.
pub fn load() -> Vec<Fallback> {
    candidates()
        .into_iter()
        .filter_map(|(name, paths)| {
            paths.iter().find_map(|path| {
                let metadata = std::fs::metadata(path).ok()?;
                if !metadata.is_file() || metadata.len() > MAX_FONT {
                    return None;
                }
                let data = std::fs::read(path).ok()?;
                if u64::try_from(data.len()).ok()? > MAX_FONT {
                    return None;
                }
                // Kept from here on; a damaged file stays unused.
                let data: &'static [u8] = Box::leak(data.into_boxed_slice());
                readable(data, 0).then(|| Fallback {
                    name: name.to_owned(),
                    data,
                    index: 0,
                })
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_is_not_a_font_is_left_out() {
        // The default hook would print the panic egui raises; it is the
        // expected outcome here.
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let refused = !readable(b"not a font at all", 0);
        std::panic::set_hook(hook);
        assert!(refused);
    }
}

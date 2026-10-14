//! Fonts the system already has, for the scripts the bundled fonts lack
//! (`docs/adr/0012-design-system.md`): from a fixed list of paths, when
//! they exist and are not larger than [`MAX_FONT`].

use std::path::PathBuf;
use std::sync::Arc;

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

/// The fallback fonts found.
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
                (u64::try_from(data.len()).ok()? <= MAX_FONT).then(|| Fallback {
                    name: name.to_owned(),
                    data: Arc::new(data),
                    index: 0,
                })
            })
        })
        .collect()
}

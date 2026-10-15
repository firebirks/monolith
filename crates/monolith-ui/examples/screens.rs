//! Renders the window in every fixture state, dialogs included, light and
//! dark, to PNG files, at the sizes the screens are reviewed at: the first
//! window (1180 by 760), a narrow one (820 by 600) and the smallest (760 by
//! 520), each in a folder of its own:
//! `cargo run -p monolith-ui --features fixtures --example screens -- <dir> [name] [width height] [scale]`.
//! A width and a height render that size alone.

#![allow(clippy::print_stderr, clippy::expect_used)]

use std::path::PathBuf;

use egui::{Context, Vec2};
use monolith_ui::fixtures::{NAMES, fixture};
use monolith_ui_kit::{Theme, fonts, theme};
use monolith_ui_snapshot::{Options, render};

/// The sizes of a review.
const SIZES: [(f32, f32); 3] = [(1180.0, 760.0), (820.0, 600.0), (760.0, 520.0)];

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().unwrap_or_else(|| "screens".to_owned()));
    let only = args.next().filter(|name| name != "all");
    let width: Option<f32> = args.next().and_then(|v| v.parse().ok());
    let height: Option<f32> = args.next().and_then(|v| v.parse().ok());
    let scale: f32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let sizes = match (width, height) {
        (Some(width), Some(height)) => vec![(width, height)],
        _ => SIZES.to_vec(),
    };
    for (width, height) in sizes {
        let folder = dir.join(format!("{width}x{height}"));
        std::fs::create_dir_all(&folder).expect("make the folder");
        for name in NAMES {
            if only.as_deref().is_some_and(|only| only != *name) {
                continue;
            }
            for (suffix, chosen) in [("light", Theme::light()), ("dark", Theme::dark())] {
                let Some(mut app) = fixture(name) else {
                    continue;
                };
                let ctx = Context::default();
                fonts::install(&ctx, &[]);
                let background = chosen.color.bg_app;
                theme::install(&ctx, chosen.with_reduced_motion(true));
                let options = Options {
                    size: Vec2::new(width, height),
                    pixels_per_point: scale,
                    background,
                    ..Options::default()
                };
                let image = render(&ctx, &options, |ctx| monolith_ui::show(ctx, &mut app));
                let path = folder.join(format!("{name}-{suffix}.png"));
                image.save_png(&path).expect("write the PNG");
                eprintln!("{}", path.display());
            }
        }
    }
}

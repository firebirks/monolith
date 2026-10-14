//! Renders the window in every fixture state, light and dark, at a size
//! and scale, to PNG files:
//! `cargo run -p monolith-ui --features fixtures --example screens -- <dir> [name] [width] [height] [scale]`.

#![allow(clippy::print_stderr, clippy::expect_used)]

use std::path::PathBuf;

use egui::{Context, Vec2};
use monolith_ui::fixtures::{NAMES, fixture};
use monolith_ui_kit::{Theme, fonts, theme};
use monolith_ui_snapshot::{Options, render};

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = PathBuf::from(args.next().unwrap_or_else(|| "screens".to_owned()));
    let only = args.next().filter(|name| name != "all");
    let width: f32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1180.0);
    let height: f32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(760.0);
    let scale: f32 = args.next().and_then(|v| v.parse().ok()).unwrap_or(1.0);
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
            let path = dir.join(format!("{name}-{suffix}.png"));
            image.save_png(&path).expect("write the PNG");
            eprintln!("{}", path.display());
        }
    }
}

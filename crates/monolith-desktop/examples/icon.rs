//! Writes the icon of the client as a PNG file, for the packages that
//! install one: `cargo run -p monolith-desktop --example icon -- <file> [side]`.

#![allow(clippy::expect_used)]

use std::path::PathBuf;

use monolith_ui_snapshot::Image;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().expect("the file to write"));
    let side = args
        .next()
        .map_or(256, |side| side.parse().expect("a side in pixels"));
    let icon = monolith_desktop::window::icon_of_side(side);
    let image = Image {
        width: usize::try_from(icon.width).expect("width"),
        height: usize::try_from(icon.height).expect("height"),
        rgba: icon.rgba,
    };
    image.save_png(&path).expect("write the icon");
}

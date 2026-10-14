//! Renders sheets of the components in every state, light and dark, to PNG
//! files: `cargo run -p monolith-ui-kit --example gallery -- <directory>`.

#![allow(clippy::print_stderr, clippy::expect_used)]

use std::path::{Path, PathBuf};

use egui::{Context, Vec2};
use monolith_ui_kit::button::{Kind, Size};
use monolith_ui_kit::{Button, TextField, Theme, fonts, icons, theme};
use monolith_ui_snapshot::{Options, render};

fn sheet(name: &str, size: Vec2, dir: &Path, mut body: impl FnMut(&mut egui::Ui)) {
    for (suffix, theme) in [("light", Theme::light()), ("dark", Theme::dark())] {
        let ctx = Context::default();
        fonts::install(&ctx, &[]);
        let background = theme.color.bg_app;
        theme::install(&ctx, theme.with_reduced_motion(true));
        let options = Options {
            size,
            background,
            ..Options::default()
        };
        let image = render(&ctx, &options, |ctx| {
            egui::CentralPanel::default()
                .frame(egui::Frame::new().inner_margin(24))
                .show(ctx, |ui| body(ui));
        });
        let path = dir.join(format!("{name}-{suffix}.png"));
        image.save_png(&path).expect("write the PNG");
        eprintln!("{}", path.display());
    }
}

fn main() {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .unwrap_or_else(|| "gallery".to_owned()),
    );
    sheet("buttons", Vec2::new(720.0, 300.0), &dir, |ui| {
        for size in [Size::Small, Size::Medium, Size::Large] {
            ui.horizontal(|ui| {
                for kind in [Kind::Primary, Kind::Secondary, Kind::Ghost, Kind::Danger] {
                    ui.add(Button::new(kind, "Send message").size(size));
                }
            });
        }
        ui.horizontal(|ui| {
            ui.add(Button::primary("Add contact").icon(icons::USER_PLUS));
            ui.add(Button::secondary("Copy").icon(icons::COPY));
            ui.add(Button::primary("Disabled").enabled(false));
            ui.add(Button::secondary("Disabled").enabled(false));
            ui.add(Button::primary("Creating").busy(true));
        });
    });
    sheet("fields", Vec2::new(720.0, 420.0), &dir, |ui| {
        let mut empty = String::new();
        let mut filled = "Bob at work".to_owned();
        let mut secret = "correct horse".to_owned();
        TextField::new("a", &mut empty)
            .label("Name")
            .hint("For example, Alice")
            .help(Some("Only you see this name."))
            .show(ui);
        TextField::new("b", &mut filled)
            .label("Name")
            .icon(icons::USER)
            .show(ui);
        TextField::new("c", &mut secret)
            .label("Passphrase")
            .icon(icons::KEY_ROUND)
            .secret(true)
            .error(Some("The passphrase is not right."))
            .show(ui);
        TextField::new("d", &mut empty)
            .label("Disabled")
            .hint("Nothing to type")
            .enabled(false)
            .show(ui);
    });
}

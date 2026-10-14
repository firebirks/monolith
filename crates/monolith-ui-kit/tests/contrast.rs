//! The contrast of text over what it is drawn on, in both themes, by the
//! formula of WCAG 2.1: main text at least 7 to 1 (level AAA); supporting
//! text, hints, times, the accent's text and marks, and text on the accent
//! at least 4.5 to 1 (level AA). Text of disabled controls is held to no
//! ratio, as WCAG holds it to none. A translucent colour is laid over the
//! surface it is drawn on first.

#![allow(clippy::unwrap_used)]

use egui::Color32;
use monolith_ui_kit::Theme;

/// The colour `over` makes on `under`, opaque.
fn composite(over: Color32, under: Color32) -> Color32 {
    // Color32 holds premultiplied alpha.
    let rest = 1.0 - f64::from(over.a()) / 255.0;
    let mix = |top: u8, bottom: u8| {
        let value = f64::from(top) + f64::from(bottom) * rest;
        // Within 0 to 255 by construction; rounded.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let byte = value.round().clamp(0.0, 255.0) as u8;
        byte
    };
    Color32::from_rgb(
        mix(over.r(), under.r()),
        mix(over.g(), under.g()),
        mix(over.b(), under.b()),
    )
}

/// The relative luminance of an opaque sRGB colour.
fn luminance(color: Color32) -> f64 {
    let channel = |value: u8| {
        let c = f64::from(value) / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
}

fn contrast(text: Color32, background: Color32) -> f64 {
    let (a, b) = (
        luminance(composite(text, background)),
        luminance(background),
    );
    let (light, dark) = if a > b { (a, b) } else { (b, a) };
    (light + 0.05) / (dark + 0.05)
}

fn failures(name: &str, theme: &Theme) -> Vec<String> {
    let c = &theme.color;
    let mut failures = Vec::new();
    let mut need = |what: String, ratio: f64, least: f64| {
        if ratio < least {
            failures.push(format!("{name}: {what} is {ratio:.2} to 1, under {least}"));
        }
    };
    for (surface, background) in [
        ("the window", c.bg_app),
        ("the sidebar", c.bg_sidebar),
        ("a surface", c.bg_surface),
        ("a modal", c.bg_raised),
    ] {
        need(
            format!("text on {surface}"),
            contrast(c.text, background),
            7.0,
        );
        for (what, color) in [
            ("supporting text", c.text_secondary),
            ("a hint or a time", c.text_tertiary),
            ("accent text", c.accent_text),
        ] {
            need(
                format!("{what} on {surface}"),
                contrast(color, background),
                4.5,
            );
        }
    }
    need(
        "danger text on a surface".to_owned(),
        contrast(c.danger, c.bg_surface),
        4.5,
    );
    for (what, background) in [
        ("the accent", c.accent),
        ("the accent under the pointer", c.accent_hover),
        ("the accent pressed", c.accent_pressed),
    ] {
        need(
            format!("text on {what}"),
            contrast(c.text_on_accent, background),
            4.5,
        );
    }
    for (what, text, background) in [
        ("an outgoing message", c.bubble_out_text, c.bubble_out),
        (
            "the time of an outgoing message",
            c.bubble_out_meta,
            c.bubble_out,
        ),
        ("an incoming message", c.bubble_in_text, c.bubble_in),
        (
            "the time of an incoming message",
            c.bubble_in_meta,
            c.bubble_in,
        ),
    ] {
        need(what.to_owned(), contrast(text, background), 4.5);
    }
    failures
}

#[test]
fn text_is_readable_on_everything_it_is_drawn_on_in_both_themes() {
    let mut found = failures("light", &Theme::light());
    found.extend(failures("dark", &Theme::dark()));
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn a_translucent_colour_is_laid_over_its_surface() {
    let half_white = Color32::from_rgba_unmultiplied(255, 255, 255, 128);
    let grey = composite(half_white, Color32::BLACK);
    assert!((127..=129).contains(&grey.r()));
    assert!((contrast(Color32::WHITE, Color32::BLACK) - 21.0).abs() < 0.01);
}

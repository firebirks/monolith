//! The contrast of text over what it is drawn on, in both themes, by the
//! formula of WCAG 2.1: main text at least 7 to 1 (level AAA); supporting
//! text, hints, times, the accent's text and marks, text on the accent and
//! on a destructive fill, and the text of pills, badges and controls in
//! every state at least 4.5 to 1 (level AA). Text of disabled controls is
//! held to no ratio, as WCAG holds it to none. A translucent colour is laid
//! over the surface it is drawn on first.
//!
//! Hairlines and the outlines of controls are not text, but they have to
//! be seen on every surface they are drawn on; they are held to smaller
//! ratios of their own.

#![allow(clippy::unwrap_used)]

use egui::Color32;
use monolith_ui_kit::Theme;
use monolith_ui_kit::interact::{mix, over};

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

/// The surfaces a control is laid on.
fn parents(theme: &Theme) -> [(&'static str, Color32); 3] {
    let c = &theme.color;
    [
        ("a surface", c.bg_surface),
        ("a card", c.bg_card),
        ("a modal", c.bg_raised),
    ]
}

/// Every pair of text and background the components draw, with the least
/// ratio it needs.
fn pairs(theme: &Theme) -> Vec<(String, Color32, Color32, f64)> {
    let c = &theme.color;
    let mut pairs = Vec::new();
    let mut add = |what: String, text: Color32, background: Color32, least: f64| {
        pairs.push((what, text, background, least));
    };

    for (surface, background) in [
        ("the window", c.bg_app),
        ("the sidebar", c.bg_sidebar),
        ("a surface", c.bg_surface),
        ("a card", c.bg_card),
        ("a modal", c.bg_raised),
    ] {
        add(format!("text on {surface}"), c.text, background, 7.0);
        for (what, color) in [
            ("supporting text", c.text_secondary),
            ("a hint or a time", c.text_tertiary),
            ("accent text", c.accent_text),
            ("danger text", c.danger),
        ] {
            add(format!("{what} on {surface}"), color, background, 4.5);
        }
    }
    for (what, background) in [
        ("the accent", c.accent),
        ("the accent under the pointer", c.accent_hover),
        ("the accent pressed", c.accent_pressed),
        ("a destructive fill", c.danger_fill),
        ("a destructive fill under the pointer", c.danger_fill_hover),
        ("a destructive fill pressed", c.danger_fill_pressed),
    ] {
        add(format!("text on {what}"), c.text_on_accent, background, 4.5);
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
        add(what.to_owned(), text, background, 4.5);
    }

    // Pills, one per tone, and badges.
    for (tone, text, background) in [
        ("neutral", c.text_secondary, c.bg_sunken),
        ("accent", c.accent_text, c.accent_subtle),
        ("success", c.success, c.success_subtle),
        ("warning", c.warning, c.warning_subtle),
        ("danger", c.danger, c.danger_subtle),
    ] {
        add(format!("a {tone} pill"), text, background, 4.5);
    }
    add(
        "an unread badge".to_owned(),
        c.text_on_accent,
        c.accent,
        4.5,
    );
    add(
        "a muted unread badge".to_owned(),
        c.text_secondary,
        c.border_subtle,
        4.5,
    );

    // Banners: the title in main text, the body in supporting text.
    for (tone, background) in [
        ("an informative", c.accent_subtle),
        ("a success", c.success_subtle),
        ("a warning", c.warning_subtle),
        ("a danger", c.danger_subtle),
    ] {
        add(format!("text of {tone} banner"), c.text, background, 7.0);
        add(
            format!("supporting text of {tone} banner"),
            c.text_secondary,
            background,
            4.5,
        );
    }

    // The Tor status in each state.
    add(
        "the Tor status connected".to_owned(),
        c.text_secondary,
        c.bg_sunken,
        4.5,
    );
    add(
        "the Tor status connecting".to_owned(),
        c.text,
        c.warning_subtle,
        4.5,
    );
    add(
        "the Tor status unavailable".to_owned(),
        c.danger,
        c.danger_subtle,
        4.5,
    );

    // A selected row: its preview and its time.
    add(
        "a preview on a selected row".to_owned(),
        c.text_secondary,
        c.selected,
        4.5,
    );
    add(
        "a time on a selected row".to_owned(),
        c.text_tertiary,
        c.selected,
        4.5,
    );

    // Buttons and controls in every state, on each surface they are put on.
    let dark = theme.mode == monolith_ui_kit::theme::Mode::Dark;
    for (surface, parent) in parents(theme) {
        let hovered = over(parent, c.hover);
        let pressed = over(parent, c.pressed);
        add(
            format!("a secondary button on {surface}"),
            c.text,
            parent,
            4.5,
        );
        add(
            format!("a secondary button under the pointer on {surface}"),
            c.text,
            hovered,
            4.5,
        );
        add(
            format!("a secondary button pressed on {surface}"),
            c.text,
            pressed,
            4.5,
        );
        add(
            format!("a ghost button on {surface}"),
            c.text_secondary,
            parent,
            4.5,
        );
        add(
            format!("a ghost button under the pointer on {surface}"),
            c.text,
            hovered,
            4.5,
        );
        add(
            format!("a ghost button pressed on {surface}"),
            c.text,
            pressed,
            4.5,
        );
        add(
            format!("a quiet destructive button on {surface}"),
            c.danger,
            parent,
            4.5,
        );
        add(
            format!("a quiet destructive button under the pointer on {surface}"),
            c.danger_hover,
            c.danger_subtle,
            4.5,
        );
        add(
            format!("a quiet destructive button pressed on {surface}"),
            c.danger_pressed,
            mix(c.danger_subtle, c.danger, 0.12),
            4.5,
        );
        add(
            format!("a menu item under the pointer on {surface}"),
            c.text,
            hovered,
            4.5,
        );
        // The segmented control: a wash over its parent, a lighter thumb.
        let track = pressed;
        let thumb = if dark {
            over(track, c.pressed.gamma_multiply(1.4))
        } else {
            c.bg_surface
        };
        add(
            format!("an option of a segmented control on {surface}"),
            c.text_secondary,
            track,
            4.5,
        );
        add(
            format!("the chosen option of a segmented control on {surface}"),
            c.text,
            thumb,
            4.5,
        );
    }
    pairs
}

fn failures(name: &str, theme: &Theme) -> Vec<String> {
    pairs(theme)
        .into_iter()
        .filter_map(|(what, text, background, least)| {
            let ratio = contrast(text, background);
            (ratio < least).then(|| format!("{name}: {what} is {ratio:.2} to 1, under {least}"))
        })
        .collect()
}

#[test]
fn text_is_readable_on_everything_it_is_drawn_on_in_both_themes() {
    let mut found = failures("light", &Theme::light());
    found.extend(failures("dark", &Theme::dark()));
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn hairlines_and_outlines_show_on_every_surface_in_both_themes() {
    let mut found = Vec::new();
    for (name, theme) in [("light", Theme::light()), ("dark", Theme::dark())] {
        let c = &theme.color;
        for (surface, background) in parents(&theme) {
            for (what, color, least) in [
                ("a hairline", c.border_subtle, 1.2),
                ("the outline of a control", c.border, 1.4),
            ] {
                let ratio = contrast(color, background);
                if ratio < least {
                    found.push(format!(
                        "{name}: {what} on {surface} is {ratio:.2} to 1, under {least}"
                    ));
                }
            }
        }
    }
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn a_translucent_colour_is_laid_over_its_surface() {
    let half_white = Color32::from_rgba_unmultiplied(255, 255, 255, 128);
    let grey = composite(half_white, Color32::BLACK);
    assert!((127..=129).contains(&grey.r()));
    assert!((contrast(Color32::WHITE, Color32::BLACK) - 21.0).abs() < 0.01);
}

//! The screens use the design system only (`docs/adr/0012-design-system.md`):
//! no stock egui widget, and no colour, size or font of their own. Checked
//! on the source, without its tests and fixtures, and without its comments
//! and string literals.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use std::path::Path;

/// Calls that draw a stock egui widget, or open a stock window.
const STOCK: &[&str] = &[
    "ui.button(",
    "ui.small_button(",
    "ui.checkbox(",
    "ui.radio(",
    "ui.radio_value(",
    "ui.selectable_label(",
    "ui.selectable_value(",
    "ui.toggle_value(",
    "ui.text_edit_singleline(",
    "ui.text_edit_multiline(",
    "ui.label(",
    "ui.heading(",
    "ui.monospace(",
    "ui.code(",
    "ui.strong(",
    "ui.weak(",
    "ui.small(",
    "ui.hyperlink(",
    "ui.hyperlink_to(",
    "ui.link(",
    "ui.collapsing(",
    "ui.menu_button(",
    "ui.separator(",
    "ui.spinner(",
    "ui.image(",
    "ui.color_edit_button",
    "on_hover_text(",
    "on_hover_ui(",
];

/// The widget types of egui, forbidden when they are egui's: the design
/// system has components of some of the same names.
const STOCK_TYPES: &[&str] = &[
    "Button",
    "Checkbox",
    "CollapsingHeader",
    "ComboBox",
    "DragValue",
    "Hyperlink",
    "Image",
    "Label",
    "ProgressBar",
    "RadioButton",
    "RichText",
    "ScrollArea",
    "SelectableLabel",
    "Separator",
    "Slider",
    "Spinner",
    "TextEdit",
    "Window",
    "menu",
];

/// The names a file takes from egui with `use egui::...`.
fn egui_imports(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    for statement in source.split(';') {
        let statement = statement.trim();
        let Some(rest) = statement.strip_prefix("use egui::") else {
            continue;
        };
        let list = rest.trim_start_matches('{').trim_end_matches('}');
        for name in list.split(',') {
            // `X as Y` makes `Y` the name in the file, but `X` the widget.
            let name = name.split_whitespace().next().unwrap_or("");
            names.push(name.to_owned());
        }
    }
    names
}

/// Colours, fonts and sizes that only the theme may give.
const OWN_STYLE: &[&str] = &[
    "Color32::",
    "FontId::",
    "FontFamily::",
    "TextStyle::",
    "Visuals",
    "set_style(",
    "style_mut(",
];

/// The source without string and character literals and comments, so that
/// words in them are not taken for code.
fn code_only(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'/') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '"' => {
                out.push_str("\"\"");
                let mut escaped = false;
                for c in chars.by_ref() {
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == '"' {
                        break;
                    }
                }
            }
            '\'' => {
                // A character literal ('x', '\n', '"'), or a lifetime.
                let rest: String = chars.clone().take(4).collect();
                let literal = rest.starts_with('\\') || rest.chars().nth(1) == Some('\'');
                if literal {
                    let mut escaped = false;
                    for c in chars.by_ref() {
                        if escaped {
                            escaped = false;
                        } else if c == '\\' {
                            escaped = true;
                        } else if c == '\'' {
                            break;
                        }
                    }
                    out.push_str("' '");
                } else {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// The screens' source, file by file, without tests and fixtures.
fn screens() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files: Vec<(String, String)> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .filter(|path| !path.ends_with("fixtures.rs"))
        .map(|path| {
            let source = std::fs::read_to_string(&path).unwrap();
            // Test modules come last in a file.
            let end = ["#[cfg(test)]", "#[cfg(all(test"]
                .iter()
                .filter_map(|marker| source.find(marker))
                .min()
                .unwrap_or(source.len());
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, code_only(&source[..end]))
        })
        .collect();
    files.sort();
    assert!(files.len() >= 8, "the screens were not found");
    files
}

/// A float literal, other than the few numbers of arithmetic.
fn sized_literal(line: &str) -> Option<String> {
    const ARITHMETIC: &[&str] = &["0.0", "0.5", "1.0", "2.0"];
    let bytes = line.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let starts = bytes[index].is_ascii_digit()
            && (index == 0
                || !(bytes[index - 1].is_ascii_alphanumeric() || bytes[index - 1] == b'_'));
        if !starts {
            index += 1;
            continue;
        }
        let begin = index;
        while index < bytes.len()
            && (bytes[index].is_ascii_digit() || bytes[index] == b'.' || bytes[index] == b'_')
        {
            index += 1;
        }
        let number = line[begin..index].trim_end_matches('.');
        if number.contains('.') && !ARITHMETIC.contains(&number) {
            return Some(number.to_owned());
        }
    }
    None
}

#[test]
fn the_screens_use_no_stock_widget() {
    let mut found = Vec::new();
    for (file, source) in screens() {
        let imported = egui_imports(&source);
        for name in STOCK_TYPES {
            if imported.iter().any(|import| import == name) {
                found.push(format!("{file}: use egui::{name}"));
            }
        }
        for (number, line) in source.lines().enumerate() {
            let qualified = STOCK_TYPES.iter().map(|name| format!("egui::{name}"));
            for pattern in STOCK.iter().map(|call| (*call).to_owned()).chain(qualified) {
                if line.contains(&pattern) {
                    found.push(format!("{file}:{}: {pattern}", number + 1));
                }
            }
        }
    }
    assert!(found.is_empty(), "stock widgets: {found:#?}");
}

#[test]
fn the_screens_have_no_colour_font_or_size_of_their_own() {
    let mut found = Vec::new();
    for (file, source) in screens() {
        for (number, line) in source.lines().enumerate() {
            for pattern in OWN_STYLE {
                if line.contains(pattern) {
                    found.push(format!("{file}:{}: {pattern}", number + 1));
                }
            }
            if let Some(literal) = sized_literal(line) {
                found.push(format!("{file}:{}: {literal}", number + 1));
            }
            // Integer radii and margins, which egui takes as u8 and i8.
            for pattern in [
                "CornerRadius::same(",
                "Margin::same(",
                "Margin::symmetric(",
                "Stroke::new(",
            ] {
                if let Some(at) = line.find(pattern) {
                    let argument = line[at + pattern.len()..].trim_start();
                    if argument.starts_with(|c: char| c.is_ascii_digit()) {
                        found.push(format!("{file}:{}: {pattern}", number + 1));
                    }
                }
            }
        }
    }
    assert!(found.is_empty(), "own styles: {found:#?}");
}

#[test]
fn the_scan_sees_through_strings_but_not_code() {
    let source =
        "let a = \"ui.button( 12.5 Color32::\"; // ui.label( 3.5\nui.button(x); let b = 13.5;";
    let code = code_only(source);
    assert!(!code.contains("Color32"));
    assert!(!code.contains("ui.label("));
    assert!(code.contains("ui.button(x)"));
    assert_eq!(sized_literal(&code), Some("13.5".to_owned()));
    assert_eq!(sized_literal("a.x2.0 + b_3.5"), None);
    assert_eq!(sized_literal("x * 2.0 - 0.5"), None);
    let imports = egui_imports(
        "use egui::{Align, Label as L};
use egui::Window;
",
    );
    assert_eq!(imports, ["Align", "Label", "Window"]);
}

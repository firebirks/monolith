//! QR codes, generated locally.
//!
//! The data of a QR code may be an invitation card, which carries a
//! capability and is secret. Nothing here keeps it: the caller owns the
//! [`QrMatrix`] in its own state and drops it when the dialog closes, and
//! the widget reads the matrix it is given and stores nothing in egui's
//! memory.

use egui::{
    Color32, CornerRadius, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget,
    WidgetType,
};
use qrcodegen::{QrCode as Encoder, QrCodeEcc};

use crate::interact;
use crate::theme;

/// The most data, in bytes, a code is made of. A code of more is too dense
/// to scan from a screen.
pub const MAX_BYTES: usize = 1200;

/// Quiet zone around the code, in modules, as the QR standard asks.
const QUIET_ZONE: usize = 4;

/// Dark modules. Fixed, whatever the theme: scanners need a dark code on a
/// light ground.
const MODULE_DARK: Color32 = Color32::from_rgb(0x0B, 0x0B, 0x0E);

/// The card behind the code and its quiet zone. Fixed, like the modules.
const CARD_LIGHT: Color32 = Color32::WHITE;

/// Why data could not be encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QrError {
    /// More than [`MAX_BYTES`] bytes.
    TooLong,
    /// The data does not fit in any QR code at the chosen error
    /// correction.
    DoesNotFit,
}

impl core::fmt::Display for QrError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooLong => f.write_str("the data is too long for a QR code"),
            Self::DoesNotFit => f.write_str("the data does not fit in a QR code"),
        }
    }
}

impl std::error::Error for QrError {}

/// The modules of a QR code: a square of `size` by `size`, true for dark.
///
/// It holds the data in another form, so it is as secret as the data. Its
/// `Debug` shows only the size.
#[derive(Clone, PartialEq, Eq)]
pub struct QrMatrix {
    size: usize,
    modules: Vec<bool>,
}

impl core::fmt::Debug for QrMatrix {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "QrMatrix({0}x{0})", self.size)
    }
}

impl QrMatrix {
    /// Encodes `data` at medium error correction.
    pub fn encode(data: &str) -> Result<Self, QrError> {
        if data.len() > MAX_BYTES {
            return Err(QrError::TooLong);
        }
        let code =
            Encoder::encode_text(data, QrCodeEcc::Medium).map_err(|_| QrError::DoesNotFit)?;
        let size = usize::try_from(code.size()).map_err(|_| QrError::DoesNotFit)?;
        let side = i32::try_from(size).map_err(|_| QrError::DoesNotFit)?;
        let mut modules = Vec::with_capacity(size.saturating_mul(size));
        for y in 0..side {
            for x in 0..side {
                modules.push(code.get_module(x, y));
            }
        }
        Ok(Self { size, modules })
    }

    /// Modules per side, without the quiet zone.
    pub fn size(&self) -> usize {
        self.size
    }

    /// Whether the module at (`x`, `y`) is dark; false outside the code.
    pub fn is_dark(&self, x: usize, y: usize) -> bool {
        if x >= self.size || y >= self.size {
            return false;
        }
        y.checked_mul(self.size)
            .and_then(|row| row.checked_add(x))
            .and_then(|index| self.modules.get(index))
            .copied()
            .unwrap_or(false)
    }
}

/// Draws a [`QrMatrix`] on a white card, with its quiet zone.
#[derive(Debug)]
#[must_use = "add it with `ui.add(...)`"]
pub struct QrCode<'a> {
    matrix: &'a QrMatrix,
    max_side: f32,
    label: &'a str,
}

impl<'a> QrCode<'a> {
    /// A view of `matrix`. `label` is the accessible description of the
    /// image (for example "QR code of the invitation").
    pub fn new(matrix: &'a QrMatrix, label: &'a str) -> Self {
        Self {
            matrix,
            max_side: 264.0,
            label,
        }
    }

    /// The card is at most this many points on a side (the module size is
    /// whole pixels, so it can be a little smaller).
    pub fn max_side(mut self, side: f32) -> Self {
        self.max_side = side;
        self
    }
}

/// Total modules per side with the quiet zone, as a float.
fn total_modules(matrix: &QrMatrix) -> f32 {
    let total = matrix.size().saturating_add(QUIET_ZONE.saturating_mul(2));
    // A few hundred at most.
    #[allow(clippy::cast_precision_loss)]
    let total = total as f32;
    total
}

/// Physical pixels per module: the largest whole number that keeps the
/// card within `max_px`, and at least one.
fn module_pixels(matrix: &QrMatrix, max_px: f32) -> f32 {
    (max_px / total_modules(matrix)).floor().max(1.0)
}

impl Widget for QrCode<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let ppp = ui.ctx().pixels_per_point();
        let max_side = self.max_side.min(ui.available_width()).max(1.0);
        let module_px = module_pixels(self.matrix, max_side * ppp);
        let total = total_modules(self.matrix);
        let side = module_px * total / ppp;
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
        interact::describe(&response, WidgetType::Image, self.label);
        if !ui.is_rect_visible(rect) {
            return response;
        }

        // Every edge lies on a whole physical pixel: the origin is rounded
        // and a module is a whole number of pixels.
        let origin = Pos2::new(
            (rect.left() * ppp).round() / ppp,
            (rect.top() * ppp).round() / ppp,
        );
        let module = module_px / ppp;
        let card = Rect::from_min_size(origin, Vec2::splat(side));
        let painter = ui.painter();
        painter.rect(
            card,
            CornerRadius::same(theme.radius.lg),
            CARD_LIGHT,
            Stroke::new(theme.stroke.hairline, theme.color.border_subtle),
            StrokeKind::Inside,
        );

        let quiet = module * count_f32(QUIET_ZONE);
        let mut shapes = Vec::new();
        for y in 0..self.matrix.size() {
            // Horizontal runs of dark modules are one rectangle each, so
            // that no seam shows between neighbours.
            let mut run_start: Option<usize> = None;
            for x in 0..=self.matrix.size() {
                let dark = self.matrix.is_dark(x, y);
                match (dark, run_start) {
                    (true, None) => run_start = Some(x),
                    (false, Some(start)) => {
                        run_start = None;
                        shapes.push(egui::Shape::rect_filled(
                            Rect::from_min_max(
                                module_pos(origin, quiet, module, start, y),
                                module_pos(origin, quiet, module, x, y.saturating_add(1)),
                            ),
                            0.0,
                            MODULE_DARK,
                        ));
                    }
                    _ => {}
                }
            }
        }
        painter.extend(shapes);
        response
    }
}

fn count_f32(count: usize) -> f32 {
    // Small counts only.
    #[allow(clippy::cast_precision_loss)]
    let value = count as f32;
    value
}

fn module_pos(origin: Pos2, quiet: f32, module: f32, x: usize, y: usize) -> Pos2 {
    Pos2::new(
        origin.x + quiet + module * count_f32(x),
        origin.y + quiet + module * count_f32(y),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_makes_the_smallest_code() {
        let matrix = QrMatrix::encode("hello");
        assert!(matches!(&matrix, Ok(m) if m.size() == 21));
    }

    #[test]
    fn finder_patterns_are_in_three_corners() {
        let Ok(matrix) = QrMatrix::encode("monolith") else {
            panic!("encodes");
        };
        let last = matrix.size().saturating_sub(1);
        assert!(matrix.is_dark(0, 0));
        assert!(matrix.is_dark(last, 0));
        assert!(matrix.is_dark(0, last));
    }

    #[test]
    fn encoding_is_deterministic() {
        assert_eq!(QrMatrix::encode("same"), QrMatrix::encode("same"));
        assert_ne!(QrMatrix::encode("one"), QrMatrix::encode("two"));
    }

    #[test]
    fn data_over_the_limit_is_refused() {
        let at_limit = "a".repeat(MAX_BYTES);
        assert!(QrMatrix::encode(&at_limit).is_ok());
        let over = "a".repeat(MAX_BYTES.saturating_add(1));
        assert_eq!(QrMatrix::encode(&over), Err(QrError::TooLong));
    }

    #[test]
    fn outside_the_code_is_light() {
        let Ok(matrix) = QrMatrix::encode("x") else {
            panic!("encodes");
        };
        assert!(!matrix.is_dark(matrix.size(), 0));
        assert!(!matrix.is_dark(0, usize::MAX));
    }

    #[test]
    fn debug_does_not_show_modules() {
        let Ok(matrix) = QrMatrix::encode("secret") else {
            panic!("encodes");
        };
        let shown = format!("{matrix:?}");
        assert!(!shown.contains("true") && !shown.contains("secret"));
    }

    #[test]
    fn module_pixels_are_whole_and_at_least_one() {
        let Ok(matrix) = QrMatrix::encode("hello") else {
            panic!("encodes");
        };
        // 21 + 8 = 29 modules.
        assert!((module_pixels(&matrix, 300.0) - 10.0).abs() < f32::EPSILON);
        assert!((module_pixels(&matrix, 5.0) - 1.0).abs() < f32::EPSILON);
    }
}

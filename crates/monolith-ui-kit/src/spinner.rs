//! A small progress mark for actions under way. It turns while it is
//! shown, which keeps the window repainting only for as long as an action
//! runs; with reduced motion it is a still arc and asks for no repaint.

use egui::{Color32, Pos2, Shape, Stroke, Ui};

use crate::theme;

/// Paints the progress mark centred on `center`, `size` points across.
pub fn paint(ui: &Ui, center: Pos2, size: f32, color: Color32) {
    let theme = theme::current(ui.ctx());
    let radius = size * 0.38;
    let start = if theme.motion.reduced {
        0.0
    } else {
        ui.ctx().request_repaint();
        // Wrapped to one turn, so that the angle stays small.
        (ui.input(|input| input.time) * 5.0).rem_euclid(core::f64::consts::TAU)
    };
    // A turn is less than 7, which f32 holds exactly enough.
    #[allow(clippy::cast_possible_truncation)]
    let start = start as f32;
    let steps = 24_u8;
    let sweep = core::f32::consts::TAU * 0.7;
    let points: Vec<Pos2> = (0..=steps)
        .map(|step| {
            let angle = start + sweep * f32::from(step) / f32::from(steps);
            center + radius * egui::vec2(angle.cos(), angle.sin())
        })
        .collect();
    ui.painter().add(Shape::line(
        points,
        Stroke::new(theme.stroke.focus.max(size / 9.0), color),
    ));
}

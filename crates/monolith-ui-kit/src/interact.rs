//! What every interactive component shares: the resolution of its
//! interaction state, colour mixing, the focus ring and the accessible
//! description.

use egui::{
    Color32, CornerRadius, CursorIcon, Id, Painter, Rect, Response, Stroke, StrokeKind, Ui,
    WidgetInfo, WidgetType,
};

use crate::theme::Theme;

/// The interaction state of a component in this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct State {
    /// The component accepts input.
    pub enabled: bool,
    /// How far the hover animation is, 0 to 1.
    pub hover: f32,
    /// The pointer is pressed on it.
    pub pressed: bool,
    /// It has keyboard focus.
    pub focused: bool,
    /// It is selected (rows, tabs, toggles).
    pub selected: bool,
}

impl State {
    /// The state of `response`, with the hover eased over the theme's fast
    /// duration under the id `id`.
    pub fn of(ui: &Ui, response: &Response, id: Id, theme: &Theme, selected: bool) -> Self {
        let enabled = response.enabled();
        let hovered = enabled && response.hovered();
        let hover = ui.ctx().animate_bool_with_time(
            id.with("hover"),
            hovered,
            theme.motion.duration(theme.motion.fast),
        );
        if hovered {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        Self {
            enabled,
            hover,
            pressed: enabled && response.is_pointer_button_down_on(),
            focused: enabled && response.has_focus(),
            selected,
        }
    }
}

/// `a` mixed towards `b` by `t` (0 to 1), in gamma space as egui blends.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t.clamp(0.0, 1.0))
}

/// `tint` laid over `base`, as if painted on top of it.
pub fn over(base: Color32, tint: Color32) -> Color32 {
    let alpha = f32::from(tint.a()) / 255.0;
    let opaque = Color32::from_rgb(tint.r(), tint.g(), tint.b());
    // `tint` is premultiplied; undo that for the mix.
    let unmultiplied = if alpha > 0.0 {
        let channel = |value: u8| -> u8 {
            let scaled = (f32::from(value) / alpha).round().clamp(0.0, 255.0);
            // In range by the clamp above.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let byte = scaled as u8;
            byte
        };
        Color32::from_rgb(
            channel(opaque.r()),
            channel(opaque.g()),
            channel(opaque.b()),
        )
    } else {
        opaque
    };
    mix(base, unmultiplied, alpha)
}

/// Paints the focus ring around `rect`, whose corners are `radius`.
pub fn focus_ring(painter: &Painter, rect: Rect, radius: CornerRadius, theme: &Theme) {
    let gap = theme.stroke.focus_gap;
    let grow = |value: u8| -> u8 {
        // A radius grows by the gap, at most to the full u8 range.
        let added = f32::from(value) + gap;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let byte = added.clamp(0.0, 255.0) as u8;
        byte
    };
    let ring = CornerRadius {
        nw: grow(radius.nw),
        ne: grow(radius.ne),
        sw: grow(radius.sw),
        se: grow(radius.se),
    };
    painter.rect_stroke(
        rect.expand(gap),
        ring,
        Stroke::new(theme.stroke.focus, theme.color.focus_ring),
        StrokeKind::Outside,
    );
}

/// Tells AccessKit what `response` is: its role, label and state.
pub fn describe(response: &Response, kind: WidgetType, label: &str) {
    let enabled = response.enabled();
    response.widget_info(|| WidgetInfo::labeled(kind, enabled, label));
}

/// Tells AccessKit what a selectable `response` is, selected or not.
pub fn describe_selectable(response: &Response, kind: WidgetType, label: &str, selected: bool) {
    let enabled = response.enabled();
    response.widget_info(|| WidgetInfo::selected(kind, enabled, selected, label));
}

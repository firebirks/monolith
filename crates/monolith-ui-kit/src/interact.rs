//! What every interactive component shares: the resolution of its
//! interaction state, colour mixing, the focus ring and the accessible
//! description.

use std::sync::Arc;

use egui::{
    Color32, Context, CornerRadius, CursorIcon, Event, Id, Key, Painter, Rect, Response, Stroke,
    StrokeKind, Ui, WidgetInfo, WidgetType,
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
    /// duration under the id `id`. A response that senses neither clicks
    /// nor drags (a control that takes no input) is never hovered or
    /// pressed, and does not change the pointer.
    pub fn of(ui: &Ui, response: &Response, id: Id, theme: &Theme, selected: bool) -> Self {
        let enabled = response.enabled();
        let takes_input =
            enabled && (response.sense.senses_click() || response.sense.senses_drag());
        let hovered = takes_input && response.hovered();
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
            pressed: takes_input && response.is_pointer_button_down_on(),
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

fn keyboard_focus_id() -> Id {
    Id::new("monolith-ui-kit/keyboard-focus")
}

fn tracking_id() -> Id {
    Id::new("monolith-ui-kit/focus-tracking")
}

/// What the input of this pass says of where the focus comes from: the
/// keyboard (Tab, Shift+Tab, a screen reader), the pointer, or neither.
fn focus_source(ctx: &Context) -> Option<bool> {
    ctx.input(|input| {
        input.raw.events.iter().rev().find_map(|event| match event {
            Event::Key {
                key: Key::Tab,
                pressed: true,
                ..
            } => Some(true),
            Event::AccessKitActionRequest(request)
                if request.action == egui::accesskit::Action::Focus =>
            {
                Some(true)
            }
            Event::PointerButton { pressed: true, .. } | Event::Touch { .. } => Some(false),
            _ => None,
        })
    })
}

/// Starts following, at the start of every pass, whether the focus was
/// last moved with the keyboard. [`crate::theme::install`] calls it; a
/// second call does nothing.
pub(crate) fn track_focus_source(ctx: &Context) {
    let started = ctx.data(|data| data.get_temp::<bool>(tracking_id()).unwrap_or(false));
    if started {
        return;
    }
    ctx.data_mut(|data| data.insert_temp(tracking_id(), true));
    ctx.on_begin_pass(
        "monolith-ui-kit focus source",
        Arc::new(|ctx: &Context| {
            if let Some(keyboard) = focus_source(ctx) {
                ctx.data_mut(|data| data.insert_temp(keyboard_focus_id(), keyboard));
            }
        }),
    );
}

/// Whether focus rings are shown: the focus was last moved with the
/// keyboard (Tab or Shift+Tab), and no pointer button was pressed since.
/// Focus given by a click or by a screen asking for it shows no ring.
pub fn focus_visible(ctx: &Context) -> bool {
    ctx.data(|data| data.get_temp::<bool>(keyboard_focus_id()))
        .unwrap_or(false)
}

/// Shows or hides focus rings until the next Tab or pointer press, as if
/// the focus had come from the keyboard (`true`) or from the pointer. For
/// galleries and tests that put the focus somewhere by hand.
pub fn set_focus_visible(ctx: &Context, visible: bool) {
    ctx.data_mut(|data| data.insert_temp(keyboard_focus_id(), visible));
}

/// Paints the focus ring around `rect`, whose corners are `radius`, when
/// focus rings are shown ([`focus_visible`]).
pub fn focus_ring(painter: &Painter, rect: Rect, radius: CornerRadius, theme: &Theme) {
    if !focus_visible(painter.ctx()) {
        return;
    }
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
    describe_enabled(response, kind, label, true);
}

/// Like [`describe`], for a control that can turn itself off: `enabled`
/// is false while it takes no input (disabled or busy), even in an enabled
/// `Ui`.
pub fn describe_enabled(response: &Response, kind: WidgetType, label: &str, enabled: bool) {
    let enabled = enabled && response.enabled();
    response.widget_info(|| WidgetInfo::labeled(kind, enabled, label));
}

/// Tells AccessKit what a selectable `response` is, selected or not.
pub fn describe_selectable(response: &Response, kind: WidgetType, label: &str, selected: bool) {
    describe_selectable_enabled(response, kind, label, selected, true);
}

/// Like [`describe_selectable`], for a control that can turn itself off.
pub fn describe_selectable_enabled(
    response: &Response,
    kind: WidgetType,
    label: &str,
    selected: bool,
    enabled: bool,
) {
    let enabled = enabled && response.enabled();
    response.widget_info(|| WidgetInfo::selected(kind, enabled, selected, label));
}

//! The tooltip: a short caption on the raised surface, attached to the
//! response of a control. egui decides when it shows (after a pause, when
//! the pointer is still); the frame, the colours and the shadow come from
//! the installed theme through egui's style, the text from the caption
//! token.

use egui::{Frame, Margin, Response, RichText, Tooltip, Ui};

use crate::theme;

/// The widest a tooltip grows before its text wraps.
const MAX_WIDTH: f32 = 240.0;

fn content(ui: &mut Ui, text: &str) {
    let theme = theme::current(ui.ctx());
    ui.set_max_width(MAX_WIDTH);
    Frame::NONE
        .inner_margin(Margin::symmetric(4, 2))
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(
                    RichText::new(text)
                        .font(theme.text.caption.font_id())
                        .color(theme.color.text),
                )
                .selectable(false),
            );
        });
}

/// Shows `text` as a tooltip while `response` is hovered, enabled or not,
/// and returns the response.
pub fn attach(response: Response, text: &str) -> Response {
    show(&response, text);
    response
}

/// Shows `text` as a tooltip of `response`, for callers that keep the
/// response themselves.
pub fn show(response: &Response, text: &str) {
    if text.is_empty() {
        return;
    }
    Tooltip::for_enabled(response).show(|ui| content(ui, text));
    Tooltip::for_disabled(response).show(|ui| content(ui, text));
}

/// Paints a tooltip in place, in the current layout, for galleries and for
/// screens that draw one by hand.
pub fn paint_inline(ui: &mut Ui, text: &str) {
    let theme = theme::current(ui.ctx());
    Frame::new()
        .fill(theme.color.bg_raised)
        .stroke(egui::Stroke::new(
            theme.stroke.hairline,
            theme.color.border_subtle,
        ))
        .corner_radius(egui::CornerRadius::same(theme.radius.md))
        .shadow(theme.shadow.md)
        .inner_margin(Margin::same(6))
        .show(ui, |ui| content(ui, text));
}

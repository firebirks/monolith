//! The Tor status pill of the top bar: a coloured dot and a short label.
//! The label says the state in words, so the colour is never the only
//! carrier of it. The pill is a button that opens the details.

use std::time::Duration;

use egui::{
    Align2, Color32, CornerRadius, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget,
    WidgetType,
};

use crate::interact::{self, State, mix, over};
use crate::theme::{self, Theme};

/// The state of Tor as the runtime reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TorState {
    /// The Tor client is starting.
    Starting,
    /// Building circuits.
    Connecting,
    /// Connected through Tor.
    Connected,
    /// Tor cannot be used.
    Unavailable,
}

/// How often a pulsing dot is redrawn: four times a second. Tor can take
/// long to connect, and the whole window is drawn for each step of the
/// glow; a slow glow in steps is enough to say that something goes on.
const PULSE_FRAME: Duration = Duration::from_millis(250);

/// The pill. Add it with `ui.add(...)`; the response is clicked when the
/// user asks for the details.
#[derive(Clone, Debug)]
#[must_use = "add it with `ui.add(...)`"]
pub struct TorStatus<'a> {
    state: TorState,
    label: &'a str,
}

impl<'a> TorStatus<'a> {
    /// A pill for `state` that reads `label`, for example "Connected
    /// through Tor".
    pub fn new(state: TorState, label: &'a str) -> Self {
        Self { state, label }
    }
}

fn dot_color(theme: &Theme, state: TorState) -> Color32 {
    let c = &theme.color;
    match state {
        TorState::Connected => c.success,
        TorState::Connecting => c.warning,
        TorState::Unavailable => c.danger,
        TorState::Starting => c.text_tertiary,
    }
}

struct Colors {
    fill: Color32,
    stroke: Color32,
    text: Color32,
}

fn colors(theme: &Theme, state: TorState, ui_state: State) -> Colors {
    let c = &theme.color;
    let (base, text) = match state {
        TorState::Connected | TorState::Starting => (c.bg_sunken, c.text_secondary),
        TorState::Connecting => (c.warning_subtle, c.text),
        TorState::Unavailable => (c.danger_subtle, c.danger),
    };
    let tint = if ui_state.pressed {
        c.pressed
    } else {
        c.hover.gamma_multiply(ui_state.hover)
    };
    Colors {
        fill: over(base, tint),
        stroke: mix(c.border_subtle, c.border, ui_state.hover),
        text: mix(text, c.text, ui_state.hover * 0.5),
    }
}

impl Widget for TorStatus<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let dot = 8.0;
        let gap = theme.space.sm;
        let pad = theme.space.md;
        let galley = ui.painter().layout_no_wrap(
            self.label.to_owned(),
            theme.text.caption_strong.font_id(),
            Color32::PLACEHOLDER,
        );
        let width = pad + dot + gap + galley.size().x + pad;
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(width, theme.size.control_sm), Sense::click());
        interact::describe(&response, WidgetType::Button, self.label);

        if ui.is_rect_visible(rect) {
            let ui_state = State::of(ui, &response, response.id, &theme, false);
            let colors = colors(&theme, self.state, ui_state);
            let radius = CornerRadius::same(theme.radius.pill);
            let painter = ui.painter();
            painter.rect(
                rect,
                radius,
                colors.fill,
                Stroke::new(theme.stroke.hairline, colors.stroke),
                StrokeKind::Inside,
            );

            let center = egui::pos2(rect.left() + pad + dot / 2.0, rect.center().y);
            let color = dot_color(&theme, self.state);
            if self.state == TorState::Connecting && !theme.motion.reduced {
                // A slow glow around the dot. The next frame is requested
                // only while the pill is shown in this state.
                let time = ui.input(|input| input.time);
                let phase = (time * core::f64::consts::TAU / 1.6).sin();
                // Between 0 and 1; precision beyond f32 is of no use here.
                #[allow(clippy::cast_possible_truncation)]
                let glow = (phase as f32 * 0.5 + 0.5).clamp(0.0, 1.0);
                painter.circle_filled(
                    center,
                    dot / 2.0 + 1.0 + glow * 3.0,
                    color.gamma_multiply(0.30 * (1.0 - glow)),
                );
                ui.ctx().request_repaint_after(PULSE_FRAME);
            }
            painter.circle_filled(center, dot / 2.0, color);

            let text_pos = Align2::LEFT_CENTER
                .anchor_size(
                    egui::pos2(rect.left() + pad + dot + gap, rect.center().y),
                    galley.size(),
                )
                .min;
            painter.galley(text_pos, galley, colors.text);
            if ui_state.focused {
                interact::focus_ring(painter, rect, radius, &theme);
            }
        }
        response
    }
}

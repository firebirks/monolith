//! A headless rig that drives egui frame by frame, for the component tests.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

use egui::epaint::Shape;
use egui::output::OutputEvent;
use egui::{
    Context, Event, FullOutput, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, WidgetInfo,
    pos2, vec2,
};
use monolith_ui_kit::{Theme, fonts, theme};

/// The size of the window in every test.
pub fn screen() -> Rect {
    Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 600.0))
}

/// A context with the fonts and the light theme, and a clock.
pub struct Rig {
    pub ctx: Context,
    time: f64,
}

impl Rig {
    pub fn new() -> Self {
        Self::with_theme(Theme::light())
    }

    pub fn with_theme(theme: Theme) -> Self {
        let ctx = Context::default();
        fonts::install(&ctx, &[]);
        theme::install(&ctx, theme.with_reduced_motion(true));
        Self { ctx, time: 1.0 }
    }

    /// Runs one frame with `events`, 50 ms after the last.
    pub fn frame(&mut self, events: Vec<Event>, run: impl FnMut(&Context)) -> FullOutput {
        self.frame_after(0.05, events, run)
    }

    /// Lets `seconds` pass, then runs one frame with `events`.
    pub fn frame_after(
        &mut self,
        seconds: f64,
        events: Vec<Event>,
        run: impl FnMut(&Context),
    ) -> FullOutput {
        self.time += seconds;
        let input = RawInput {
            screen_rect: Some(screen()),
            time: Some(self.time),
            events,
            ..RawInput::default()
        };
        self.ctx.run(input, run)
    }

    /// Frames without events, so that layout settles.
    pub fn settle(&mut self, frames: usize, mut run: impl FnMut(&Context)) -> FullOutput {
        let mut last = self.frame(vec![], &mut run);
        for _ in 1..frames {
            last = self.frame(vec![], &mut run);
        }
        last
    }

    /// Moves the pointer to `pos`, presses the primary button and releases
    /// it, one frame each. Returns the outputs of the three frames.
    pub fn click(&mut self, pos: Pos2, mut run: impl FnMut(&Context)) -> Vec<FullOutput> {
        vec![
            self.frame(vec![Event::PointerMoved(pos)], &mut run),
            self.frame(vec![button(pos, true)], &mut run),
            self.frame(vec![button(pos, false)], &mut run),
        ]
    }

    /// Presses and releases `key` in two frames.
    pub fn key(&mut self, key: Key, run: impl FnMut(&Context)) -> Vec<FullOutput> {
        self.key_with(key, Modifiers::NONE, run)
    }

    /// Presses and releases `key` with `modifiers` in two frames.
    pub fn key_with(
        &mut self,
        key: Key,
        modifiers: Modifiers,
        mut run: impl FnMut(&Context),
    ) -> Vec<FullOutput> {
        let event = |pressed| Event::Key {
            key,
            physical_key: Some(key),
            pressed,
            repeat: false,
            modifiers,
        };
        vec![
            self.frame(vec![event(true)], &mut run),
            self.frame(vec![event(false)], &mut run),
        ]
    }

    /// Types `text` in one frame.
    pub fn type_text(&mut self, text: &str, run: impl FnMut(&Context)) -> FullOutput {
        self.frame(vec![Event::Text(text.to_owned())], run)
    }
}

/// A press or a release of the primary button at `pos`.
pub fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    }
}

/// The Ctrl+Z modifiers.
pub fn command() -> Modifiers {
    Modifiers {
        ctrl: true,
        command: true,
        ..Modifiers::NONE
    }
}

fn collect(shape: &Shape, into: &mut Vec<String>) {
    match shape {
        Shape::Vec(shapes) => {
            for shape in shapes {
                collect(shape, into);
            }
        }
        Shape::Text(text) => into.push(text.galley.text().to_owned()),
        _ => {}
    }
}

/// Every string painted in `output`, in paint order.
pub fn texts(output: &FullOutput) -> Vec<String> {
    let mut found = Vec::new();
    for clipped in &output.shapes {
        collect(&clipped.shape, &mut found);
    }
    found
}

/// Whether `needle` is painted as, or inside, a string of `output`.
pub fn paints(output: &FullOutput, needle: &str) -> bool {
    texts(output).iter().any(|text| text.contains(needle))
}

/// Everything painted in `output`, as text, to compare two frames.
pub fn dump(output: &FullOutput) -> String {
    format!("{:?}", output.shapes)
}

/// What the widgets reported as clicked in `outputs`, as AccessKit would
/// be told.
pub fn clicked_infos(outputs: &[FullOutput]) -> Vec<WidgetInfo> {
    outputs
        .iter()
        .flat_map(|output| output.platform_output.events.iter())
        .filter_map(|event| match event {
            OutputEvent::Clicked(info) => Some(info.clone()),
            _ => None,
        })
        .collect()
}

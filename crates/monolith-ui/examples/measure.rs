//! Measures what a frame of the window costs on the CPU, layout and
//! tessellation included, in the states that matter for performance:
//! `cargo run --release -p monolith-ui --features fixtures --example measure`.
//!
//! Each line gives the median and the slowest of a number of frames, and
//! the vertices the last one produced. Painting on the GPU is not included;
//! it depends on the machine.

#![allow(
    clippy::print_stdout,
    clippy::expect_used,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

use std::time::Instant;

use core::time::Duration;

use egui::{Context, Event, Pos2, RawInput, Rect, Vec2};
use monolith_ui::App;
use monolith_ui::fixtures::{fixture, long_conversation, long_conversation_of, many_contacts};
use monolith_ui_kit::{Theme, fonts, theme};
use monolith_ui_model::{RuntimeEvent, UiContactId, UiIdentityId, UiMessageId, Update};

const FRAMES: usize = 60;

struct Window {
    ctx: Context,
    app: App,
    time: f64,
}

impl Window {
    fn new(app: App) -> Self {
        let ctx = Context::default();
        fonts::install(&ctx, &[]);
        theme::install(&ctx, Theme::light());
        ctx.style_mut(|style| style.visuals.text_cursor.blink = false);
        Self {
            ctx,
            app,
            time: 0.0,
        }
    }

    /// One frame, with `events`; returns its time and its vertices.
    fn frame(&mut self, events: Vec<Event>) -> (Duration, usize) {
        self.time += 1.0 / 60.0;
        let size = Theme::light().size.window_default;
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            time: Some(self.time),
            focused: true,
            events,
            ..RawInput::default()
        };
        let started = Instant::now();
        let output = self
            .ctx
            .run(input, |ctx| monolith_ui::show(ctx, &mut self.app));
        let meshes = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        let elapsed = started.elapsed();
        let vertices = meshes
            .iter()
            .map(|mesh| match &mesh.primitive {
                egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.len(),
                egui::epaint::Primitive::Callback(_) => 0,
            })
            .sum();
        (elapsed, vertices)
    }

    /// Frames until layout and animations settle.
    fn settle(&mut self) {
        for _ in 0..30 {
            self.frame(Vec::new());
        }
    }
}

fn report(what: &str, mut times: Vec<Duration>, vertices: usize) {
    times.sort();
    let median = times[times.len() / 2];
    let slowest = times[times.len() - 1];
    println!(
        "{what:<44} median {:>7.2} ms   slowest {:>7.2} ms   {vertices:>7} vertices",
        median.as_secs_f64() * 1000.0,
        slowest.as_secs_f64() * 1000.0
    );
}

/// The frames of a state with nothing happening, as a blinking cursor or
/// an event would cause them.
fn idle(what: &str, app: App) {
    let mut window = Window::new(app);
    window.settle();
    let mut times = Vec::new();
    let mut vertices = 0;
    for _ in 0..FRAMES {
        let (time, count) = window.frame(Vec::new());
        times.push(time);
        vertices = count;
    }
    report(what, times, vertices);
}

/// The frame that opens the conversation of `app` after another was shown:
/// its messages are taken and the newest laid out for the first time.
fn opening(what: &str, app: App) {
    let mut window = Window::new(app);
    let alice = UiContactId::new(UiIdentityId::from_bytes([1; 16]), [10; 16]);
    let bob = UiContactId::new(UiIdentityId::from_bytes([1; 16]), [11; 16]);
    window.app.model.select_conversation(bob);
    window.settle();
    window.app.model.select_conversation(alice);
    let (time, vertices) = window.frame(Vec::new());
    report(what, vec![time], vertices);
}

/// The frames of scrolling up through `app` with the pointer at `at`, a
/// wheel turn a frame.
fn scrolling(what: &str, app: App, at: Pos2) {
    let mut window = Window::new(app);
    window.settle();
    let mut times = Vec::new();
    let mut vertices = 0;
    for _ in 0..FRAMES {
        let (time, count) = window.frame(vec![
            Event::PointerMoved(at),
            Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: Vec2::new(0.0, 120.0),
                modifiers: egui::Modifiers::NONE,
            },
        ]);
        times.push(time);
        vertices = count;
    }
    report(what, times, vertices);
}

fn main() {
    // The first frame of a window builds the font atlas.
    let started = Instant::now();
    let mut window = Window::new(fixture("welcome").expect("fixture"));
    let (first, _) = window.frame(Vec::new());
    println!(
        "start: context, fonts and first frame {:.1} ms (the frame alone {:.1} ms)",
        started.elapsed().as_secs_f64() * 1000.0,
        first.as_secs_f64() * 1000.0
    );

    idle("welcome", fixture("welcome").expect("fixture"));
    idle(
        "conversation of 8 messages",
        fixture("conversation").expect("fixture"),
    );
    idle("requests", fixture("requests").expect("fixture"));
    idle(
        "settings, identities",
        fixture("settings-identities").expect("fixture"),
    );
    for count in [100, 500, 2_000] {
        idle(
            &format!("conversation of {count} messages"),
            long_conversation(count),
        );
    }
    // The longest window of long messages: 1.5 KB, and the largest a
    // message can be.
    for (bytes, label) in [(1_536, "1.5 KB"), (16_384, "16 KB")] {
        idle(
            &format!("conversation of 2000 messages of {label}"),
            long_conversation_of(2_000, bytes),
        );
    }
    // Many contacts, each with a long latest message.
    idle(
        "sidebar of 500 contacts, 16 KB previews",
        many_contacts(500, 16_384),
    );

    // Opening the longest windows.
    opening("opening 2000 messages", long_conversation(2_000));
    for (bytes, label) in [(1_536, "1.5 KB"), (16_384, "16 KB")] {
        opening(
            &format!("opening 2000 messages of {label}"),
            long_conversation_of(2_000, bytes),
        );
    }

    // Scrolling through the longest windows and the long sidebar, a wheel
    // turn a frame.
    let middle = Pos2::new(800.0, 400.0);
    scrolling("scrolling 2000 messages", long_conversation(2_000), middle);
    for (bytes, label) in [(1_536, "1.5 KB"), (16_384, "16 KB")] {
        scrolling(
            &format!("scrolling 2000 messages of {label}"),
            long_conversation_of(2_000, bytes),
            middle,
        );
    }
    scrolling(
        "scrolling the sidebar of 500 contacts",
        many_contacts(500, 16_384),
        Pos2::new(150.0, 400.0),
    );

    // Switching between the two identities, a switch a frame.
    let mut window = Window::new(long_conversation(500));
    window.settle();
    let identities = [
        UiIdentityId::from_bytes([2; 16]),
        UiIdentityId::from_bytes([1; 16]),
    ];
    let mut times = Vec::new();
    let mut vertices = 0;
    for index in 0..FRAMES {
        window.app.model.select_identity(identities[index % 2]);
        let (time, count) = window.frame(Vec::new());
        times.push(time);
        vertices = count;
    }
    report("identity switch a frame (500 messages)", times, vertices);

    // A burst of a thousand events between two frames: what the model does
    // with them, then the frame.
    let mut window = Window::new(long_conversation(500));
    window.settle();
    let alice = UiContactId::new(UiIdentityId::from_bytes([1; 16]), [10; 16]);
    let started = Instant::now();
    for sequence in 0..1_000 {
        window
            .app
            .model
            .apply(Update::Event(RuntimeEvent::MessageReceived(
                UiMessageId::new(alice, 10_000 + sequence),
            )));
    }
    let commands = window.app.model.take_commands();
    let applied = started.elapsed();
    let (frame, _) = window.frame(Vec::new());
    println!(
        "burst of 1000 events: applied in {:.2} ms, {} reads asked, next frame {:.2} ms",
        applied.as_secs_f64() * 1000.0,
        commands.len(),
        frame.as_secs_f64() * 1000.0
    );
}

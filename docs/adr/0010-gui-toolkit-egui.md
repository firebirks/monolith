# ADR 0010: GUI toolkit: egui and eframe

Status: decided for Phase 6. Supersedes ADR 0006, which leaned towards
GTK 4 and was never decided.
Date: 2026-10-14.

## Context

ADR 0006 compared toolkits for a front end over `monolith-core` and
excluded egui, because it has no bidirectional text and accessibility was
uncertain. The brief of Phase 6 (2026-10-14) decides the toolkit: Rust
with egui and eframe, no GTK, Qt, web view, Electron or Tauri. This ADR
records what that choice brings and how its weak points are handled.

The minimum Rust version stays 1.85.1 (`ARCHITECTURE.md` section 10). The
newest egui is 0.36; the newest release that builds with 1.85 is 0.32.3.

## Decision

- `egui` and `eframe` at exactly 0.32.3. eframe with the `glow` renderer
  (OpenGL through glutin, which finds EGL or GLX at run time, so Mesa's
  software renderer serves where there is no GPU), and the `x11`,
  `wayland` and `accesskit` features. No `wgpu`, no `persistence`, no
  default fonts.
- Fonts, icons and the look are Monolith's own (ADR 0012).
- Accessibility goes through AccessKit, which on Linux publishes the
  widget tree to AT-SPI over the session bus (`zbus`). This brings about
  70 of the roughly 206 crates of a Linux build. It is kept: without it a
  screen reader sees nothing.
- eframe enables egui-winit's `links` feature, which opens URLs with the
  system browser through the `webbrowser` crate. Monolith opens no URL
  (`PROTOCOL.md` section 9.1): the application removes every `OpenUrl`
  command from egui's output at the end of every frame, before eframe
  hands the output to egui-winit, and draws no hyperlink widget. A test
  checks the filter; a mutation fault removes it.
- Window geometry and the few preferences are kept by Monolith in its own
  file (ADR 0011), not by eframe's persistence, which would also store
  egui's memory.

## Consequences

- Text: egui lays out text without the Unicode bidirectional algorithm
  and without complex shaping. Right-to-left scripts are drawn in logical
  order, and scripts that need shaping (Arabic, Indic scripts) are drawn
  glyph by glyph. No text is reordered, so a bidirectional control in a
  peer's text cannot move other text; the controls are drawn as visible
  marks (ADR 0011). Readers of those scripts get a degraded display; this
  is a documented limitation of version 1.
- Input methods: winit passes IME composition to egui, which supports it
  on X11 and Wayland with known gaps; documented, not claimed complete.
- Rendering, text and input code is statically linked Rust that Monolith
  ships, not a system library that Debian updates. Monolith is
  responsible for updating it. 0.32.3 is behind the current release
  because of the minimum Rust version; raising that version is the way to
  newer egui.
- About 206 crates for a Linux build, against 62 for GTK 4. Each new crate
  is checked by `cargo deny` like the rest; `docs/DEPENDENCIES.md` lists
  the direct ones.
- The display connection (X11 or Wayland) and the session bus are local
  sockets of the desktop session. They carry no peer data, and no Monolith
  code opens another socket from the GUI (ADR 0011).

## Alternatives considered

ADR 0006 has the comparison. The brief rules out GTK, Qt, web views,
Electron and Tauri.

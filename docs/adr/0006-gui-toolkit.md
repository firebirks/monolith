# ADR 0006: GUI toolkit

Status: undecided. To be decided before Phase 8. This records the
evaluation so far.
Date: 2026-10-01

## Context

The desktop front end is an adapter over `monolith-core`. It must run on:

| Target | Desktop | Display server |
| --- | --- | --- |
| Tails 7 | GNOME 48 | Wayland, Xwayland present |
| Whonix 18, VirtualBox and KVM | LXQt | Wayland (labwc), Xwayland present |
| Qubes-Whonix 18 | LXQt | X11 |
| Other Linux | any | either |

Requirements: correct rendering of hostile text (bidirectional text,
combining sequences, long unbroken strings), input methods, accessibility,
no web engine, a dependency footprint that can be reviewed, and security
updates for the rendering and text stack.

Debian 13 provides GTK 4.18.6, libadwaita 1.7.6 and Qt 6.8.2. Tails 7.14
has all three in the image. Whonix-Workstation 18 has Qt 6 through LXQt and,
by its default application set, GTK 4 and libadwaita (inferred from package
dependencies, not from an image manifest).

## Evaluation

Crate counts are for a Linux build with default features, measured on
2026-10-01.

| Toolkit | Native libraries | Wayland / X11 | Accessibility | Bidi and IME | Crates | Notes |
| --- | --- | --- | --- | --- | --- | --- |
| GTK 4 (`gtk4` crate, optional libadwaita) | system, dynamically linked, Debian security updates | both | AT-SPI, built in | Pango, HarfBuzz, FriBidi; mature | 62 | Current bindings need Rust 1.92; 0.10.x builds with 1.85. X11 backend deprecated for a future GTK 5. |
| Relm4 on GTK 4 | same | both | same | same | 79 | Convenience layer; Rust 1.93. |
| Qt 6 (`cxx-qt`) | system, dynamically linked | both | AT-SPI bridge | mature | 52 | C++ bridge and toolchain; bindings describe themselves as early; QML brings a JavaScript engine. Native look on LXQt. |
| Slint | none, statically linked Rust | both | AT-SPI through AccessKit | present, quality untested | 365 | GPL-3.0 or a royalty-free licence with attribution. |
| iced | none | both | none | open bidi issues | 240 | Describes itself as experimental. |
| egui | none | both | AT-SPI through AccessKit | no bidi | 262 | Immediate mode. |
| FLTK (`fltk`) | bundled C++, static | both | none built in | no bidi | 14 | |
| Tauri | system WebKitGTK | both | through WebKit | through WebKit | 270 | Embeds a web engine and a JavaScript runtime. |

Excluded outright:

- Electron and Tauri: a web engine is the largest attack surface available
  and brings back the class of problems around links and remote content.
- iced, egui, FLTK: each lacks accessibility or bidirectional text, and
  hostile bidirectional text is one of the inputs the UI must handle.

## Leaning

GTK 4 through the `gtk4` crate.

- The rendering, text, input method and accessibility code is in system
  libraries that Debian maintains and updates. A statically linked Rust
  toolkit would make Monolith responsible for shipping every fix in that
  stack.
- Present on both primary targets.
- A small Rust dependency tree: 62 crates, against 240 or more for the
  statically linked toolkits. Only the FLTK and Qt bindings are smaller.
- Works on Wayland and on X11, which Qubes-Whonix still needs.
- Software rendering is available where there is no GPU.

Against it: libadwaita styling looks foreign on LXQt, so plain GTK 4 without
libadwaita may be the better fit; the X11 backend is deprecated upstream;
current binding releases move faster than Debian's compiler.

Runner-up: Qt 6 through `cxx-qt`, which is the native toolkit on Whonix and
builds with Rust 1.85, at the cost of a C++ layer and a JavaScript engine if
QML is used.

## What does not depend on the choice

- No protocol logic in callbacks. The front end sends commands and draws
  events.
- Peer text is drawn with plain-text widgets, with bidirectional isolation
  and the rendering limits of `RESOURCE_LIMITS.md` section 10.
- No link is opened. No remote resource is loaded. No URL handler is
  registered.
- No status is shown as secure because Tor is connected. Pinned and
  verified contacts look different. Ephemeral sessions are labeled.
- QR codes are generated locally.

## To do before deciding

- Check the default Whonix-Workstation image for GTK 4.
- Prototype one hostile-text screen in GTK 4 and in Qt 6 and compare
  behavior with bidirectional overrides, large combining sequences and long
  unbroken strings.
- Check screen reader behavior on Tails.
- Decide whether the minimum Rust version may differ for the desktop crate.

## Sources

Accessed 2026-10-01.

- https://tails.net/torrents/files/tails-amd64-7.14.packages
- https://forums.whonix.org/t/whonix-18-0-8-7-released-major-release-upgrade/22469
- https://forums.whonix.org/t/is-whonix-18-on-qubesos-4-3-going-to-use-wayland-lxqt/22342
- https://github.com/gtk-rs/gtk4-rs
- https://docs.gtk.org/gtk4/section-accessibility.html
- https://github.com/KDAB/cxx-qt
- https://github.com/slint-ui/slint/blob/master/LICENSE.md
- https://github.com/iced-rs/iced/issues/552
- https://github.com/emilk/egui/pull/8577
- https://www.fltk.org/doc-1.4/unicode.html
- https://v2.tauri.app/start/prerequisites/
- https://www.debian.org/releases/trixie/release-notes/issues.en.html

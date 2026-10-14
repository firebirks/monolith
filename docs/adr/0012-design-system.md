# ADR 0012: The design system of the desktop client

Status: decided for Phase 6.
Date: 2026-10-14.

## Context

egui draws every widget in its own default style, which is recognisable
and built for tools. The brief of Phase 6 asks for a client that a person
who knows egui would not recognise as one, and that looks deliberate and
calm rather than technical. Visual consistency cannot be left to each
screen: an immediate mode GUI makes it easy to call a stock widget, or to
write a colour or a size inline, anywhere.

## Decision

`monolith-ui-kit` holds the whole visual language. Screens use it and
nothing else.

### Tokens

One `Theme` value carries every visual constant, and both palettes are
built from the same structure:

- colours by role: backgrounds (app, sidebar, surface, sunken, raised),
  borders, text (primary, secondary, tertiary, disabled, on accent), one
  accent with its hover, pressed and subtle forms, the two message
  bubbles, and the status colours (success, warning, danger);
- typography: Inter for text (Display, Title, Heading, Body, Body strong,
  Caption, Micro) and JetBrains Mono for technical identifiers only;
- spacing on a 4-pixel scale, corner radii, stroke widths, three shadows,
  icon sizes, panel sizes, and the durations of motion, which collapse to
  zero when reduced motion is chosen.

The light and the dark palette are designed together; the window follows
the system's choice unless the user picks one.

### Components

Buttons, icon buttons, text fields, the composer, the search field,
avatars, rows, bubbles, message status, badges, pills, toasts, modals,
menus, the sidebar and the top bar, section headers, empty states, inline
errors, the Tor status, the identity switcher, invitation rows, request
cards, settings rows, fingerprints, dividers, tooltips and the scroll
treatment are widgets of `monolith-ui-kit`, built from egui's painter,
responses and layouts. Each defines its normal, hovered, pressed, focused,
selected, disabled and error states from the theme, and reports its role,
label and whether it is enabled to AccessKit. The focus ring shows when
the focus was moved from the keyboard (Tab, Shift+Tab, or a screen
reader), not after a click; a text field also marks its focus with its
border. A control that takes no input gives no hover, no press and no
pointing hand.

egui's own style is set from the theme as well (selection, scroll bars,
text cursor, tooltips, popups), so that what egui still draws inside a
component, such as the text cursor of a text field, matches.

### Assets

Fonts and icons are compiled into the binary:

- Inter 4.1 (SIL Open Font License 1.1), Regular, Medium and SemiBold;
- JetBrains Mono 2.304 (SIL Open Font License 1.1), Regular;
- Lucide 1.57.0 (ISC), the icon font cut down to the 68 icons used;
- Noto Emoji (SIL Open Font License 1.1), monochrome, as the first
  fallback, as distributed with egui's default fonts 0.32.3.

Each comes with its licence in `crates/monolith-ui-kit/assets/`, with its
source and checksum. For scripts none of these covers (CJK, Arabic,
Hebrew and others), the desktop adds fonts the system already has, from a
fixed list of paths: those of Debian packages on Linux (DejaVu Sans, Noto
Sans CJK), and Segoe UI, Microsoft YaHei and Malgun Gothic on Windows
(P6-17), when they exist, are not larger than 32 MiB and can be read. A
glyph that no font has is drawn as a box, so that it is visible.

### Enforcement

- `monolith-ui` contains no stock egui widget (`ui.button`, `ui.label`,
  `ui.checkbox`, `ui.text_edit_singleline`, `egui::Window`, `RichText` and
  the like) and no colour, size or font of its own; every line of text is
  the kit's `Text` or a component. A source scan in its tests
  (`tests/screens.rs`) fails on any of these, and on a text with a run of
  spaces.
- Text meets the contrast of P6-19 in both themes
  (`monolith-ui-kit/tests/contrast.rs`).
- Components are rendered to images by a software renderer
  (`monolith-ui-snapshot`), so that every state can be looked at and
  compared without a GPU; the screens are rendered the same way, in every
  state of `monolith-ui`'s fixtures, light and dark.

## Consequences

- A change of look is a change of the theme or of one component, and it
  reaches every screen.
- The bundled fonts add about 1.8 MB to the binary.
- egui's layout of text still applies (ADR 0010): the design system
  chooses fonts and sizes but cannot add shaping or bidirectional layout.

## Alternatives considered

- egui's built-in style with adjusted colours: still recognisably egui
  (widget shapes, spacing, default fonts).
- A third-party egui theme crate: not the brief's look, and another
  dependency to review.
- Icons drawn as vector paths at run time: needs an SVG parser, or
  hand-made paths that would not stay consistent; an icon font gives one
  style and needs only the font rasteriser egui already has.

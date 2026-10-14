//! The tokens of the design system (`docs/adr/0012-design-system.md`).
//!
//! Every colour, font, size, radius, stroke, shadow and duration a
//! component or a screen uses is a field of [`Theme`]. Components read the
//! theme installed in the context with [`current`]; nothing else writes a
//! visual constant.

use std::sync::Arc;

use egui::{
    Color32, Context, CornerRadius, CursorIcon, FontFamily, FontId, Id, Margin, Shadow, Stroke,
    Style, TextStyle, Vec2,
};

use crate::fonts;

/// Light or dark.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    /// The light palette.
    Light,
    /// The dark palette.
    Dark,
}

/// Colours by role.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    /// Behind everything: the window.
    pub bg_app: Color32,
    /// The sidebar of conversations.
    pub bg_sidebar: Color32,
    /// Content surfaces: the conversation, cards, rows.
    pub bg_surface: Color32,
    /// Inside fields and wells.
    pub bg_sunken: Color32,
    /// Modals, menus, toasts and other things above the surface.
    pub bg_raised: Color32,
    /// The scrim behind a modal.
    pub scrim: Color32,
    /// Laid over a control that the pointer is over.
    pub hover: Color32,
    /// Laid over a control that is pressed.
    pub pressed: Color32,
    /// The background of a selected row.
    pub selected: Color32,
    /// Hairlines between areas.
    pub border_subtle: Color32,
    /// The border of controls.
    pub border: Color32,
    /// The border of a control the pointer is over.
    pub border_strong: Color32,
    /// Main text.
    pub text: Color32,
    /// Supporting text.
    pub text_secondary: Color32,
    /// Hints, timestamps, placeholders.
    pub text_tertiary: Color32,
    /// Text of a disabled control.
    pub text_disabled: Color32,
    /// Text on the accent colour.
    pub text_on_accent: Color32,
    /// The one accent.
    pub accent: Color32,
    /// The accent under the pointer.
    pub accent_hover: Color32,
    /// The accent pressed.
    pub accent_pressed: Color32,
    /// A light wash of the accent, for selected and informative areas.
    pub accent_subtle: Color32,
    /// Accent-coloured text on a surface.
    pub accent_text: Color32,
    /// An outgoing message.
    pub bubble_out: Color32,
    /// Text of an outgoing message.
    pub bubble_out_text: Color32,
    /// Time and status inside an outgoing message.
    pub bubble_out_meta: Color32,
    /// An incoming message.
    pub bubble_in: Color32,
    /// Text of an incoming message.
    pub bubble_in_text: Color32,
    /// Time inside an incoming message.
    pub bubble_in_meta: Color32,
    /// Something done or working.
    pub success: Color32,
    /// A wash of the success colour.
    pub success_subtle: Color32,
    /// Something that needs attention but is not wrong.
    pub warning: Color32,
    /// A wash of the warning colour.
    pub warning_subtle: Color32,
    /// Something wrong, or an action that destroys.
    pub danger: Color32,
    /// The danger colour under the pointer.
    pub danger_hover: Color32,
    /// The danger colour pressed.
    pub danger_pressed: Color32,
    /// A wash of the danger colour.
    pub danger_subtle: Color32,
    /// The ring around the control that has keyboard focus.
    pub focus_ring: Color32,
    /// Selected text.
    pub text_selection: Color32,
    /// The scroll bar handle.
    pub scrollbar: Color32,
    /// Background colours of avatars, chosen by a stable hash.
    pub avatar: [Color32; 8],
    /// Text on an avatar.
    pub avatar_text: Color32,
}

/// One text style.
#[derive(Clone, Debug, PartialEq)]
pub struct TextToken {
    /// Size in points.
    pub size: f32,
    /// The font family (see [`crate::fonts`]).
    pub family: FontFamily,
    /// Height of a line in points.
    pub line_height: f32,
    /// Extra space between characters in points.
    pub letter_spacing: f32,
}

impl TextToken {
    /// The font of this style.
    pub fn font_id(&self) -> FontId {
        FontId::new(self.size, self.family.clone())
    }

    /// A text format of this style in `color`, for layout jobs.
    pub fn format(&self, color: Color32) -> egui::TextFormat {
        egui::TextFormat {
            font_id: self.font_id(),
            color,
            line_height: Some(self.line_height),
            extra_letter_spacing: self.letter_spacing,
            ..Default::default()
        }
    }
}

/// The text styles.
#[derive(Clone, Debug, PartialEq)]
pub struct Typography {
    /// The largest heading, on onboarding and empty screens.
    pub display: TextToken,
    /// The title of a view or a modal.
    pub title: TextToken,
    /// A heading inside a view.
    pub heading: TextToken,
    /// Running text, messages included.
    pub body: TextToken,
    /// Running text with weight: names, button labels.
    pub body_strong: TextToken,
    /// Supporting text.
    pub caption: TextToken,
    /// Supporting text with weight: section headers, badges.
    pub caption_strong: TextToken,
    /// The smallest text: timestamps in bubbles, counters.
    pub micro: TextToken,
    /// Fingerprints and other technical identifiers.
    pub mono: TextToken,
}

/// The spacing scale, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spacing {
    /// 2.
    pub xxs: f32,
    /// 4.
    pub xs: f32,
    /// 8.
    pub sm: f32,
    /// 12.
    pub md: f32,
    /// 16.
    pub lg: f32,
    /// 20.
    pub xl: f32,
    /// 24.
    pub xxl: f32,
    /// 32.
    pub xxxl: f32,
    /// 48.
    pub huge: f32,
}

/// Corner radii.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Radii {
    /// Badges, small marks.
    pub xs: u8,
    /// Small controls.
    pub sm: u8,
    /// Buttons and fields.
    pub md: u8,
    /// Cards, rows, menus.
    pub lg: u8,
    /// Modals and large surfaces.
    pub xl: u8,
    /// Message bubbles.
    pub bubble: u8,
    /// The corner of a bubble next to its sender's side.
    pub bubble_tail: u8,
    /// Fully rounded ends.
    pub pill: u8,
}

/// Stroke widths in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strokes {
    /// Hairlines and borders.
    pub hairline: f32,
    /// The focus ring.
    pub focus: f32,
    /// Space between a control and its focus ring.
    pub focus_gap: f32,
}

/// Shadows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadows {
    /// Under a small raised element.
    pub sm: Shadow,
    /// Under menus and toasts.
    pub md: Shadow,
    /// Under modals.
    pub lg: Shadow,
}

/// Durations of motion in seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    /// Hover and press feedback.
    pub fast: f32,
    /// Selection, a status that changes, a toast.
    pub normal: f32,
    /// A modal coming in.
    pub slow: f32,
    /// Reduced motion: every duration is zero.
    pub reduced: bool,
}

impl Motion {
    /// `seconds`, or zero with reduced motion.
    pub fn duration(&self, seconds: f32) -> f32 {
        if self.reduced { 0.0 } else { seconds }
    }
}

/// Sizes of controls, icons, avatars and panels, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sizes {
    /// Height of a small control.
    pub control_sm: f32,
    /// Height of a control.
    pub control: f32,
    /// Height of a large control.
    pub control_lg: f32,
    /// A small icon.
    pub icon_sm: f32,
    /// An icon in a control.
    pub icon: f32,
    /// An icon in a toolbar.
    pub icon_md: f32,
    /// A large icon, in empty states.
    pub icon_lg: f32,
    /// A small avatar.
    pub avatar_sm: f32,
    /// An avatar in a row.
    pub avatar: f32,
    /// An avatar in a header.
    pub avatar_lg: f32,
    /// An avatar in a details view.
    pub avatar_xl: f32,
    /// Narrowest sidebar.
    pub sidebar_min: f32,
    /// Default sidebar.
    pub sidebar: f32,
    /// Widest sidebar.
    pub sidebar_max: f32,
    /// Height of the top bar.
    pub topbar: f32,
    /// Height of a conversation row.
    pub row: f32,
    /// Smallest height of the composer.
    pub composer_min: f32,
    /// Largest height of the composer.
    pub composer_max: f32,
    /// Widest message bubble.
    pub bubble_max: f32,
    /// Width of a modal.
    pub modal: f32,
    /// Width of a wide modal.
    pub modal_wide: f32,
    /// Width of a toast.
    pub toast: f32,
    /// Side of a QR code in a modal.
    pub qr: f32,
    /// Smallest window.
    pub window_min: Vec2,
    /// Window at first start.
    pub window_default: Vec2,
    /// Below this width the window shows one pane at a time.
    pub narrow_below: f32,
}

/// Every visual constant.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// Light or dark.
    pub mode: Mode,
    /// Colours.
    pub color: Palette,
    /// Text styles.
    pub text: Typography,
    /// Spacing.
    pub space: Spacing,
    /// Corner radii.
    pub radius: Radii,
    /// Stroke widths.
    pub stroke: Strokes,
    /// Shadows.
    pub shadow: Shadows,
    /// Motion.
    pub motion: Motion,
    /// Sizes.
    pub size: Sizes,
}

const fn rgb(hex: u32) -> Color32 {
    let [_, r, g, b] = hex.to_be_bytes();
    Color32::from_rgb(r, g, b)
}

/// `hex` at `alpha` (0 to 255), premultiplied.
fn rgba(hex: u32, alpha: u8) -> Color32 {
    let [_, r, g, b] = hex.to_be_bytes();
    Color32::from_rgba_unmultiplied(r, g, b, alpha)
}

fn text_token(size: f32, family: &str, line_height: f32) -> TextToken {
    TextToken {
        size,
        family: FontFamily::Name(family.into()),
        line_height,
        letter_spacing: 0.0,
    }
}

fn typography() -> Typography {
    Typography {
        display: text_token(26.0, fonts::SEMIBOLD, 32.0),
        title: text_token(19.0, fonts::SEMIBOLD, 26.0),
        heading: text_token(15.0, fonts::SEMIBOLD, 21.0),
        body: text_token(14.0, fonts::REGULAR, 20.0),
        body_strong: text_token(14.0, fonts::MEDIUM, 20.0),
        caption: text_token(12.5, fonts::REGULAR, 17.0),
        caption_strong: text_token(12.5, fonts::MEDIUM, 17.0),
        micro: text_token(11.0, fonts::REGULAR, 14.0),
        mono: text_token(13.0, fonts::MONO, 19.0),
    }
}

fn shared(mode: Mode, color: Palette) -> Theme {
    let dark = mode == Mode::Dark;
    let shadow = |y: i8, blur: u8, alpha: u8| Shadow {
        offset: [0, y],
        blur,
        spread: 0,
        color: Color32::from_black_alpha(if dark { alpha.saturating_mul(2) } else { alpha }),
    };
    Theme {
        mode,
        color,
        text: typography(),
        space: Spacing {
            xxs: 2.0,
            xs: 4.0,
            sm: 8.0,
            md: 12.0,
            lg: 16.0,
            xl: 20.0,
            xxl: 24.0,
            xxxl: 32.0,
            huge: 48.0,
        },
        radius: Radii {
            xs: 4,
            sm: 6,
            md: 8,
            lg: 12,
            xl: 16,
            bubble: 16,
            bubble_tail: 5,
            pill: 255,
        },
        stroke: Strokes {
            hairline: 1.0,
            focus: 2.0,
            focus_gap: 2.0,
        },
        shadow: Shadows {
            sm: shadow(1, 3, 14),
            md: shadow(6, 20, 26),
            lg: shadow(16, 40, 38),
        },
        motion: Motion {
            fast: 0.10,
            normal: 0.18,
            slow: 0.24,
            reduced: false,
        },
        size: Sizes {
            control_sm: 28.0,
            control: 36.0,
            control_lg: 44.0,
            icon_sm: 14.0,
            icon: 16.0,
            icon_md: 18.0,
            icon_lg: 28.0,
            avatar_sm: 28.0,
            avatar: 40.0,
            avatar_lg: 44.0,
            avatar_xl: 72.0,
            sidebar_min: 260.0,
            sidebar: 320.0,
            sidebar_max: 420.0,
            topbar: 56.0,
            row: 64.0,
            composer_min: 44.0,
            composer_max: 180.0,
            bubble_max: 560.0,
            modal: 440.0,
            modal_wide: 560.0,
            toast: 360.0,
            qr: 204.0,
            window_min: Vec2::new(760.0, 520.0),
            window_default: Vec2::new(1180.0, 760.0),
            narrow_below: 880.0,
        },
    }
}

impl Theme {
    /// The light theme.
    pub fn light() -> Self {
        shared(
            Mode::Light,
            Palette {
                bg_app: rgb(0xF4F4F1),
                bg_sidebar: rgb(0xEEEEEA),
                bg_surface: rgb(0xFFFFFF),
                bg_sunken: rgb(0xF2F2EF),
                bg_raised: rgb(0xFFFFFF),
                scrim: rgba(0x14151A, 92),
                hover: rgba(0x1B1C22, 10),
                pressed: rgba(0x1B1C22, 20),
                selected: rgb(0xE6E9FA),
                border_subtle: rgb(0xE5E4DF),
                border: rgb(0xD6D5CF),
                border_strong: rgb(0xB9B8B1),
                text: rgb(0x1B1C21),
                text_secondary: rgb(0x55565E),
                text_tertiary: rgb(0x85868E),
                text_disabled: rgb(0xAEAFB5),
                text_on_accent: rgb(0xFFFFFF),
                accent: rgb(0x4A5BD4),
                accent_hover: rgb(0x3F50C4),
                accent_pressed: rgb(0x3544AE),
                accent_subtle: rgb(0xEBEEFC),
                accent_text: rgb(0x3D4EC2),
                bubble_out: rgb(0x4A5BD4),
                bubble_out_text: rgb(0xFFFFFF),
                bubble_out_meta: rgba(0xFFFFFF, 190),
                bubble_in: rgb(0xF0F0EC),
                bubble_in_text: rgb(0x1B1C21),
                bubble_in_meta: rgb(0x7E7F87),
                success: rgb(0x2E7D52),
                success_subtle: rgb(0xE5F3EA),
                warning: rgb(0xA8650A),
                warning_subtle: rgb(0xFBF0DC),
                danger: rgb(0xC03A2B),
                danger_hover: rgb(0xAA3122),
                danger_pressed: rgb(0x932A1C),
                danger_subtle: rgb(0xFBE9E6),
                focus_ring: rgba(0x4A5BD4, 150),
                text_selection: rgba(0x4A5BD4, 70),
                scrollbar: rgba(0x1B1C22, 70),
                avatar: [
                    rgb(0x5B6CC9),
                    rgb(0x3F8F7E),
                    rgb(0xB0684A),
                    rgb(0x8A5BB8),
                    rgb(0x4C86B5),
                    rgb(0x9C7B32),
                    rgb(0xB5577A),
                    rgb(0x5E8A44),
                ],
                avatar_text: rgb(0xFFFFFF),
            },
        )
    }

    /// The dark theme.
    pub fn dark() -> Self {
        shared(
            Mode::Dark,
            Palette {
                bg_app: rgb(0x111215),
                bg_sidebar: rgb(0x16171B),
                bg_surface: rgb(0x1B1C20),
                bg_sunken: rgb(0x232429),
                bg_raised: rgb(0x25262B),
                scrim: rgba(0x000000, 150),
                hover: rgba(0xFFFFFF, 12),
                pressed: rgba(0xFFFFFF, 22),
                selected: rgb(0x262B48),
                border_subtle: rgb(0x26272C),
                border: rgb(0x33343A),
                border_strong: rgb(0x4A4B53),
                text: rgb(0xEDEDF0),
                text_secondary: rgb(0xA6A7AF),
                text_tertiary: rgb(0x75767E),
                text_disabled: rgb(0x55565D),
                text_on_accent: rgb(0xFFFFFF),
                accent: rgb(0x6F7FF0),
                accent_hover: rgb(0x8190F3),
                accent_pressed: rgb(0x5E6EE0),
                accent_subtle: rgb(0x23284A),
                accent_text: rgb(0x9AA6F7),
                bubble_out: rgb(0x4655C8),
                bubble_out_text: rgb(0xFFFFFF),
                bubble_out_meta: rgba(0xFFFFFF, 180),
                bubble_in: rgb(0x26272C),
                bubble_in_text: rgb(0xEDEDF0),
                bubble_in_meta: rgb(0x8A8B93),
                success: rgb(0x52C189),
                success_subtle: rgb(0x173325),
                warning: rgb(0xE3A445),
                warning_subtle: rgb(0x3A2B12),
                danger: rgb(0xEC6A5B),
                danger_hover: rgb(0xF07C6F),
                danger_pressed: rgb(0xD9594A),
                danger_subtle: rgb(0x3D1D19),
                focus_ring: rgba(0x8190F3, 170),
                text_selection: rgba(0x6F7FF0, 90),
                scrollbar: rgba(0xFFFFFF, 70),
                avatar: [
                    rgb(0x5363C2),
                    rgb(0x36806F),
                    rgb(0xA05C40),
                    rgb(0x7B51A6),
                    rgb(0x437AA6),
                    rgb(0x8C6E2C),
                    rgb(0xA34E6D),
                    rgb(0x54793D),
                ],
                avatar_text: rgb(0xFFFFFF),
            },
        )
    }

    /// This theme with reduced motion on or off.
    pub fn with_reduced_motion(mut self, reduced: bool) -> Self {
        self.motion.reduced = reduced;
        self
    }

    /// The egui style that matches the theme, so that what egui itself
    /// draws (the text cursor, selections, scroll bars, tooltips) belongs
    /// to the same design.
    pub fn egui_style(&self) -> Style {
        let c = &self.color;
        let mut style = Style {
            text_styles: [
                (TextStyle::Small, self.text.caption.font_id()),
                (TextStyle::Body, self.text.body.font_id()),
                (TextStyle::Button, self.text.body_strong.font_id()),
                (TextStyle::Heading, self.text.title.font_id()),
                (TextStyle::Monospace, self.text.mono.font_id()),
            ]
            .into(),
            animation_time: self.motion.duration(self.motion.normal),
            explanation_tooltips: false,
            url_in_tooltip: false,
            ..Style::default()
        };
        style.interaction.tooltip_delay = 0.45;
        style.interaction.show_tooltips_only_when_still = true;
        style.interaction.selectable_labels = true;

        let spacing = &mut style.spacing;
        spacing.item_spacing = Vec2::new(self.space.sm, self.space.sm);
        spacing.button_padding = Vec2::new(self.space.md, self.space.xs);
        spacing.interact_size = Vec2::new(self.size.control, self.size.control_sm);
        spacing.window_margin = Margin::same(16);
        spacing.menu_margin = Margin::same(6);
        spacing.tooltip_width = 280.0;
        spacing.scroll.floating = true;
        spacing.scroll.bar_width = 8.0;
        spacing.scroll.floating_width = 2.0;
        spacing.scroll.floating_allocated_width = 0.0;
        spacing.scroll.handle_min_length = 32.0;
        spacing.scroll.bar_inner_margin = 2.0;
        spacing.scroll.bar_outer_margin = 2.0;
        spacing.scroll.foreground_color = true;
        spacing.scroll.dormant_background_opacity = 0.0;
        spacing.scroll.active_background_opacity = 0.0;
        spacing.scroll.interact_background_opacity = 0.0;
        spacing.scroll.dormant_handle_opacity = 0.0;
        spacing.scroll.active_handle_opacity = 0.6;
        spacing.scroll.interact_handle_opacity = 1.0;

        let visuals = &mut style.visuals;
        visuals.dark_mode = self.mode == Mode::Dark;
        visuals.override_text_color = None;
        visuals.panel_fill = c.bg_app;
        visuals.window_fill = c.bg_raised;
        visuals.window_stroke = Stroke::new(self.stroke.hairline, c.border_subtle);
        visuals.window_corner_radius = CornerRadius::same(self.radius.xl);
        visuals.window_shadow = self.shadow.lg;
        visuals.window_highlight_topmost = false;
        visuals.popup_shadow = self.shadow.md;
        visuals.menu_corner_radius = CornerRadius::same(self.radius.lg);
        visuals.faint_bg_color = c.bg_sunken;
        visuals.extreme_bg_color = c.bg_sunken;
        visuals.text_edit_bg_color = Some(c.bg_sunken);
        visuals.code_bg_color = c.bg_sunken;
        visuals.hyperlink_color = c.text;
        visuals.warn_fg_color = c.warning;
        visuals.error_fg_color = c.danger;
        visuals.selection.bg_fill = c.text_selection;
        visuals.selection.stroke = Stroke::new(self.stroke.hairline, c.accent);
        visuals.text_cursor.stroke = Stroke::new(2.0, c.accent);
        visuals.text_cursor.blink = true;
        visuals.text_cursor.on_duration = 0.6;
        visuals.text_cursor.off_duration = 0.4;
        visuals.text_cursor.preview = false;
        visuals.interact_cursor = Some(CursorIcon::PointingHand);
        visuals.button_frame = true;
        visuals.striped = false;
        visuals.disabled_alpha = 0.5;
        visuals.clip_rect_margin = 3.0;
        visuals.image_loading_spinners = false;

        let radius = CornerRadius::same(self.radius.md);
        let widget = |bg: Color32, border: Color32, fg: Color32| egui::style::WidgetVisuals {
            bg_fill: bg,
            weak_bg_fill: bg,
            bg_stroke: Stroke::new(self.stroke.hairline, border),
            corner_radius: radius,
            fg_stroke: Stroke::new(self.stroke.hairline, fg),
            expansion: 0.0,
        };
        visuals.widgets.noninteractive = widget(c.bg_surface, c.border_subtle, c.text_secondary);
        visuals.widgets.inactive = widget(c.bg_sunken, c.border, c.text);
        visuals.widgets.hovered = widget(c.bg_sunken, c.border_strong, c.text);
        visuals.widgets.active = widget(c.bg_sunken, c.accent, c.text);
        visuals.widgets.open = widget(c.bg_sunken, c.accent, c.text);
        style
    }
}

fn theme_id() -> Id {
    Id::new("monolith-ui-kit/theme")
}

/// Installs `theme` in `ctx`: egui's style follows it, and components read
/// it with [`current`]. Fonts are installed separately, once
/// ([`crate::fonts::install`]).
pub fn install(ctx: &Context, theme: Theme) {
    ctx.set_style(theme.egui_style());
    ctx.data_mut(|data| data.insert_temp(theme_id(), Arc::new(theme)));
}

/// The installed theme, or the light theme if none was installed.
pub fn current(ctx: &Context) -> Arc<Theme> {
    ctx.data(|data| data.get_temp::<Arc<Theme>>(theme_id()))
        .unwrap_or_else(|| Arc::new(Theme::light()))
}

//! Context menus in the style of the design system: a raised card with
//! full-width rows. The popup machinery is egui's (placement, closing on
//! a click or Escape); the frame and the rows are ours.

use egui::{
    Align2, Color32, Context, CornerRadius, Frame, InnerResponse, Popup, PopupCloseBehavior,
    RectAlign, Response, Sense, Stroke, StrokeKind, Ui, Vec2, Widget, WidgetType,
};

use crate::icons::Icon;
use crate::interact::{self, State, mix, over};
use crate::theme::{self, Theme};

/// The frame of a menu: raised surface, hairline, large radius, shadow and
/// the menu margin of the style.
pub fn menu_frame(ctx: &Context) -> Frame {
    let theme = theme::current(ctx);
    Frame::new()
        .fill(theme.color.bg_raised)
        .stroke(Stroke::new(
            theme.stroke.hairline,
            theme.color.border_subtle,
        ))
        .corner_radius(CornerRadius::same(theme.radius.lg))
        .shadow(theme.shadow.md)
        .inner_margin(ctx.style().spacing.menu_margin)
}

/// The narrowest a menu is.
fn min_width(theme: &Theme) -> f32 {
    theme.size.modal / 2.0
}

/// Fills a popup with `add_items`: the minimum width, and a small gap
/// between rows.
fn contents<R>(ui: &mut Ui, add_items: impl FnOnce(&mut Ui) -> R) -> R {
    let theme = theme::current(ui.ctx());
    ui.set_min_width(min_width(&theme));
    ui.spacing_mut().item_spacing = Vec2::new(0.0, theme.space.xxs);
    add_items(ui)
}

/// Opens a menu at the pointer when `response` is clicked with the
/// secondary button (or long-pressed), and shows `add_items` in it while
/// it is open. A click on a row closes it; so do Escape and a click
/// elsewhere. Returns `None` while it is closed.
pub fn context_menu<R>(
    response: &Response,
    add_items: impl FnOnce(&mut Ui) -> R,
) -> Option<InnerResponse<R>> {
    Popup::context_menu(response)
        .frame(menu_frame(&response.ctx))
        .show(|ui| contents(ui, add_items))
}

/// Opens a menu under `response` when it is clicked (a "more" button), its
/// right edge aligned with the button's, and shows `add_items` in it while
/// it is open. Another click on the button closes it. Returns `None` while
/// it is closed.
pub fn menu_below<R>(
    response: &Response,
    add_items: impl FnOnce(&mut Ui) -> R,
) -> Option<InnerResponse<R>> {
    let theme = theme::current(&response.ctx);
    Popup::menu(response)
        .align(RectAlign::BOTTOM_END)
        .align_alternatives(&[
            RectAlign::BOTTOM_START,
            RectAlign::TOP_END,
            RectAlign::TOP_START,
        ])
        .close_behavior(PopupCloseBehavior::CloseOnClick)
        .frame(menu_frame(&response.ctx))
        .gap(theme.space.xs)
        .show(|ui| contents(ui, add_items))
}

/// A hairline between groups of rows.
pub fn menu_separator(ui: &mut Ui) {
    let theme = theme::current(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(
        // The justified layout of a menu gives it the width of the menu.
        Vec2::new(1.0, theme.space.xs * 2.0 + theme.stroke.hairline),
        Sense::hover(),
    );
    if ui.is_rect_visible(rect) {
        ui.painter().hline(
            rect.x_range(),
            rect.center().y,
            Stroke::new(theme.stroke.hairline, theme.color.border_subtle),
        );
    }
}

/// One row of a menu. Add it with `ui.add(...)`; the response is clicked
/// when the user chose it.
#[derive(Clone, Debug)]
#[must_use = "add it with `ui.add(...)`"]
pub struct MenuItem<'a> {
    icon: Option<Icon>,
    label: &'a str,
    danger: bool,
    enabled: bool,
}

impl<'a> MenuItem<'a> {
    /// A row that reads `label`.
    pub fn new(label: &'a str) -> Self {
        Self {
            icon: None,
            label,
            danger: false,
            enabled: true,
        }
    }

    /// A leading icon.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Draws the row as destructive.
    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }

    /// Whether the row accepts input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

impl Widget for MenuItem<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let icon_size = theme.size.icon;
        let pad = theme.space.md - theme.space.xs;
        let icon_slot = icon_size + theme.space.sm + theme.space.xs;
        let galley = ui.painter().layout_no_wrap(
            self.label.to_owned(),
            theme.text.body.font_id(),
            Color32::PLACEHOLDER,
        );
        let content = galley.size().x + if self.icon.is_some() { icon_slot } else { 0.0 };
        let height = theme.size.control_sm + theme.space.xs;
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(content + pad * 2.0, height), sense);
        interact::describe(&response, WidgetType::Button, self.label);
        if ui.is_rect_visible(rect) {
            let state = State::of(ui, &response, response.id, &theme, false);
            let state = State {
                enabled: self.enabled,
                ..state
            };
            // Keyboard focus reads like hover, with the ring on top.
            let level = if state.focused { 1.0 } else { state.hover };
            let (wash, pressed) = if self.danger {
                (c.danger_subtle, mix(c.danger_subtle, c.danger, 0.15))
            } else {
                (c.hover, c.pressed)
            };
            let fill = if !state.enabled {
                c.bg_raised
            } else if state.pressed {
                over(c.bg_raised, pressed)
            } else {
                over(c.bg_raised, wash.gamma_multiply(level))
            };
            let (text, icon_color) = if !state.enabled {
                (c.text_disabled, c.text_disabled)
            } else if self.danger {
                (c.danger, c.danger)
            } else {
                (c.text, mix(c.text_secondary, c.text, state.hover))
            };
            let radius = CornerRadius::same(theme.radius.sm);
            let painter = ui.painter();
            painter.rect(rect, radius, fill, Stroke::NONE, StrokeKind::Inside);
            let mut x = rect.left() + pad;
            if let Some(icon) = self.icon {
                icon.paint(
                    painter,
                    egui::pos2(x + icon_size / 2.0, rect.center().y),
                    icon_size,
                    icon_color,
                );
                x += icon_slot;
            }
            let pos = Align2::LEFT_CENTER
                .anchor_size(egui::pos2(x, rect.center().y), galley.size())
                .min;
            painter.galley(pos, galley, text);
            if state.focused {
                interact::focus_ring(painter, rect, radius, &theme);
            }
        }
        response
    }
}

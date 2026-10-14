//! The modal dialog: a scrim over the whole window that takes every click,
//! and a centred card with a title, a body and a row of actions.
//!
//! The caller owns whether the modal is open: it calls [`Modal::show`] in
//! every frame in which the modal is open, and stops when it is closed. The
//! modal never closes itself; it reports that the user asked
//! ([`ModalOutput::close_requested`]) and the caller decides.

use egui::{
    Align, Area, Context, CornerRadius, Frame, Id, Key, Label, Layout, Modifiers, Order, Popup,
    RichText, Sense, Stroke, Ui, Vec2, emath::TSTransform,
};

use crate::button::{Button, Size};
use crate::theme;

/// What came of showing a modal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModalOutput<R> {
    /// What the body closure returned.
    pub inner: R,
    /// The user asked to close it (Escape or a click on the scrim) and the
    /// modal is dismissable. Always false for a modal that is not.
    pub close_requested: bool,
}

/// The answer to a confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// The user confirmed the action.
    Confirm,
    /// The user cancelled, or asked to close the dialog.
    Cancel,
}

/// A modal dialog.
#[derive(Clone, Debug)]
#[must_use = "show it with `.show(ctx, ...)`"]
pub struct Modal<'a> {
    id: Id,
    title: &'a str,
    subtitle: Option<&'a str>,
    wide: bool,
    dismissable: bool,
}

/// How small the card is when it starts to appear.
const APPEAR_SCALE: f32 = 0.96;

impl<'a> Modal<'a> {
    /// A modal titled `title`. Its egui ids derive from `id_salt`.
    pub fn new(id_salt: impl core::hash::Hash, title: &'a str) -> Self {
        Self {
            id: Id::new(("monolith-modal", id_salt)),
            title,
            subtitle: None,
            wide: false,
            dismissable: true,
        }
    }

    /// A line of explanation under the title.
    pub fn subtitle(mut self, subtitle: &'a str) -> Self {
        self.subtitle = Some(subtitle);
        self
    }

    /// The wide card.
    pub fn wide(mut self, wide: bool) -> Self {
        self.wide = wide;
        self
    }

    /// Whether Escape and a click on the scrim ask to close it. Turn it
    /// off while an operation runs, and for a question that must be
    /// answered explicitly.
    pub fn dismissable(mut self, dismissable: bool) -> Self {
        self.dismissable = dismissable;
        self
    }

    fn card_id(&self) -> Id {
        self.id.with("card")
    }

    /// Whether the modal was not shown in the previous frame.
    fn is_first_frame(&self, ctx: &Context) -> bool {
        ctx.read_response(self.card_id()).is_none()
    }

    /// Shows the modal over the window. `body` draws the content; `footer`
    /// draws the actions in a right-to-left row, so the first widget it
    /// adds, which should be the main action, is the rightmost.
    pub fn show<R>(
        self,
        ctx: &Context,
        body: impl FnOnce(&mut Ui) -> R,
        footer: impl FnOnce(&mut Ui),
    ) -> ModalOutput<R> {
        let theme = theme::current(ctx);
        let screen = ctx.screen_rect();
        let first = self.is_first_frame(ctx);
        let appear_id = self.id.with("appear");
        if first {
            // Starts from nothing, whatever an earlier opening left.
            ctx.animate_bool_with_time(appear_id, false, 0.0);
        }
        let appear = ctx
            .animate_bool_with_time(appear_id, true, theme.motion.duration(theme.motion.slow))
            .clamp(0.0, 1.0);

        // The scrim takes every click, so that nothing behind can be used.
        let scrim_area = Area::new(self.id.with("scrim"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .interactable(true)
            .movable(false);
        // Everything below this layer is out of reach, keyboard included.
        ctx.memory_mut(|memory| memory.set_modal_layer(scrim_area.layer()));
        let scrim = scrim_area
            .show(ctx, |ui| {
                let (rect, response) =
                    ui.allocate_exact_size(screen.size(), Sense::click_and_drag());
                ui.painter()
                    .rect_filled(rect, 0.0, theme.color.scrim.gamma_multiply(appear));
                response
            })
            .inner;

        let card_width = if self.wide {
            theme.size.modal_wide
        } else {
            theme.size.modal
        }
        .min(screen.width() - theme.space.lg * 2.0)
        .max(theme.size.control);
        let padding = theme.space.xxl;
        let card_layer = Area::new(self.card_id()).layer();
        let title = self.title;
        let subtitle = self.subtitle;
        let card = Area::new(self.card_id())
            .order(Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
            .interactable(true)
            .movable(false)
            // Clicks on the card are the card's, not the scrim's.
            .sense(Sense::click_and_drag())
            .show(ctx, |ui| {
                ui.set_opacity(appear);
                Frame::new()
                    .fill(theme.color.bg_raised)
                    .stroke(Stroke::new(
                        theme.stroke.hairline,
                        theme.color.border_subtle,
                    ))
                    .corner_radius(CornerRadius::same(theme.radius.xl))
                    .shadow(theme.shadow.lg)
                    .inner_margin(padding)
                    .show(ui, |ui| {
                        ui.set_width(card_width - padding * 2.0);
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        ui.add(
                            Label::new(
                                RichText::new(title)
                                    .font(theme.text.title.font_id())
                                    .line_height(Some(theme.text.title.line_height))
                                    .color(theme.color.text),
                            )
                            .selectable(false)
                            .wrap(),
                        );
                        if let Some(subtitle) = subtitle {
                            ui.add_space(theme.space.xs);
                            ui.add(
                                Label::new(
                                    RichText::new(subtitle)
                                        .font(theme.text.body.font_id())
                                        .line_height(Some(theme.text.body.line_height))
                                        .color(theme.color.text_secondary),
                                )
                                .selectable(false)
                                .wrap(),
                            );
                        }
                        ui.add_space(theme.space.xl);
                        ui.spacing_mut().item_spacing = Vec2::new(theme.space.sm, theme.space.sm);
                        let inner = body(ui);
                        ui.add_space(theme.space.xl);
                        ui.allocate_ui_with_layout(
                            Vec2::new(ui.available_width(), theme.size.control),
                            Layout::right_to_left(Align::Center),
                            footer,
                        );
                        inner
                    })
                    .inner
            });

        // A press on the scrim raises its area; the card goes back above.
        if scrim.is_pointer_button_down_on() || scrim.clicked() {
            ctx.move_to_top(card_layer);
        }
        if appear < 1.0 {
            let rect = card.response.rect;
            let scale = APPEAR_SCALE + (1.0 - APPEAR_SCALE) * appear;
            let center = rect.center().to_vec2();
            ctx.transform_layer_shapes(card_layer, TSTransform::new(center * (1.0 - scale), scale));
        }

        let mut close_requested = false;
        if self.dismissable {
            close_requested = scrim.clicked();
            // Escape belongs to an open menu first.
            if !Popup::is_any_open(ctx)
                && ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
            {
                close_requested = true;
            }
        }
        ModalOutput {
            inner: card.inner,
            close_requested,
        }
    }

    /// Shows a confirmation: the title, `message`, a secondary button that
    /// reads `cancel` and a destructive one that reads `confirm`. Returns
    /// the answer once there is one. The cancel button has the focus when
    /// the dialog opens, so that Enter does not destroy anything. While
    /// `busy`, the destructive button shows progress and nothing can be
    /// answered.
    pub fn confirm(
        self,
        ctx: &Context,
        message: &str,
        cancel: &str,
        confirm: &str,
        busy: bool,
    ) -> Option<Decision> {
        let theme = theme::current(ctx);
        let first = self.is_first_frame(ctx);
        let mut decision = None;
        let output = self.dismissable(!busy).show(
            ctx,
            |ui| {
                ui.add(
                    Label::new(
                        RichText::new(message)
                            .font(theme.text.body.font_id())
                            .line_height(Some(theme.text.body.line_height))
                            .color(theme.color.text_secondary),
                    )
                    .selectable(false)
                    .wrap(),
                );
            },
            |ui| {
                let confirmed = ui
                    .add(
                        Button::danger(confirm)
                            .size(Size::Medium)
                            .busy(busy)
                            .min_width(theme.size.control_lg),
                    )
                    .clicked();
                let cancelled = ui.add(Button::secondary(cancel).enabled(!busy));
                if first {
                    cancelled.request_focus();
                }
                if !busy {
                    if confirmed {
                        decision = Some(Decision::Confirm);
                    } else if cancelled.clicked() {
                        decision = Some(Decision::Cancel);
                    }
                }
            },
        );
        if decision.is_none() && output.close_requested {
            decision = Some(Decision::Cancel);
        }
        decision
    }
}

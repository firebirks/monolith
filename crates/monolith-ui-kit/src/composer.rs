//! The composer: the message box at the bottom of a conversation. It
//! grows with the draft up to a limit, then scrolls; Enter asks to send and
//! Shift+Enter starts a new line. It never clears the draft: the caller does
//! that once the runtime has accepted the message.

use std::hash::Hash;

use egui::{
    Align, Align2, Color32, CornerRadius, Frame, Id, Key, KeyboardShortcut, Layout, Margin,
    Modifiers, Rect, Response, ScrollArea, Stroke, TextEdit, Ui, Vec2, pos2,
};

use crate::chrome::ScrollView;
use crate::icon_button::IconButton;
use crate::icons;
use crate::interact::{self, mix};
use crate::theme;

/// What came of showing the composer.
#[derive(Debug)]
pub struct ComposerOutput {
    /// The response of the editing area.
    pub response: Response,
    /// The user asked to send the draft (Enter or the send button) and the
    /// draft can be sent: it has text, is within the limit, the
    /// conversation accepts messages and no send is under way. The caller
    /// clears the draft only after the runtime accepted the message.
    pub send_requested: bool,
    /// The draft changed in this frame.
    pub changed: bool,
}

/// A message composer editing a draft.
#[must_use = "show it with `.show(ui)`"]
pub struct Composer<'a> {
    id: Id,
    text: &'a mut String,
    max_bytes: usize,
    sendable: bool,
    blocked_reason: Option<&'a str>,
    busy: bool,
    request_focus: bool,
}

impl<'a> Composer<'a> {
    /// A composer editing `text`, whose egui id derives from `id_salt`
    /// (use the conversation, so that each has its own scroll and cursor).
    pub fn new(id_salt: impl Hash, text: &'a mut String) -> Self {
        Self {
            id: Id::new(("monolith-composer", id_salt)),
            text,
            max_bytes: usize::MAX,
            sendable: true,
            blocked_reason: None,
            busy: false,
            request_focus: false,
        }
    }

    /// The most bytes a message holds. A counter shows above 85% of it.
    pub fn max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// Whether the conversation accepts a message now.
    pub fn sendable(mut self, sendable: bool) -> Self {
        self.sendable = sendable;
        self
    }

    /// Why it does not, such as "This contact is blocked". With a reason
    /// the draft cannot be edited either; without one it can.
    pub fn blocked_reason(mut self, reason: Option<&'a str>) -> Self {
        self.blocked_reason = reason;
        self
    }

    /// Whether a send is under way: the button shows progress and no
    /// other send is requested.
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    /// Takes keyboard focus when shown.
    pub fn request_focus(mut self, request: bool) -> Self {
        self.request_focus = request;
        self
    }

    /// The egui id of the editing area.
    pub fn edit_id(&self) -> Id {
        self.id.with("edit")
    }

    /// Shows the composer across the available width.
    pub fn show(self, ui: &mut Ui) -> ComposerOutput {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let edit_id = self.edit_id();
        let locked = !self.sendable && self.blocked_reason.is_some();
        let focused = !locked && ui.memory(|memory| memory.has_focus(edit_id));

        let used = self.text.len();
        let over_limit = used > self.max_bytes;
        let near_limit = used.saturating_mul(100) > self.max_bytes.saturating_mul(85);
        let can_send = self.sendable && !self.busy && !over_limit && !self.text.trim().is_empty();

        // Plain Enter asks to send. The edit only treats Shift+Enter as a
        // new line, so it leaves plain Enter alone.
        let enter = focused
            && ui.input_mut(|input| {
                !input.modifiers.shift && input.consume_key(Modifiers::NONE, Key::Enter)
            });

        let mut send_clicked = false;
        let mut edited: Option<Response> = None;
        let mut changed = false;

        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(0.0, theme.space.xs);

            if let (false, Some(reason)) = (self.sendable, self.blocked_reason) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = theme.space.sm;
                    let (icon_rect, _) = ui
                        .allocate_exact_size(Vec2::splat(theme.size.icon_sm), egui::Sense::hover());
                    icons::INFO.paint(
                        ui.painter(),
                        icon_rect.center(),
                        theme.size.icon_sm,
                        c.text_tertiary,
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(reason)
                                .font(theme.text.caption.font_id())
                                .color(c.text_secondary),
                        )
                        .selectable(false),
                    );
                });
            }

            let pad_v = (theme.size.composer_min - theme.size.control) / 2.0;
            let radius = CornerRadius::same(theme.radius.lg);
            let step = |points: f32| Margin::from(points).left;
            let margin = Margin {
                left: step(theme.space.md),
                right: step(theme.space.xs),
                top: step(pad_v),
                bottom: step(pad_v),
            };
            // The stroke is given now so that the content area leaves room for
            // it; its colour is set once the state is known.
            let mut prepared = Frame::new()
                .stroke(Stroke::new(theme.stroke.hairline, Color32::TRANSPARENT))
                .corner_radius(radius)
                .inner_margin(margin)
                .begin(ui);
            let inner = &mut prepared.content_ui;
            inner.set_width(inner.available_width());
            inner.set_min_height(theme.size.control);
            let button_gap = theme.space.sm;
            let edit_w =
                (inner.available_width() - theme.size.control - button_gap).max(theme.size.control);
            let max_edit_h = theme.size.composer_max - pad_v * 2.0;
            let edit_area = inner.allocate_ui_with_layout(
                Vec2::new(edit_w, max_edit_h),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(edit_w);
                    // No stop of the Tab key but the edit: the area neither
                    // drags nor takes the focus with its bar.
                    ScrollView::new(self.id.with("scroll"), ScrollArea::vertical())
                        .max_height(max_edit_h)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            let hint = egui::RichText::new("Write a message")
                                .font(theme.text.body.font_id())
                                .color(c.text_tertiary);
                            let edit = TextEdit::multiline(&mut *self.text)
                                .id(edit_id)
                                .frame(false)
                                .margin(Margin::from(Vec2::new(0.0, theme.space.sm)))
                                .font(theme.text.body.font_id())
                                .text_color(if locked { c.text_disabled } else { c.text })
                                .hint_text(hint)
                                .interactive(!locked)
                                .desired_rows(1)
                                .desired_width(edit_w)
                                .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter));
                            let response = edit.show(ui).response;
                            changed = response.changed();
                            edited = Some(response);
                            // No undo history: once a message is sent and the
                            // draft cleared, Ctrl+Z must not bring it back, and
                            // message text stays out of egui's memory.
                            if let Some(mut state) = TextEdit::load_state(ui.ctx(), edit_id) {
                                state.clear_undoer();
                                TextEdit::store_state(ui.ctx(), edit_id, state);
                            }
                        });
                },
            );
            // The send button stays at the bottom edge as the draft grows.
            let row = edit_area.response.rect;
            let side = theme.size.control;
            let slot = Rect::from_min_size(
                pos2(inner.max_rect().right() - side, row.bottom() - side),
                Vec2::splat(side),
            );
            let send = IconButton::filled(icons::SEND, "Send message")
                .enabled(can_send || self.busy)
                .busy(self.busy)
                .id_salt((self.id, "send"));
            send_clicked = inner.put(slot, send).clicked();

            let content_rect = prepared.content_ui.min_rect();
            let outer = content_rect + margin;
            let hovered = !locked && ui.rect_contains_pointer(outer);
            let hover = ui.ctx().animate_bool_with_time(
                self.id.with("hover"),
                hovered,
                theme.motion.duration(theme.motion.fast),
            );
            // The composer has the focus most of the time: a quiet border
            // says so, and the ring shows only when Tab brought it here.
            let border = if over_limit {
                c.danger
            } else if focused {
                c.border_strong
            } else {
                mix(c.border, c.border_strong, hover)
            };
            prepared.frame.fill = if locked { c.bg_app } else { c.bg_sunken };
            prepared.frame.stroke = Stroke::new(theme.stroke.hairline, border);
            let frame_response = prepared.end(ui);
            if focused {
                interact::focus_ring(ui.painter(), frame_response.rect, radius, &theme);
            }

            if near_limit {
                let (rect, _) = ui.allocate_exact_size(
                    Vec2::new(ui.available_width(), theme.text.micro.line_height),
                    egui::Sense::hover(),
                );
                let galley = ui.painter().layout_no_wrap(
                    format!("{used} / {}", self.max_bytes),
                    theme.text.micro.font_id(),
                    Color32::PLACEHOLDER,
                );
                let pos = Align2::RIGHT_CENTER
                    .anchor_size(
                        pos2(rect.right() - theme.space.xs, rect.center().y),
                        galley.size(),
                    )
                    .min;
                let ink = if over_limit {
                    c.danger
                } else {
                    c.text_tertiary
                };
                ui.painter().galley(pos, galley, ink);
            }
        });

        let response =
            edited.unwrap_or_else(|| ui.allocate_response(Vec2::ZERO, egui::Sense::hover()));
        if locked {
            ui.ctx()
                .accesskit_node_builder(edit_id, |node| node.set_disabled());
        }
        if self.request_focus && !focused && !locked {
            response.request_focus();
        }
        if send_clicked && !locked {
            ui.memory_mut(|memory| memory.request_focus(edit_id));
        }
        ComposerOutput {
            send_requested: (enter || send_clicked) && can_send,
            changed,
            response,
        }
    }
}

/// Removes everything egui keeps for the composer `edit_id` (its cursor
/// and selection). Call when the messages of its conversation are deleted
/// or the conversation goes away.
pub fn forget(ctx: &egui::Context, edit_id: Id) {
    ctx.data_mut(|data| data.remove::<egui::text_edit::TextEditState>(edit_id));
}

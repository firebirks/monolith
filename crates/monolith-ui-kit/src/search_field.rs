//! The search field: a single line with a leading search icon, a clear
//! button while it has text, and Escape to clear it. egui's `TextEdit` does
//! the editing, without its own frame.

use std::hash::Hash;

use egui::{
    CornerRadius, Id, Key, Margin, Modifiers, Rect, Response, Sense, Stroke, StrokeKind, TextEdit,
    Ui, Vec2, pos2,
};

use crate::icon_button::IconButton;
use crate::icons;
use crate::interact::{self, mix};
use crate::theme;

/// What came of showing a search field.
#[derive(Debug)]
pub struct Output {
    /// The response of the editing area.
    pub response: Response,
    /// The text changed in this frame (typing, clearing).
    pub changed: bool,
}

/// A single-line search field.
#[must_use = "show it with `.show(ui)`"]
pub struct SearchField<'a> {
    id: Id,
    text: &'a mut String,
    hint: &'a str,
    compact: bool,
    enabled: bool,
    width: Option<f32>,
    request_focus: bool,
}

impl<'a> SearchField<'a> {
    /// A field editing `text`, whose egui id derives from `id_salt`.
    pub fn new(id_salt: impl Hash, text: &'a mut String) -> Self {
        Self {
            id: Id::new(("monolith-search-field", id_salt)),
            text,
            hint: "Search",
            compact: false,
            enabled: true,
            width: None,
            request_focus: false,
        }
    }

    /// Shown while the field is empty; also its accessible name.
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = hint;
        self
    }

    /// The compact height, for the sidebar.
    pub fn compact(mut self, compact: bool) -> Self {
        self.compact = compact;
        self
    }

    /// Whether it accepts input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// This wide; the available width otherwise.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
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

    /// Shows the field.
    pub fn show(self, ui: &mut Ui) -> Output {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let edit_id = self.edit_id();
        let width = self.width.unwrap_or_else(|| ui.available_width());
        let height = if self.compact {
            theme.size.control_sm + theme.space.xs
        } else {
            theme.size.control
        };
        let focused = self.enabled && ui.memory(|memory| memory.has_focus(edit_id));
        let mut changed = false;

        // Escape empties a field that has text; on an empty one it passes
        // through, so that a view can close.
        if focused
            && !self.text.is_empty()
            && ui.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape))
        {
            self.text.clear();
            changed = true;
        }

        let (rect, frame) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
        if self.enabled && frame.clicked() {
            ui.memory_mut(|memory| memory.request_focus(edit_id));
        }
        let hovered = self.enabled && frame.contains_pointer();
        let hover = ui.ctx().animate_bool_with_time(
            self.id.with("hover"),
            hovered,
            theme.motion.duration(theme.motion.fast),
        );
        let radius = CornerRadius::same(theme.radius.md);
        let border = if focused {
            c.accent
        } else {
            mix(c.border, c.border_strong, hover)
        };
        let fill = if self.enabled { c.bg_sunken } else { c.bg_app };
        ui.painter().rect(
            rect,
            radius,
            fill,
            Stroke::new(theme.stroke.hairline, border),
            StrokeKind::Inside,
        );
        if focused {
            interact::focus_ring(ui.painter(), rect, radius, &theme);
        }

        let icon_size = theme.size.icon;
        let left = rect.left() + theme.space.md;
        ui.painter().text(
            pos2(left + icon_size / 2.0, rect.center().y),
            egui::Align2::CENTER_CENTER,
            icons::SEARCH.char(),
            crate::icons::Icon::font(icon_size),
            if !self.enabled {
                c.text_disabled
            } else if focused {
                c.text_secondary
            } else {
                c.text_tertiary
            },
        );

        // The slot of the clear button is always reserved, so that the text
        // does not shift when the first character is typed.
        let side = theme.size.control_sm - theme.space.xs;
        let slot_margin = (height - side) / 2.0;
        let slot = Rect::from_min_size(
            pos2(
                rect.right() - slot_margin - side,
                rect.center().y - side / 2.0,
            ),
            Vec2::splat(side),
        );
        let text_rect = Rect::from_min_max(
            pos2(left + icon_size + theme.space.sm, rect.top()),
            pos2(slot.left() - theme.space.xs, rect.bottom()),
        );

        let font = theme.text.body.font_id();
        let hint = egui::RichText::new(self.hint)
            .font(font.clone())
            .color(if self.enabled {
                c.text_tertiary
            } else {
                c.text_disabled
            });
        let edit = TextEdit::singleline(&mut *self.text)
            .id(edit_id)
            .frame(false)
            .margin(Margin::ZERO)
            .font(font)
            .text_color(if self.enabled {
                c.text
            } else {
                c.text_disabled
            })
            .hint_text(hint)
            .interactive(self.enabled)
            .desired_width(text_rect.width())
            .vertical_align(egui::Align::Center);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(text_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let edited = edit.show(&mut child);
        let response = edited.response;
        changed |= response.changed();
        if self.request_focus && !focused {
            response.request_focus();
        }

        if self.enabled && !self.text.is_empty() {
            let clear = IconButton::new(icons::X, "Clear search")
                .tiny()
                .id_salt((self.id, "clear"));
            if ui.put(slot, clear).clicked() {
                self.text.clear();
                changed = true;
                ui.memory_mut(|memory| memory.request_focus(edit_id));
            }
        }
        Output { response, changed }
    }
}

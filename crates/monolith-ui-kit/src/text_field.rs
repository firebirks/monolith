//! The text field: a label, a framed input, and a line of help or error
//! under it. egui's `TextEdit` does the editing, without its own frame; the
//! frame, the states and the text around it are the design system's.

use egui::{
    Color32, CornerRadius, Id, Key, Margin, Response, Sense, Stroke, StrokeKind, TextBuffer,
    TextEdit, Ui, Vec2,
};

use crate::icons::Icon;
use crate::interact::{self, mix};
use crate::theme;

/// What came of showing a text field.
#[derive(Debug)]
pub struct Output {
    /// The response of the editing area.
    pub response: Response,
    /// The text changed in this frame.
    pub changed: bool,
    /// Enter was pressed while the field had focus.
    pub submitted: bool,
}

/// A single-line text field.
#[must_use = "show it with `.show(ui)`"]
pub struct TextField<'a> {
    id: Id,
    text: &'a mut dyn TextBuffer,
    label: Option<&'a str>,
    hint: &'a str,
    help: Option<&'a str>,
    error: Option<&'a str>,
    icon: Option<Icon>,
    secret: bool,
    enabled: bool,
    char_limit: Option<usize>,
    width: Option<f32>,
    request_focus: bool,
}

impl<'a> TextField<'a> {
    /// A field editing `text`, whose egui id derives from `id_salt`.
    pub fn new(id_salt: impl core::hash::Hash, text: &'a mut dyn TextBuffer) -> Self {
        Self {
            id: Id::new(("monolith-text-field", id_salt)),
            text,
            label: None,
            hint: "",
            help: None,
            error: None,
            icon: None,
            secret: false,
            enabled: true,
            char_limit: None,
            width: None,
            request_focus: false,
        }
    }

    /// A label above the field; also its accessible name.
    pub fn label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Shown while the field is empty.
    pub fn hint(mut self, hint: &'a str) -> Self {
        self.hint = hint;
        self
    }

    /// A line of help under the field, unless there is an error.
    pub fn help(mut self, help: Option<&'a str>) -> Self {
        self.help = help;
        self
    }

    /// An error under the field, which also draws it as wrong.
    pub fn error(mut self, error: Option<&'a str>) -> Self {
        self.error = error;
        self
    }

    /// A leading icon.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// A secret, such as a passphrase: masked, and nothing of it is kept
    /// in egui's memory (no undo history; [`forget`] removes the rest).
    pub fn secret(mut self, secret: bool) -> Self {
        self.secret = secret;
        self
    }

    /// Whether it accepts input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// At most this many characters.
    pub fn char_limit(mut self, limit: usize) -> Self {
        self.char_limit = Some(limit);
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

        let mut output = None;
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(0.0, theme.space.xs + 2.0);
            let mut label_id = None;
            if let Some(label) = self.label {
                let label_response = ui.label(
                    egui::RichText::new(label)
                        .font(theme.text.caption_strong.font_id())
                        .color(if self.enabled {
                            c.text_secondary
                        } else {
                            c.text_disabled
                        }),
                );
                label_id = Some(label_response.id);
            }

            let height = theme.size.control;
            let (rect, frame_response) =
                ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
            let hovered = self.enabled && frame_response.hovered();
            let hover = ui.ctx().animate_bool_with_time(
                self.id.with("hover"),
                hovered,
                theme.motion.duration(theme.motion.fast),
            );
            let focused = ui.memory(|memory| memory.has_focus(edit_id));
            let radius = CornerRadius::same(theme.radius.md);
            let border = if self.error.is_some() {
                c.danger
            } else if focused {
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

            let mut inner = rect.shrink2(Vec2::new(theme.space.md, 0.0));
            if let Some(icon) = self.icon {
                let center = egui::pos2(inner.left() + theme.size.icon / 2.0, inner.center().y);
                icon.paint(
                    ui.painter(),
                    center,
                    theme.size.icon,
                    if focused {
                        c.text_secondary
                    } else {
                        c.text_tertiary
                    },
                );
                inner.min.x += theme.size.icon + theme.space.sm;
            }

            let font = theme.text.body.font_id();
            let text_color = if self.enabled {
                c.text
            } else {
                c.text_disabled
            };
            let hint = egui::RichText::new(self.hint)
                .font(font.clone())
                .color(c.text_tertiary);
            let mut edit = TextEdit::singleline(self.text)
                .id(edit_id)
                .frame(false)
                .margin(Margin::ZERO)
                .font(font)
                .text_color(text_color)
                .hint_text(hint)
                .password(self.secret)
                .interactive(self.enabled)
                .desired_width(inner.width())
                .vertical_align(egui::Align::Center);
            if let Some(limit) = self.char_limit {
                edit = edit.char_limit(limit);
            }
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(inner)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            let edited = edit.show(&mut child);
            let mut response = edited.response;
            if let Some(label_id) = label_id {
                // The label above is the accessible name of the field.
                response = response.labelled_by(label_id);
            }
            if self.request_focus && !focused {
                response.request_focus();
            }
            if self.secret {
                // No copy of the secret stays in an undo history.
                if let Some(mut state) = TextEdit::load_state(ui.ctx(), edit_id) {
                    state.clear_undoer();
                    TextEdit::store_state(ui.ctx(), edit_id, state);
                }
            }
            let submitted =
                response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter));

            let note = self.error.or(self.help);
            if let Some(note) = note {
                let color = if self.error.is_some() {
                    c.danger
                } else {
                    c.text_tertiary
                };
                ui.label(
                    egui::RichText::new(note)
                        .font(theme.text.caption.font_id())
                        .color(color),
                );
            }
            output = Some(Output {
                changed: response.changed(),
                submitted,
                response,
            });
        });
        output.unwrap_or_else(|| Output {
            response: ui.allocate_response(Vec2::ZERO, Sense::hover()),
            changed: false,
            submitted: false,
        })
    }
}

/// Removes everything egui keeps for the text field `edit_id` (its cursor
/// and selection). Call when a dialog with a secret field closes.
pub fn forget(ctx: &egui::Context, edit_id: Id) {
    ctx.data_mut(|data| data.remove::<egui::text_edit::TextEditState>(edit_id));
}

/// The colour of placeholder text, for components that draw their own.
pub fn placeholder_color(ctx: &egui::Context) -> Color32 {
    theme::current(ctx).color.text_tertiary
}

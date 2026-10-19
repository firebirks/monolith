//! Rows of the sidebar and of lists: the conversation row, the contact
//! row, and the single-line and wrapped text that the other components of
//! this group share.

use std::sync::Arc;

use egui::epaint::text::{LayoutJob, TextWrapping};
use egui::{
    Align, Align2, Color32, CornerRadius, Galley, Id, Layout, Rect, Response, Sense, Ui, UiBuilder,
    Vec2, Widget, WidgetType,
};

use crate::avatar::{self, Face, Outline};
use crate::icons::{self, Icon};
use crate::interact::{self, State};
use crate::theme::{self, TextToken, Theme};
use crate::tooltip;

/// Lays out `text` in one line of at most `max_width`, ending in an
/// ellipsis where it does not fit. Control characters (line breaks in a
/// preview, say) become spaces. The galley is in the placeholder colour:
/// paint it with a colour of the state.
pub(crate) fn single_line(ui: &Ui, text: &str, token: &TextToken, max_width: f32) -> Arc<Galley> {
    let clean: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut job = LayoutJob::single_section(clean, token.format(Color32::PLACEHOLDER));
    job.wrap = TextWrapping {
        max_width: max_width.max(1.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('\u{2026}'),
    };
    ui.painter().layout_job(job)
}

/// Lays out `text` in `color` wrapped at `max_width`, breaking inside
/// words when a word does not fit, with at most `max_rows` rows and an
/// ellipsis after the last. `color` can be the placeholder colour.
pub(crate) fn wrapped(
    ui: &Ui,
    text: &str,
    token: &TextToken,
    color: Color32,
    max_width: f32,
    max_rows: usize,
) -> Arc<Galley> {
    let mut job = LayoutJob::single_section(text.to_owned(), token.format(color));
    job.wrap = TextWrapping {
        max_width: max_width.max(1.0),
        max_rows,
        break_anywhere: false,
        overflow_character: Some('\u{2026}'),
    };
    ui.painter().layout_job(job)
}

/// The parts that are not empty, joined for a screen reader.
pub(crate) fn join_label(parts: &[&str]) -> String {
    parts
        .iter()
        .copied()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The colours of primary, secondary and tertiary text in a state.
fn state_color(theme: &Theme, state: State) -> (Color32, Color32, Color32) {
    let c = &theme.color;
    if state.enabled {
        (c.text, c.text_secondary, c.text_tertiary)
    } else {
        (c.text_disabled, c.text_disabled, c.text_disabled)
    }
}

/// The background of a row in its state: the selection, then the tint of
/// the pointer or the press.
fn paint_row_background(ui: &Ui, theme: &Theme, rect: Rect, state: State) {
    let c = &theme.color;
    let radius = CornerRadius::same(theme.radius.lg);
    let painter = ui.painter();
    if state.selected {
        painter.rect_filled(rect, radius, c.selected);
    }
    if state.enabled {
        let tint = if state.pressed {
            c.pressed
        } else {
            c.hover.gamma_multiply(state.hover)
        };
        if tint.a() > 0 {
            painter.rect_filled(rect, radius, tint);
        }
    }
}

/// A small marker on the preview line of a conversation: a glyph, which
/// says its word under the pointer and to a screen reader.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowStatus {
    /// The other side has not accepted yet.
    WaitingForAcceptance,
    /// The other side has to add the card of the local identity: the card
    /// added for them was not an invitation.
    WaitingForTheirCard,
    /// Nothing was written yet, so no request has left.
    RequestNotSent,
    /// The conversation ended: the contact was removed or blocked. One
    /// mark for both, which does not say which.
    Ended,
    /// A message waits for the network.
    Queued,
}

impl RowStatus {
    fn icon(self) -> Icon {
        match self {
            Self::WaitingForAcceptance | Self::WaitingForTheirCard | Self::RequestNotSent => {
                icons::HOURGLASS
            }
            Self::Ended => icons::CIRCLE_DASHED,
            Self::Queued => icons::CLOCK,
        }
    }

    /// The marker in words, as a screen reader reads it in the row.
    pub fn label(self) -> &'static str {
        match self {
            Self::WaitingForAcceptance => "waiting for acceptance",
            Self::WaitingForTheirCard => "waiting for them to add your card",
            Self::RequestNotSent => "request not sent yet",
            Self::Ended => "no longer a contact",
            Self::Queued => "messages queued",
        }
    }

    /// The marker in words as its tooltip says them: a capital first.
    fn words(self) -> String {
        let label = self.label();
        let mut chars = label.chars();
        chars.next().map_or_else(String::new, |first| {
            first.to_uppercase().chain(chars).collect()
        })
    }
}

/// A conversation in the sidebar.
#[derive(Clone, Debug)]
#[must_use = "add it with `ui.add(...)`"]
pub struct ConversationRow<'a> {
    id: Id,
    name: &'a str,
    seed: &'a [u8],
    preview: &'a str,
    outgoing_prefix: Option<&'a str>,
    timestamp: &'a str,
    unread: u32,
    statuses: &'a [RowStatus],
    selected: bool,
    enabled: bool,
    anonymous: bool,
}

impl<'a> ConversationRow<'a> {
    /// The egui id of the row whose key is `id_salt`, for callers that
    /// move focus to it.
    pub fn id_for(id_salt: impl core::hash::Hash) -> Id {
        Id::new(("monolith-conversation-row", id_salt))
    }

    /// A row for the contact called `name`. `id_salt` is the caller's key
    /// of the conversation, never a list position; `seed` picks the
    /// colour of the avatar.
    pub fn new(id_salt: impl core::hash::Hash, name: &'a str, seed: &'a [u8]) -> Self {
        Self {
            id: Self::id_for(id_salt),
            name,
            seed,
            preview: "",
            outgoing_prefix: None,
            timestamp: "",
            unread: 0,
            statuses: &[],
            selected: false,
            enabled: true,
            anonymous: false,
        }
    }

    /// The avatar shows a person glyph instead of initials, for a contact
    /// with no name of its own (one shown as "Contact" and a fingerprint
    /// group).
    pub fn anonymous(mut self, anonymous: bool) -> Self {
        self.anonymous = anonymous;
        self
    }

    /// The last message, one line.
    pub fn preview(mut self, preview: &'a str) -> Self {
        self.preview = preview;
        self
    }

    /// Marks the preview as a message of the user, with the caller's
    /// prefix (for example "You: ").
    pub fn outgoing(mut self, prefix: &'a str) -> Self {
        self.outgoing_prefix = Some(prefix);
        self
    }

    /// The time of the last message, already formatted.
    pub fn timestamp(mut self, timestamp: &'a str) -> Self {
        self.timestamp = timestamp;
        self
    }

    /// The number of unread messages; zero shows nothing.
    pub fn unread(mut self, unread: u32) -> Self {
        self.unread = unread;
        self
    }

    /// Markers after the preview.
    pub fn statuses(mut self, statuses: &'a [RowStatus]) -> Self {
        self.statuses = statuses;
        self
    }

    /// Whether this is the open conversation.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Whether it accepts input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

impl Widget for ConversationRow<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let width = ui.available_width();
        let (_, outer) = ui.allocate_space(Vec2::new(width, theme.size.row));
        // Rows float inside the sidebar: inset from its edges, and apart
        // from each other.
        let rect = outer.shrink2(Vec2::new(theme.space.sm, theme.space.xxs));
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let response = ui.interact(rect, self.id, sense);

        let unread_text = self.unread.to_string();
        let unread_label = if self.unread > 0 {
            unread_text.as_str()
        } else {
            ""
        };
        let preview_text = match self.outgoing_prefix {
            Some(prefix) => format!("{prefix}{}", self.preview),
            None => self.preview.to_owned(),
        };
        // The markers are glyphs on screen; a screen reader reads them. What
        // the client says comes first, and the preview, which can be what a
        // contact wrote, last and named as a message, so that it cannot pass
        // for a status (P7-23).
        let message = if preview_text.is_empty() {
            String::new()
        } else {
            format!("last message: {preview_text}")
        };
        let mut parts = vec![self.name];
        parts.extend(self.statuses.iter().map(|status| status.label()));
        parts.extend([self.timestamp, unread_label, message.as_str()]);
        let label = join_label(&parts);
        interact::describe_selectable_enabled(
            &response,
            WidgetType::SelectableLabel,
            &label,
            self.selected,
            self.enabled,
        );

        if !ui.is_rect_visible(rect) {
            return response;
        }
        let state = State {
            enabled: self.enabled,
            ..State::of(ui, &response, response.id, &theme, self.selected)
        };
        paint_row_background(ui, &theme, rect, state);
        let (primary, secondary, tertiary) = state_color(&theme, state);

        let inner = rect.shrink2(Vec2::new(theme.space.md, 0.0));
        let avatar_edge = theme.size.avatar;
        let avatar_rect = Rect::from_center_size(
            egui::pos2(inner.left() + avatar_edge / 2.0, inner.center().y),
            Vec2::splat(avatar_edge),
        );
        avatar::paint(
            ui.painter(),
            &theme,
            avatar_rect,
            Face {
                name: self.name,
                seed: self.seed,
                outline: Outline::Circle,
                verified: false,
                dim: !state.enabled,
                anonymous: self.anonymous,
            },
        );

        let text_left = avatar_rect.right() + theme.space.md;
        let text_right = inner.right();
        let gap = theme.space.sm;

        // Name line: the name, and the time at the right.
        let stamp = (!self.timestamp.is_empty()).then(|| {
            ui.painter().layout_no_wrap(
                self.timestamp.to_owned(),
                theme.text.micro.font_id(),
                Color32::PLACEHOLDER,
            )
        });
        let stamp_width = stamp.as_ref().map_or(0.0, |g| g.size().x + gap);
        let name_galley = single_line(
            ui,
            self.name,
            &theme.text.body_strong,
            text_right - text_left - stamp_width,
        );

        // Preview line: markers and the unread count at the right.
        let preview_galley_height = theme.text.caption.line_height;
        let block_height = name_galley.size().y + preview_galley_height + 1.0;
        let top = rect.center().y - block_height / 2.0;
        let name_pos = egui::pos2(text_left, top);
        let preview_center_y = top + name_galley.size().y + 1.0 + preview_galley_height / 2.0;

        let mut right = text_right;
        let mut have_right = false;
        if self.unread > 0 {
            let shown = if self.unread > 99 {
                "99+".to_owned()
            } else {
                unread_text.clone()
            };
            let text = ui.painter().layout_no_wrap(
                shown,
                theme.text.caption_strong.font_id(),
                Color32::PLACEHOLDER,
            );
            let pill_height = 18.0;
            let pill_width = (text.size().x + theme.space.sm * 1.5).max(pill_height);
            let pill = Rect::from_center_size(
                egui::pos2(right - pill_width / 2.0, preview_center_y),
                Vec2::new(pill_width, pill_height),
            );
            let fill = if state.enabled {
                c.accent
            } else {
                c.text_disabled
            };
            ui.painter()
                .rect_filled(pill, CornerRadius::same(theme.radius.pill), fill);
            let text_pos = Align2::CENTER_CENTER
                .anchor_size(pill.center(), text.size())
                .min;
            ui.painter().galley(text_pos, text, c.text_on_accent);
            right -= pill_width;
            have_right = true;
        }
        for status in self.statuses.iter().rev() {
            if have_right {
                right -= theme.space.xs + 1.0;
            }
            let size = theme.size.icon_sm;
            let center = egui::pos2(right - size / 2.0, preview_center_y);
            status.icon().paint(ui.painter(), center, size, tertiary);
            // The glyph says its word under the pointer; the row says it to
            // a screen reader.
            let area = Rect::from_center_size(center, Vec2::splat(size));
            let mark = ui.interact(
                area,
                self.id.with(("status", status.label())),
                Sense::hover(),
            );
            tooltip::show(&mark, &status.words());
            right -= size;
            have_right = true;
        }
        let preview_right = if have_right { right - gap } else { right };
        let preview_galley = single_line(
            ui,
            &preview_text,
            &theme.text.caption,
            preview_right - text_left,
        );
        let preview_pos = egui::pos2(text_left, preview_center_y - preview_galley.size().y / 2.0);

        let painter = ui.painter();
        painter.galley(name_pos, name_galley.clone(), primary);
        if let Some(stamp) = stamp {
            // The small text sits on the baseline of the larger one.
            let pos = egui::pos2(
                text_right - stamp.size().x,
                name_pos.y + (name_galley.size().y - stamp.size().y) / 2.0 + 1.0,
            );
            painter.galley(pos, stamp, tertiary);
        }
        painter.galley(preview_pos, preview_galley, secondary);
        if state.focused {
            interact::focus_ring(painter, rect, CornerRadius::same(theme.radius.lg), &theme);
        }
        response
    }
}

/// The second line of a contact row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Secondary<'a> {
    /// A sentence, such as a status.
    Text(&'a str),
    /// A technical identifier, such as a short fingerprint.
    Mono(&'a str),
}

/// Content added at the right edge of a row.
pub(crate) type Trailing<'a> = Box<dyn FnOnce(&mut Ui) + 'a>;

/// A contact in a list, with room for a pill or an action at the right.
#[must_use = "add it with `ui.add(...)`"]
pub struct ContactRow<'a> {
    id: Id,
    name: &'a str,
    seed: &'a [u8],
    secondary: Option<Secondary<'a>>,
    selected: bool,
    enabled: bool,
    verified: bool,
    anonymous: bool,
    trailing: Option<Trailing<'a>>,
}

impl<'a> ContactRow<'a> {
    /// The egui id of the row whose key is `id_salt`.
    pub fn id_for(id_salt: impl core::hash::Hash) -> Id {
        Id::new(("monolith-contact-row", id_salt))
    }

    /// A row for the contact called `name`; `id_salt` is the caller's key
    /// of the contact.
    pub fn new(id_salt: impl core::hash::Hash, name: &'a str, seed: &'a [u8]) -> Self {
        Self {
            id: Self::id_for(id_salt),
            name,
            seed,
            secondary: None,
            selected: false,
            enabled: true,
            verified: false,
            anonymous: false,
            trailing: None,
        }
    }

    /// The avatar shows a person glyph instead of initials, for a contact
    /// with no name of its own.
    pub fn anonymous(mut self, anonymous: bool) -> Self {
        self.anonymous = anonymous;
        self
    }

    /// The line under the name.
    pub fn secondary(mut self, secondary: Secondary<'a>) -> Self {
        self.secondary = Some(secondary);
        self
    }

    /// Whether it is the selected entry.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Whether it accepts input.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Shows the verified mark on the avatar.
    pub fn verified(mut self, verified: bool) -> Self {
        self.verified = verified;
        self
    }

    /// What goes at the right edge, laid out right to left and centred.
    pub fn trailing(mut self, add: impl FnOnce(&mut Ui) + 'a) -> Self {
        self.trailing = Some(Box::new(add));
        self
    }
}

impl Widget for ContactRow<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let width = ui.available_width();
        let height = theme.size.avatar + theme.space.md;
        let (_, outer) = ui.allocate_space(Vec2::new(width, height));
        let rect = outer.shrink2(Vec2::new(theme.space.sm, 0.0));
        let sense = if self.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let response = ui.interact(rect, self.id, sense);
        let second = match self.secondary {
            Some(Secondary::Text(text) | Secondary::Mono(text)) => text,
            None => "",
        };
        let label = join_label(&[self.name, second]);
        interact::describe_selectable_enabled(
            &response,
            WidgetType::SelectableLabel,
            &label,
            self.selected,
            self.enabled,
        );

        if !ui.is_rect_visible(rect) {
            return response;
        }
        let state = State {
            enabled: self.enabled,
            ..State::of(ui, &response, response.id, &theme, self.selected)
        };
        paint_row_background(ui, &theme, rect, state);
        let (primary, secondary_color, _) = state_color(&theme, state);

        let inner = rect.shrink2(Vec2::new(theme.space.md, 0.0));
        let avatar_edge = theme.size.avatar;
        let avatar_rect = Rect::from_center_size(
            egui::pos2(inner.left() + avatar_edge / 2.0, inner.center().y),
            Vec2::splat(avatar_edge),
        );
        avatar::paint(
            ui.painter(),
            &theme,
            avatar_rect,
            Face {
                name: self.name,
                seed: self.seed,
                outline: Outline::Circle,
                verified: self.verified,
                dim: !state.enabled,
                anonymous: self.anonymous,
            },
        );
        let text_left = avatar_rect.right() + theme.space.md;

        // The trailing slot first: its width limits the text.
        let mut text_right = inner.right();
        if let Some(add) = self.trailing {
            let slot = Rect::from_min_max(
                egui::pos2(text_left + inner.width() * 0.3, rect.top()),
                egui::pos2(inner.right(), rect.bottom()),
            );
            let mut child = ui.new_child(
                UiBuilder::new()
                    .id_salt(("monolith-contact-row-trailing", self.id))
                    .max_rect(slot)
                    .layout(Layout::right_to_left(Align::Center)),
            );
            add(&mut child);
            let used = child.min_rect();
            if used.width() > 0.0 {
                text_right = used.left() - theme.space.sm;
            }
        }

        let available = text_right - text_left;
        let name_galley = single_line(ui, self.name, &theme.text.body_strong, available);
        let second_token = match self.secondary {
            Some(Secondary::Mono(_)) => Some(TextToken {
                size: theme.text.caption.size,
                line_height: theme.text.caption.line_height,
                ..theme.text.mono.clone()
            }),
            Some(Secondary::Text(_)) => Some(theme.text.caption.clone()),
            None => None,
        };
        let second_galley = second_token
            .as_ref()
            .map(|token| single_line(ui, second, token, available));
        let second_height = second_galley.as_ref().map_or(0.0, |g| g.size().y + 1.0);
        let top = rect.center().y - (name_galley.size().y + second_height) / 2.0;
        let painter = ui.painter();
        let name_height = name_galley.size().y;
        painter.galley(egui::pos2(text_left, top), name_galley, primary);
        if let Some(galley) = second_galley {
            painter.galley(
                egui::pos2(text_left, top + name_height + 1.0),
                galley,
                secondary_color,
            );
        }
        if state.focused {
            interact::focus_ring(painter, rect, CornerRadius::same(theme.radius.lg), &theme);
        }
        response
    }
}

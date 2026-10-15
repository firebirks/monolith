//! A row of the list of invitations: a ticket icon, the label of the
//! invitation, a caption, and the actions to show its card and to revoke
//! it. The row has no background and no horizontal padding of its own; it
//! sits in a card or a settings group that provides both.

use egui::{Align, Id, Layout, Rect, Sense, Ui, UiBuilder, Vec2, WidgetType};

use crate::button::{Button, Size};
use crate::icons;
use crate::interact;
use crate::rows::single_line;
use crate::theme;

/// What the user did with an invitation row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Output {
    /// "Show" was clicked.
    pub show: bool,
    /// "Revoke" was clicked.
    pub revoke: bool,
}

/// An invitation the user has made.
#[derive(Clone, Debug)]
#[must_use = "show it with `.show(ui)`"]
pub struct InvitationRow<'a> {
    id: Id,
    label: &'a str,
    fallback: &'a str,
    caption: &'a str,
    show_label: &'a str,
    revoke_label: &'a str,
    busy: bool,
}

impl<'a> InvitationRow<'a> {
    /// A row for the invitation whose key is `id_salt`. `label` is what the
    /// user called it, `fallback` what to show when that is empty.
    pub fn new(
        id_salt: impl core::hash::Hash,
        label: &'a str,
        fallback: &'a str,
        show_label: &'a str,
        revoke_label: &'a str,
    ) -> Self {
        Self {
            id: Id::new(("monolith-invitation-row", id_salt)),
            label,
            fallback,
            caption: "",
            show_label,
            revoke_label,
            busy: false,
        }
    }

    /// The line under the label (when it was made, how often it was used).
    pub fn caption(mut self, caption: &'a str) -> Self {
        self.caption = caption;
        self
    }

    /// The invitation is being revoked: no input, and a progress mark.
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    /// Shows the row.
    pub fn show(self, ui: &mut Ui) -> Output {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let height = theme.size.avatar + theme.space.md;
        let (_, rect) = ui.allocate_space(Vec2::new(ui.available_width(), height));
        let label = if self.label.trim().is_empty() {
            self.fallback
        } else {
            self.label
        };

        // The actions first: their width limits the text.
        let mut output = Output::default();
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt(self.id)
                .max_rect(rect)
                .layout(Layout::right_to_left(Align::Center)),
        );
        child.spacing_mut().item_spacing.x = theme.space.xs;
        let revoke = child.add(
            Button::quiet_danger(self.revoke_label)
                .size(Size::Small)
                .enabled(!self.busy)
                .busy(self.busy),
        );
        let show = child.add(
            Button::secondary(self.show_label)
                .size(Size::Small)
                .enabled(!self.busy),
        );
        output.revoke = revoke.clicked();
        output.show = show.clicked();
        let actions_left = child.min_rect().left();

        let edge = theme.size.avatar_sm + 8.0;
        let circle = Rect::from_center_size(
            egui::pos2(rect.left() + edge / 2.0, rect.center().y),
            Vec2::splat(edge),
        );
        let (fill, ink) = if self.busy {
            (c.bg_sunken, c.text_disabled)
        } else {
            (c.accent_subtle, c.accent_text)
        };
        let text_left = circle.right() + theme.space.md;
        let text_width = actions_left - theme.space.md - text_left;
        let name = single_line(ui, label, &theme.text.body_strong, text_width);
        let caption = (!self.caption.is_empty())
            .then(|| single_line(ui, self.caption, &theme.text.caption, text_width));
        let caption_height = caption.as_ref().map_or(0.0, |g| g.size().y);
        let top = rect.center().y - (name.size().y + caption_height) / 2.0;
        let name_height = name.size().y;
        let text_rect = Rect::from_min_max(
            egui::pos2(circle.left(), rect.top()),
            egui::pos2(text_left + text_width, rect.bottom()),
        );
        let info = ui.interact(text_rect, self.id.with("text"), Sense::hover());
        interact::describe(&info, WidgetType::Label, label);
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            painter.circle_filled(circle.center(), edge / 2.0, fill);
            icons::TICKET.paint(painter, circle.center(), theme.size.icon_md, ink);
            let primary = if self.busy { c.text_secondary } else { c.text };
            painter.galley(egui::pos2(text_left, top), name, primary);
            if let Some(galley) = caption {
                painter.galley(
                    egui::pos2(text_left, top + name_height),
                    galley,
                    if self.busy {
                        c.text_disabled
                    } else {
                        c.text_secondary
                    },
                );
            }
        }
        output
    }
}

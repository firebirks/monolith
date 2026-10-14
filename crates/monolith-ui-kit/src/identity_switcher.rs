//! The identity switcher at the top of the sidebar: a trigger that shows
//! the current identity, and a popup that lists the identities with the
//! entries to create a new one and to manage them.

use core::hash::Hash;

use egui::containers::{Popup, PopupCloseBehavior, SetOpenCommand};
use egui::{
    Align2, Context, CornerRadius, Frame, Id, Margin, Rect, Response, Sense, Stroke, StrokeKind,
    Ui, Vec2, WidgetType,
};

use crate::avatar::{self, Face, Outline};
use crate::icons::{self, Icon};
use crate::interact::{self, State, mix, over};
use crate::rows::{join_label, single_line};
use crate::theme::{self, Theme};

/// One identity in the list.
#[derive(Clone, Debug)]
pub struct IdentityItem<'a, K> {
    /// The caller's key of the identity; reported back when it is chosen.
    pub key: K,
    /// The name of the identity.
    pub name: &'a str,
    /// A second line, such as a short fingerprint.
    pub detail: &'a str,
    /// Picks the colour of the avatar.
    pub seed: &'a [u8],
}

/// What came of showing the switcher.
#[derive(Debug)]
pub struct Output<K> {
    /// The response of the trigger.
    pub response: Response,
    /// The key of the identity that was clicked in the list. It can be the
    /// current one.
    pub selected: Option<K>,
    /// "New identity" was clicked.
    pub new_identity: bool,
    /// "Manage identities" was clicked.
    pub manage: bool,
    /// The popup is open.
    pub open: bool,
}

fn trigger_id(id_salt: impl Hash) -> Id {
    Id::new(("monolith-identity-switcher", id_salt))
}

/// Opens or closes the popup of the switcher whose key is `id_salt`.
pub fn set_open(ctx: &Context, id_salt: impl Hash, open: bool) {
    let id = trigger_id(id_salt).with("popup");
    if open {
        Popup::open_id(ctx, id);
    } else {
        Popup::close_id(ctx, id);
    }
}

/// Whether the popup of the switcher whose key is `id_salt` is open.
pub fn is_open(ctx: &Context, id_salt: impl Hash) -> bool {
    Popup::is_id_open(ctx, trigger_id(id_salt).with("popup"))
}

/// The switcher.
#[must_use = "show it with `.show(ui)`"]
pub struct IdentitySwitcher<'a, K> {
    id: Id,
    current: &'a K,
    items: &'a [IdentityItem<'a, K>],
    caption: &'a str,
    new_label: &'a str,
    manage_label: &'a str,
}

impl<'a, K: Hash + PartialEq + Clone> IdentitySwitcher<'a, K> {
    /// A switcher whose current identity is the one with key `current`
    /// among `items`. `id_salt` identifies the switcher; the labels are the
    /// texts of the two entries under the list.
    pub fn new(
        id_salt: impl Hash,
        current: &'a K,
        items: &'a [IdentityItem<'a, K>],
        new_label: &'a str,
        manage_label: &'a str,
    ) -> Self {
        Self {
            id: trigger_id(id_salt),
            current,
            items,
            caption: "",
            new_label,
            manage_label,
        }
    }

    /// The line under the name in the trigger (for example the number of
    /// contacts).
    pub fn caption(mut self, caption: &'a str) -> Self {
        self.caption = caption;
        self
    }

    /// Shows the trigger and, when it is open, the popup.
    pub fn show(self, ui: &mut Ui) -> Output<K> {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let width = ui.available_width();
        let (_, rect) = ui.allocate_space(Vec2::new(width, theme.size.topbar));
        let response = ui.interact(rect, self.id, Sense::click());
        let current = self.items.iter().find(|item| item.key == *self.current);
        let name = current.map_or("", |item| item.name);
        let open = Popup::is_id_open(ui.ctx(), Popup::default_response_id(&response));
        interact::describe(
            &response,
            WidgetType::ComboBox,
            &join_label(&[name, self.caption]),
        );

        if ui.is_rect_visible(rect) {
            let state = State::of(ui, &response, response.id, &theme, open);
            let radius = CornerRadius::same(theme.radius.lg);
            let tint = if state.pressed {
                c.pressed
            } else {
                c.hover.gamma_multiply(state.hover)
            };
            let fill = over(c.bg_surface, tint);
            let border = if open {
                c.border_strong
            } else {
                mix(c.border_subtle, c.border, state.hover)
            };
            ui.painter().rect(
                rect,
                radius,
                fill,
                Stroke::new(theme.stroke.hairline, border),
                StrokeKind::Inside,
            );
            let inner = rect.shrink2(Vec2::new(theme.space.md, 0.0));
            let edge = theme.size.avatar;
            let avatar_rect = Rect::from_center_size(
                egui::pos2(inner.left() + edge / 2.0, inner.center().y),
                Vec2::splat(edge),
            );
            avatar::paint(
                ui.painter(),
                &theme,
                avatar_rect,
                Face {
                    name,
                    seed: current.map_or(&[][..], |item| item.seed),
                    outline: Outline::RoundedSquare,
                    verified: false,
                    dim: false,
                },
            );
            let chevron = if open {
                icons::CHEVRON_UP
            } else {
                icons::CHEVRON_DOWN
            };
            let chevron_size = theme.size.icon;
            chevron.paint(
                ui.painter(),
                egui::pos2(inner.right() - chevron_size / 2.0, inner.center().y),
                chevron_size,
                mix(c.text_tertiary, c.text_secondary, state.hover),
            );
            let text_left = avatar_rect.right() + theme.space.md;
            let text_width = inner.right() - chevron_size - theme.space.sm - text_left;
            let name_galley = single_line(ui, name, &theme.text.body_strong, text_width);
            let caption_galley = (!self.caption.is_empty())
                .then(|| single_line(ui, self.caption, &theme.text.caption, text_width));
            let caption_height = caption_galley.as_ref().map_or(0.0, |g| g.size().y);
            let top = inner.center().y - (name_galley.size().y + caption_height) / 2.0;
            let name_height = name_galley.size().y;
            ui.painter()
                .galley(egui::pos2(text_left, top), name_galley, c.text);
            if let Some(galley) = caption_galley {
                ui.painter().galley(
                    egui::pos2(text_left, top + name_height),
                    galley,
                    c.text_secondary,
                );
            }
            if state.focused {
                interact::focus_ring(ui.painter(), rect, radius, &theme);
            }
        }

        let mut selected = None;
        let mut new_identity = false;
        let mut manage = false;
        let frame = Frame::new()
            .fill(c.bg_raised)
            .stroke(Stroke::new(theme.stroke.hairline, c.border_subtle))
            .corner_radius(CornerRadius::same(theme.radius.lg))
            .shadow(theme.shadow.md)
            .inner_margin(Margin::same(6));
        let popup = Popup::from_response(&response)
            .open_memory(response.clicked().then_some(SetOpenCommand::Toggle))
            .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
            .gap(theme.space.xs)
            .frame(frame)
            .width(rect.width());
        let popup_open_id = Popup::default_response_id(&response);
        let id = self.id;
        popup.show(|ui| {
            ui.set_width(rect.width() - 12.0);
            ui.spacing_mut().item_spacing = Vec2::new(0.0, 2.0);
            for item in self.items {
                let current_item = item.key == *self.current;
                let row_id = Id::new((id, "item", &item.key));
                let clicked = item_row(
                    ui,
                    &theme,
                    row_id,
                    current_item,
                    |ui, rect| {
                        let painter = ui.painter();
                        let edge = theme.size.avatar_sm + 4.0;
                        let avatar_rect = Rect::from_center_size(
                            egui::pos2(rect.left() + theme.space.sm + edge / 2.0, rect.center().y),
                            Vec2::splat(edge),
                        );
                        avatar::paint(
                            painter,
                            &theme,
                            avatar_rect,
                            Face {
                                name: item.name,
                                seed: item.seed,
                                outline: Outline::RoundedSquare,
                                verified: false,
                                dim: false,
                            },
                        );
                        let left = avatar_rect.right() + theme.space.md;
                        let right_edge = rect.right() - theme.space.md - theme.size.icon;
                        let name_galley = single_line(
                            ui,
                            item.name,
                            &theme.text.body_strong,
                            right_edge - left - theme.space.sm,
                        );
                        let detail_galley = (!item.detail.is_empty()).then(|| {
                            single_line(
                                ui,
                                item.detail,
                                &theme.text.caption,
                                right_edge - left - theme.space.sm,
                            )
                        });
                        let detail_height = detail_galley.as_ref().map_or(0.0, |g| g.size().y);
                        let top = rect.center().y - (name_galley.size().y + detail_height) / 2.0;
                        let name_height = name_galley.size().y;
                        painter.galley(egui::pos2(left, top), name_galley, theme.color.text);
                        if let Some(galley) = detail_galley {
                            painter.galley(
                                egui::pos2(left, top + name_height),
                                galley,
                                theme.color.text_secondary,
                            );
                        }
                        if current_item {
                            icons::CHECK.paint(
                                painter,
                                egui::pos2(
                                    rect.right() - theme.space.md - theme.size.icon / 2.0,
                                    rect.center().y,
                                ),
                                theme.size.icon,
                                theme.color.accent_text,
                            );
                        }
                    },
                    join_label(&[item.name, item.detail]),
                );
                if clicked {
                    selected = Some(item.key.clone());
                }
            }
            // A hairline between the identities and the entries below.
            let (_, line) = ui.allocate_space(Vec2::new(ui.available_width(), 9.0));
            ui.painter().hline(
                (line.left() + theme.space.xs)..=(line.right() - theme.space.xs),
                line.center().y,
                Stroke::new(theme.stroke.hairline, c.border_subtle),
            );
            let entries: [(&str, Icon, &str); 2] = [
                (self.new_label, icons::PLUS, "new"),
                (self.manage_label, icons::SETTINGS, "manage"),
            ];
            for (label, icon, key) in entries {
                let row_id = Id::new((id, "entry", key));
                let clicked = entry_row(ui, &theme, row_id, label, icon);
                if clicked {
                    if key == "new" {
                        new_identity = true;
                    } else {
                        manage = true;
                    }
                }
            }
        });
        if selected.is_some() || new_identity || manage {
            Popup::close_id(ui.ctx(), popup_open_id);
        }
        let open = Popup::is_id_open(ui.ctx(), popup_open_id);
        Output {
            response,
            selected,
            new_identity,
            manage,
            open,
        }
    }
}

/// A row of the popup. `paint` draws its content; returns whether it was
/// clicked.
fn item_row(
    ui: &mut Ui,
    theme: &Theme,
    id: Id,
    current: bool,
    paint: impl FnOnce(&Ui, Rect),
    label: String,
) -> bool {
    let height = 48.0;
    let (_, rect) = ui.allocate_space(Vec2::new(ui.available_width(), height));
    let response = ui.interact(rect, id, Sense::click());
    interact::describe_selectable(&response, WidgetType::SelectableLabel, &label, current);
    if ui.is_rect_visible(rect) {
        let state = State::of(ui, &response, id, theme, current);
        let radius = CornerRadius::same(theme.radius.md);
        paint_item_background(ui, theme, rect, state);
        paint(ui, rect);
        if state.focused {
            interact::focus_ring(
                ui.painter(),
                rect.shrink(theme.stroke.focus_gap + 1.0),
                radius,
                theme,
            );
        }
    }
    response.clicked()
}

fn paint_item_background(ui: &Ui, theme: &Theme, rect: Rect, state: State) {
    let c = &theme.color;
    let radius = CornerRadius::same(theme.radius.md);
    let tint = if state.pressed {
        c.pressed
    } else {
        c.hover.gamma_multiply(state.hover)
    };
    if tint.a() > 0 {
        ui.painter().rect_filled(rect, radius, tint);
    }
}

/// An entry under the list: an icon in the column of the avatars, a
/// label.
fn entry_row(ui: &mut Ui, theme: &Theme, id: Id, label: &str, icon: Icon) -> bool {
    let c = &theme.color;
    let (_, rect) = ui.allocate_space(Vec2::new(ui.available_width(), 40.0));
    let response = ui.interact(rect, id, Sense::click());
    interact::describe(&response, WidgetType::Button, label);
    if ui.is_rect_visible(rect) {
        let state = State::of(ui, &response, id, theme, false);
        paint_item_background(ui, theme, rect, state);
        let edge = theme.size.avatar_sm + 4.0;
        let center = egui::pos2(rect.left() + theme.space.sm + edge / 2.0, rect.center().y);
        icon.paint(
            ui.painter(),
            center,
            theme.size.icon_md,
            mix(c.text_secondary, c.text, state.hover),
        );
        let left = center.x + edge / 2.0 + theme.space.md;
        let galley = single_line(
            ui,
            label,
            &theme.text.body,
            rect.right() - left - theme.space.md,
        );
        let pos = Align2::LEFT_CENTER
            .anchor_size(egui::pos2(left, rect.center().y), galley.size())
            .min;
        ui.painter().galley(pos, galley, c.text);
        if state.focused {
            interact::focus_ring(
                ui.painter(),
                rect.shrink(theme.stroke.focus_gap + 1.0),
                CornerRadius::same(theme.radius.md),
                theme,
            );
        }
    }
    response.clicked()
}

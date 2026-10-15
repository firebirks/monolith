//! The frame of a window: the top bar, the sidebar panel, the content
//! panel and the scroll area, so that every screen is laid out the same
//! way.

use egui::{
    Align, Context, CornerRadius, CursorIcon, Frame, Id, InnerResponse, LayerId, Layout, Margin,
    Rect, Response, ScrollArea, Sense, Stroke, Ui, UiBuilder, Vec2,
    containers::scroll_area::{ScrollAreaOutput, ScrollBarVisibility, ScrollSource},
};

use crate::theme;

/// The frame of the content panel: the surface colour, no margin (the
/// screens set their own).
pub fn content_frame(ctx: &Context) -> Frame {
    Frame::new().fill(theme::current(ctx).color.bg_surface)
}

/// The frame of the sidebar: its own background colour and no margin. The
/// hairline on its right edge is drawn by [`SidebarPanel`].
pub fn sidebar_frame(ctx: &Context) -> Frame {
    Frame::new().fill(theme::current(ctx).color.bg_sidebar)
}

/// The frame of a panel under a top bar or above a composer: the surface
/// colour with `margin` points inside.
pub fn surface_frame(ctx: &Context, margin: impl Into<Margin>) -> Frame {
    Frame::new()
        .fill(theme::current(ctx).color.bg_surface)
        .inner_margin(margin)
}

/// A card on a surface: the card colour (as a group of settings rows and
/// a request card), a hairline and the large radius, with `margin` points
/// inside.
pub fn card_frame(ctx: &Context, margin: impl Into<Margin>) -> Frame {
    let theme = theme::current(ctx);
    Frame::new()
        .fill(theme.color.bg_card)
        .stroke(Stroke::new(
            theme.stroke.hairline,
            theme.color.border_subtle,
        ))
        .corner_radius(CornerRadius::same(theme.radius.lg))
        .inner_margin(margin)
}

/// A vertical scroll area that fills its space, with the scroll bar of the
/// theme (thin, floating, faint at rest, clearer while hovered). `id_salt`
/// names its scroll position. It scrolls with the wheel and the bar, not
/// by dragging its content, so that a drag selects text.
pub fn scroll_area(id_salt: impl core::hash::Hash) -> ScrollView {
    let area = ScrollArea::vertical()
        .auto_shrink([false, false])
        .scroll_bar_visibility(ScrollBarVisibility::VisibleWhenNeeded)
        .animated(true);
    ScrollView::new(id_salt, area)
}

/// A vertical scroll area whose parts take no keyboard focus.
///
/// egui makes its scroll bar and the area that drags its content stops of
/// the Tab key that say nothing to a screen reader. Here the content
/// scrolls with the wheel, and the bar is dragged or clicked with the
/// pointer through an interaction of the kit's own, which the keyboard
/// never reaches; the keyboard scrolls by moving the focus.
#[must_use = "show it with `.show(ui, ...)` or `.show_viewport(ui, ...)`"]
pub struct ScrollView {
    area: ScrollArea,
    salt: Id,
    offset: Option<f32>,
}

/// What the bar of a scroll view was in the last frame, for a press or a
/// drag in the next.
#[derive(Clone, Copy, Debug)]
struct Bar {
    /// The top of the view.
    top: f32,
    /// The height of the view.
    view: f32,
    /// The height of the content.
    content: f32,
    /// The scroll position.
    offset: f32,
    /// The shortest the handle gets.
    shortest: f32,
    /// Where the pointer holds the handle, from its top, while it does.
    grab: Option<f32>,
}

impl Bar {
    /// The scroll position at the end of the content.
    fn most(&self) -> f32 {
        (self.content - self.view).max(0.0)
    }

    /// The length of the handle, as egui draws it.
    fn handle(&self) -> f32 {
        (self.view * self.view / self.content.max(1.0))
            .max(self.shortest)
            .min(self.view)
    }

    /// Where the top of the handle is.
    fn handle_top(&self) -> f32 {
        let most = self.most();
        if most > 0.0 {
            self.top + self.offset.clamp(0.0, most) / most * (self.view - self.handle())
        } else {
            self.top
        }
    }

    /// The scroll position with the top of the handle at `top`.
    fn offset_at(&self, top: f32) -> f32 {
        let travel = self.view - self.handle();
        if travel > 0.0 {
            ((top - self.top) / travel).clamp(0.0, 1.0) * self.most()
        } else {
            0.0
        }
    }
}

impl ScrollView {
    /// `area`, vertical, named by `id_salt`, with its bar and its drag area
    /// taken over.
    pub fn new(id_salt: impl core::hash::Hash, area: ScrollArea) -> Self {
        let salt = Id::new(id_salt);
        Self {
            area: area.id_salt(salt).scroll_source(ScrollSource {
                scroll_bar: false,
                drag: false,
                mouse_wheel: true,
            }),
            salt,
            offset: None,
        }
    }

    /// Scrolls to `offset` in this frame, unless the bar is held.
    pub fn vertical_scroll_offset(mut self, offset: f32) -> Self {
        self.offset = Some(offset);
        self
    }

    /// At most `height` points tall.
    pub fn max_height(mut self, height: f32) -> Self {
        self.area = self.area.max_height(height);
        self
    }

    /// Whether the area shrinks to its content, across and down.
    pub fn auto_shrink(mut self, shrink: [bool; 2]) -> Self {
        self.area = self.area.auto_shrink(shrink);
        self
    }

    /// Shows `add` in the area.
    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> ScrollAreaOutput<R> {
        self.show_viewport(ui, |ui, _| add(ui))
    }

    /// Shows `add` in the area; it is given the part of the content in
    /// view.
    pub fn show_viewport<R>(
        self,
        ui: &mut Ui,
        add: impl FnOnce(&mut Ui, Rect) -> R,
    ) -> ScrollAreaOutput<R> {
        let ctx = ui.ctx().clone();
        // Under the id egui gives the area.
        let bar_id = ui
            .make_persistent_id(Id::new(self.salt))
            .with("monolith-scroll-bar");
        let mut area = self.area;
        let mut offset = self.offset;
        let mut grab = None;
        // A press or a drag on the bar in the last frame puts the handle
        // under the pointer, held where it was taken.
        let last: Option<Bar> = ctx.data(|data| data.get_temp(bar_id));
        let held = ctx
            .read_response(bar_id)
            .filter(Response::is_pointer_button_down_on)
            .and_then(|bar| bar.interact_pointer_pos());
        if let (Some(last), Some(pointer)) = (last, held) {
            let top = last.handle_top();
            let handle = last.handle();
            let at = last.grab.unwrap_or_else(|| {
                if (top..=top + handle).contains(&pointer.y) {
                    pointer.y - top
                } else {
                    handle / 2.0
                }
            });
            offset = Some(last.offset_at(pointer.y - at));
            grab = Some(at);
        }
        if let Some(offset) = offset {
            area = area.vertical_scroll_offset(offset);
        }
        let output = area.show_viewport(ui, add);

        let view = output.inner_rect;
        let style = ui.spacing().scroll;
        let bar = Bar {
            top: view.top(),
            view: view.height(),
            content: output.content_size.y,
            offset: output.state.offset.y,
            shortest: style.handle_min_length,
            grab,
        };
        if bar.most() > 0.0 {
            // After the content, so that the bar is above it for the
            // pointer. It senses no focus: it is no stop of the Tab key.
            let strip = Rect::from_min_max(
                egui::pos2(
                    view.right() - style.bar_outer_margin - style.bar_width,
                    view.top(),
                ),
                view.right_bottom(),
            );
            let response = ui.interact(strip, bar_id, Sense::CLICK | Sense::DRAG);
            if response.is_pointer_button_down_on() {
                ctx.request_repaint();
            }
            ctx.data_mut(|data| data.insert_temp(bar_id, bar));
        } else {
            ctx.data_mut(|data| data.remove::<Bar>(bar_id));
        }
        output
    }
}

/// The bar at the top of a view: the surface colour, a hairline under it,
/// a left area and a right area, both centred vertically.
#[derive(Clone, Copy, Debug)]
#[must_use = "show it with `.show(ui, ...)` or `.panel(ctx, ...)`"]
pub struct TopBar {
    id: Id,
}

impl TopBar {
    /// A top bar whose egui ids derive from `id_salt`.
    pub fn new(id_salt: impl core::hash::Hash) -> Self {
        Self {
            id: Id::new(("monolith-top-bar", id_salt)),
        }
    }

    /// Draws the bar in the full width of `ui`. `left` is laid out from
    /// the left edge and `right` from the right edge (the first widget it
    /// adds is the rightmost). `left` gets the room `right` leaves.
    pub fn show(
        self,
        ui: &mut Ui,
        left: impl FnOnce(&mut Ui),
        right: impl FnOnce(&mut Ui),
    ) -> Response {
        let theme = theme::current(ui.ctx());
        let size = Vec2::new(ui.available_width(), theme.size.topbar);
        let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let painter = ui.painter();
        painter.rect_filled(rect, 0.0, theme.color.bg_surface);
        painter.hline(
            rect.x_range(),
            rect.bottom() - theme.stroke.hairline / 2.0,
            Stroke::new(theme.stroke.hairline, theme.color.border_subtle),
        );

        let inner = Rect::from_min_max(
            egui::pos2(rect.left() + theme.space.lg, rect.top()),
            egui::pos2(
                rect.right() - theme.space.lg,
                rect.bottom() - theme.stroke.hairline,
            ),
        );
        let mut right_ui = ui.new_child(
            UiBuilder::new()
                .id_salt(self.id.with("right"))
                .max_rect(inner)
                .layout(Layout::right_to_left(Align::Center)),
        );
        right_ui.spacing_mut().item_spacing.x = theme.space.sm;
        right(&mut right_ui);
        let used = right_ui.min_rect();
        let mut left_max = inner;
        if used.width() > 0.0 {
            left_max.max.x = (used.left() - theme.space.md).max(inner.left());
        }
        let mut left_ui = ui.new_child(
            UiBuilder::new()
                .id_salt(self.id.with("left"))
                .max_rect(left_max)
                .layout(Layout::left_to_right(Align::Center)),
        );
        left_ui.spacing_mut().item_spacing.x = theme.space.sm;
        left(&mut left_ui);
        response
    }

    /// Draws the bar as a panel at the top of the window.
    pub fn panel(
        self,
        ctx: &Context,
        left: impl FnOnce(&mut Ui),
        right: impl FnOnce(&mut Ui),
    ) -> InnerResponse<Response> {
        let theme = theme::current(ctx);
        let id = self.id;
        egui::TopBottomPanel::top(id.with("panel"))
            .frame(Frame::new())
            .exact_height(theme.size.topbar)
            .show_separator_line(false)
            .show(ctx, |ui| self.show(ui, left, right))
    }
}

/// The sidebar: a resizable panel on the left with the sidebar colour and a
/// hairline on its right edge. The edge is dragged with the pointer; it is
/// no stop of the Tab key.
#[derive(Clone, Copy, Debug)]
#[must_use = "show it with `.show(ctx, ...)`"]
pub struct SidebarPanel {
    id: Id,
    width: Option<f32>,
    resizable: bool,
}

impl SidebarPanel {
    /// A sidebar whose egui ids (and remembered width) derive from
    /// `id_salt`.
    pub fn new(id_salt: impl core::hash::Hash) -> Self {
        Self {
            id: Id::new(("monolith-sidebar", id_salt)),
            width: None,
            resizable: true,
        }
    }

    /// The width at first (the theme's sidebar width otherwise).
    pub fn default_width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Whether the user can drag its edge.
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// The egui id of the edge that is dragged.
    pub fn edge_id(&self) -> Id {
        self.id.with("edge")
    }

    /// Shows the sidebar and fills it with `add_contents`.
    pub fn show<R>(
        self,
        ctx: &Context,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> InnerResponse<R> {
        let theme = theme::current(ctx);
        let (narrowest, widest) = (theme.size.sidebar_min, theme.size.sidebar_max);
        let edge_id = self.edge_id();
        let left = ctx.available_rect().left();
        // egui's own edge is a stop of the Tab key: for egui the panel is
        // not resizable, and the edge is the kit's.
        let mut panel = egui::SidePanel::left(self.id)
            .frame(sidebar_frame(ctx))
            .resizable(false)
            .default_width(self.width.unwrap_or(theme.size.sidebar))
            .width_range(narrowest..=widest)
            .show_separator_line(false);
        // A drag of the edge in the last frame sets the width of this one.
        let dragged = ctx
            .read_response(edge_id)
            .filter(|edge| self.resizable && edge.dragged())
            .and_then(|edge| edge.interact_pointer_pos());
        if let Some(pointer) = dragged {
            panel = panel.exact_width((pointer.x - left).clamp(narrowest, widest));
        }
        let shown = panel.show(ctx, |ui| {
            let rect = ui.max_rect();
            let result = add_contents(ui);
            // Over the contents, so that nothing covers the edge.
            ui.painter().vline(
                rect.right() - theme.stroke.hairline / 2.0,
                rect.y_range(),
                Stroke::new(theme.stroke.hairline, theme.color.border_subtle),
            );
            result
        });
        if self.resizable {
            let rect = shown.response.rect;
            let reach = ctx.style().interaction.resize_grab_radius_side;
            let area = Rect::from_x_y_ranges(
                (rect.right() - reach)..=(rect.right() + reach),
                rect.y_range(),
            );
            let mut edge_ui = Ui::new(
                ctx.clone(),
                self.id.with("edge-ui"),
                UiBuilder::new()
                    .layer_id(LayerId::background())
                    .max_rect(area),
            );
            edge_ui.set_clip_rect(area);
            // Dragged, never focused.
            let edge = edge_ui.interact(area, edge_id, Sense::DRAG);
            if edge.hovered() || edge.dragged() {
                let width = rect.width();
                ctx.set_cursor_icon(if width <= narrowest {
                    CursorIcon::ResizeEast
                } else if width >= widest {
                    CursorIcon::ResizeWest
                } else {
                    CursorIcon::ResizeHorizontal
                });
            }
            // The theme's marks of the edge under the pointer and dragged.
            let color = if edge.dragged() {
                Some(theme.color.accent)
            } else if edge.hovered() {
                Some(theme.color.border_strong)
            } else {
                None
            };
            if let Some(color) = color {
                edge_ui.painter().vline(
                    rect.right() - theme.stroke.hairline / 2.0,
                    rect.y_range(),
                    Stroke::new(theme.stroke.hairline, color),
                );
            }
        }
        shown
    }
}

/// The content panel: everything right of the sidebar, in the surface
/// colour.
#[derive(Clone, Copy, Debug, Default)]
#[must_use = "show it with `.show(ctx, ...)`"]
pub struct ContentPanel;

impl ContentPanel {
    /// Shows the panel and fills it with `add_contents`.
    pub fn show<R>(
        self,
        ctx: &Context,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> InnerResponse<R> {
        egui::CentralPanel::default()
            .frame(content_frame(ctx))
            .show(ctx, add_contents)
    }
}

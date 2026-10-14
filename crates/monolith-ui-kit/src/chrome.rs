//! The frame of a window: the top bar, the sidebar panel, the content
//! panel and the scroll area, so that every screen is laid out the same
//! way.

use egui::{
    Align, Context, CornerRadius, Frame, Id, InnerResponse, Layout, Margin, Rect, Response,
    ScrollArea, Sense, Stroke, Ui, UiBuilder, Vec2, containers::scroll_area::ScrollBarVisibility,
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

/// A card on a surface: a raised background, a hairline and the large
/// radius, with `margin` points inside.
pub fn card_frame(ctx: &Context, margin: impl Into<Margin>) -> Frame {
    let theme = theme::current(ctx);
    Frame::new()
        .fill(theme.color.bg_raised)
        .stroke(Stroke::new(
            theme.stroke.hairline,
            theme.color.border_subtle,
        ))
        .corner_radius(CornerRadius::same(theme.radius.lg))
        .inner_margin(margin)
}

/// A vertical scroll area that fills its space, with the scroll bar of the
/// theme (thin, floating, visible while scrolling or hovered). `id_salt`
/// names its scroll position.
pub fn scroll_area(id_salt: impl core::hash::Hash) -> ScrollArea {
    ScrollArea::vertical()
        .id_salt(id_salt)
        .auto_shrink([false, false])
        .scroll_bar_visibility(ScrollBarVisibility::VisibleWhenNeeded)
        .animated(true)
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
/// hairline on its right edge.
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

    /// Shows the sidebar and fills it with `add_contents`.
    pub fn show<R>(
        self,
        ctx: &Context,
        add_contents: impl FnOnce(&mut Ui) -> R,
    ) -> InnerResponse<R> {
        let theme = theme::current(ctx);
        egui::SidePanel::left(self.id)
            .frame(sidebar_frame(ctx))
            .resizable(self.resizable)
            .default_width(self.width.unwrap_or(theme.size.sidebar))
            .width_range(theme.size.sidebar_min..=theme.size.sidebar_max)
            .show_separator_line(false)
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                let result = add_contents(ui);
                // Over the contents, so that nothing covers the edge.
                ui.painter().vline(
                    rect.right() - theme.stroke.hairline / 2.0,
                    rect.y_range(),
                    Stroke::new(theme.stroke.hairline, theme.color.border_subtle),
                );
                result
            })
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

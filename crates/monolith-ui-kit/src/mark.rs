//! The wordmark of the first screens: the mark of Monolith, an upright slab
//! on a tile of the accent colour drawn from the tokens of the theme alone,
//! and the name beside it, centred on one line.

use egui::{
    CornerRadius, Painter, Rect, Response, Sense, Stroke, Ui, Vec2, Widget, WidgetType, pos2,
};

use crate::interact;
use crate::theme::{self, Theme};

/// Paints the mark filling `rect` (square).
fn paint_mark(painter: &Painter, theme: &Theme, rect: Rect) {
    let c = &theme.color;
    let side = rect.width();
    painter.rect_filled(rect, CornerRadius::same(theme.radius.md), c.accent);
    // The slab stands on the ground, a little below the middle.
    let ground = rect.center().y + side * 0.28;
    let slab = Rect::from_min_max(
        pos2(rect.center().x - side * 0.13, ground - side * 0.56),
        pos2(rect.center().x + side * 0.13, ground),
    );
    painter.rect_filled(
        slab,
        CornerRadius::same(theme.radius.xs / 2),
        c.text_on_accent,
    );
    painter.hline(
        (rect.center().x - side * 0.3)..=(rect.center().x + side * 0.3),
        ground + theme.stroke.hairline,
        Stroke::new(theme.stroke.hairline, c.text_on_accent.gamma_multiply(0.6)),
    );
}

/// The mark and `name` in the display style, centred across the width.
#[derive(Clone, Copy, Debug)]
#[must_use = "add it with `ui.add(...)`"]
pub struct Wordmark<'a> {
    name: &'a str,
}

impl<'a> Wordmark<'a> {
    /// The wordmark of `name`.
    pub fn new(name: &'a str) -> Self {
        Self { name }
    }
}

impl Widget for Wordmark<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let token = &theme.text.display;
        // The mark is as high as the line of the name.
        let side = token.line_height;
        let galley =
            ui.painter()
                .layout_no_wrap(self.name.to_owned(), token.font_id(), theme.color.text);
        let gap = theme.space.md;
        let height = side.max(galley.size().y);
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        interact::describe(&response, WidgetType::Label, self.name);
        if ui.is_rect_visible(rect) {
            let left = rect.center().x - (side + gap + galley.size().x) / 2.0;
            let mark =
                Rect::from_min_size(pos2(left, rect.center().y - side / 2.0), Vec2::splat(side));
            paint_mark(ui.painter(), &theme, mark);
            let at = pos2(mark.right() + gap, rect.center().y - galley.size().y / 2.0);
            ui.painter().galley(at, galley, theme.color.text);
        }
        response
    }
}

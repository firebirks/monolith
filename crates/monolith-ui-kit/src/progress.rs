//! A progress bar for a file on its way. Its share comes from whole bytes
//! the caller was told; the bar never guesses one. A transfer whose bytes
//! are not known (a file being prepared, or one resumed after a restart
//! until it goes on) shows the bar with no share: a short segment that
//! moves, or, with reduced motion, a still track that asks for no repaint.

use egui::{CornerRadius, Rect, Response, Sense, Ui, Vec2, Widget, WidgetInfo, WidgetType, pos2};

use crate::theme;

/// How far a transfer is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    /// `done` of `total` bytes. A total of zero is all there.
    Share {
        /// The bytes done.
        done: u64,
        /// The bytes in all.
        total: u64,
    },
    /// Not known now.
    Unknown,
}

impl Fill {
    /// The share done, from 0 to 1, for drawing only.
    fn fraction(self) -> Option<f32> {
        match self {
            Self::Share { total: 0, .. } => Some(1.0),
            Self::Share { done, total } => {
                // A thousandth is finer than any bar is wide.
                let thousandths = u128::from(done.min(total))
                    .saturating_mul(1000)
                    .checked_div(u128::from(total))
                    .unwrap_or(1000);
                let thousandths = u16::try_from(thousandths).unwrap_or(1000);
                Some(f32::from(thousandths) / 1000.0)
            }
            Self::Unknown => None,
        }
    }

    /// The whole per cent done, for screen readers.
    fn percent(self) -> Option<f64> {
        let share = match self {
            Self::Share { total: 0, .. } => 100,
            Self::Share { done, total } => u128::from(done.min(total))
                .saturating_mul(100)
                .checked_div(u128::from(total))
                .unwrap_or(100),
            Self::Unknown => return None,
        };
        Some(f64::from(u8::try_from(share).unwrap_or(100)))
    }
}

/// A progress bar across the available width.
#[must_use = "add it with `ui.add(...)`"]
pub struct Progress<'a> {
    fill: Fill,
    label: &'a str,
}

impl<'a> Progress<'a> {
    /// A bar at `fill`, named `label` for screen readers (such as "Sending
    /// project.zip").
    pub fn new(fill: Fill, label: &'a str) -> Self {
        Self { fill, label }
    }
}

impl Widget for Progress<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = theme::current(ui.ctx());
        let c = &theme.color;
        let height = theme.space.xs + theme.space.xxs;
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        let radius = CornerRadius::same(theme.radius.pill);
        let painter = ui.painter();
        painter.rect_filled(rect, radius, c.bg_sunken);
        match self.fill.fraction() {
            Some(fraction) => {
                if fraction > 0.0 {
                    let width = (rect.width() * fraction).max(height);
                    let done = Rect::from_min_size(rect.min, Vec2::new(width, height));
                    painter.rect_filled(done, radius, c.accent);
                }
            }
            None => {
                let segment = rect.width() * 0.28;
                let start = if theme.motion.reduced {
                    // Still: a quiet segment at the start says "under way"
                    // without moving.
                    0.0
                } else {
                    ui.ctx().request_repaint();
                    let travel = rect.width() + segment;
                    // Wrapped to one pass, so that the value stays small.
                    let time = ui.input(|input| input.time).rem_euclid(1.6) / 1.6;
                    #[allow(clippy::cast_possible_truncation)]
                    let time = time as f32;
                    time * travel - segment
                };
                let left = (rect.left() + start).max(rect.left());
                let right = (rect.left() + start + segment).min(rect.right());
                if right > left {
                    let moving =
                        Rect::from_min_max(pos2(left, rect.top()), pos2(right, rect.bottom()));
                    let ink = if theme.motion.reduced {
                        c.accent_subtle
                    } else {
                        c.accent
                    };
                    painter.rect_filled(moving, radius, ink);
                }
            }
        }
        let percent = self.fill.percent();
        let label = self.label;
        response.widget_info(|| {
            let mut info = WidgetInfo::labeled(WidgetType::ProgressIndicator, true, label);
            info.value = percent;
            info
        });
        response
    }
}

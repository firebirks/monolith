//! Short notices that come and go.
//!
//! [`Toasts`] is owned by the caller, in its own state: it holds the
//! notices and their timers, and egui's memory holds nothing of them.
//! Errors stay until the user dismisses them; the others go by
//! themselves. At most three are shown, at the top of the window under the
//! top bar, clear of the composer and of the actions of a modal; the newest
//! is at the bottom of the stack. The others wait their turn, with their
//! timers not yet started. A screen reader is told of each as it comes.

use std::time::Duration;

use egui::accesskit::Live;
use egui::{
    Align2, Area, Color32, Context, CornerRadius, Id, Order, Rect, Sense, Stroke, StrokeKind, Vec2,
    WidgetInfo, WidgetType, text::LayoutJob,
};

use crate::icons::{self, Icon};
use crate::interact::{self, State, mix, over};
use crate::theme::{self, Theme};

/// How many notices are shown at once.
const MAX_VISIBLE: usize = 3;

/// How long a notice that goes by itself stays, in seconds.
const LIFETIME: f64 = 5.0;

/// How far a notice slides while it comes in and goes out, in points.
const SLIDE: f32 = 10.0;

/// The pause between frames while a notice moves.
const ANIMATION_FRAME: Duration = Duration::from_millis(16);

/// What a notice is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Something to know.
    Info,
    /// Something done.
    Success,
    /// Something that needs attention.
    Warning,
    /// Something failed. Stays until dismissed.
    Error,
}

impl Kind {
    fn icon(self) -> Icon {
        match self {
            Self::Info => icons::INFO,
            Self::Success => icons::CIRCLE_CHECK,
            Self::Warning => icons::TRIANGLE_ALERT,
            Self::Error => icons::CIRCLE_ALERT,
        }
    }

    fn color(self, theme: &Theme) -> Color32 {
        let c = &theme.color;
        match self {
            Self::Info => c.accent_text,
            Self::Success => c.success,
            Self::Warning => c.warning,
            Self::Error => c.danger,
        }
    }
}

#[derive(Clone, Debug)]
struct Toast {
    id: u64,
    kind: Kind,
    text: String,
    action: Option<String>,
    /// When it first appeared; its timer starts then.
    shown_at: Option<f64>,
    /// When it goes by itself, if it does.
    deadline: Option<f64>,
    /// When it started to go out.
    leaving_at: Option<f64>,
    /// The pointer was over it in the last frame (its timer waits).
    hovered: bool,
}

/// What came of showing the notices.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToastsOutput {
    /// The id of the notice whose action the user chose. The notice is
    /// dismissed with it.
    pub action: Option<u64>,
}

/// The notices of a window.
#[derive(Clone, Debug, Default)]
pub struct Toasts {
    next_id: u64,
    items: Vec<Toast>,
    last_tick: Option<f64>,
    dismiss_label: String,
    /// The notices shown in the last frame.
    shown: Vec<u64>,
}

impl Toasts {
    /// No notices.
    pub fn new() -> Self {
        Self::default()
    }

    /// The accessible name of the dismiss button of every notice (for
    /// example "Dismiss").
    pub fn dismiss_label(mut self, label: impl Into<String>) -> Self {
        self.dismiss_label = label.into();
        self
    }

    /// Shows a notice. Returns its id.
    pub fn push(&mut self, kind: Kind, text: impl Into<String>) -> u64 {
        self.push_inner(kind, text.into(), None)
    }

    /// Shows a notice with an action button that reads `action`. Returns
    /// its id, which [`ToastsOutput::action`] names when it is chosen.
    pub fn push_with_action(
        &mut self,
        kind: Kind,
        text: impl Into<String>,
        action: impl Into<String>,
    ) -> u64 {
        self.push_inner(kind, text.into(), Some(action.into()))
    }

    fn push_inner(&mut self, kind: Kind, text: String, action: Option<String>) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.items.push(Toast {
            id,
            kind,
            text,
            action,
            shown_at: None,
            deadline: None,
            leaving_at: None,
            hovered: false,
        });
        id
    }

    /// Starts to take the notice `id` away, if it is still there.
    pub fn dismiss(&mut self, id: u64, ctx: &Context) {
        let now = ctx.input(|input| input.time);
        if let Some(toast) = self.items.iter_mut().find(|toast| toast.id == id) {
            toast.leaving_at.get_or_insert(now);
        }
        ctx.request_repaint();
    }

    /// Whether there is nothing to show.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// How many notices there are, shown or waiting.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Draws the notices above everything, centred at the top of the
    /// window under the height of the top bar, and keeps their timers. Call
    /// it once per frame, last.
    pub fn show(&mut self, ctx: &Context) -> ToastsOutput {
        self.show_in(ctx, ctx.screen_rect())
    }

    /// Like [`Toasts::show`], centred in `area` (the content panel, say,
    /// so that the notices sit over the view they are about) and under the
    /// height of a top bar from its top edge.
    pub fn show_in(&mut self, ctx: &Context, area: Rect) -> ToastsOutput {
        let theme = theme::current(ctx);
        let now = ctx.input(|input| input.time);
        let elapsed = self.last_tick.map_or(0.0, |last| (now - last).max(0.0));
        self.last_tick = Some(now);
        let fade = f64::from(theme.motion.duration(theme.motion.normal));

        // Timers: only the notices that are shown run; one under the
        // pointer waits.
        let first_visible = self.items.len().saturating_sub(MAX_VISIBLE);
        for (index, toast) in self.items.iter_mut().enumerate() {
            if index < first_visible {
                continue;
            }
            if toast.shown_at.is_none() {
                toast.shown_at = Some(now);
                toast.deadline = (toast.kind != Kind::Error).then_some(now + LIFETIME);
            }
            if toast.hovered {
                toast.deadline = toast.deadline.map(|deadline| deadline + elapsed);
            }
            if toast.leaving_at.is_none() && toast.deadline.is_some_and(|d| d <= now) {
                toast.leaving_at = Some(now);
            }
        }
        // Gone once the way out is done.
        self.items
            .retain(|toast| toast.leaving_at.is_none_or(|start| now - start < fade));
        let first_visible = self.items.len().saturating_sub(MAX_VISIBLE);

        let mut output = ToastsOutput::default();
        let mut dismissed = Vec::new();
        let mut next_wake: Option<f64> = None;
        let dismiss_label = self.dismiss_label.clone();
        let shown: Vec<u64> = self
            .items
            .iter()
            .skip(first_visible)
            .map(|toast| toast.id)
            .collect();
        if shown != self.shown {
            // The stack changed: lay this frame out again with the new
            // sizes rather than present it misplaced.
            if !shown.is_empty() {
                ctx.request_discard("the notices shown changed");
            }
            self.shown = shown;
        }
        let visible: Vec<&mut Toast> = self.items.iter_mut().skip(first_visible).collect();
        if visible.is_empty() {
            return output;
        }
        let width = theme
            .size
            .toast
            .min(area.width() - theme.space.lg * 2.0)
            .max(theme.size.control);
        let origin = egui::pos2(
            area.center().x - width / 2.0,
            area.top() + theme.size.topbar + theme.space.sm,
        );
        Area::new(Id::new("monolith-toasts"))
            .order(Order::Foreground)
            .fixed_pos(origin)
            .interactable(true)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = theme.space.sm;
                for toast in visible {
                    let shown = toast.shown_at.unwrap_or(now);
                    let appear = if fade > 0.0 {
                        ((now - shown) / fade).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                    let disappear = match toast.leaving_at {
                        Some(start) if fade > 0.0 => ((now - start) / fade).clamp(0.0, 1.0),
                        Some(_) => 1.0,
                        None => 0.0,
                    };
                    // Narrowed from f64: a fraction.
                    #[allow(clippy::cast_possible_truncation)]
                    let level = (ease(appear) * (1.0 - ease(disappear))) as f32;
                    let slide = (1.0 - level) * SLIDE;
                    let moving = appear < 1.0 || toast.leaving_at.is_some();
                    let drawn = draw(ui, &theme, toast, width, level, slide, &dismiss_label);
                    toast.hovered = drawn.hovered;
                    if drawn.action {
                        output.action = Some(toast.id);
                        dismissed.push(toast.id);
                    }
                    if drawn.dismissed {
                        dismissed.push(toast.id);
                    }
                    let wake = if moving {
                        Some(now + ANIMATION_FRAME.as_secs_f64())
                    } else {
                        toast.deadline
                    };
                    next_wake = match (next_wake, wake) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                }
            });

        if !dismissed.is_empty() {
            for toast in &mut self.items {
                if dismissed.contains(&toast.id) {
                    toast.leaving_at.get_or_insert(now);
                }
            }
            ctx.request_repaint();
        } else if let Some(wake) = next_wake {
            ctx.request_repaint_after(Duration::from_secs_f64((wake - now).max(0.0)));
        }
        output
    }
}

/// Smooth start and end.
fn ease(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}

struct Drawn {
    hovered: bool,
    action: bool,
    dismissed: bool,
}

fn draw(
    ui: &mut egui::Ui,
    theme: &Theme,
    toast: &Toast,
    width: f32,
    level: f32,
    slide: f32,
    dismiss_label: &str,
) -> Drawn {
    let c = &theme.color;
    let pad = theme.space.md;
    let icon_size = theme.size.icon_md;
    let icon_slot = icon_size + theme.space.sm;
    let button = theme.size.control_sm - theme.space.xs;
    let fade = |color: Color32| color.gamma_multiply(level);

    let action_galley = toast.action.as_ref().map(|label| {
        ui.painter().layout_no_wrap(
            label.clone(),
            theme.text.caption_strong.font_id(),
            Color32::PLACEHOLDER,
        )
    });
    let action_width = action_galley
        .as_ref()
        .map_or(0.0, |galley| galley.size().x + theme.space.md * 2.0);
    let text_width = (width - pad * 2.0 - icon_slot - button - theme.space.xs - action_width)
        .max(theme.size.control);
    let mut job = LayoutJob::single_section(
        toast.text.clone(),
        theme.text.body.format(Color32::PLACEHOLDER),
    );
    job.wrap.max_width = text_width;
    let galley = ui.painter().layout_job(job);

    let height = galley.size().y + pad * 2.0;
    let (slot, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    // Comes down from above and goes back up.
    let rect = slot.translate(Vec2::new(0.0, -slide));
    let id = Id::new(("monolith-toast", toast.id));
    let card = ui.interact(rect, id, Sense::hover());
    card.widget_info(|| WidgetInfo::labeled(WidgetType::Label, true, toast.text.as_str()));
    // A live region: a screen reader reads the notice when it comes, at
    // once for an error, after what it is reading for the others.
    let live = if toast.kind == Kind::Error {
        Live::Assertive
    } else {
        Live::Polite
    };
    ui.ctx()
        .accesskit_node_builder(id, |node| node.set_live(live));

    let radius = CornerRadius::same(theme.radius.lg);
    let painter = ui.painter();
    let mut shadow = theme.shadow.md;
    shadow.color = fade(shadow.color);
    painter.add(shadow.as_shape(rect, radius));
    painter.rect(
        rect,
        radius,
        fade(c.bg_raised),
        Stroke::new(theme.stroke.hairline, fade(c.border_subtle)),
        StrokeKind::Inside,
    );

    let first_line = rect.top() + pad + theme.text.body.line_height / 2.0;
    toast.kind.icon().paint(
        painter,
        egui::pos2(rect.left() + pad + icon_size / 2.0, first_line),
        icon_size,
        fade(toast.kind.color(theme)),
    );
    let text_pos = egui::pos2(rect.left() + pad + icon_slot, rect.top() + pad);
    painter.galley(text_pos, galley, fade(c.text));

    // The dismiss button, in the top right corner.
    let dismiss_rect = Rect::from_center_size(
        egui::pos2(rect.right() - theme.space.sm - button / 2.0, first_line),
        Vec2::splat(button),
    );
    let dismiss = ui.interact(dismiss_rect, id.with("dismiss"), Sense::click());
    interact::describe(&dismiss, WidgetType::Button, dismiss_label);
    let state = State::of(ui, &dismiss, dismiss.id, theme, false);
    let painter = ui.painter();
    let small = CornerRadius::same(theme.radius.sm);
    let tint = if state.pressed {
        c.pressed
    } else {
        c.hover.gamma_multiply(state.hover)
    };
    painter.rect_filled(dismiss_rect, small, fade(over(c.bg_raised, tint)));
    icons::X.paint(
        painter,
        dismiss_rect.center(),
        theme.size.icon_sm,
        fade(mix(c.text_tertiary, c.text, state.hover)),
    );
    if state.focused {
        interact::focus_ring(painter, dismiss_rect, small, theme);
    }

    // The action, left of the dismiss button.
    let mut action_clicked = false;
    if let Some(galley) = action_galley {
        let action_rect = Rect::from_center_size(
            egui::pos2(
                dismiss_rect.left() - theme.space.xs - action_width / 2.0,
                first_line,
            ),
            Vec2::new(action_width, button),
        );
        let response = ui.interact(action_rect, id.with("action"), Sense::click());
        interact::describe(
            &response,
            WidgetType::Button,
            toast.action.as_deref().unwrap_or_default(),
        );
        let state = State::of(ui, &response, response.id, theme, false);
        let painter = ui.painter();
        let tint = if state.pressed {
            c.pressed
        } else {
            c.hover.gamma_multiply(state.hover)
        };
        painter.rect_filled(action_rect, small, fade(over(c.bg_raised, tint)));
        let pos = Align2::CENTER_CENTER
            .anchor_size(action_rect.center(), galley.size())
            .min;
        painter.galley(pos, galley, fade(c.accent_text));
        if state.focused {
            interact::focus_ring(painter, action_rect, small, theme);
        }
        action_clicked = response.clicked();
    }

    Drawn {
        hovered: card.contains_pointer() || dismiss.contains_pointer(),
        action: action_clicked,
        dismissed: dismiss.clicked(),
    }
}

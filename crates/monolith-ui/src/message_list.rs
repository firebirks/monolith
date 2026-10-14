//! The messages of a conversation in a scroll area that lays out and
//! paints only the messages in or near view.
//!
//! A message has a height at the width it is shown in, measured the first
//! time it is laid out and kept; one never laid out has an estimate. Above
//! and below the messages drawn is empty space of those heights. The view
//! holds on to the end of the message at its top: when the heights above
//! it change (older messages read, a message measured, a day heading that
//! goes), the scroll position moves by as much, so that what the reader
//! sees stays where it is. At the bottom the view stays at the bottom as
//! messages arrive. Messages are laid out
//! half a view beyond it, so that most are measured before they come into
//! sight.

use std::collections::{BTreeMap, BTreeSet};

use egui::{Align, Context, Id, Layout, Rect, Ui, UiBuilder, Vec2, pos2};
use monolith_ui_kit::chrome::scroll_area;
use monolith_ui_model::UiMessageId;

use crate::conversation::inset;

/// What decides the height of a message besides the width: the message,
/// whether it is expanded, and whether a day heading or the gap of a new
/// group comes before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Key {
    /// The message.
    pub(crate) message: UiMessageId,
    /// It is shown whole, past the line limit.
    pub(crate) expanded: bool,
    /// A day heading comes before it.
    pub(crate) new_day: bool,
    /// It starts a group of messages.
    pub(crate) first_in_group: bool,
}

/// What a list shows.
pub(crate) trait Content {
    /// A number that changes whenever the messages or the way they are
    /// shown change.
    fn revision(&self) -> u64;
    /// Every message, oldest first, with an estimate of its height at
    /// `width`, its spacing included.
    fn entries(&self, width: f32) -> Vec<(Key, f32)>;
    /// Draws what comes above the messages.
    fn header(&mut self, ui: &mut Ui);
    /// Draws message `index` from the top of `ui`.
    fn message(&mut self, ui: &mut Ui, index: usize);
}

/// What is kept of a list between frames.
#[derive(Clone)]
struct State {
    /// The revision, width and scale `keys` and `tops` were made for.
    made_for: Option<(u64, f32, f32)>,
    /// The messages, in order.
    keys: Vec<Key>,
    /// The estimate of each message, for those not measured.
    estimates: Vec<f32>,
    /// The height of each message measured at `width`, spacing included.
    heights: BTreeMap<Key, f32>,
    /// Where each message starts, from the top of the list, and where the
    /// last one ends.
    tops: Vec<f32>,
    /// The width the heights were measured at.
    width: f32,
    /// The scale they were measured at.
    scale: f32,
    /// The space above the messages in the content.
    lead: f32,
    /// The space below them.
    trail: f32,
    /// The scroll position at the end of the last frame.
    offset: f32,
    /// The view stays at the bottom.
    bottom: bool,
    /// The message at the top of the view, and where it ended in the
    /// content.
    anchor: Option<(UiMessageId, f32)>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            made_for: None,
            keys: Vec::new(),
            estimates: Vec::new(),
            heights: BTreeMap::new(),
            tops: vec![0.0],
            width: 0.0,
            scale: 0.0,
            lead: 0.0,
            trail: 0.0,
            offset: 0.0,
            // A conversation opens at its newest message.
            bottom: true,
            anchor: None,
        }
    }
}

impl State {
    /// Takes the messages of `content` again if they, the width or the
    /// scale changed. Returns whether anything was taken.
    fn refresh(&mut self, content: &dyn Content, width: f32, scale: f32) -> bool {
        let revision = content.revision();
        if self.made_for == Some((revision, width, scale)) {
            return false;
        }
        if self.width != width || self.scale != scale {
            self.heights.clear();
            self.width = width;
            self.scale = scale;
        }
        let (keys, estimates): (Vec<Key>, Vec<f32>) = content.entries(width).into_iter().unzip();
        let present: BTreeSet<Key> = keys.iter().copied().collect();
        self.heights.retain(|key, _| present.contains(key));
        self.keys = keys;
        self.estimates = estimates;
        self.made_for = Some((revision, width, scale));
        self.layout();
        true
    }

    /// The height of message `index`: measured, or else estimated.
    fn height(&self, index: usize) -> f32 {
        self.keys
            .get(index)
            .and_then(|key| self.heights.get(key))
            .or_else(|| self.estimates.get(index))
            .copied()
            .unwrap_or(0.0)
    }

    /// Where message `index` starts, from the top of the list.
    fn top(&self, index: usize) -> f32 {
        self.tops.get(index).copied().unwrap_or(0.0)
    }

    /// The height of all the messages.
    fn total(&self) -> f32 {
        self.tops.last().copied().unwrap_or(0.0)
    }

    /// The message at `y` from the top of the list.
    fn index_at(&self, y: f32) -> usize {
        self.tops
            .partition_point(|top| *top <= y)
            .saturating_sub(1)
            .min(self.keys.len().saturating_sub(1))
    }

    /// Where `message` is in the list.
    fn position(&self, message: UiMessageId) -> Option<usize> {
        self.keys.iter().position(|key| key.message == message)
    }

    /// Adds the heights up into `tops`.
    fn layout(&mut self) {
        let mut tops = Vec::with_capacity(self.keys.len().saturating_add(1));
        let mut y = 0.0;
        tops.push(y);
        for index in 0..self.keys.len() {
            y += self.height(index);
            tops.push(y);
        }
        self.tops = tops;
    }

    /// The scroll position for this frame in a view `view` high: at the
    /// bottom, or such that the end of the message at the top of the view
    /// stays where it was. With it, that message and where it ends now.
    fn offset(&self, view: f32) -> (f32, Option<(UiMessageId, f32)>) {
        if self.bottom {
            return (
                (self.lead + self.total() + self.trail - view).max(0.0),
                None,
            );
        }
        let Some((message, was)) = self.anchor else {
            return (self.offset, None);
        };
        let Some(index) = self.position(message) else {
            return (self.offset, None);
        };
        let now = self.lead + self.top(index.saturating_add(1));
        let offset = if now == was {
            self.offset
        } else {
            (self.offset + now - was).max(0.0)
        };
        (offset, Some((message, now)))
    }
}

/// A message laid out in this frame: its position, where it was put from
/// the top of the list, and the height it took.
struct Drawn {
    index: usize,
    top: f32,
    height: f32,
}

/// The id under which the state of the list of `id` is kept.
fn memory(id: Id) -> Id {
    id.with("message-list")
}

/// Shows `content` in a scroll area named by `id`, its messages in a
/// column narrower than the area by `pad` on both sides, with `gap` above
/// and below.
pub(crate) fn show(ui: &mut Ui, id: Id, pad: f32, gap: f32, content: &mut dyn Content) {
    let ctx = ui.ctx().clone();
    let mut state: State = ctx
        .data_mut(|data| data.remove_temp(memory(id)))
        .unwrap_or_default();
    let scale = ctx.pixels_per_point();
    let guess = if state.width > 0.0 {
        state.width
    } else {
        ui.available_width() - pad * 2.0
    };
    state.refresh(content, guess, scale);
    let (offset, held_at) = state.offset(ui.available_height());

    let mut drawn: Vec<Drawn> = Vec::new();
    let mut lead = state.lead;
    let mut end = state.total();
    // What was laid out is not where the scroll position expected it.
    let mut moved = false;
    // Something in view was put in the wrong place.
    let mut unsettled = false;
    let output = scroll_area(id)
        .vertical_scroll_offset(offset)
        .show_viewport(ui, |ui, viewport| {
            let origin = ui.max_rect().top();
            ui.add_space(gap);
            inset(ui, pad, |ui| {
                content.header(ui);
                let top = ui.cursor().top();
                let left = ui.max_rect().left();
                let width = ui.available_width();
                let spacing = ui.spacing().item_spacing.y;
                lead = top - origin;
                moved |= state.refresh(&*content, width, scale);
                moved |= (lead - state.lead).abs() > 0.5;
                let count = state.keys.len();
                let ahead = viewport.height() / 2.0;
                let low = viewport.min.y - ahead - lead;
                let high = viewport.max.y + ahead - lead;
                // Messages up to the one at the top of the view are put
                // where the heights say, those after it one under the
                // other, so that a height measured now moves only what is
                // below it. At the bottom, the end is what stays.
                let held = if state.bottom {
                    count
                } else {
                    held_at
                        .and_then(|(message, _)| state.position(message))
                        .unwrap_or_else(|| state.index_at(viewport.min.y - lead))
                };
                let mut index = state.index_at(low);
                let mut next = state.top(index);
                while index < count {
                    let at = if index <= held {
                        state.top(index)
                    } else {
                        next
                    };
                    if at >= high {
                        break;
                    }
                    let message = state.keys.get(index).map(|key| key.message);
                    let rect = Rect::from_min_size(
                        pos2(left, top + at),
                        Vec2::new(width, state.height(index)),
                    );
                    let mut child = ui.new_child(
                        UiBuilder::new()
                            .id_salt(("message", message))
                            .max_rect(rect)
                            .layout(Layout::top_down(Align::Min)),
                    );
                    content.message(&mut child, index);
                    let height = child.min_rect().height() + spacing;
                    drawn.push(Drawn {
                        index,
                        top: at,
                        height,
                    });
                    next = at + height;
                    index = index.saturating_add(1);
                }
                // A height measured now that differs from the one the
                // message was put by misplaces its end, which the next
                // message or the view holds to; in view, that shows.
                let mut measured = false;
                for message in &drawn {
                    let used = state.height(message.index);
                    if (message.height - used).abs() <= 0.5 {
                        continue;
                    }
                    measured = true;
                    let reach = lead + message.top + message.height.max(used);
                    let seen = reach > viewport.min.y && lead + message.top < viewport.max.y;
                    unsettled |= message.index <= held && seen;
                }
                if measured {
                    for message in &drawn {
                        if let Some(key) = state.keys.get(message.index) {
                            state.heights.insert(*key, message.height);
                        }
                    }
                    state.layout();
                }
                if !drawn.is_empty() {
                    end = next + (state.total() - state.top(index));
                }
                ui.allocate_space(Vec2::new(width, end));
            });
            ui.add_space(gap);
        });

    let content_height = output.content_size.y;
    let last = output.state.offset.y;
    let most = (content_height - output.inner_rect.height()).max(0.0);
    if last < offset - 0.5 && last < most - 0.5 {
        // The reader moved up.
        state.bottom = false;
    } else if last >= most - 0.5 {
        state.bottom = true;
    }
    state.lead = lead;
    state.trail = content_height - lead - end;
    state.offset = last;
    state.anchor = if state.bottom {
        None
    } else if moved && held_at.is_some() {
        // What was laid out moved under the view: the message held stays
        // the one to keep in place.
        held_at
    } else {
        let y = last - lead;
        drawn
            .iter()
            .find(|message| message.top <= y && y < message.top + message.height)
            .map(|message| (message.index, lead + message.top + message.height))
            .or_else(|| {
                let index = state.index_at(y);
                Some((index, lead + state.top(index.saturating_add(1))))
            })
            .and_then(|(index, top)| state.keys.get(index).map(|key| (key.message, top)))
    };
    ctx.data_mut(|data| data.insert_temp(memory(id), state));
    if moved || unsettled {
        ctx.request_discard("messages were measured");
        if !ctx.will_discard() {
            ctx.request_repaint();
        }
    }
}

/// Forgets what is kept of the list named by `id`.
pub(crate) fn forget(ctx: &Context, id: Id) {
    ctx.data_mut(|data| data.remove::<State>(memory(id)));
}

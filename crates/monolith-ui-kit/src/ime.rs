//! Input methods: whether one composes in an edit, so that the Enter that
//! commits a composition is not taken for the Enter of the user, which
//! sends a message or confirms a dialog (`docs/DESIGN_QUESTIONS.md` P7-19
//! and P7-23). egui puts the text in composition into the edit before it
//! is committed, and egui-winit hands on every key the platform delivers.

use egui::{Event, Id, ImeEvent, Ui};

/// Where an edit keeps whether an input method composes in it.
const COMPOSING: &str = "composing";

/// Whether an input method composes in the edit `edit_id` in this frame:
/// it has text in composition from earlier frames or from this one, or a
/// composition ended in this frame (a commit, which only a composition
/// makes, or the text in composition emptied or the input method stopped
/// while it composed). Kept from frame to frame while the edit has the
/// focus (`focused`), which the input method writes to. Called once a
/// frame for each edit.
pub(crate) fn composing(ui: &Ui, edit_id: Id, focused: bool) -> bool {
    let key = edit_id.with(COMPOSING);
    if !focused {
        ui.data_mut(|data| data.remove::<bool>(key));
        return false;
    }
    let before = ui.data(|data| data.get_temp::<bool>(key).unwrap_or(false));
    let (now, ended) = ui.input(|input| {
        input
            .events
            .iter()
            .fold((before, false), |(now, ended), event| match event {
                Event::Ime(ImeEvent::Preedit(text)) if text.is_empty() => (false, ended || now),
                Event::Ime(ImeEvent::Preedit(_)) => (true, ended),
                Event::Ime(ImeEvent::Commit(_)) => (false, true),
                Event::Ime(ImeEvent::Disabled) => (false, ended || now),
                _ => (now, ended),
            })
    });
    ui.data_mut(|data| data.insert_temp(key, now));
    before || now || ended
}

/// Removes what [`composing`] keeps for the edit `edit_id`.
pub(crate) fn forget(ctx: &egui::Context, edit_id: Id) {
    ctx.data_mut(|data| data.remove::<bool>(edit_id.with(COMPOSING)));
}

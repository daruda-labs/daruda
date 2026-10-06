//! Shared picker behaviour — the invisible half of a "search and pick"
//! overlay: the typed query, which row is selected, filtering, keyboard
//! navigation, and key mapping.
//!
//! Imports no GPUI types, so every rule below is unit-testable without a
//! window. The visible half is [`crate::ui::picker_row`]; the keystroke
//! that reaches [`PickerState::on_key`] is decoded in
//! [`super::picker_key`], which is GPUI-facing and therefore separate.
//!
//! Deliberately *not* here: whether the overlay is open. A `bool` cannot
//! express the flow picker's `Closed | Choosing(_) | Stopping`, so this
//! type would either hold a flag that contradicts that enum or force the
//! flow picker to mirror one. Each view keeps its own openness.

use crate::{fuzzy::fuzzy_match, ui::theme};

/// Query + selection for one picker overlay. Plain data: the view clones
/// it into its `RenderOnce` overlay every frame.
#[derive(Default, Clone, Debug)]
pub(in crate::workspace) struct PickerState {
    query: String,
    focused_index: usize,
}

/// What a keystroke means to a picker. [`PickerState`] applies the query
/// and selection changes itself; everything with a side effect outside
/// the picker (activate, close, repaint) is the caller's to run.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::workspace) enum PickerKey {
    /// Query or selection changed here; the caller repaints.
    Changed,
    /// Not a picker key, so nothing changed and there is nothing to
    /// repaint. Still swallowed by the overlay — the axis is whether to
    /// repaint, not whether to consume.
    Unchanged,
    /// Enter — activate the focused row.
    Confirm,
    /// Escape — close the overlay.
    Dismiss,
}

/// The two keys that mean the same thing with or without a list.
///
/// A picker with no rows still has to answer both — the flow picker's
/// stop prompt reaches its stop through Enter — so this mapping cannot
/// live behind the list-key path, and owning it here is what keeps the
/// flow picker from carrying a second copy of it.
pub(in crate::workspace) fn overlay_key(key: &str) -> Option<PickerKey> {
    match key {
        "escape" => Some(PickerKey::Dismiss),
        "enter" => Some(PickerKey::Confirm),
        _ => None,
    }
}

impl PickerState {
    /// Back to "just opened": empty query, first row focused. Callers run
    /// this alongside seeding or dropping their own payload.
    pub fn reset(&mut self) {
        self.query.clear();
        self.focused_index = 0;
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// The query changed, so the old index no longer names the same row —
    /// start over at the top.
    fn append(&mut self, ch: char) {
        self.query.push(ch);
        self.focused_index = 0;
    }

    fn backspace(&mut self) {
        self.query.pop();
        self.focused_index = 0;
    }

    /// Move the focus to a row the mouse named. Clicking is the same
    /// gesture as arrowing there and pressing Enter, so it goes through
    /// the same field rather than a second path to the same decision.
    pub fn focus(&mut self, ix: usize) {
        self.focused_index = ix;
    }

    fn move_up(&mut self) {
        if self.focused_index > 0 {
            self.focused_index -= 1;
        }
    }

    fn move_down(&mut self, visible_len: usize) {
        if self.focused_index + 1 < visible_len {
            self.focused_index += 1;
        }
    }

    /// Never clamped against the current candidate set: every read goes
    /// through [`Self::visible`], where an out-of-range index resolves to
    /// `None` and the action no-ops.
    pub fn focused_index(&self) -> usize {
        self.focused_index
    }

    /// The single definition of "the rows on screen": `fuzzy_match`'s
    /// ranking of `labels`, capped at `PALETTE_MAX_VISIBLE` — which is
    /// also the row count `theme::PALETTE_MAX_HEIGHT` leaves room for.
    ///
    /// The cap is applied *after* the ranking. Capping the input first
    /// would drop the best match whenever it sits past the cap in the
    /// caller's order.
    ///
    /// The ranking hook is the *order of `labels`* — `fuzzy_match` keeps
    /// the input order for equal scores, so a caller that pre-sorts its
    /// candidates gets that as the secondary sort. No extra API.
    pub fn visible(&self, labels: &[impl AsRef<str>]) -> Vec<usize> {
        let mut matched = fuzzy_match(&self.query, labels);
        matched.truncate(theme::PALETTE_MAX_VISIBLE);
        matched
    }

    /// Map a keystroke onto a picker intent, applying the query or
    /// selection change it implies.
    ///
    /// `visible_len` is a parameter because "down" needs the on-screen
    /// row count, that count comes only from the labels, and
    /// `PickerState` does not own labels. The caller computes it via
    /// [`Self::visible`] and passes it in.
    pub fn on_key(&mut self, key: &str, ch: Option<char>, visible_len: usize) -> PickerKey {
        if let Some(k) = overlay_key(key) {
            return k;
        }
        match key {
            "up" => {
                self.move_up();
                PickerKey::Changed
            }
            "down" => {
                self.move_down(visible_len);
                PickerKey::Changed
            }
            "backspace" => {
                self.backspace();
                PickerKey::Changed
            }
            // Printable ASCII only: the query is a key-down accumulator,
            // and CLAUDE.md pitfall #4 forbids routing IME text through
            // one — so anything an IME would compose is dropped rather
            // than appended half-formed. Detail in the test below.
            _ => match ch {
                Some(ch) if ch.is_ascii_graphic() || ch == ' ' => {
                    self.append(ch);
                    PickerKey::Changed
                }
                _ => PickerKey::Unchanged,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every helper drives the state the way production does — through
    /// [`PickerState::on_key`] — so the tests cover the path the views
    /// actually take rather than the private steps behind it.
    fn typed(query: &str) -> PickerState {
        let mut state = PickerState::default();
        for ch in query.chars() {
            state.on_key(&ch.to_string(), Some(ch), 0);
        }
        state
    }

    fn press_down(state: &mut PickerState, visible_len: usize) -> PickerKey {
        state.on_key("down", None, visible_len)
    }

    #[test]
    fn move_down_caps_at_visible_rows() {
        let mut state = PickerState::default();
        for _ in 0..100 {
            press_down(&mut state, 3);
        }
        assert_eq!(state.focused_index(), 2);
    }

    #[test]
    fn move_down_on_an_empty_list_stays_put() {
        let mut state = PickerState::default();
        press_down(&mut state, 0);
        assert_eq!(state.focused_index(), 0);
    }

    #[test]
    fn move_up_stops_at_the_first_row() {
        let mut state = PickerState::default();
        state.on_key("up", None, 3);
        assert_eq!(state.focused_index(), 0);
    }

    /// The `PALETTE_MAX_VISIBLE` cap lives in `visible()` and nowhere
    /// else. `move_down` is handed an already-capped length, so a longer
    /// one must be honoured rather than clamped — a regression here would
    /// silently restore the second copy of the cap.
    #[test]
    fn move_down_does_not_reapply_the_visible_cap() {
        let beyond_cap = theme::PALETTE_MAX_VISIBLE + 5;
        let mut state = PickerState::default();
        for _ in 0..beyond_cap {
            press_down(&mut state, beyond_cap);
        }
        assert_eq!(state.focused_index(), beyond_cap - 1);
    }

    #[test]
    fn visible_caps_the_row_count() {
        let labels: Vec<String> = (0..theme::PALETTE_MAX_VISIBLE + 7)
            .map(|i| format!("lane-{i}"))
            .collect();
        let state = PickerState::default();
        assert_eq!(state.visible(&labels).len(), theme::PALETTE_MAX_VISIBLE);
    }

    /// The cap is applied to the *ranking*, not to the input. Reversing
    /// the two — `fuzzy_match(query, &labels[..MAX])` — reads as a
    /// harmless optimisation and passes every other test here, while
    /// dropping the only match a user typed for whenever it sits past the
    /// cap in the caller's order. That is the "the palette can't find my
    /// command" report.
    #[test]
    fn the_cap_is_applied_after_the_ranking() {
        let last = theme::PALETTE_MAX_VISIBLE + 4;
        let mut labels: Vec<String> = (0..last).map(|i| format!("filler-{i}")).collect();
        labels.push("unique-target".to_string());
        let state = typed("target");
        assert_eq!(state.visible(&labels), vec![last]);
    }

    #[test]
    fn input_resets_the_selection() {
        let mut state = PickerState::default();
        state.focus(4);
        state.on_key("a", Some('a'), 5);
        assert_eq!(state.focused_index(), 0);

        state.focus(4);
        state.on_key("backspace", None, 5);
        assert_eq!(state.focused_index(), 0);
        assert_eq!(state.query(), "");
    }

    #[test]
    fn reset_clears_both_the_query_and_the_selection() {
        let mut state = typed("feat");
        state.focus(3);
        state.reset();
        assert_eq!(state.query(), "");
        assert_eq!(state.focused_index(), 0);
    }

    /// An index left dangling by a candidate swap is kept, not clamped;
    /// the safety net is that it reads back as `None` so Enter no-ops.
    #[test]
    fn an_out_of_range_focus_is_kept_and_reads_as_none() {
        let labels = ["daruda / main", "daruda / feat"];
        let mut state = PickerState::default();
        state.focus(9);
        assert_eq!(state.focused_index(), 9);
        assert!(state.visible(&labels).get(state.focused_index()).is_none());
    }

    #[test]
    fn on_key_maps_escape_and_enter_to_intents() {
        let mut state = PickerState::default();
        assert_eq!(state.on_key("escape", None, 3), PickerKey::Dismiss);
        assert_eq!(state.on_key("enter", None, 3), PickerKey::Confirm);
        // Neither touches the picker's own state.
        assert_eq!(state.query(), "");
        assert_eq!(state.focused_index(), 0);
    }

    /// The two overlay keys come from the shared mapping, so a picker
    /// with no list answers them identically to one with a list.
    #[test]
    fn overlay_key_owns_escape_and_enter_and_nothing_else() {
        assert_eq!(overlay_key("escape"), Some(PickerKey::Dismiss));
        assert_eq!(overlay_key("enter"), Some(PickerKey::Confirm));
        for key in ["up", "down", "backspace", "x", ""] {
            assert_eq!(overlay_key(key), None, "{key}");
        }
    }

    #[test]
    fn on_key_navigates_and_edits_in_place() {
        let mut state = typed("ab");
        assert_eq!(state.on_key("down", None, 3), PickerKey::Changed);
        assert_eq!(state.focused_index(), 1);
        assert_eq!(state.on_key("up", None, 3), PickerKey::Changed);
        assert_eq!(state.focused_index(), 0);
        assert_eq!(state.on_key("backspace", None, 3), PickerKey::Changed);
        assert_eq!(state.query(), "a");
    }

    #[test]
    fn on_key_appends_printable_ascii() {
        let mut state = PickerState::default();
        assert_eq!(state.on_key("x", Some('x'), 3), PickerKey::Changed);
        assert_eq!(state.on_key("space", Some(' '), 3), PickerKey::Changed);
        assert_eq!(state.on_key("slash", Some('/'), 3), PickerKey::Changed);
        assert_eq!(state.query(), "x /");
    }

    /// Pins today's behaviour on purpose: a non-ASCII character is
    /// dropped, so a Korean user cannot type Hangul into a picker even
    /// though the labels are localized (`locales/ko.yml` has
    /// `"레인 전환…"`). Not fixable here — the query is a key-down
    /// accumulator, and CLAUDE.md pitfall #4 forbids routing IME text
    /// through `on_key_down`. The real fix is adopting
    /// `crate::ui::input` / `InputState`, as `command/history.rs`
    /// already does.
    #[test]
    fn on_key_ignores_non_ascii_input() {
        let mut state = PickerState::default();
        assert_eq!(state.on_key("한", Some('한'), 3), PickerKey::Unchanged);
        assert_eq!(state.query(), "");
    }
}

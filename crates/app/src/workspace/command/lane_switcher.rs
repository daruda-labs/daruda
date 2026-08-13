//! Lane switcher — Cmd+P fuzzy quick-switch across every project's
//! lanes.
//!
//! Mirrors the command-palette overlay shape: a pure
//! [`LaneSwitcherState`] snapshot plus a [`RenderOnce`] overlay, so the
//! Workspace render path carries no state-transition logic. Candidates
//! are snapshotted from `Workspace::projects` when the switcher opens;
//! the overlay only reads that snapshot. Enter activates the focused
//! lane via `Workspace::activate_lane`.

use daruda_store::project::LaneRef;
use gpui::{
    App, IntoElement, MouseButton, MouseDownEvent, RenderOnce, SharedString, Window, div,
    prelude::*, px,
};

use crate::{fuzzy::fuzzy_match, surface::strings as s, ui::theme};
use std::rc::Rc;

/// One selectable lane, captured when the switcher opens so the overlay
/// render never reaches back into live project state.
#[derive(Clone)]
pub(in crate::workspace) struct LaneCandidate {
    pub lane_ref: LaneRef,
    /// Display label, e.g. `"daruda / feat-sidebar"`.
    pub label: String,
}

/// State for the Lane switcher overlay. `candidates` is the snapshot
/// taken at open time; `query` / `focused_index` drive filtering and
/// keyboard selection.
#[derive(Default, Clone)]
pub(in crate::workspace) struct LaneSwitcherState {
    pub is_open: bool,
    pub query: String,
    pub focused_index: usize,
    pub candidates: Vec<LaneCandidate>,
}

impl LaneSwitcherState {
    pub fn open(&mut self, candidates: Vec<LaneCandidate>) {
        self.is_open = true;
        self.query.clear();
        self.focused_index = 0;
        self.candidates = candidates;
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.query.clear();
        self.focused_index = 0;
        self.candidates = Vec::new();
    }

    pub fn append(&mut self, ch: char) {
        self.query.push(ch);
        self.focused_index = 0;
    }

    pub fn backspace(&mut self) {
        self.query.pop();
        self.focused_index = 0;
    }

    /// Move the focus to a row the mouse named. Clicking is the same
    /// gesture as arrowing there and pressing Enter, so it goes through the
    /// same field rather than a second path to the same decision.
    pub fn focus(&mut self, index: usize) {
        self.focused_index = index;
    }

    pub fn move_up(&mut self) {
        if self.focused_index > 0 {
            self.focused_index -= 1;
        }
    }

    pub fn move_down(&mut self, max: usize) {
        let cap = max.min(theme::PALETTE_MAX_VISIBLE);
        if cap > 0 && self.focused_index < cap - 1 {
            self.focused_index += 1;
        }
    }

    /// Candidate indices matching `query`, best match first. An empty
    /// query yields every candidate in original order.
    pub fn filtered(&self) -> Vec<usize> {
        let labels: Vec<&str> = self.candidates.iter().map(|c| c.label.as_str()).collect();
        fuzzy_match(&self.query, &labels)
    }

    /// The `LaneRef` of the currently focused row, if any.
    pub fn focused_lane_ref(&self) -> Option<LaneRef> {
        let filtered = self.filtered();
        filtered
            .get(self.focused_index)
            .map(|&i| self.candidates[i].lane_ref)
    }
}

/// GPUI render-once wrapper for the Lane switcher floating overlay.
/// Renders an empty invisible div when the switcher is closed.
#[derive(IntoElement)]
pub(in crate::workspace) struct LaneSwitcherOverlay {
    pub(in crate::workspace) state: LaneSwitcherState,
    #[allow(clippy::type_complexity)]
    pub(in crate::workspace) on_close:
        Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>,
    /// Activate the row at this visible index. `Rc` because every row needs
    /// its own handle to it.
    #[allow(clippy::type_complexity)]
    pub(in crate::workspace) on_pick: Rc<dyn Fn(&usize, &mut Window, &mut App) + 'static>,
}

impl LaneSwitcherOverlay {
    pub(in crate::workspace) fn new(
        state: LaneSwitcherState,
        on_close: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
        on_pick: impl Fn(&usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            state,
            on_close: Box::new(on_close),
            on_pick: Rc::new(on_pick),
        }
    }
}

/// Full-screen absolute overlay — click-to-dismiss hit target. Mirrors
/// the command palette's `backdrop()`; duplicated because that helper
/// is private to its module.
fn backdrop() -> gpui::Div {
    div().absolute().size_full().top_0().left_0()
}

impl RenderOnce for LaneSwitcherOverlay {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        if !self.state.is_open {
            return div().into_any_element();
        }
        let state = self.state;
        let on_close = self.on_close;
        let filtered = state.filtered();

        let t = theme::current(cx);
        let input_border = t.border;
        let query_text = t.text_primary;
        let focused_bg = t.palette_focused_bg;
        let focused_text = t.text_primary;
        let entry_text = t.text_body;
        let empty_text = t.text_subtle;
        let panel_bg = t.palette_bg;
        let panel_border = t.border;

        let input = div()
            .flex()
            .flex_row()
            .items_center()
            .w_full()
            .px(px(theme::PALETTE_INPUT_PAD_X))
            .py(px(theme::PALETTE_INPUT_PAD_Y))
            .border_b_1()
            .border_color(input_border)
            .child(
                div()
                    .text_size(px(theme::PALETTE_QUERY_FONT_SIZE))
                    .text_color(query_text)
                    .child(if state.query.is_empty() {
                        SharedString::from(s::command_switch_lane_placeholder())
                    } else {
                        SharedString::from(state.query.clone())
                    }),
            );

        let entries = div()
            .flex()
            .flex_col()
            .max_h(px(theme::PALETTE_MAX_HEIGHT))
            .overflow_hidden()
            .children(
                filtered
                    .iter()
                    .take(theme::PALETTE_MAX_VISIBLE)
                    .enumerate()
                    .map(|(vis_idx, &cand_idx)| {
                        let candidate = &state.candidates[cand_idx];
                        let is_focused = vis_idx == state.focused_index;
                        let on_pick = self.on_pick.clone();
                        div()
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                move |_: &MouseDownEvent, window, cx| {
                                    cx.stop_propagation();
                                    on_pick(&vis_idx, window, cx);
                                },
                            )
                            .hover(|d| d.bg(focused_bg))
                            .flex()
                            .flex_row()
                            .items_center()
                            .w_full()
                            .px(px(theme::PALETTE_ENTRY_PAD_X))
                            .py(px(theme::PALETTE_ENTRY_PAD_Y))
                            .text_size(px(theme::PALETTE_ENTRY_FONT_SIZE))
                            // Reserve the same-width transparent border on
                            // unfocused rows so the label does not shift when
                            // the accent rule appears — same idiom as the lane
                            // rows in the left dock.
                            .border_l(px(theme::PALETTE_FOCUS_BORDER_W))
                            .border_color(theme::TRANSPARENT)
                            .when(is_focused, |d| {
                                d.bg(focused_bg)
                                    .text_color(focused_text)
                                    .border_color(theme::PRIMARY)
                            })
                            .when(!is_focused, |d| d.text_color(entry_text))
                            .child(SharedString::from(candidate.label.clone()))
                    }),
            );

        let no_results = if filtered.is_empty() {
            Some(
                div()
                    .px(px(theme::PALETTE_ENTRY_PAD_X))
                    .py(px(theme::PALETTE_EMPTY_PAD_Y))
                    .text_size(px(theme::PALETTE_ENTRY_FONT_SIZE))
                    .text_color(empty_text)
                    .child(s::command_no_matching_lanes()),
            )
        } else {
            None
        };

        let panel = div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .mx_auto()
            .mt(px(theme::PALETTE_TOP_OFFSET))
            .w(px(theme::PALETTE_WIDTH))
            .bg(panel_bg)
            .border_1()
            .border_color(panel_border)
            .rounded(px(theme::PALETTE_RADIUS))
            .shadow_lg()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
            })
            .child(input)
            .child(entries)
            .when_some(no_results, |el, nr| el.child(nr));

        backdrop()
            .on_mouse_down(MouseButton::Left, on_close)
            .child(panel)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(project: u64, lane: u64, label: &str) -> LaneCandidate {
        LaneCandidate {
            lane_ref: LaneRef { project, lane },
            label: label.to_string(),
        }
    }

    fn opened(candidates: Vec<LaneCandidate>) -> LaneSwitcherState {
        let mut state = LaneSwitcherState::default();
        state.open(candidates);
        state
    }

    #[test]
    fn open_seeds_candidates_and_resets() {
        let state = opened(vec![candidate(1, 0, "a / main")]);
        assert!(state.is_open);
        assert_eq!(state.query, "");
        assert_eq!(state.focused_index, 0);
        assert_eq!(state.candidates.len(), 1);
    }

    #[test]
    fn close_clears_candidates() {
        let mut state = opened(vec![candidate(1, 0, "a / main")]);
        state.close();
        assert!(!state.is_open);
        assert!(state.candidates.is_empty());
    }

    #[test]
    fn filtered_empty_query_returns_all() {
        let state = opened(vec![
            candidate(1, 0, "daruda / main"),
            candidate(1, 1, "daruda / feat"),
        ]);
        assert_eq!(state.filtered(), vec![0, 1]);
    }

    #[test]
    fn filtered_narrows_to_query() {
        let mut state = opened(vec![
            candidate(1, 0, "daruda / main"),
            candidate(1, 1, "daruda / feat-login"),
        ]);
        state.append('l');
        state.append('o');
        state.append('g');
        // Only the "feat-login" lane carries the `log` subsequence.
        assert_eq!(state.filtered(), vec![1]);
    }

    #[test]
    fn focused_lane_ref_tracks_selection() {
        let mut state = opened(vec![
            candidate(1, 0, "daruda / main"),
            candidate(2, 5, "other / fix"),
        ]);
        assert_eq!(
            state.focused_lane_ref(),
            Some(LaneRef {
                project: 1,
                lane: 0
            })
        );
        state.move_down(2);
        assert_eq!(
            state.focused_lane_ref(),
            Some(LaneRef {
                project: 2,
                lane: 5
            })
        );
    }
}

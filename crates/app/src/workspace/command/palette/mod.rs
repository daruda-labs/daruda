//! Command palette — Cmd+Shift+P fuzzy action search overlay.
//!
//! Renders a centered input box at the top of the workspace. Typing
//! narrows the action list by subsequence match; Enter executes the
//! focused action; Escape closes. Query and selection live in the shared
//! [`PickerState`]; the row chrome is [`crate::ui::picker_row`]; the
//! command table itself is [`entries`].

pub(in crate::workspace) mod entries;

/// Re-exported so the command table keeps the path the 4-point chain rule
/// names (`command::palette::PALETTE_ENTRIES`, `crates/app/src/CLAUDE.md`).
pub(in crate::workspace) use entries::PALETTE_ENTRIES;

use super::picker::PickerState;
use crate::{surface::strings as s, ui::theme};
use gpui::{
    App, IntoElement, MouseButton, MouseDownEvent, RenderOnce, SharedString, Window, div,
    prelude::*, px,
};
use std::rc::Rc;

/// Every entry's label resolved, paired with its `PALETTE_ENTRIES`
/// index, sorted by the lowercased label.
///
/// The palette's one ordering decision, and the reason it cannot be
/// baked into `PALETTE_ENTRIES`: `label` is an i18n function, so the
/// order the reader sees exists only once the labels are resolved.
/// Lowercased `str::cmp` makes English case-insensitive and leaves
/// Hangul syllables in 가나다 order — no collation dependency.
///
/// Handing this pre-sorted slice to the matcher is also what makes
/// alphabetical the tiebreak: `fuzzy_match` keeps the input order for
/// equal scores, and an empty query scores everything equally.
fn sorted_labels() -> Vec<(usize, String)> {
    let mut labels: Vec<(usize, String)> = PALETTE_ENTRIES
        .iter()
        .enumerate()
        .map(|(i, entry)| (i, (entry.label)()))
        .collect();
    labels.sort_by_cached_key(|(_, label)| label.to_lowercase());
    labels
}

/// State for the command palette overlay. `is_open` stays here rather
/// than in [`PickerState`] — it is this view's own modal flag.
#[derive(Default, Clone)]
pub(in crate::workspace) struct CommandPaletteState {
    pub is_open: bool,
    pub picker: PickerState,
}

impl CommandPaletteState {
    pub fn open(&mut self) {
        self.is_open = true;
        self.picker.reset();
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.picker.reset();
    }

    /// `PALETTE_ENTRIES` indices for the rows actually drawn, best match
    /// first, alphabetical among equal scores.
    ///
    /// The single index space of the palette: the render path draws this
    /// in order and [`Self::focused_action_id`] indexes the same vector,
    /// so the highlighted row and the action Enter runs cannot drift
    /// apart. Any second, independently-sorted candidate list here would
    /// be exactly that bug.
    pub fn visible(&self) -> Vec<usize> {
        let sorted = sorted_labels();
        let labels: Vec<&str> = sorted.iter().map(|(_, label)| label.as_str()).collect();
        self.picker
            .visible(&labels)
            .into_iter()
            .map(|i| sorted[i].0)
            .collect()
    }

    /// Get the action id of the currently focused entry, if any.
    pub fn focused_action_id(&self) -> Option<&'static str> {
        self.visible()
            .get(self.picker.focused_index())
            .map(|&i| PALETTE_ENTRIES[i].id)
    }
}

/// GPUI render-once wrapper for the command palette floating overlay.
/// Renders an empty invisible div when the palette is closed.
#[derive(IntoElement)]
pub(in crate::workspace) struct CommandPaletteOverlay {
    pub(in crate::workspace) state: CommandPaletteState,
    #[allow(clippy::type_complexity)]
    pub(in crate::workspace) on_close:
        Box<dyn Fn(&MouseDownEvent, &mut Window, &mut App) + 'static>,
    /// Activate the row at this visible index. `Rc` because every row needs
    /// its own handle to it.
    #[allow(clippy::type_complexity)]
    pub(in crate::workspace) on_pick: Rc<dyn Fn(&usize, &mut Window, &mut App) + 'static>,
}

impl CommandPaletteOverlay {
    pub(in crate::workspace) fn new(
        state: CommandPaletteState,
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

/// Full-screen absolute overlay — click-to-dismiss hit target for the
/// command palette.
fn backdrop() -> gpui::Div {
    div().absolute().size_full().top_0().left_0()
}

impl RenderOnce for CommandPaletteOverlay {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        if !self.state.is_open {
            return div().into_any_element();
        }
        let state = self.state;
        let on_close = self.on_close;
        let visible = state.visible();

        let t = theme::current(cx);
        let input_border = t.border;
        let query_text = t.text_primary;
        let shortcut_text = t.text_subtle;
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
                    .child(if state.picker.query().is_empty() {
                        SharedString::from(s::command_type_command_placeholder())
                    } else {
                        SharedString::from(state.picker.query().to_string())
                    }),
            );

        let entries = div()
            .flex()
            .flex_col()
            .max_h(px(theme::PALETTE_MAX_HEIGHT))
            .overflow_hidden()
            .children(visible.iter().enumerate().map(|(vis_idx, &entry_idx)| {
                let entry = &PALETTE_ENTRIES[entry_idx];
                let shortcut = (!entry.shortcut.is_empty()).then(|| {
                    div()
                        .text_size(px(theme::PALETTE_SHORTCUT_FONT_SIZE))
                        .text_color(shortcut_text)
                        .child(entry.shortcut)
                        .into_any_element()
                });
                let on_pick = self.on_pick.clone();
                crate::ui::picker_row(
                    vis_idx == state.picker.focused_index(),
                    SharedString::from((entry.label)()),
                    shortcut,
                    move |window, cx| on_pick(&vis_idx, window, cx),
                    cx,
                )
            }));

        let no_results = visible
            .is_empty()
            .then(|| crate::ui::picker_empty(s::command_no_matching_commands().into(), cx));

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

    fn typed(query: &str) -> CommandPaletteState {
        let mut state = CommandPaletteState::default();
        state.open();
        for ch in query.chars() {
            let visible_len = state.visible().len();
            state.picker.on_key(&ch.to_string(), Some(ch), visible_len);
        }
        state
    }

    /// One arrow-down press, the way the key handler makes it.
    fn press_down(state: &mut CommandPaletteState) {
        let visible_len = state.visible().len();
        state.picker.on_key("down", None, visible_len);
    }

    /// Labels + ids in the order the palette must present them, derived
    /// here independently of `sorted_labels` so a change to the ordering
    /// rule shows up as a failure rather than moving both sides at once.
    fn alphabetical_entries() -> Vec<(String, &'static str)> {
        let mut entries: Vec<(String, &'static str)> = PALETTE_ENTRIES
            .iter()
            .map(|entry| ((entry.label)(), entry.id))
            .collect();
        entries.sort_by_cached_key(|(label, _)| label.to_lowercase());
        entries
    }

    fn label_of(id: &str) -> String {
        let entry = PALETTE_ENTRIES
            .iter()
            .find(|entry| entry.id == id)
            .expect("palette entry exists");
        (entry.label)()
    }

    #[test]
    fn palette_open_close_lifecycle() {
        let state = CommandPaletteState::default();
        assert!(!state.is_open);
        assert!(state.picker.query().is_empty());

        let mut state = typed("old");
        state.open();
        assert!(state.is_open);
        assert!(state.picker.query().is_empty());
        assert_eq!(state.picker.focused_index(), 0);

        let mut state = typed("a");
        state.picker.focus(3);
        state.close();
        assert!(!state.is_open);
        assert!(state.picker.query().is_empty());
        assert_eq!(state.picker.focused_index(), 0);
    }

    /// A query that is only a subsequence of the label — no substring
    /// anywhere in it — still reaches the command. This is the whole
    /// point of the matcher swap.
    #[test]
    fn a_subsequence_query_finds_the_command() {
        let state = typed("mvproj");
        let ids: Vec<&str> = state
            .visible()
            .iter()
            .map(|&i| PALETTE_ENTRIES[i].id)
            .collect();
        assert!(
            ids.contains(&"move_project_to_group"),
            "`mvproj` should reach {:?}, got {ids:?}",
            label_of("move_project_to_group")
        );
        assert!(
            !label_of("move_project_to_group")
                .to_lowercase()
                .contains("mvproj"),
            "the query must not be a substring, or this proves nothing"
        );
    }

    /// With no query every candidate scores equally, so the order is
    /// whatever `sorted_labels` handed the matcher — the alphabetically
    /// first screenful, not the declaration order of `PALETTE_ENTRIES`.
    #[test]
    fn an_empty_query_lists_commands_alphabetically() {
        let state = typed("");
        let shown: Vec<String> = state
            .visible()
            .iter()
            .map(|&i| (PALETTE_ENTRIES[i].label)())
            .collect();
        let expected: Vec<String> = alphabetical_entries()
            .into_iter()
            .take(theme::PALETTE_MAX_VISIBLE)
            .map(|(label, _)| label)
            .collect();
        assert_eq!(shown, expected);
    }

    /// P1: the row the render path highlights and the action Enter runs
    /// must be the same entry. Render walks `visible()` in order and
    /// highlights `focused_index`; `focused_action_id` reads the same
    /// vector at the same offset. Asserted on the action id — a second,
    /// differently-ordered candidate list on either path would make the
    /// highlighted label and the executed action disagree.
    #[test]
    fn the_highlighted_row_is_the_action_enter_runs() {
        // Value-based, independent of `sorted_labels`: two rows down from
        // an empty query is the third alphabetical command.
        let alphabetical = alphabetical_entries();
        let mut state = typed("");
        press_down(&mut state);
        press_down(&mut state);
        assert_eq!(state.focused_action_id(), Some(alphabetical[2].1));

        // The same identity holds for a narrowed list, and at every row of
        // it: what the render draws at `focused_index` is what Enter runs.
        for query in ["", "split", "settings", "mvproj"] {
            let mut state = typed(query);
            let visible = state.visible();
            assert!(!visible.is_empty(), "query {query:?} matched nothing");
            for row in 0..visible.len() {
                state.picker.focus(row);
                let drawn = (PALETTE_ENTRIES[visible[row]].label)();
                let run = state.focused_action_id().map(label_of);
                assert_eq!(run.as_deref(), Some(drawn.as_str()), "query {query:?}");
            }
        }
    }

    #[test]
    fn palette_filter_cases() {
        // An empty query offers every entry, capped at one screenful.
        let state = typed("");
        assert_eq!(state.visible().len(), theme::PALETTE_MAX_VISIBLE);

        // A prefix query ranks the entries that start with it first.
        let state = typed("split");
        let ids: Vec<&str> = state
            .visible()
            .iter()
            .map(|&i| PALETTE_ENTRIES[i].id)
            .collect();
        assert_eq!(
            ids.iter().take(2).copied().collect::<Vec<_>>(),
            vec!["split_down", "split_right"],
            "the two Split commands should outrank incidental subsequence hits: {ids:?}"
        );

        // Smart-case: a lowercase query is case-insensitive.
        let state = typed("quit");
        assert!(
            state
                .visible()
                .iter()
                .any(|&i| PALETTE_ENTRIES[i].id == "quit")
        );

        // Nothing matches — the overlay shows its empty row.
        assert!(typed("zzzzzzz").visible().is_empty());
    }

    #[test]
    fn palette_editing_and_focus_movement_cases() {
        // Navigation and editing are `PickerState`'s, including the
        // visible-row cap; the palette only supplies the row count.
        let mut state = typed("");
        let visible_len = state.visible().len();
        for _ in 0..100 {
            press_down(&mut state);
        }
        assert_eq!(state.picker.focused_index(), visible_len - 1);
        assert_eq!(visible_len, theme::PALETTE_MAX_VISIBLE);

        let mut state = typed("");
        state.picker.on_key("up", None, state.visible().len());
        assert_eq!(state.picker.focused_index(), 0);

        let mut state = typed("ab");
        state
            .picker
            .on_key("backspace", None, state.visible().len());
        assert_eq!(state.picker.query(), "a");

        let mut state = typed("");
        state.picker.focus(5);
        state.picker.on_key("x", Some('x'), state.visible().len());
        assert_eq!(state.picker.focused_index(), 0);
    }

    #[test]
    fn palette_entries_have_unique_ids() {
        let mut ids: Vec<&str> = PALETTE_ENTRIES.iter().map(|e| e.id).collect();
        let len = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), len, "duplicate palette entry IDs found");
    }
}

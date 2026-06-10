//! Character-level text-selection model for the file viewer (GPUI-free).
//!
//! `CharPos` / `CharSelection` / `SelectionDrag` encode the selection state, and
//! the `PaneFileView` mouse/drag handlers that mutate it live alongside them.
//! Rendering and workspace ops reach these types through the re-exports in the
//! parent module.

use std::ops::Range;

use super::{FileViewMode, PaneFileContent, PaneFileView};

/// A byte position within a visual row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) struct CharPos {
    pub row: usize,
    /// UTF-8 byte offset within `VisualRow::content`.
    pub byte: usize,
}

/// A character-level selection range. `anchor` is fixed; `active` moves with
/// the cursor during drag. Either end may come first — use `ordered()` to
/// get `(start, end)` in document order.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::workspace) struct CharSelection {
    pub anchor: CharPos,
    pub active: CharPos,
}

impl CharSelection {
    /// True when the anchor and active ends coincide (zero-width selection).
    pub(in crate::workspace) fn is_empty(&self) -> bool {
        self.anchor == self.active
    }

    /// Return `(start, end)` in document order (start.row ≤ end.row).
    pub(in crate::workspace) fn ordered(&self) -> (&CharPos, &CharPos) {
        if self.anchor.row < self.active.row
            || (self.anchor.row == self.active.row && self.anchor.byte <= self.active.byte)
        {
            (&self.anchor, &self.active)
        } else {
            (&self.active, &self.anchor)
        }
    }

    /// The selected byte range within `row`, or `None` when the row is not selected.
    ///
    /// `row_len` is `VisualRow::content.len()`. Bytes are clamped to `row_len`
    /// so out-of-bounds anchors from a previous content update are harmless.
    pub(in crate::workspace) fn byte_range_for_row(
        &self,
        row: usize,
        row_len: usize,
    ) -> Option<Range<usize>> {
        let (start, end) = self.ordered();
        if row < start.row || row > end.row {
            return None;
        }
        let start_byte = if row == start.row {
            start.byte.min(row_len)
        } else {
            0
        };
        let end_byte = if row == end.row {
            end.byte.min(row_len)
        } else {
            row_len
        };
        if start_byte >= end_byte {
            return None;
        }
        Some(start_byte..end_byte)
    }
}

/// File-viewer text-selection drag state. Encodes the three valid states the
/// three former `bool`/`Option` fields allowed only by convention.
#[derive(Default, Clone, Debug, PartialEq)]
pub(in crate::workspace) enum SelectionDrag {
    #[default]
    None,
    /// Button held, dragging. `sel.anchor` fixed, `sel.active` tracks the cursor.
    InProgress(CharSelection),
    /// Button released but anchor retained — shift+click can extend from `sel.anchor`.
    Complete(CharSelection),
}

impl SelectionDrag {
    /// The current selection range, regardless of drag phase. `None` when there
    /// is no selection (Cmd+C then copies all visible rows).
    pub(in crate::workspace) fn char_selection(&self) -> Option<&CharSelection> {
        match self {
            Self::InProgress(sel) | Self::Complete(sel) => Some(sel),
            Self::None => None,
        }
    }

    /// The fixed end of the current selection, or `None` when there is none.
    pub(in crate::workspace) fn anchor(&self) -> Option<CharPos> {
        self.char_selection().map(|s| s.anchor)
    }

    /// True while the left button is held (drag-select in progress).
    pub(in crate::workspace) fn is_in_progress(&self) -> bool {
        matches!(self, Self::InProgress(_))
    }
}

impl PaneFileView {
    /// Apply a mouse-down hit. `shift=true` extends the existing selection
    /// from the retained anchor (or `hit` if no anchor) to `hit` and settles
    /// immediately — a shift+click adjusts the selection without starting a
    /// drag. Otherwise resets anchor + selection to `hit` and starts a drag.
    /// Mirrors [`Self::handle_block_mouse_down`]. Caller is responsible for
    /// `cx.notify()` afterwards.
    pub(in crate::workspace) fn handle_mouse_down(&mut self, hit: CharPos, shift: bool) {
        self.selection_drag = if shift {
            let anchor = self.selection_drag.anchor().unwrap_or(hit);
            SelectionDrag::Complete(CharSelection {
                anchor,
                active: hit,
            })
        } else {
            SelectionDrag::InProgress(CharSelection {
                anchor: hit,
                active: hit,
            })
        };
    }

    /// Apply a mouse-move event during (or after) a drag-select.
    /// Returns `true` when internal state changed so the caller can
    /// decide whether to `cx.notify()`. Branch order:
    ///   1. not in progress → noop (false)
    ///   2. button released → settle to `Complete` (or `None` if empty) (true)
    ///   3. cursor outside hitbox → noop (false)
    ///   4. new selection differs from current → set (true)
    ///   5. otherwise → noop (false)
    pub(in crate::workspace) fn handle_mouse_drag(
        &mut self,
        active: CharPos,
        still_pressed: bool,
        hovered: bool,
    ) -> bool {
        if !self.selection_drag.is_in_progress() {
            return false;
        }
        if !still_pressed {
            // Release detected on a (possibly lazy) move event. Settle to the
            // last confirmed position via the shared path — never adopt this
            // event's `active`, which may be wherever the cursor drifted after
            // the button came up (the file viewer has no pane-local mouse-up
            // handler; the workspace-level one also routes through here).
            return self.end_selection_drag();
        }
        if !hovered {
            return false;
        }
        let Some(anchor) = self.selection_drag.anchor() else {
            return false;
        };
        let new_sel = CharSelection { anchor, active };
        if self.selection_drag.char_selection() != Some(&new_sel) {
            self.selection_drag = SelectionDrag::InProgress(new_sel);
            return true;
        }
        false
    }

    /// Select every visible row (Cmd+A). Anchors at the first row and extends
    /// past the end of the last row. Returns `true` when a selection was made
    /// so the caller can `cx.notify()`; `false` when there is nothing to select.
    pub(in crate::workspace) fn select_all(&mut self) -> bool {
        let n = self.visible_row_count();
        if n == 0 {
            return false;
        }
        self.selection_drag = SelectionDrag::Complete(CharSelection {
            anchor: CharPos { row: 0, byte: 0 },
            active: CharPos {
                row: n - 1,
                byte: usize::MAX,
            },
        });
        true
    }

    /// Settle an in-progress drag on button release: a non-empty range becomes
    /// `Complete` (anchor retained for shift+click), an empty range collapses to
    /// `None`. Returns `true` when state changed; a no-op when not dragging.
    pub(in crate::workspace) fn end_selection_drag(&mut self) -> bool {
        if !self.selection_drag.is_in_progress() {
            return false;
        }
        self.selection_drag = match self.selection_drag.char_selection() {
            Some(sel) if !sel.is_empty() => SelectionDrag::Complete(sel.clone()),
            _ => SelectionDrag::None,
        };
        true
    }

    /// Block-level mouse-down for the Markdown preview (selection is row-granular,
    /// `byte` is always 0). `shift=true` extends from the retained anchor and
    /// completes immediately; otherwise it starts a fresh in-progress drag.
    pub(in crate::workspace) fn handle_block_mouse_down(&mut self, block_idx: usize, shift: bool) {
        let pos = CharPos {
            row: block_idx,
            byte: 0,
        };
        self.selection_drag = if shift {
            let anchor = self.selection_drag.anchor().unwrap_or(pos);
            SelectionDrag::Complete(CharSelection {
                anchor,
                active: pos,
            })
        } else {
            SelectionDrag::InProgress(CharSelection {
                anchor: pos,
                active: pos,
            })
        };
    }

    /// Block-level mouse-move for the Markdown preview. While the left button is
    /// held the active end tracks `block_idx`; once released the drag settles via
    /// [`Self::end_selection_drag`]. Returns `true` when state changed.
    pub(in crate::workspace) fn handle_block_mouse_move(
        &mut self,
        block_idx: usize,
        left_pressed: bool,
    ) -> bool {
        if !self.selection_drag.is_in_progress() {
            return false;
        }
        if !left_pressed {
            return self.end_selection_drag();
        }
        let Some(anchor) = self.selection_drag.anchor() else {
            return false;
        };
        let active = CharPos {
            row: block_idx,
            byte: 0,
        };
        let new_sel = CharSelection { anchor, active };
        if self.selection_drag.char_selection() != Some(&new_sel) {
            self.selection_drag = SelectionDrag::InProgress(new_sel);
            return true;
        }
        false
    }

    /// Number of selectable units. Used by Cmd+A select-all.
    pub(in crate::workspace) fn visible_row_count(&self) -> usize {
        if let PaneFileContent::LoadedMarkdown { blocks, .. } = &self.content
            && self.view_mode == FileViewMode::Preview
        {
            return blocks.len();
        }
        self.active_rows().len()
    }
}

#[cfg(test)]
mod tests {
    use super::{CharPos, CharSelection, SelectionDrag};
    use crate::workspace::main_area::file_view_pane::{
        FileViewMode, PaneFileContent, PaneFileView, VisualRow, VisualRowKind,
    };

    fn raw_viewer(_contents: &[&str]) -> PaneFileView {
        PaneFileView {
            lane_id: 0,
            path: "test.txt".into(),
            staged: false,
            file_status: None,
            content: PaneFileContent::LoadedRaw,
            view_mode: FileViewMode::Raw,
            hide_unchanged: false,
            selection_drag: SelectionDrag::None,
            search: None,
        }
    }

    fn diff_viewer(contents: &[&str]) -> PaneFileView {
        let rows_all: Vec<VisualRow> = contents
            .iter()
            .enumerate()
            .map(|(i, s)| VisualRow {
                kind: VisualRowKind::Context,
                line_no_left: (i + 1).to_string(),
                line_no_right: (i + 1).to_string(),
                content: s.to_string(),
                header_context: String::new(),
                spans: Vec::new(),
                word_changes: Vec::new(),
            })
            .collect();
        PaneFileView {
            lane_id: 0,
            path: "test.diff".into(),
            staged: false,
            file_status: None,
            content: PaneFileContent::LoadedDiff {
                rows_all,
                rows_no_ctx: Vec::new(),
                added: 0,
                removed: 0,
            },
            view_mode: FileViewMode::Changes,
            hide_unchanged: false,
            selection_drag: SelectionDrag::None,
            search: None,
        }
    }

    // ------------------------------------------------------------
    // Mouse-down / mouse-drag state transitions
    // ------------------------------------------------------------

    #[test]
    fn mouse_down_clears_anchor_and_starts_drag() {
        let mut fv = raw_viewer(&["hello world"]);
        fv.handle_mouse_down(CharPos { row: 0, byte: 5 }, false);
        assert_eq!(
            fv.selection_drag.anchor(),
            Some(CharPos { row: 0, byte: 5 })
        );
        assert_eq!(
            fv.selection_drag,
            SelectionDrag::InProgress(CharSelection {
                anchor: CharPos { row: 0, byte: 5 },
                active: CharPos { row: 0, byte: 5 },
            })
        );
        assert!(fv.selection_drag.is_in_progress());
    }

    #[test]
    fn shift_click_extends_selection_from_retained_anchor() {
        let mut fv = raw_viewer(&["hello world"]);
        // Prime: click at (0, 0), drag to (0, 5), then release so the anchor at
        // (0, 0) is retained in a `Complete` state. This lets us observe
        // shift-click extending from that retained anchor.
        fv.handle_mouse_down(CharPos { row: 0, byte: 0 }, false);
        fv.handle_mouse_drag(CharPos { row: 0, byte: 5 }, true, true);
        fv.handle_mouse_drag(CharPos { row: 0, byte: 5 }, false, true);

        fv.handle_mouse_down(CharPos { row: 0, byte: 10 }, true);

        assert_eq!(
            fv.selection_drag.anchor(),
            Some(CharPos { row: 0, byte: 0 }),
            "shift-click extends from the retained anchor"
        );
        assert_eq!(
            fv.selection_drag.char_selection().map(|s| s.active),
            Some(CharPos { row: 0, byte: 10 })
        );
        assert!(
            !fv.selection_drag.is_in_progress(),
            "shift-click settles immediately and does not start a drag"
        );
    }

    #[test]
    fn drag_release_settles_to_complete() {
        let mut fv = raw_viewer(&["hello world"]);
        fv.handle_mouse_down(CharPos { row: 0, byte: 3 }, false);
        // Extend the active end while the button is held.
        fv.handle_mouse_drag(CharPos { row: 0, byte: 7 }, true, true);
        // Button released — settle at the last confirmed end.
        let changed = fv.handle_mouse_drag(CharPos { row: 0, byte: 7 }, false, true);
        assert!(changed, "releasing must report state changed");
        assert!(!fv.selection_drag.is_in_progress());
        assert_eq!(
            fv.selection_drag,
            SelectionDrag::Complete(CharSelection {
                anchor: CharPos { row: 0, byte: 3 },
                active: CharPos { row: 0, byte: 7 },
            })
        );
    }

    #[test]
    fn release_uses_last_confirmed_position_not_release_event() {
        let mut fv = raw_viewer(&["hello world"]);
        fv.handle_mouse_down(CharPos { row: 0, byte: 1 }, false);
        // Confirmed drag end while the button is held: byte 5.
        fv.handle_mouse_drag(CharPos { row: 0, byte: 5 }, true, true);
        // A lazy release-move reports byte 9 — it must NOT be adopted.
        fv.handle_mouse_drag(CharPos { row: 0, byte: 9 }, false, true);
        assert_eq!(
            fv.selection_drag,
            SelectionDrag::Complete(CharSelection {
                anchor: CharPos { row: 0, byte: 1 },
                active: CharPos { row: 0, byte: 5 },
            }),
            "release settles to the last in-hitbox position, not the release-move byte"
        );
    }

    #[test]
    fn plain_click_without_drag_clears_to_none() {
        let mut fv = raw_viewer(&["hello world"]);
        fv.handle_mouse_down(CharPos { row: 0, byte: 3 }, false);
        // Release-move at a different byte with no intervening pressed drag:
        // the click never produced a confirmed range, so no selection remains.
        fv.handle_mouse_drag(CharPos { row: 0, byte: 8 }, false, true);
        assert_eq!(
            fv.selection_drag,
            SelectionDrag::None,
            "a click with no confirmed drag leaves no selection"
        );
    }

    #[test]
    fn drag_outside_hitbox_does_not_update_selection() {
        let mut fv = raw_viewer(&["hello world"]);
        fv.handle_mouse_down(CharPos { row: 0, byte: 3 }, false);
        let baseline = fv.selection_drag.clone();

        // drag while not hovered.
        let changed = fv.handle_mouse_drag(CharPos { row: 0, byte: 50 }, true, false);
        assert!(!changed, "out-of-hitbox drag must not change state");
        assert_eq!(fv.selection_drag, baseline, "selection unchanged");
        assert!(
            fv.selection_drag.is_in_progress(),
            "drag still in progress while button held"
        );
    }

    // ------------------------------------------------------------
    // select_all / end_selection_drag / block-level selection
    // ------------------------------------------------------------

    #[test]
    fn select_all_spans_all_visible_rows() {
        let mut fv = diff_viewer(&["a", "b", "c"]);
        assert!(fv.select_all(), "select-all reports a change");
        assert_eq!(
            fv.selection_drag,
            SelectionDrag::Complete(CharSelection {
                anchor: CharPos { row: 0, byte: 0 },
                active: CharPos {
                    row: 2,
                    byte: usize::MAX,
                },
            })
        );
    }

    #[test]
    fn select_all_noop_when_no_rows() {
        let mut fv = diff_viewer(&[]);
        assert!(!fv.select_all(), "no rows → no change");
        assert_eq!(fv.selection_drag, SelectionDrag::None);
    }

    #[test]
    fn end_selection_drag_settles_nonempty_to_complete() {
        let mut fv = raw_viewer(&["hello world"]);
        fv.handle_mouse_down(CharPos { row: 0, byte: 0 }, false);
        fv.handle_mouse_drag(CharPos { row: 0, byte: 5 }, true, true);
        assert!(fv.selection_drag.is_in_progress());

        assert!(fv.end_selection_drag(), "settling reports a change");
        assert_eq!(
            fv.selection_drag,
            SelectionDrag::Complete(CharSelection {
                anchor: CharPos { row: 0, byte: 0 },
                active: CharPos { row: 0, byte: 5 },
            })
        );
    }

    #[test]
    fn end_selection_drag_empty_becomes_none() {
        let mut fv = raw_viewer(&["hello world"]);
        // mouse-down at a single point → in-progress but zero-width.
        fv.handle_mouse_down(CharPos { row: 0, byte: 3 }, false);
        assert!(fv.end_selection_drag());
        assert_eq!(fv.selection_drag, SelectionDrag::None);
    }

    #[test]
    fn end_selection_drag_noop_when_not_in_progress() {
        let mut fv = raw_viewer(&["hello world"]);
        assert!(!fv.end_selection_drag(), "no drag in progress → no change");
        assert_eq!(fv.selection_drag, SelectionDrag::None);
    }

    #[test]
    fn block_mouse_down_starts_in_progress() {
        let mut fv = raw_viewer(&["x"]);
        fv.handle_block_mouse_down(2, false);
        assert_eq!(
            fv.selection_drag,
            SelectionDrag::InProgress(CharSelection {
                anchor: CharPos { row: 2, byte: 0 },
                active: CharPos { row: 2, byte: 0 },
            })
        );
    }

    #[test]
    fn block_mouse_down_shift_extends_to_complete() {
        let mut fv = raw_viewer(&["x"]);
        // Prime an anchor at block 1.
        fv.handle_block_mouse_down(1, false);
        // Shift+click block 4 extends from the retained anchor and completes.
        fv.handle_block_mouse_down(4, true);
        assert_eq!(
            fv.selection_drag,
            SelectionDrag::Complete(CharSelection {
                anchor: CharPos { row: 1, byte: 0 },
                active: CharPos { row: 4, byte: 0 },
            })
        );
    }

    #[test]
    fn block_mouse_move_updates_active_while_pressed() {
        let mut fv = raw_viewer(&["x"]);
        fv.handle_block_mouse_down(0, false);
        assert!(fv.handle_block_mouse_move(3, true));
        assert_eq!(
            fv.selection_drag,
            SelectionDrag::InProgress(CharSelection {
                anchor: CharPos { row: 0, byte: 0 },
                active: CharPos { row: 3, byte: 0 },
            })
        );
    }

    #[test]
    fn block_mouse_move_settles_when_button_released() {
        let mut fv = raw_viewer(&["x"]);
        fv.handle_block_mouse_down(0, false);
        fv.handle_block_mouse_move(3, true);
        // Button no longer held → settle to Complete.
        assert!(fv.handle_block_mouse_move(3, false));
        assert_eq!(
            fv.selection_drag,
            SelectionDrag::Complete(CharSelection {
                anchor: CharPos { row: 0, byte: 0 },
                active: CharPos { row: 3, byte: 0 },
            })
        );
    }

    #[test]
    fn block_mouse_move_noop_when_not_in_progress() {
        let mut fv = raw_viewer(&["x"]);
        assert!(!fv.handle_block_mouse_move(2, true));
        assert_eq!(fv.selection_drag, SelectionDrag::None);
    }
}

//! Data model and diff parser for the file viewer.
//!
//! The viewer replaces the terminal area in the focused pane only while
//! a file or diff is open. PTY processes keep running; removing the view
//! from the render tree does not affect Entity lifetime.
//!
//! Rendering lives in the sibling `render/` module.

mod diff_parser;
pub(in crate::workspace) mod file_content;
pub(in crate::workspace) mod highlighter;
pub(in crate::workspace) mod markdown_viewer;
pub(in crate::workspace) mod search_ops;
pub(in crate::workspace) mod visual;
pub(in crate::workspace) mod word_diff;

pub mod render;

pub(in crate::workspace) use diff_parser::{DiffHunk, DiffLine, parse_diff_hunks};

use std::ops::Range;
use std::path::PathBuf;

use daruda_store::project::LaneId;

// ----------------------------------------------------------------
// Character-level selection types (GPUI-free)
// ----------------------------------------------------------------

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

// ----------------------------------------------------------------
// Visual row — pre-computed flat render unit
// ----------------------------------------------------------------

/// A single syntax-highlighted text segment within a diff line.
#[derive(Clone)]
pub(in crate::workspace) struct HighlightedSpan {
    pub text: String,
    /// `None` means use the default text color for the row kind.
    pub color: Option<gpui::Hsla>,
}

/// A byte range within a `VisualRow::content` string that differs at the
/// word level vs. the adjacent Removed/Added line pair.
#[derive(Clone)]
pub(in crate::workspace) struct WordChange {
    pub start: usize,
    pub end: usize,
}

/// A single display row produced from either a raw file line or a diff line.
/// Built once at load time; the renderer and copy helpers consume this directly.
#[derive(Clone)]
pub(in crate::workspace) struct VisualRow {
    pub kind: VisualRowKind,
    /// Left line-number column (empty string when absent).
    pub line_no_left: String,
    /// Right line-number column (diff view only; empty string when absent).
    pub line_no_right: String,
    /// Display content — no marker prefix; that is added by the renderer.
    pub content: String,
    /// Trailing context text after `@@ -N,M +N,M @@` for HunkHeader rows.
    /// Empty for all other kinds.
    pub header_context: String,
    /// Syntax-highlighted spans. Empty means fall back to plain `content` text.
    pub spans: Vec<HighlightedSpan>,
    /// Word-level change byte ranges within `content` (Added/Removed rows only).
    pub word_changes: Vec<WordChange>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum VisualRowKind {
    Plain,
    HunkHeader,
    Context,
    Added,
    Removed,
    NoNewline,
}

impl VisualRow {
    /// Text placed in the clipboard for this row (marker prefix included).
    pub(in crate::workspace) fn copy_text(&self) -> String {
        match self.kind {
            VisualRowKind::Added => format!("+{}", self.content),
            VisualRowKind::Removed => format!("-{}", self.content),
            VisualRowKind::Context => format!(" {}", self.content),
            _ => self.content.clone(),
        }
    }
}

// ----------------------------------------------------------------
// Core types
// ----------------------------------------------------------------

/// Live state of the find panel inside the file viewer.
pub(in crate::workspace) struct FileViewerSearch {
    /// The current query string typed by the user.
    pub query: String,
    /// Row indices (into `active_rows()`) that contain the query string.
    pub matches: Vec<usize>,
    /// Index into `matches` that is currently highlighted.
    /// `None` when the query is empty or there are no matches.
    pub focused: Option<usize>,
}

pub(in crate::workspace) struct PaneFileView {
    pub lane_id: LaneId,
    pub path: PathBuf,
    pub staged: bool,
    /// Git status character for the file (M / A / D / R / ? …).
    /// Shown as a badge in the toolbar. `None` when opened without git context.
    pub file_status: Option<char>,
    pub content: PaneFileContent,
    pub view_mode: FileViewMode,
    pub hide_unchanged: bool,
    /// Character-level selection + drag state. `SelectionDrag::None` means no
    /// selection (Cmd+C copies all); the anchor is retained across mouse-up so
    /// subsequent shift+clicks extend from it.
    pub selection_drag: SelectionDrag,
    /// Active find-panel state. `None` when the panel is closed.
    pub search: Option<FileViewerSearch>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum FileViewMode {
    Raw,
    Preview,
    Changes,
}

pub(in crate::workspace) enum PaneFileContent {
    Loading,
    /// Raw file content — owned by the `InputState` editor entity, so the
    /// variant itself carries no data.
    LoadedRaw,
    /// Unified diff content.
    ///
    /// `rows_all` includes context lines; `rows_no_ctx` omits them.
    /// The renderer picks the right list based on `hide_unchanged`.
    /// `added` and `removed` are the total change-line counts (for LineStats).
    LoadedDiff {
        rows_all: Vec<VisualRow>,
        rows_no_ctx: Vec<VisualRow>,
        added: usize,
        removed: usize,
    },
    /// Parsed Markdown: preview blocks + raw rows (both built at load time).
    /// Preview mode renders `blocks`; Raw mode renders `raw_rows`.
    LoadedMarkdown {
        blocks: Vec<self::markdown_viewer::MdBlock>,
        raw_rows: Vec<VisualRow>,
        total_count: usize,
        byte_truncated: bool,
    },
    Error(String),
    Binary,
    Deleted,
}

// ----------------------------------------------------------------
// Row builders (called at load time, never at render time)
// ----------------------------------------------------------------

/// Build the flat row list for a raw file. Capped at `FILE_VIEWER_MAX_LINES`.
pub(in crate::workspace) fn build_raw_rows(lines: &[String]) -> Vec<VisualRow> {
    use crate::ui::theme;
    lines
        .iter()
        .take(theme::FILE_VIEWER_MAX_LINES)
        .enumerate()
        .map(|(i, line)| VisualRow {
            kind: VisualRowKind::Plain,
            line_no_left: (i + 1).to_string(),
            line_no_right: String::new(),
            content: line.clone(),
            header_context: String::new(),
            spans: Vec::new(),
            word_changes: Vec::new(),
        })
        .collect()
}

/// Build the flat row list for a unified diff.
/// When `hide_ctx` is true, `DiffLine::Context` rows are omitted and hunks
/// that contain only context lines are skipped entirely (no orphan headers).
pub(in crate::workspace) fn build_diff_rows(hunks: &[DiffHunk], hide_ctx: bool) -> Vec<VisualRow> {
    use crate::surface::strings::file_viewer_no_newline;
    let mut rows = Vec::new();
    for hunk in hunks {
        // When hiding context, skip hunks that have no non-context lines.
        if hide_ctx
            && hunk
                .lines
                .iter()
                .all(|l| matches!(l, DiffLine::Context { .. }))
        {
            continue;
        }
        rows.push(VisualRow {
            kind: VisualRowKind::HunkHeader,
            line_no_left: String::new(),
            line_no_right: String::new(),
            content: hunk.header.clone(),
            header_context: hunk.header_context.clone(),
            spans: Vec::new(),
            word_changes: Vec::new(),
        });
        for line in &hunk.lines {
            match line {
                DiffLine::Context { .. } if hide_ctx => {}
                DiffLine::Context {
                    old_no,
                    new_no,
                    content,
                    spans,
                } => {
                    rows.push(VisualRow {
                        kind: VisualRowKind::Context,
                        line_no_left: old_no.to_string(),
                        line_no_right: new_no.to_string(),
                        content: content.clone(),
                        header_context: String::new(),
                        spans: spans_to_row_spans(spans),
                        word_changes: Vec::new(),
                    });
                }
                DiffLine::Added {
                    new_no,
                    content,
                    spans,
                    word_changes,
                } => {
                    rows.push(VisualRow {
                        kind: VisualRowKind::Added,
                        line_no_left: String::new(),
                        line_no_right: new_no.to_string(),
                        content: content.clone(),
                        header_context: String::new(),
                        spans: spans_to_row_spans(spans),
                        word_changes: word_changes.clone(),
                    });
                }
                DiffLine::Removed {
                    old_no,
                    content,
                    spans,
                    word_changes,
                } => {
                    rows.push(VisualRow {
                        kind: VisualRowKind::Removed,
                        line_no_left: old_no.to_string(),
                        line_no_right: String::new(),
                        content: content.clone(),
                        header_context: String::new(),
                        spans: spans_to_row_spans(spans),
                        word_changes: word_changes.clone(),
                    });
                }
                DiffLine::NoNewline => {
                    rows.push(VisualRow {
                        kind: VisualRowKind::NoNewline,
                        line_no_left: String::new(),
                        line_no_right: String::new(),
                        content: file_viewer_no_newline().to_owned(),
                        header_context: String::new(),
                        spans: Vec::new(),
                        word_changes: Vec::new(),
                    });
                }
            }
        }
    }
    rows
}

/// Convert `&[HighlightedSpan]` to an owned `Vec<HighlightedSpan>`.
fn spans_to_row_spans(spans: &[HighlightedSpan]) -> Vec<HighlightedSpan> {
    spans.to_vec()
}

/// Count added and removed lines across all hunks.
pub(in crate::workspace) fn count_diff_stats(hunks: &[DiffHunk]) -> (usize, usize) {
    let mut added = 0usize;
    let mut removed = 0usize;
    for hunk in hunks {
        for line in &hunk.lines {
            match line {
                DiffLine::Added { .. } => added += 1,
                DiffLine::Removed { .. } => removed += 1,
                _ => {}
            }
        }
    }
    (added, removed)
}

// ----------------------------------------------------------------
// PaneFileView helpers (GPUI-free)
// ----------------------------------------------------------------

impl PaneFileView {
    /// Returns a slice over the currently visible rows (respects `hide_unchanged`).
    pub(in crate::workspace) fn active_rows(&self) -> &[VisualRow] {
        match &self.content {
            PaneFileContent::LoadedRaw => &[],
            PaneFileContent::LoadedDiff {
                rows_all,
                rows_no_ctx,
                ..
            } => {
                if self.hide_unchanged {
                    rows_no_ctx
                } else {
                    rows_all
                }
            }
            PaneFileContent::LoadedMarkdown { raw_rows, .. }
                if self.view_mode == FileViewMode::Raw =>
            {
                raw_rows
            }
            _ => &[],
        }
    }

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

    /// Open the search panel. Resets the query to empty if the panel was already open.
    pub(in crate::workspace) fn search_open(&mut self) {
        if self.search.is_none() {
            self.search = Some(FileViewerSearch {
                query: String::new(),
                matches: Vec::new(),
                focused: None,
            });
        }
    }

    /// Close the search panel.
    pub(in crate::workspace) fn search_close(&mut self) {
        self.search = None;
    }

    /// Update the search query from the TextInput widget and recompute matches.
    pub(in crate::workspace) fn search_update_query(&mut self, query: &str) {
        if let Some(s) = &mut self.search {
            s.query = query.to_string();
        }
        self.search_recompute();
    }

    /// Append a character to the search query and recompute matches.
    #[allow(dead_code)]
    pub(in crate::workspace) fn search_insert_char(&mut self, ch: char) {
        if let Some(s) = &mut self.search {
            s.query.push(ch);
        }
        self.search_recompute();
    }

    /// Remove the last character from the search query and recompute matches.
    #[allow(dead_code)]
    pub(in crate::workspace) fn search_backspace(&mut self) {
        if let Some(s) = &mut self.search {
            s.query.pop();
        }
        self.search_recompute();
    }

    /// Advance the focused match to the next one (wraps around).
    pub(in crate::workspace) fn search_next_match(&mut self) {
        if let Some(s) = &mut self.search {
            if s.matches.is_empty() {
                return;
            }
            s.focused = Some(match s.focused {
                None => 0,
                Some(i) => (i + 1) % s.matches.len(),
            });
        }
    }

    /// Move the focused match to the previous one (wraps around).
    pub(in crate::workspace) fn search_prev_match(&mut self) {
        if let Some(s) = &mut self.search {
            if s.matches.is_empty() {
                return;
            }
            s.focused = Some(match s.focused {
                None => s.matches.len().saturating_sub(1),
                Some(0) => s.matches.len() - 1,
                Some(i) => i - 1,
            });
        }
    }

    /// Clear the search query and all match state without closing the panel.
    #[allow(dead_code)]
    pub(in crate::workspace) fn search_clear(&mut self) {
        if let Some(s) = &mut self.search {
            s.query.clear();
            s.matches.clear();
            s.focused = None;
        }
    }

    /// Row index of the currently focused match, or `None`.
    pub(in crate::workspace) fn search_focused_row(&self) -> Option<usize> {
        let s = self.search.as_ref()?;
        let fi = s.focused?;
        s.matches.get(fi).copied()
    }

    /// Recompute which rows match the current query.
    fn search_recompute(&mut self) {
        let Some(mut s) = self.search.take() else {
            return;
        };
        s.matches.clear();
        s.focused = None;
        if !s.query.is_empty() {
            let query_lower = s.query.to_lowercase();

            // Preview mode: search block plain text (one match index = one block index).
            if let PaneFileContent::LoadedMarkdown { blocks, .. } = &self.content
                && self.view_mode == FileViewMode::Preview
            {
                for (i, block) in blocks.iter().enumerate() {
                    let text = self::markdown_viewer::md_block_plain_text(block);
                    if text.to_lowercase().contains(&query_lower) {
                        s.matches.push(i);
                    }
                }
            } else {
                let rows: &[VisualRow] = match &self.content {
                    PaneFileContent::LoadedRaw => &[],
                    PaneFileContent::LoadedDiff {
                        rows_all,
                        rows_no_ctx,
                        ..
                    } => {
                        if self.hide_unchanged {
                            rows_no_ctx
                        } else {
                            rows_all
                        }
                    }
                    PaneFileContent::LoadedMarkdown { raw_rows, .. } => raw_rows,
                    _ => &[],
                };
                for (i, row) in rows.iter().enumerate() {
                    if row.content.to_lowercase().contains(&query_lower) {
                        s.matches.push(i);
                    }
                }
            }

            if !s.matches.is_empty() {
                s.focused = Some(0);
            }
        }
        self.search = Some(s);
    }

    /// Text to put in the clipboard for Cmd+C.
    /// When there is no selection, copies all visible rows (or blocks for Markdown).
    pub(in crate::workspace) fn selected_text_for_copy(&self) -> String {
        // Markdown preview: block-level copy only — byte offsets don't apply to rendered blocks.
        if let PaneFileContent::LoadedMarkdown { blocks, .. } = &self.content
            && self.view_mode == FileViewMode::Preview
        {
            let n = blocks.len();
            if n == 0 {
                return String::new();
            }
            let (s, e) = self
                .selection_drag
                .char_selection()
                .map(|sel| {
                    let (start, end) = sel.ordered();
                    (start.row.min(n - 1), end.row.min(n - 1))
                })
                .unwrap_or((0, n - 1));
            return blocks[s..=e]
                .iter()
                .map(self::markdown_viewer::md_block_plain_text)
                .collect::<Vec<_>>()
                .join("\n\n");
        }

        let rows = self.active_rows();
        if rows.is_empty() {
            return String::new();
        }
        let n = rows.len();

        let Some(sel) = self.selection_drag.char_selection() else {
            // No selection: copy all rows with diff markers.
            return rows
                .iter()
                .map(|r| r.copy_text())
                .collect::<Vec<_>>()
                .join("\n");
        };

        let (start, end) = sel.ordered();
        let row_start = start.row.min(n - 1);
        let row_end = end.row.min(n - 1);

        let mut parts: Vec<String> = Vec::new();
        for (row_idx, row) in rows.iter().enumerate().take(row_end + 1).skip(row_start) {
            let content = &row.content;
            let Some(range) = sel.byte_range_for_row(row_idx, content.len()) else {
                continue;
            };
            // Guard against non-char-boundary byte positions from stale state.
            if content.is_char_boundary(range.start) && content.is_char_boundary(range.end) {
                parts.push(content[range].to_owned());
            } else {
                parts.push(content.to_owned());
            }
        }
        parts.join("\n")
    }
}

// ----------------------------------------------------------------
// Tests
// ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_plain_row(content: &str, line_no: usize) -> VisualRow {
        VisualRow {
            kind: VisualRowKind::Plain,
            line_no_left: line_no.to_string(),
            line_no_right: String::new(),
            content: content.to_owned(),
            header_context: String::new(),
            spans: Vec::new(),
            word_changes: Vec::new(),
        }
    }

    #[test]
    fn parse_diff_hunks_basic() {
        let diff = "\
diff --git a/foo.rs b/foo.rs
index abc..def 100644
--- a/foo.rs
+++ b/foo.rs
@@ -1,3 +1,4 @@
 fn main() {
-    println!(\"old\");
+    println!(\"new\");
+    println!(\"extra\");
 }
";
        let hunks = parse_diff_hunks(diff);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].old_start, 1);
        assert_eq!(hunks[0].header, "@@ -1,3 +1,4 @@");
        assert_eq!(hunks[0].header_context, "");
        let added = hunks[0]
            .lines
            .iter()
            .filter(|l| matches!(l, DiffLine::Added { .. }))
            .count();
        let removed = hunks[0]
            .lines
            .iter()
            .filter(|l| matches!(l, DiffLine::Removed { .. }))
            .count();
        assert_eq!(added, 2);
        assert_eq!(removed, 1);
    }

    #[test]
    fn parse_diff_hunks_header_context() {
        let diff = "@@ -5,3 +5,3 @@ fn bar() {\n-old\n+new\n";
        let hunks = parse_diff_hunks(diff);
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].header, "@@ -5,3 +5,3 @@");
        assert_eq!(hunks[0].header_context, "fn bar() {");
    }

    #[test]
    fn parse_diff_hunks_multiple() {
        let diff = "\
diff --git a/bar.rs b/bar.rs
--- a/bar.rs
+++ b/bar.rs
@@ -1,3 +1,3 @@
 a
-b
+B
 c
@@ -10,3 +10,3 @@
 x
-y
+Y
 z
";
        let hunks = parse_diff_hunks(diff);
        assert_eq!(hunks.len(), 2);
        assert_eq!(hunks[0].old_start, 1);
        assert_eq!(hunks[1].old_start, 10);
    }

    #[test]
    fn parse_diff_empty_diff() {
        let hunks = parse_diff_hunks("");
        assert!(hunks.is_empty());
    }

    #[test]
    fn build_raw_rows_line_numbers() {
        let lines: Vec<String> = vec!["alpha".into(), "beta".into(), "gamma".into()];
        let rows = build_raw_rows(&lines);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].line_no_left, "1");
        assert_eq!(rows[2].line_no_left, "3");
        assert!(matches!(rows[0].kind, VisualRowKind::Plain));
        assert!(rows[0].header_context.is_empty());
        assert!(rows[0].spans.is_empty());
    }

    #[test]
    fn build_diff_rows_hide_ctx() {
        let hunks = parse_diff_hunks("@@ -1,3 +1,3 @@\n context\n-old\n+new\n");
        let all = build_diff_rows(&hunks, false);
        let no_ctx = build_diff_rows(&hunks, true);
        // all: header + context + removed + added = 4
        assert_eq!(all.len(), 4);
        // no_ctx: header + removed + added = 3 (context skipped)
        assert_eq!(no_ctx.len(), 3);
        assert!(
            no_ctx
                .iter()
                .all(|r| !matches!(r.kind, VisualRowKind::Context))
        );
    }

    #[test]
    fn build_diff_rows_header_context_propagated() {
        let hunks = parse_diff_hunks("@@ -1,2 +1,2 @@ fn foo() {\n-old\n+new\n");
        let rows = build_diff_rows(&hunks, false);
        assert!(matches!(rows[0].kind, VisualRowKind::HunkHeader));
        assert_eq!(rows[0].content, "@@ -1,2 +1,2 @@");
        assert_eq!(rows[0].header_context, "fn foo() {");
    }

    #[test]
    fn count_diff_stats_basic() {
        let hunks = parse_diff_hunks("@@ -1,3 +1,4 @@\n ctx\n-old\n+new1\n+new2\n");
        let (added, removed) = count_diff_stats(&hunks);
        assert_eq!(added, 2);
        assert_eq!(removed, 1);
    }

    #[test]
    fn copy_text_markers() {
        let added = VisualRow {
            kind: VisualRowKind::Added,
            line_no_left: String::new(),
            line_no_right: "1".into(),
            content: "hello".into(),
            header_context: String::new(),
            spans: Vec::new(),
            word_changes: Vec::new(),
        };
        assert_eq!(added.copy_text(), "+hello");

        let removed = VisualRow {
            kind: VisualRowKind::Removed,
            line_no_left: "1".into(),
            line_no_right: String::new(),
            content: "world".into(),
            header_context: String::new(),
            spans: Vec::new(),
            word_changes: Vec::new(),
        };
        assert_eq!(removed.copy_text(), "-world");

        let ctx = VisualRow {
            kind: VisualRowKind::Context,
            line_no_left: "1".into(),
            line_no_right: "1".into(),
            content: "ctx".into(),
            header_context: String::new(),
            spans: Vec::new(),
            word_changes: Vec::new(),
        };
        assert_eq!(ctx.copy_text(), " ctx");
    }

    #[test]
    fn selected_text_for_copy_no_selection() {
        let hunks = parse_diff_hunks("@@ -1,2 +1,2 @@\n-old\n+new\n");
        let rows_all = build_diff_rows(&hunks, false);
        let rows_no_ctx = build_diff_rows(&hunks, true);
        let (added, removed) = count_diff_stats(&hunks);
        let fv = PaneFileView {
            lane_id: 0,
            path: "test.rs".into(),
            staged: false,
            file_status: None,
            content: PaneFileContent::LoadedDiff {
                rows_all,
                rows_no_ctx,
                added,
                removed,
            },
            view_mode: FileViewMode::Changes,
            hide_unchanged: false,
            selection_drag: SelectionDrag::None,
            search: None,
        };
        // No selection → all rows copied.
        let text = fv.selected_text_for_copy();
        assert!(text.contains("-old"));
        assert!(text.contains("+new"));
    }

    #[test]
    fn make_plain_row_helper() {
        let r = make_plain_row("foo", 1);
        assert_eq!(r.content, "foo");
        assert_eq!(r.line_no_left, "1");
        assert!(r.spans.is_empty());
        assert!(r.word_changes.is_empty());
    }

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

    #[test]
    fn search_open_and_close() {
        let mut fv = raw_viewer(&["alpha", "beta"]);
        assert!(fv.search.is_none());
        fv.search_open();
        assert!(fv.search.is_some());
        fv.search_close();
        assert!(fv.search.is_none());
    }

    #[test]
    fn search_clear_resets_query_and_matches() {
        let mut fv = diff_viewer(&["hello world", "foo bar"]);
        fv.search_open();
        "hello".chars().for_each(|c| fv.search_insert_char(c));
        {
            let s = fv.search.as_ref().unwrap();
            assert_eq!(s.query, "hello");
            assert!(!s.matches.is_empty());
        }
        fv.search_clear();
        let s = fv.search.as_ref().unwrap();
        assert_eq!(s.query, "");
        assert!(s.matches.is_empty());
        assert!(s.focused.is_none());
        // Panel remains open after clear.
        assert!(fv.search.is_some());
    }

    #[test]
    fn search_insert_and_backspace() {
        let mut fv = raw_viewer(&["hello world", "foo bar"]);
        fv.search_open();
        fv.search_insert_char('h');
        fv.search_insert_char('e');
        assert_eq!(fv.search.as_ref().unwrap().query, "he");
        fv.search_backspace();
        assert_eq!(fv.search.as_ref().unwrap().query, "h");
        fv.search_backspace();
        assert_eq!(fv.search.as_ref().unwrap().query, "");
        fv.search_backspace(); // no-op on empty
        assert_eq!(fv.search.as_ref().unwrap().query, "");
    }

    #[test]
    fn search_matches_rows() {
        let mut fv = diff_viewer(&["hello world", "nothing here", "hello again"]);
        fv.search_open();
        "hello".chars().for_each(|c| fv.search_insert_char(c));
        let s = fv.search.as_ref().unwrap();
        // rows 0 and 2 contain "hello"; row 1 does not
        assert_eq!(s.matches, vec![0, 2]);
        assert_eq!(s.focused, Some(0));
    }

    #[test]
    fn search_no_match() {
        let mut fv = raw_viewer(&["alpha", "beta"]);
        fv.search_open();
        "zzz".chars().for_each(|c| fv.search_insert_char(c));
        let s = fv.search.as_ref().unwrap();
        assert!(s.matches.is_empty());
        assert!(s.focused.is_none());
    }

    #[test]
    fn search_next_and_prev_match() {
        let mut fv = diff_viewer(&["aaa", "bbb", "aaa", "aaa"]);
        fv.search_open();
        fv.search_insert_char('a');
        // matches: [0, 2, 3], focused = Some(0)
        fv.search_next_match();
        assert_eq!(fv.search.as_ref().unwrap().focused, Some(1));
        fv.search_next_match();
        assert_eq!(fv.search.as_ref().unwrap().focused, Some(2));
        fv.search_next_match(); // wraps
        assert_eq!(fv.search.as_ref().unwrap().focused, Some(0));
        fv.search_prev_match();
        assert_eq!(fv.search.as_ref().unwrap().focused, Some(2));
    }

    #[test]
    fn search_focused_row_returns_correct_row_index() {
        let mut fv = diff_viewer(&["aaa", "bbb", "aaa"]);
        fv.search_open();
        fv.search_insert_char('a');
        // matches = [0, 2], focused = Some(0) → row 0
        assert_eq!(fv.search_focused_row(), Some(0));
        fv.search_next_match();
        // focused = Some(1) → row 2
        assert_eq!(fv.search_focused_row(), Some(2));
    }

    #[test]
    fn search_case_insensitive() {
        let mut fv = diff_viewer(&["Hello", "world", "HELLO"]);
        fv.search_open();
        "hello".chars().for_each(|c| fv.search_insert_char(c));
        let s = fv.search.as_ref().unwrap();
        assert_eq!(s.matches, vec![0, 2]);
    }

    #[test]
    fn search_recomputes_on_query_change() {
        let mut fv = diff_viewer(&["hello", "world"]);
        fv.search_open();
        "hello".chars().for_each(|c| fv.search_insert_char(c));
        assert_eq!(fv.search.as_ref().unwrap().matches, vec![0]);
        // clear query
        for _ in 0.."hello".len() {
            fv.search_backspace();
        }
        assert!(fv.search.as_ref().unwrap().matches.is_empty());
        assert!(fv.search.as_ref().unwrap().focused.is_none());
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

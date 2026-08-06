//! Data model and diff parser for the file viewer.
//!
//! The viewer replaces the terminal area in the focused pane only while
//! a file or diff is open. PTY processes keep running; removing the view
//! from the render tree does not affect Entity lifetime.
//!
//! Rendering lives in the sibling `render/` module.

pub(in crate::workspace) mod diff_editor;
mod diff_parser;
pub(in crate::workspace) mod file_content;
pub(in crate::workspace) mod highlighter;
pub(in crate::workspace) mod line_diff;
pub(in crate::workspace) mod markdown_viewer;
mod mermaid_contrast;
mod mermaid_label_geometry;
mod mermaid_label_stroke;
mod mermaid_node_contrast;
mod mermaid_text_measurer;
pub(in crate::workspace) mod mermaid_theme;
pub(in crate::workspace) mod search_ops;
mod search_state;
mod selection;
pub(in crate::workspace) mod visual;
pub(in crate::workspace) mod word_diff;

pub mod render;

#[cfg(test)]
mod tests;

pub(in crate::workspace) use diff_parser::{DiffHunk, DiffLine, parse_diff_hunks};
pub(in crate::workspace) use search_state::FileViewerSearch;
pub(in crate::workspace) use selection::{CharPos, CharSelection, SelectionDrag};

use std::path::PathBuf;

use daruda_store::project::LaneId;

// ----------------------------------------------------------------
// Visual row — pre-computed flat render unit
// ----------------------------------------------------------------

/// A single syntax-highlighted text segment within a diff line.
#[derive(Clone)]
pub(in crate::workspace) struct HighlightedSpan {
    pub text: String,
    /// `None` means use the default text color for the row kind.
    pub color: Option<gpui::Hsla>,
    /// Non-color channel (bold/italic) for this segment, from the palette.
    pub style: crate::ui::theme::TokenStyle,
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

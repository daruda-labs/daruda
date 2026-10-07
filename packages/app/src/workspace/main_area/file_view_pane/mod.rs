//! Data model for the file viewer; the diff rows it holds are
//! `daruda_content::diff`'s. The viewer replaces the terminal area in the
//! focused pane only while a file or diff is open — PTY processes keep
//! running. Rendering lives in the sibling `render/` module.

pub(in crate::workspace) mod diff_editor;
pub(in crate::workspace) mod file_content;
pub(super) mod highlighter;
mod image_source;
pub(in crate::workspace) mod images;
pub(super) mod markdown_viewer;
pub(in crate::workspace) mod mermaid_theme;
pub(super) mod search_ops;
mod search_state;
mod selection;
pub(in crate::workspace) use daruda_content::visual;

pub mod render;

#[cfg(test)]
mod tests;

pub(in crate::workspace) use daruda_content::diff::{
    DiffHunk, DiffLine, HighlightedSpan, VisualRow, VisualRowKind, WordChange, count_diff_stats,
    line_diff, parse_diff_hunks, word_diff,
};
pub(in crate::workspace) use search_state::FileViewerSearch;
pub(in crate::workspace) use selection::{CharPos, CharSelection, SelectionDrag};

use std::path::PathBuf;

use daruda_store::project::LaneId;

/// Maximum lines shown in the file viewer body before truncation.
pub(in crate::workspace) const FILE_VIEWER_MAX_LINES: usize = 2000;
/// Maximum bytes read from a file in Raw mode. Files larger than this are
/// truncated before line-splitting so the process never loads unbounded data.
pub(in crate::workspace) const FILE_VIEWER_MAX_BYTES: usize = 5 * 1024 * 1024;
/// Rows rendered above and below the visible viewport.
pub(in crate::workspace) const FILE_VIEWER_VIRTUAL_OVERSCAN: usize = 8;

// ----------------------------------------------------------------
// Core types
// ----------------------------------------------------------------

/// Which snapshot of a file a pane shows — and, in Changes mode, what its
/// diff compares. Part of a file pane's identity: the same path opened from
/// two sources is two panes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum DiffSource {
    /// The file on disk; its diff is index → working tree.
    WorkingTree,
    /// The staged blob; its diff is HEAD → index.
    Index,
    /// The file as of commit `to`; its diff is `from` → `to`. `old_path` is
    /// the rename source, needed in the pathspec for git to pair the two.
    /// `status` is git's letter for the file across that pair — fixed with
    /// the commits, so it stays true when the lane's refs move on.
    Range {
        from: String,
        to: String,
        old_path: Option<PathBuf>,
        status: char,
    },
}

impl DiffSource {
    /// The source a Git Changes row names: its staged flag is the whole of it.
    pub(in crate::workspace) fn from_staged(staged: bool) -> Self {
        if staged {
            Self::Index
        } else {
            Self::WorkingTree
        }
    }

    pub(in crate::workspace) fn is_index(&self) -> bool {
        matches!(self, Self::Index)
    }

    /// Whether an edit to this pane's text can be saved back: only the file on
    /// disk can — the index and a commit are snapshots, not buffers.
    pub(in crate::workspace) fn is_editable(&self) -> bool {
        matches!(self, Self::WorkingTree)
    }

    /// Whether the pane tracks the lane as it is now — the working tree or
    /// the index — rather than two fixed commits.
    pub(in crate::workspace) fn is_live(&self) -> bool {
        matches!(self, Self::WorkingTree | Self::Index)
    }

    /// Whether the pane's change is already past the working tree — staged
    /// or committed — which is what the status colour tells apart.
    pub(in crate::workspace) fn reads_as_committed(&self) -> bool {
        matches!(self, Self::Index | Self::Range { .. })
    }

    /// The status letter fixed with a range's commits; `None` for a live pane.
    pub(in crate::workspace) fn pinned_status(&self) -> Option<char> {
        match self {
            Self::Range { status, .. } => Some(*status),
            Self::WorkingTree | Self::Index => None,
        }
    }

    /// The persisted form: the legacy `staged` flag beside an optional range.
    pub(in crate::workspace) fn to_serialized(
        &self,
    ) -> (bool, Option<daruda_store::project::SerializedDiffRange>) {
        match self {
            Self::WorkingTree => (false, None),
            Self::Index => (true, None),
            Self::Range {
                from,
                to,
                old_path,
                status,
            } => (
                false,
                Some(daruda_store::project::SerializedDiffRange {
                    from: from.clone(),
                    to: to.clone(),
                    old_path: old_path.clone(),
                    status: *status,
                }),
            ),
        }
    }

    /// Read a persisted pane back. A range wins over `staged`: the two are
    /// written together only by a build that knew about ranges.
    pub(in crate::workspace) fn from_serialized(
        fc: &daruda_store::project::SerializedFileContent,
    ) -> Self {
        match &fc.range {
            Some(range) => Self::Range {
                from: range.from.clone(),
                to: range.to.clone(),
                old_path: range.old_path.clone(),
                status: range.status,
            },
            None => Self::from_staged(fc.staged),
        }
    }
}

pub(in crate::workspace) struct PaneFileView {
    pub lane_id: LaneId,
    pub path: PathBuf,
    pub source: DiffSource,
    /// Git's letter (M / A / D / R / ? …) for a change pending in the lane
    /// now, projected from its cached status. Always `None` for a range pane,
    /// whose letter is fixed with its commits — read [`Self::status`], never
    /// this field, to learn what to show.
    pub live_status: Option<char>,
    pub content: PaneFileContent,
    pub view_mode: FileViewMode,
    pub hide_unchanged: bool,
    /// Character-level selection + drag state. `SelectionDrag::None` means no
    /// selection (Cmd+C copies all); the anchor is retained across mouse-up so
    /// subsequent shift+clicks extend from it.
    pub selection_drag: SelectionDrag,
    /// Active find-panel state. `None` when the panel is closed.
    pub search: Option<FileViewerSearch>,
    /// One-shot 1-based source line to reveal after opening/loading from an
    /// external reference such as an agent-chat Markdown file link.
    pub pending_scroll_line: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::workspace) enum FileViewMode {
    Raw,
    Preview,
    Changes,
}

impl FileViewMode {
    pub(in crate::workspace) fn effective_for_path(
        requested: FileViewMode,
        path: &std::path::Path,
    ) -> Self {
        match (requested, is_markdown_path(path)) {
            (FileViewMode::Raw, true) => FileViewMode::Preview,
            (FileViewMode::Preview, false) => FileViewMode::Raw,
            _ => requested,
        }
    }
}

pub(in crate::workspace) enum PaneFileContent {
    Loading,
    /// Raw file content — owned by the `InputState` editor entity, so the
    /// variant carries only whether the text was cut at
    /// `FILE_VIEWER_MAX_BYTES`, which makes the buffer read-only: saving it
    /// would truncate the file.
    LoadedRaw {
        truncated: bool,
    },
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

impl PaneFileContent {
    pub(super) fn visible_rows(&self, mode: FileViewMode, hide_unchanged: bool) -> &[VisualRow] {
        match self {
            PaneFileContent::LoadedRaw { .. } => &[],
            PaneFileContent::LoadedDiff {
                rows_all,
                rows_no_ctx,
                ..
            } => {
                if hide_unchanged {
                    rows_no_ctx
                } else {
                    rows_all
                }
            }
            PaneFileContent::LoadedMarkdown { raw_rows, .. } if mode == FileViewMode::Raw => {
                raw_rows
            }
            _ => &[],
        }
    }

    pub(in crate::workspace) fn diff_stats(&self) -> Option<(usize, usize)> {
        match self {
            PaneFileContent::LoadedDiff { added, removed, .. } => Some((*added, *removed)),
            _ => None,
        }
    }

    pub(in crate::workspace) fn is_loaded_diff(&self) -> bool {
        matches!(self, PaneFileContent::LoadedDiff { .. })
    }
}

/// [`daruda_content::diff::build_raw_rows`] under the viewer's line cap.
pub(in crate::workspace) fn build_raw_rows(lines: &[String]) -> Vec<VisualRow> {
    daruda_content::diff::build_raw_rows(lines, FILE_VIEWER_MAX_LINES)
}

/// [`daruda_content::diff::build_diff_rows`] with the marker row in the
/// user's language.
pub(in crate::workspace) fn build_diff_rows(hunks: &[DiffHunk], hide_ctx: bool) -> Vec<VisualRow> {
    let no_newline = crate::surface::strings::file_viewer::no_newline();
    daruda_content::diff::build_diff_rows(hunks, hide_ctx, &no_newline)
}

// ----------------------------------------------------------------
// PaneFileView helpers (GPUI-free)
// ----------------------------------------------------------------

impl PaneFileView {
    /// The git letter to show for this pane: a range's own, else the lane's
    /// pending change. `None` means nothing to diff, so no Changes mode.
    pub(in crate::workspace) fn status(&self) -> Option<char> {
        self.source.pinned_status().or(self.live_status)
    }

    /// Whether the pane holds text the user can edit and save: raw content of
    /// the file on disk. The one answer the save, dirty and can-save checks read.
    pub(in crate::workspace) fn holds_editable_buffer(&self) -> bool {
        self.source.is_editable()
            && matches!(
                self.content,
                PaneFileContent::LoadedRaw { truncated: false }
            )
    }

    pub(super) fn loading(
        lane_id: LaneId,
        path: PathBuf,
        source: DiffSource,
        live_status: Option<char>,
        view_mode: FileViewMode,
    ) -> Self {
        Self {
            lane_id,
            path,
            source,
            live_status,
            content: PaneFileContent::Loading,
            view_mode,
            hide_unchanged: false,
            selection_drag: SelectionDrag::None,
            search: None,
            pending_scroll_line: None,
        }
    }

    pub(in crate::workspace) fn replace_with_loading(
        &mut self,
        lane_id: LaneId,
        path: PathBuf,
        source: DiffSource,
        live_status: Option<char>,
        view_mode: FileViewMode,
    ) {
        self.lane_id = lane_id;
        self.path = path;
        self.source = source;
        self.live_status = live_status;
        self.content = PaneFileContent::Loading;
        self.view_mode = view_mode;
        self.hide_unchanged = false;
        self.clear_transient_state();
    }

    /// Sealed to `main_area`: outside callers go through
    /// [`crate::workspace::main_area::pane::FileContent::begin_mode_change`],
    /// which releases the GPU images this wipe orphans.
    pub(super) fn begin_mode_change(&mut self, mode: FileViewMode) -> Option<bool> {
        if self.view_mode == mode {
            return None;
        }

        let can_switch_markdown_without_reload =
            self.content_loaded_markdown() && mode != FileViewMode::Changes;
        self.view_mode = mode;
        self.clear_transient_state();

        if can_switch_markdown_without_reload {
            Some(false)
        } else {
            self.content = PaneFileContent::Loading;
            Some(true)
        }
    }

    pub(in crate::workspace) fn toggle_hide_unchanged(&mut self) -> bool {
        self.hide_unchanged = !self.hide_unchanged;
        self.clear_search_and_selection();
        self.content.is_loaded_diff()
    }

    /// Sealed to `main_area`: outside callers go through
    /// [`crate::workspace::main_area::pane::FileContent::install_content`],
    /// which pairs the content with the GPU image table its slots index and
    /// releases the previous one.
    pub(super) fn set_content(&mut self, content: PaneFileContent) {
        self.content = content;
    }

    pub(in crate::workspace) fn set_pending_scroll_line(&mut self, line: usize) {
        self.pending_scroll_line = Some(line);
    }

    pub(in crate::workspace) fn pending_scroll_line(&self) -> Option<usize> {
        self.pending_scroll_line
    }

    pub(in crate::workspace) fn clear_pending_scroll_line(&mut self) {
        self.pending_scroll_line = None;
    }

    pub(in crate::workspace) fn take_pending_scroll_line(&mut self) -> Option<usize> {
        self.pending_scroll_line.take()
    }

    pub(in crate::workspace) fn is_markdown_path(&self) -> bool {
        is_markdown_path(&self.path)
    }

    pub(in crate::workspace) fn loaded_diff_stats(&self) -> Option<(usize, usize)> {
        self.content.diff_stats()
    }

    pub(in crate::workspace) fn rows_for_content<'a>(
        &self,
        content: &'a PaneFileContent,
    ) -> &'a [VisualRow] {
        content.visible_rows(self.view_mode, self.hide_unchanged)
    }

    fn content_loaded_markdown(&self) -> bool {
        matches!(self.content, PaneFileContent::LoadedMarkdown { .. })
    }

    fn clear_transient_state(&mut self) {
        self.clear_search_and_selection();
        self.pending_scroll_line = None;
    }

    fn clear_search_and_selection(&mut self) {
        self.search = None;
        self.selection_drag = SelectionDrag::None;
    }

    /// Returns a slice over the currently visible rows (respects `hide_unchanged`).
    pub(in crate::workspace) fn active_rows(&self) -> &[VisualRow] {
        self.content
            .visible_rows(self.view_mode, self.hide_unchanged)
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

/// Case-insensitively — the toolbar offers Preview by this, so the loader
/// has to parse by it too, or `README.MD` previews as plain text.
pub(super) fn is_markdown_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
}

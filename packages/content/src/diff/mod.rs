//! A unified diff as rows: parse `git diff` text into hunks, pair changed
//! lines at the word level, and flatten either a diff or a raw file into the
//! [`VisualRow`]s a viewer renders and copies from. Built once at load time,
//! never at render time; a row stores token buckets, not colours.

pub mod diff_parser;
pub mod line_diff;
pub mod word_diff;

#[cfg(test)]
mod tests;

pub use diff_parser::{DiffHunk, DiffLine, parse_diff_hunks};

use crate::syntax::SyntaxBucket;

/// A single syntax-highlighted text segment within a diff line. Carries what
/// the token *is*; the host turns that into a colour with the `SyntaxTheme`
/// it paints under, so a stored row survives a palette switch.
#[derive(Clone)]
pub struct HighlightedSpan {
    pub text: String,
    /// `None` means use the default text color for the row kind.
    pub bucket: Option<SyntaxBucket>,
}

/// A byte range within a `VisualRow::content` string that differs at the
/// word level vs. the adjacent Removed/Added line pair.
#[derive(Clone)]
pub struct WordChange {
    pub start: usize,
    pub end: usize,
}

/// A single display row produced from either a raw file line or a diff line.
/// Built once at load time; the renderer and copy helpers consume this directly.
#[derive(Clone)]
pub struct VisualRow {
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
pub enum VisualRowKind {
    Plain,
    HunkHeader,
    Context,
    Added,
    Removed,
    NoNewline,
}

impl VisualRow {
    /// Text placed in the clipboard for this row (marker prefix included).
    pub fn copy_text(&self) -> String {
        match self.kind {
            VisualRowKind::Added => format!("+{}", self.content),
            VisualRowKind::Removed => format!("-{}", self.content),
            VisualRowKind::Context => format!(" {}", self.content),
            _ => self.content.clone(),
        }
    }
}

/// Build the flat row list for a raw file, at most `max_lines` rows.
pub fn build_raw_rows(lines: &[String], max_lines: usize) -> Vec<VisualRow> {
    lines
        .iter()
        .take(max_lines)
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
/// `no_newline` is the text of a "\ No newline at end of file" marker row,
/// in the host's language.
pub fn build_diff_rows(hunks: &[DiffHunk], hide_ctx: bool, no_newline: &str) -> Vec<VisualRow> {
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
                        content: no_newline.to_owned(),
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
pub fn count_diff_stats(hunks: &[DiffHunk]) -> (usize, usize) {
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

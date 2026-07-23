//! Convert diff [`VisualRow`]s into unified-editor inputs.
//!
//! The synthetic buffer omits `+`/`-` markers; row backgrounds and gutter
//! decorations carry add/remove meaning. Highlight overrides combine syntax
//! foregrounds with word-diff backgrounds because interleaved diff text is not
//! valid source for the editor's highlighter.

use std::ops::Range;

use gpui::{App, FontStyle, FontWeight, HighlightStyle, Hsla, SharedString};

use crate::ui::theme::TokenStyle;

use crate::ui::LineDecoration;

use super::{VisualRow, VisualRowKind};

/// Theme colours the diff editor model needs, snapshotted from
/// `DarudaTheme` by the caller so this module stays GPUI-free.
#[derive(Clone, Copy)]
pub(in crate::workspace) struct DiffColors {
    pub add_bg: Hsla,
    pub del_bg: Hsla,
    pub hunk_bg: Hsla,
    pub add_text: Hsla,
    pub del_text: Hsla,
    pub ctx_text: Hsla,
    pub hunk_text: Hsla,
    pub hunk_ctx_text: Hsla,
    pub word_add_bg: Hsla,
    pub word_del_bg: Hsla,
}

impl DiffColors {
    /// Snapshot the diff palette from the active `DarudaTheme`. Used by the
    /// File viewer's own diff pane, which paints on the UI theme's fixed
    /// `file_viewer_bg` editor surface.
    pub(in crate::workspace) fn from_theme(t: &crate::ui::theme::DarudaTheme) -> Self {
        Self {
            add_bg: t.file_diff_add_bg,
            del_bg: t.file_diff_del_bg,
            hunk_bg: t.file_diff_hunk_bg,
            add_text: t.file_diff_add_text,
            del_text: t.file_diff_del_text,
            ctx_text: t.text_muted,
            hunk_text: t.file_diff_hunk_text,
            hunk_ctx_text: t.file_diff_hunk_ctx_text,
            word_add_bg: t.file_diff_word_add_bg,
            word_del_bg: t.file_diff_word_del_bg,
        }
    }

    /// [`Self::from_theme`]'s agent-chat variant. The add/del/word-diff
    /// colours stay the git-convention palette shared with the File viewer
    /// (unchanged semantics, not a surface colour) — only the hunk-header row
    /// (`@@ -a,b +c,d @@`) switches from the fixed UI `BG_RAISED` surface to
    /// the same terminal-preset-derived tint the rest of the diff embed's
    /// chrome uses (header row, editor background — see `render/diff.rs`),
    /// so the header row blends with its own card instead of standing out as
    /// a UI-theme island.
    pub(in crate::workspace) fn from_agent_chat_theme(
        t: &crate::ui::theme::DarudaTheme,
        cx: &App,
    ) -> Self {
        Self {
            hunk_bg: crate::ui::theme::agent_chat_tint(cx),
            hunk_text: crate::ui::theme::agent_chat_fg(cx),
            hunk_ctx_text: crate::ui::theme::agent_chat_fg_muted(cx),
            ..Self::from_theme(t)
        }
    }
}

/// Editor inputs derived from a diff's `VisualRow`s.
pub(in crate::workspace) struct DiffEditorModel {
    pub text: String,
    pub decorations: Vec<LineDecoration>,
    pub highlights: Vec<(Range<usize>, HighlightStyle)>,
}

/// `(background, base_foreground)` for a row kind — mirrors the bespoke
/// renderer's `row_style`.
fn row_colors(kind: VisualRowKind, c: &DiffColors) -> (Option<Hsla>, Hsla) {
    match kind {
        VisualRowKind::HunkHeader => (Some(c.hunk_bg), c.hunk_text),
        VisualRowKind::Added => (Some(c.add_bg), c.add_text),
        VisualRowKind::Removed => (Some(c.del_bg), c.del_text),
        VisualRowKind::Context | VisualRowKind::NoNewline => (None, c.ctx_text),
        VisualRowKind::Plain => (None, c.ctx_text),
    }
}

fn fg_only(color: Hsla) -> HighlightStyle {
    HighlightStyle {
        color: Some(color),
        ..Default::default()
    }
}

/// Build contiguous per-line highlight spans combining syntax and word diff.
fn line_spans(
    row: &VisualRow,
    line_text: &str,
    base_fg: Hsla,
    colors: &DiffColors,
) -> Vec<(Range<usize>, HighlightStyle)> {
    // Hunk headers carry no code: foreground is the hunk colour, with the
    // trailing context (everything past `content`) dimmed. No word diff.
    if row.kind == VisualRowKind::HunkHeader {
        let split = row.content.len().min(line_text.len());
        let mut out = vec![(0..split, fg_only(base_fg))];
        if split < line_text.len() {
            out.push((split..line_text.len(), fg_only(colors.hunk_ctx_text)));
        }
        return out;
    }

    // Foreground segments from the syntax spans (their `text` concatenates
    // to `content`). Fall back to one base-coloured segment when absent or
    // short.
    let mut fg: Vec<(usize, usize, Hsla, TokenStyle)> = Vec::new();
    let mut off = 0usize;
    for span in &row.spans {
        let end = (off + span.text.len()).min(line_text.len());
        if end > off {
            fg.push((off, end, span.color.unwrap_or(base_fg), span.style));
        }
        off = end;
    }
    if off < line_text.len() {
        fg.push((off, line_text.len(), base_fg, TokenStyle::default()));
    }
    if fg.is_empty() {
        // Empty line: nothing to cover.
        return Vec::new();
    }

    let word_bg = match row.kind {
        VisualRowKind::Removed => colors.word_del_bg,
        _ => colors.word_add_bg,
    };

    // Boundary set: segment edges + word-change edges, clamped to the line.
    let mut bounds: Vec<usize> = vec![0, line_text.len()];
    for (s, e, _, _) in &fg {
        bounds.push(*s);
        bounds.push(*e);
    }
    for w in &row.word_changes {
        bounds.push(w.start.min(line_text.len()));
        bounds.push(w.end.min(line_text.len()));
    }
    bounds.sort_unstable();
    bounds.dedup();

    let mut out = Vec::with_capacity(bounds.len());
    for win in bounds.windows(2) {
        let (a, b) = (win[0], win[1]);
        if a >= b {
            continue;
        }
        let (color, style) = fg
            .iter()
            .find(|(s, e, _, _)| *s <= a && a < *e)
            .map(|(_, _, c, st)| (*c, *st))
            .unwrap_or((base_fg, TokenStyle::default()));
        let bg = row
            .word_changes
            .iter()
            .any(|w| w.start <= a && a < w.end)
            .then_some(word_bg);
        out.push((
            a..b,
            HighlightStyle {
                color: Some(color),
                background_color: bg,
                font_weight: style.bold.then_some(FontWeight::BOLD),
                font_style: style.italic.then_some(FontStyle::Italic),
                ..Default::default()
            },
        ));
    }
    out
}

/// Convert the diff rows into editor inputs.
///
/// `show_line_numbers` drives the gutter: `true` packs the dual old/new line
/// numbers (a full-file diff whose numbers are the real file lines — the git
/// file viewer, an agent-chat file *creation*); `false` blanks every gutter
/// (a snippet diff — an agent-chat `Edit` whose old/new text is only the
/// replaced region, so "line 1" is not the file's line 1). Blank gutters keep
/// the row backgrounds (those are decoration-driven, independent of the
/// number) but reserve no gutter width.
pub(in crate::workspace) fn build_diff_editor_model(
    rows: &[VisualRow],
    colors: &DiffColors,
    show_line_numbers: bool,
) -> DiffEditorModel {
    let left_w = rows
        .iter()
        .map(|r| r.line_no_left.chars().count())
        .max()
        .unwrap_or(0);
    let right_w = rows
        .iter()
        .map(|r| r.line_no_right.chars().count())
        .max()
        .unwrap_or(0);

    let mut text = String::new();
    let mut decorations = Vec::with_capacity(rows.len());
    let mut highlights = Vec::new();

    for (i, row) in rows.iter().enumerate() {
        let line_start = text.len();
        let line_text = if row.kind == VisualRowKind::HunkHeader && !row.header_context.is_empty() {
            format!("{}  {}", row.content, row.header_context)
        } else {
            row.content.clone()
        };

        let (bg, base_fg) = row_colors(row.kind, colors);

        // Gutter: "old new", each column right-aligned; blank for headers.
        // When numbers are hidden every row's gutter is empty (an empty custom
        // gutter reserves no width in the editor element).
        let gutter = if !show_line_numbers {
            String::new()
        } else if row.kind == VisualRowKind::HunkHeader {
            format!("{:>w$}", "", w = left_w + 1 + right_w)
        } else {
            format!(
                "{:>lw$} {:>rw$}",
                row.line_no_left,
                row.line_no_right,
                lw = left_w,
                rw = right_w
            )
        };
        decorations.push(LineDecoration {
            background: bg,
            gutter: Some(SharedString::from(gutter)),
        });

        for (r, style) in line_spans(row, &line_text, base_fg, colors) {
            highlights.push((line_start + r.start..line_start + r.end, style));
        }

        text.push_str(&line_text);
        if i + 1 < rows.len() {
            // The newline keeps base colour so coverage stays contiguous,
            // matching the highlighter's `line.len() + 1` ranges.
            let nl = text.len();
            text.push('\n');
            highlights.push((nl..nl + 1, fg_only(base_fg)));
        }
    }

    DiffEditorModel {
        text,
        decorations,
        highlights,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::main_area::file_view_pane::{HighlightedSpan, WordChange};

    fn colors() -> DiffColors {
        let c = |l: f32| Hsla {
            h: 0.,
            s: 0.,
            l,
            a: 1.,
        };
        DiffColors {
            add_bg: c(0.1),
            del_bg: c(0.11),
            hunk_bg: c(0.12),
            add_text: c(0.2),
            del_text: c(0.21),
            ctx_text: c(0.22),
            hunk_text: c(0.23),
            hunk_ctx_text: c(0.24),
            word_add_bg: c(0.3),
            word_del_bg: c(0.31),
        }
    }

    fn row(kind: VisualRowKind, left: &str, right: &str, content: &str) -> VisualRow {
        VisualRow {
            kind,
            line_no_left: left.into(),
            line_no_right: right.into(),
            content: content.into(),
            header_context: String::new(),
            spans: Vec::new(),
            word_changes: Vec::new(),
        }
    }

    /// Highlight ranges must tile the whole buffer with no gaps or overlaps.
    fn assert_contiguous(model: &DiffEditorModel) {
        let mut sorted = model.highlights.clone();
        sorted.sort_by_key(|(r, _)| r.start);
        let mut cursor = 0;
        for (r, _) in &sorted {
            assert_eq!(r.start, cursor, "gap/overlap at {cursor}");
            assert!(r.end > r.start, "empty range");
            cursor = r.end;
        }
        assert_eq!(cursor, model.text.len(), "coverage must reach buffer end");
    }

    #[test]
    fn buffer_has_no_marker_prefix_and_dual_gutter() {
        let rows = vec![
            row(VisualRowKind::Context, "1", "1", "fn a() {}"),
            row(VisualRowKind::Removed, "2", "", "let x = 1;"),
            row(VisualRowKind::Added, "", "2", "let y = 2;"),
        ];
        let m = build_diff_editor_model(&rows, &colors(), true);
        // No `+`/`-` markers: lines are the bare content.
        assert_eq!(m.text, "fn a() {}\nlet x = 1;\nlet y = 2;");
        // Dual gutter, right-aligned columns ("old new").
        assert_eq!(m.decorations[0].gutter.as_deref(), Some("1 1"));
        assert_eq!(m.decorations[1].gutter.as_deref(), Some("2  "));
        assert_eq!(m.decorations[2].gutter.as_deref(), Some("  2"));
        // Add/remove rows carry a background; context does not.
        assert!(m.decorations[0].background.is_none());
        assert!(m.decorations[1].background.is_some());
        assert!(m.decorations[2].background.is_some());
        assert_contiguous(&m);
    }

    /// With `show_line_numbers = false` (a snippet diff whose numbers would
    /// mislead) every gutter is blank — but the row backgrounds still stand,
    /// so the diff still reads as add/remove.
    #[test]
    fn hidden_line_numbers_blank_every_gutter_but_keep_backgrounds() {
        let rows = vec![
            row(VisualRowKind::Context, "1", "1", "fn a() {}"),
            row(VisualRowKind::Removed, "2", "", "let x = 1;"),
            row(VisualRowKind::Added, "", "2", "let y = 2;"),
        ];
        let m = build_diff_editor_model(&rows, &colors(), false);
        // Every gutter is the empty string (present, so the editor doesn't
        // fall back to its own sequential 1,2,3 numbering).
        for d in &m.decorations {
            assert_eq!(d.gutter.as_deref(), Some(""));
        }
        // Backgrounds are unchanged — add/remove tint survives hidden numbers.
        assert!(m.decorations[0].background.is_none());
        assert!(m.decorations[1].background.is_some());
        assert!(m.decorations[2].background.is_some());
        assert_contiguous(&m);
    }

    #[test]
    fn word_change_sets_background_within_syntax_spans() {
        let mut r = row(VisualRowKind::Added, "", "1", "let y = 2;");
        r.spans = vec![
            HighlightedSpan {
                text: "let ".into(),
                color: Some(Hsla {
                    h: 0.,
                    s: 0.,
                    l: 0.9,
                    a: 1.,
                }),
                style: TokenStyle::default(),
            },
            HighlightedSpan {
                text: "y = 2;".into(),
                color: None,
                style: TokenStyle::default(),
            },
        ];
        // The "y" differs at the word level (bytes 4..5).
        r.word_changes = vec![WordChange { start: 4, end: 5 }];
        let m = build_diff_editor_model(&[r], &colors(), true);
        assert_contiguous(&m);
        // Exactly the word-change range carries the add word background.
        let with_bg: Vec<_> = m
            .highlights
            .iter()
            .filter(|(_, s)| s.background_color == Some(colors().word_add_bg))
            .collect();
        assert_eq!(with_bg.len(), 1);
        assert_eq!(with_bg[0].0, 4..5);
    }

    #[test]
    fn hunk_header_packs_context_and_blank_gutter() {
        let mut r = row(VisualRowKind::HunkHeader, "", "", "@@ -1,3 +1,4 @@");
        r.header_context = "fn a()".into();
        let m = build_diff_editor_model(&[r], &colors(), true);
        assert_eq!(m.text, "@@ -1,3 +1,4 @@  fn a()");
        // Blank gutter for headers.
        assert_eq!(m.decorations[0].gutter.as_deref(), Some(" "));
        assert_contiguous(&m);
    }
}

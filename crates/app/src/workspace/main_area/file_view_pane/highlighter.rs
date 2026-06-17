//! Syntax highlighting for the diff / file viewer.
//!
//! Uses tree-sitter (via `tree-sitter-highlight`) over the grammars and
//! highlight queries bundled in `gpui_component`'s language registry
//! (reached through `crate::ui::highlighter`). The whole text of a hunk /
//! file is parsed in one pass so multi-line tokens (block comments,
//! string literals, …) are coloured correctly across line boundaries;
//! the resulting capture ranges are then split back into per-line spans.
//!
//! Colours come from the `base16-ocean.dark` palette via
//! [`dt_theme::syntax_color`], keyed by tree-sitter capture name — one
//! colour per capture.
//!
//! The `theme_name` argument is accepted for call-site compatibility but
//! unused: the palette is fixed to `base16-ocean.dark`.
//!
//! All public functions are GPUI-free and safe to call on `background_executor`.

use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter};

use super::{DiffHunk, DiffLine, HighlightedSpan, VisualRow};
use crate::ui::highlighter::LanguageRegistry;
use crate::ui::theme as dt_theme;

/// Capture names recognised by the highlighter. Mirrors
/// `gpui_component::highlighter`'s `HIGHLIGHT_NAMES` (which is `pub(super)`
/// and so cannot be imported); the order is irrelevant, but every name a
/// bundled `highlights.scm` query references must appear here for
/// tree-sitter-highlight to report it. [`dt_theme::syntax_color`] maps
/// each name to a colour.
const HIGHLIGHT_NAMES: [&str; 40] = [
    "attribute",
    "boolean",
    "comment",
    "comment.doc",
    "constant",
    "constructor",
    "embedded",
    "emphasis",
    "emphasis.strong",
    "enum",
    "function",
    "hint",
    "keyword",
    "label",
    "link_text",
    "link_uri",
    "number",
    "operator",
    "predictive",
    "preproc",
    "primary",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.list_marker",
    "punctuation.special",
    "string",
    "string.escape",
    "string.regex",
    "string.special",
    "string.special.symbol",
    "tag",
    "tag.doctype",
    "text.literal",
    "title",
    "type",
    "variable",
    "variable.special",
    "variant",
];

// ----------------------------------------------------------------
// Public API
// ----------------------------------------------------------------

/// Apply syntax highlighting to all lines in `hunks` in-place.
/// `ext` is the file extension (e.g. `"rs"`, `"py"`) used to select the
/// language. Each hunk is highlighted independently (its display lines
/// joined and parsed together). Unknown extensions leave lines un-highlighted.
pub(in crate::workspace) fn highlight_hunks(hunks: &mut [DiffHunk], ext: &str, _theme_name: &str) {
    let Some(config) = build_config(ext) else {
        return;
    };

    for hunk in hunks.iter_mut() {
        // Collect the display-line contents and remember their index in
        // `hunk.lines` so the spans can be written back.
        let mut indices = Vec::new();
        let mut contents = Vec::new();
        for (idx, line) in hunk.lines.iter().enumerate() {
            let content = match line {
                DiffLine::Context { content, .. }
                | DiffLine::Added { content, .. }
                | DiffLine::Removed { content, .. } => content.as_str(),
                DiffLine::NoNewline => continue,
            };
            indices.push(idx);
            contents.push(content);
        }

        let per_line = highlight_lines(&config, &contents);
        for (n, spans) in per_line.into_iter().enumerate() {
            if spans.is_empty() {
                continue;
            }
            match &mut hunk.lines[indices[n]] {
                DiffLine::Context { spans: s, .. }
                | DiffLine::Added { spans: s, .. }
                | DiffLine::Removed { spans: s, .. } => *s = spans,
                DiffLine::NoNewline => {}
            }
        }
    }
}

/// Apply syntax highlighting to a flat list of raw `VisualRow`s in-place.
/// All rows are parsed as a single document so multi-line tokens are
/// coloured correctly throughout the file. Unknown extensions leave rows
/// un-highlighted.
pub(in crate::workspace) fn highlight_raw_rows(
    rows: &mut [VisualRow],
    ext: &str,
    _theme_name: &str,
) {
    let Some(config) = build_config(ext) else {
        return;
    };

    let contents: Vec<&str> = rows.iter().map(|r| r.content.as_str()).collect();
    let per_line = highlight_lines(&config, &contents);
    for (i, spans) in per_line.into_iter().enumerate() {
        if !spans.is_empty() {
            rows[i].spans = spans;
        }
    }
}

// ----------------------------------------------------------------
// Internal
// ----------------------------------------------------------------

/// Build a configured tree-sitter highlight configuration for the language
/// resolved from `ext` (the registry resolves short names / extensions,
/// e.g. `"rs"` → Rust). Returns `None` for unknown languages or invalid
/// queries — the caller then leaves the text un-highlighted.
fn build_config(ext: &str) -> Option<HighlightConfiguration> {
    let lang = LanguageRegistry::singleton().language(ext)?;
    // Unknown extensions resolve to a "plain" language with no highlight
    // query; leave that text un-highlighted (renderer uses the row's
    // default colour) instead of emitting a default-coloured span per line.
    if lang.highlights.trim().is_empty() {
        return None;
    }
    let mut config = HighlightConfiguration::new(
        lang.language.clone(),
        lang.name.to_string(),
        lang.highlights.as_ref(),
        lang.injections.as_ref(),
        lang.locals.as_ref(),
    )
    .ok()?;
    config.configure(&HIGHLIGHT_NAMES);
    Some(config)
}

/// Highlight `contents` (one entry per line) as a single joined document
/// and return the per-line spans. Every line is fully covered: bytes not
/// inside a recognised capture get the default foreground, so every token
/// carries an explicit colour.
fn highlight_lines(
    config: &HighlightConfiguration,
    contents: &[&str],
) -> Vec<Vec<HighlightedSpan>> {
    let mut out = vec![Vec::new(); contents.len()];
    if contents.is_empty() {
        return out;
    }

    // Join into one source, recording each line's byte range.
    let mut src = String::new();
    let mut line_ranges = Vec::with_capacity(contents.len());
    for (i, content) in contents.iter().enumerate() {
        let start = src.len();
        src.push_str(content);
        line_ranges.push(start..src.len());
        if i + 1 < contents.len() {
            src.push('\n');
        }
    }

    let mut highlighter = Highlighter::new();
    let Ok(events) = highlighter.highlight(config, src.as_bytes(), None, |_| None) else {
        return out;
    };

    let default = dt_theme::syntax_color("");

    // Flatten the event stream into contiguous coloured byte ranges. The
    // active capture is the top of the start/end stack; `Source` events
    // cover the whole text, so uncaptured gaps fall through to `default`.
    let mut stack: Vec<usize> = Vec::new();
    let mut ranges: Vec<(usize, usize, gpui::Hsla)> = Vec::new();
    for event in events {
        let Ok(event) = event else {
            return out;
        };
        match event {
            HighlightEvent::HighlightStart(Highlight(idx)) => stack.push(idx),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                if start >= end {
                    continue;
                }
                let color = stack
                    .last()
                    .and_then(|&idx| HIGHLIGHT_NAMES.get(idx))
                    .map(|name| dt_theme::syntax_color(name))
                    .unwrap_or(default);
                ranges.push((start, end, color));
            }
        }
    }

    // Split coloured ranges back into per-line spans, clipping at line
    // boundaries (the joining `\n` bytes sit between ranges and are dropped).
    for (li, lr) in line_ranges.iter().enumerate() {
        let mut spans: Vec<HighlightedSpan> = Vec::new();
        for &(start, end, color) in &ranges {
            let clip_start = start.max(lr.start);
            let clip_end = end.min(lr.end);
            if clip_start >= clip_end {
                continue;
            }
            let text = &src[clip_start..clip_end];
            match spans.last_mut() {
                Some(last) if last.color == Some(color) => last.text.push_str(text),
                _ => spans.push(HighlightedSpan {
                    text: text.to_owned(),
                    color: Some(color),
                }),
            }
        }
        if !spans.is_empty() {
            out[li] = spans;
        }
    }

    out
}

// ----------------------------------------------------------------
// Tests
// ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_hunks_unknown_ext_is_plain() {
        use crate::workspace::main_area::file_view_pane::diff_parser::parse_diff_hunks;
        let diff = "@@ -1,2 +1,2 @@\n-old\n+new\n";
        let mut hunks = parse_diff_hunks(diff);
        // Unknown extension → no language → lines left intact, no panic.
        highlight_hunks(&mut hunks, "unknown_ext_xyz", "base16-ocean.dark");
        assert_eq!(hunks[0].lines.len(), 2);
        for line in &hunks[0].lines {
            if let DiffLine::Removed { spans, .. } | DiffLine::Added { spans, .. } = line {
                assert!(spans.is_empty(), "unknown ext should leave spans empty");
            }
        }
    }

    #[test]
    fn highlight_hunks_rust_colours_keyword() {
        use crate::workspace::main_area::file_view_pane::diff_parser::parse_diff_hunks;
        let diff = "@@ -1,1 +1,1 @@\n-let x = 1;\n+let y = 2;\n";
        let mut hunks = parse_diff_hunks(diff);
        highlight_hunks(&mut hunks, "rs", "base16-ocean.dark");

        // Every display line should be fully covered by spans, and the
        // `let` keyword should not be coloured with the default foreground.
        let keyword_color = dt_theme::syntax_color("keyword");
        let default = dt_theme::syntax_color("");
        assert_ne!(keyword_color, default, "test palette sanity");

        let mut saw_keyword = false;
        for line in &hunks[0].lines {
            if let DiffLine::Added { spans, .. } | DiffLine::Removed { spans, .. } = line {
                assert!(!spans.is_empty(), "rust lines should be highlighted");
                let joined: String = spans.iter().map(|s| s.text.as_str()).collect();
                assert!(joined.contains("let"), "spans must cover full content");
                if spans
                    .iter()
                    .any(|s| s.text.contains("let") && s.color == Some(keyword_color))
                {
                    saw_keyword = true;
                }
            }
        }
        assert!(saw_keyword, "`let` should be coloured as a keyword");
    }

    #[test]
    fn highlight_raw_rows_unknown_ext_is_plain() {
        use super::super::{VisualRow, VisualRowKind};
        let mut rows = vec![VisualRow {
            kind: VisualRowKind::Plain,
            line_no_left: String::new(),
            line_no_right: String::new(),
            content: "let x = 1;".to_owned(),
            header_context: String::new(),
            spans: Vec::new(),
            word_changes: Vec::new(),
        }];
        highlight_raw_rows(&mut rows, "unknown_ext_xyz", "base16-ocean.dark");
        assert!(rows[0].spans.is_empty());
    }

    #[test]
    fn highlight_raw_rows_colours_multi_line_block_comment() {
        use super::super::{VisualRow, VisualRowKind};
        let make = |content: &str| VisualRow {
            kind: VisualRowKind::Plain,
            line_no_left: String::new(),
            line_no_right: String::new(),
            content: content.to_owned(),
            header_context: String::new(),
            spans: Vec::new(),
            word_changes: Vec::new(),
        };

        // A Rust block comment spanning two rows. Whole-document parsing
        // must colour BOTH rows as `comment`; a per-line parse would miss
        // the continuation line — this is the reason the rows are joined.
        let mut rows = vec![make("/* a block comment"), make("that spans lines */")];
        highlight_raw_rows(&mut rows, "rs", "base16-ocean.dark");

        let comment = dt_theme::syntax_color("comment");
        assert_ne!(comment, dt_theme::syntax_color(""), "palette sanity");
        for (i, row) in rows.iter().enumerate() {
            assert!(!row.spans.is_empty(), "row {i} should be highlighted");
            assert!(
                row.spans.iter().all(|s| s.color == Some(comment)),
                "row {i} should be entirely comment-coloured, got {:?}",
                row.spans.iter().map(|s| s.color).collect::<Vec<_>>()
            );
        }
    }
}

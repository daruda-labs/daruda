//! Syntax highlighting for the diff / file viewer.
//!
//! Uses `syntect` (TextMate grammars) to tokenise each line into coloured
//! spans. The `SyntaxSet` and `Theme` are loaded once via `OnceLock` and
//! reused across background tasks.
//!
//! `highlight_hunks`: stateful per-hunk (fresh state each hunk, correct
//! within the hunk at the cost of cross-boundary token state).
//! `highlight_raw_rows`: single stateful pass over the whole file (correct
//! multi-line token state across all rows).
//!
//! All public functions are GPUI-free and safe to call on `background_executor`.

use std::sync::OnceLock;

use syntect::{
    easy::HighlightLines,
    highlighting::{Color as SynColor, ThemeSet},
    parsing::SyntaxSet,
};

use super::pane_file_view::{DiffHunk, DiffLine, HighlightedSpan, VisualRow};
use crate::ui::theme as dt_theme;

// ----------------------------------------------------------------
// Lazy-loaded globals (initialised once per process)
// ----------------------------------------------------------------

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

fn syntax_set() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(two_face::syntax::extra_newlines)
}

fn theme_set() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

// ----------------------------------------------------------------
// Public API
// ----------------------------------------------------------------

/// Apply syntax highlighting to all lines in `hunks` in-place.
/// `ext` is the file extension (e.g. `"rs"`, `"py"`) used to detect the language.
/// `theme_name` selects the syntect theme; unknown names fall back to the first
/// bundled theme. Lines that fail to highlight keep empty `spans` (plain-text fallback).
pub(in crate::workspace) fn highlight_hunks(hunks: &mut [DiffHunk], ext: &str, theme_name: &str) {
    let ss = syntax_set();
    let ts = theme_set();

    let syntax = ss
        .find_syntax_by_extension(ext)
        .unwrap_or_else(|| ss.find_syntax_plain_text());
    let theme = ts
        .themes
        .get(theme_name)
        .or_else(|| ts.themes.get(dt_theme::FILE_VIEWER_SYNTAX_THEME))
        .or_else(|| ts.themes.values().next())
        .expect("syntect bundled themes are never empty");

    for hunk in hunks.iter_mut() {
        // A fresh highlighter per hunk gives correct multi-line state within the hunk.
        let mut h = HighlightLines::new(syntax, theme);
        for line in hunk.lines.iter_mut() {
            let content = match line {
                DiffLine::Context { content, .. } => content.as_str(),
                DiffLine::Added { content, .. } => content.as_str(),
                DiffLine::Removed { content, .. } => content.as_str(),
                DiffLine::NoNewline => continue,
            };

            // syntect expects a newline-terminated string.
            let line_nl = format!("{content}\n");
            let Ok(ranges) = h.highlight_line(&line_nl, ss) else {
                continue;
            };

            let spans: Vec<HighlightedSpan> = ranges
                .into_iter()
                .filter_map(|(style, text)| {
                    // Drop the trailing newline we added.
                    let t = text.trim_end_matches('\n');
                    if t.is_empty() {
                        return None;
                    }
                    Some(HighlightedSpan {
                        text: t.to_owned(),
                        color: Some(syn_color_to_hsla(style.foreground)),
                    })
                })
                .collect();

            if spans.is_empty() {
                continue;
            }

            match line {
                DiffLine::Context { spans: s, .. } => *s = spans,
                DiffLine::Added { spans: s, .. } => *s = spans,
                DiffLine::Removed { spans: s, .. } => *s = spans,
                DiffLine::NoNewline => {}
            }
        }
    }
}

/// Apply syntax highlighting to a flat list of raw `VisualRow`s in-place.
/// A single `HighlightLines` state is maintained across all rows so that
/// multi-line tokens (block comments, string literals, …) are coloured
/// correctly throughout the whole file.
/// `theme_name` selects the syntect theme; unknown names fall back to the
/// built-in default. Rows that fail to highlight keep empty `spans`.
pub(in crate::workspace) fn highlight_raw_rows(
    rows: &mut [VisualRow],
    ext: &str,
    theme_name: &str,
) {
    let ss = syntax_set();
    let ts = theme_set();

    let syntax = ss
        .find_syntax_by_extension(ext)
        .unwrap_or_else(|| ss.find_syntax_plain_text());
    let theme = ts
        .themes
        .get(theme_name)
        .or_else(|| ts.themes.get(dt_theme::FILE_VIEWER_SYNTAX_THEME))
        .or_else(|| ts.themes.values().next())
        .expect("syntect bundled themes are never empty");

    let mut h = HighlightLines::new(syntax, theme);
    for row in rows.iter_mut() {
        let line_nl = format!("{}\n", row.content);
        let Ok(ranges) = h.highlight_line(&line_nl, ss) else {
            continue;
        };
        let spans: Vec<HighlightedSpan> = ranges
            .into_iter()
            .filter_map(|(style, text)| {
                let t = text.trim_end_matches('\n');
                if t.is_empty() {
                    return None;
                }
                Some(HighlightedSpan {
                    text: t.to_owned(),
                    color: Some(syn_color_to_hsla(style.foreground)),
                })
            })
            .collect();
        if !spans.is_empty() {
            row.spans = spans;
        }
    }
}

// ----------------------------------------------------------------
// Colour conversion
// ----------------------------------------------------------------

fn syn_color_to_hsla(c: SynColor) -> gpui::Hsla {
    let r = c.r as f32 / 255.0;
    let g = c.g as f32 / 255.0;
    let b = c.b as f32 / 255.0;
    let a = c.a as f32 / 255.0;
    rgb_to_hsla(r, g, b, a)
}

fn rgb_to_hsla(r: f32, g: f32, b: f32, a: f32) -> gpui::Hsla {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;

    if (max - min).abs() < f32::EPSILON {
        return dt_theme::monochrome(l, a);
    }

    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };

    let h = if (max - r).abs() < f32::EPSILON {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (max - g).abs() < f32::EPSILON {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;

    gpui::hsla(h, s, l, a)
}

// ----------------------------------------------------------------
// Tests
// ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_to_hsla_white() {
        let h = rgb_to_hsla(1.0, 1.0, 1.0, 1.0);
        assert!((h.l - 1.0).abs() < 0.01);
        assert!(h.s < 0.01);
    }

    #[test]
    fn rgb_to_hsla_black() {
        let h = rgb_to_hsla(0.0, 0.0, 0.0, 1.0);
        assert!(h.l < 0.01);
    }

    #[test]
    fn highlight_hunks_plain_text_fallback() {
        use super::super::pane_file_view::parse_diff_hunks;
        let diff = "@@ -1,2 +1,2 @@\n-old\n+new\n";
        let mut hunks = parse_diff_hunks(diff);
        // "plain text" syntax produces no meaningful tokens — spans may be empty
        // or contain a single span with the default colour. Either way, no panic.
        highlight_hunks(&mut hunks, "unknown_ext_xyz", "base16-ocean.dark");
        // The hunk still has its lines.
        assert_eq!(hunks[0].lines.len(), 2);
    }
}

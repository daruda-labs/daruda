//! Syntax highlighting for the diff/file viewer.
//!
//! Parses each hunk/file text with tree-sitter in one pass so multi-line tokens
//! survive the later split into per-line spans. Capture names map through the
//! configured syntax palette, with unknown names falling back to Daruda tokens.
//! Public functions are GPUI-free and background-executor safe.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter};

use super::{DiffHunk, DiffLine, HighlightedSpan, VisualRow};
use daruda_content::syntax::{SyntaxBucket, bucket_for_capture};

/// Capture names recognised by bundled queries. Duplicated because
/// `gpui_component::highlighter::HIGHLIGHT_NAMES` is not public.
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
/// How the caller named the language to highlight.
///
/// The two are different vocabularies and resolve through different tables, so
/// they are separate variants rather than one `&str`: passing a fence's info
/// string to the extension resolver is what left ```` ```rust ```` (and
/// `python`, `javascript`, …) un-highlighted while the handful of tokens that
/// happen to equal their extension (`bash`, `java`, `go`) worked.
#[derive(Clone, Copy, Debug)]
pub(in crate::workspace) enum LanguageHint<'a> {
    /// A file's extension, without the dot — `rs`, `md`.
    Extension(&'a str),
    /// A fenced code block's info string — `rust`, `jsx`.
    FenceToken(&'a str),
}

pub(in crate::workspace) fn highlight_hunks(hunks: &mut [DiffHunk], lang: LanguageHint<'_>) {
    let Some(config) = build_config(lang) else {
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
pub(super) fn highlight_raw_rows(rows: &mut [VisualRow], lang: LanguageHint<'_>) {
    let Some(config) = build_config(lang) else {
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

/// Compiled highlight configurations by canonical language name, for the
/// lifetime of the process.
///
/// `HighlightConfiguration::new` compiles the language's highlights, injections
/// and locals queries — the expensive part of highlighting, and immutable once
/// `configure`d, so it is built once and shared. Keyed by the *resolved*
/// `LanguageConfig::name`, not the caller's spelling: fence tokens are
/// arbitrary user text, so keying by the input would let a document mint an
/// unbounded number of entries, each paying its own compile.
static CONFIG_CACHE: LazyLock<Mutex<HashMap<String, Arc<HighlightConfiguration>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The configured tree-sitter highlight configuration for `hint`, compiling it
/// on first use. Returns `None` for unknown languages or invalid queries — the
/// caller then leaves the text un-highlighted (the renderer uses the row's
/// default colour) instead of emitting a default-coloured span per line.
///
/// A concurrent miss may compile twice: identical result, last write wins, and
/// cheaper than holding the lock across the compile.
fn build_config(hint: LanguageHint<'_>) -> Option<Arc<HighlightConfiguration>> {
    let lang = match hint {
        LanguageHint::Extension(ext) => daruda_core::language::from_extension(ext)
            .and_then(crate::ui::highlighter::highlightable_config)?,
        // A fence info string has no single vocabulary — `rust` and `rs`,
        // `bash` and `zsh` all appear in real documents. The registry answers
        // language names plus the short forms it knows; the extension table
        // covers the rest (`jsx`, `zsh`, `patch`, `hpp`).
        LanguageHint::FenceToken(token) => crate::ui::highlighter::highlightable_config(token)
            .or_else(|| {
                daruda_core::language::from_extension(token)
                    .and_then(crate::ui::highlighter::highlightable_config)
            })?,
    };
    if let Some(hit) = CONFIG_CACHE.lock().unwrap().get(lang.name.as_ref()) {
        return Some(hit.clone());
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
    let config = Arc::new(config);
    CONFIG_CACHE
        .lock()
        .unwrap()
        .insert(lang.name.to_string(), config.clone());
    Some(config)
}

/// Highlight `contents` (one entry per line) as a single joined document
/// and return the per-line spans. Every line is fully covered: bytes not
/// inside a recognised capture get the default bucket, so every token
/// carries an explicit one.
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

    // Flatten the event stream into contiguous bucketed byte ranges. The
    // active capture is the top of the start/end stack; `Source` events
    // cover the whole text, so uncaptured gaps fall through to the default
    // bucket.
    let mut stack: Vec<usize> = Vec::new();
    let mut ranges: Vec<(usize, usize, SyntaxBucket)> = Vec::new();
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
                let bucket = stack
                    .last()
                    .and_then(|&idx| HIGHLIGHT_NAMES.get(idx))
                    .map(|name| bucket_for_capture(name))
                    .unwrap_or(SyntaxBucket::Default);
                ranges.push((start, end, bucket));
            }
        }
    }

    // Split coloured ranges back into per-line spans, clipping at line
    // boundaries (the joining `\n` bytes sit between ranges and are dropped).
    for (li, lr) in line_ranges.iter().enumerate() {
        let mut spans: Vec<HighlightedSpan> = Vec::new();
        for &(start, end, bucket) in &ranges {
            let clip_start = start.max(lr.start);
            let clip_end = end.min(lr.end);
            if clip_start >= clip_end {
                continue;
            }
            let text = &src[clip_start..clip_end];
            match spans.last_mut() {
                Some(last) if last.bucket == Some(bucket) => last.text.push_str(text),
                _ => spans.push(HighlightedSpan {
                    text: text.to_owned(),
                    bucket: Some(bucket),
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
        highlight_hunks(&mut hunks, LanguageHint::Extension("unknown_ext_xyz"));
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
        highlight_hunks(&mut hunks, LanguageHint::Extension("rs"));

        // Every display line should be fully covered by spans, and the
        // `let` keyword should land in the keyword bucket, not the default.

        let mut saw_keyword = false;
        for line in &hunks[0].lines {
            if let DiffLine::Added { spans, .. } | DiffLine::Removed { spans, .. } = line {
                assert!(!spans.is_empty(), "rust lines should be highlighted");
                let joined: String = spans.iter().map(|s| s.text.as_str()).collect();
                assert!(joined.contains("let"), "spans must cover full content");
                if spans
                    .iter()
                    .any(|s| s.text.contains("let") && s.bucket == Some(SyntaxBucket::Keyword))
                {
                    saw_keyword = true;
                }
            }
        }
        assert!(saw_keyword, "`let` should be coloured as a keyword");
    }

    #[test]
    fn highlight_raw_rows_colours_java_and_aliased_extensions() {
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

        // `java` is a registry language name; `hpp` only resolves through
        // the shared alias table. Both must reach a real highlight query.
        for (ext, source) in [("java", "class A { int x = 1; }"), ("hpp", "int x = 1;")] {
            let mut rows = vec![make(source)];
            highlight_raw_rows(&mut rows, LanguageHint::Extension(ext));
            assert!(
                !rows[0].spans.is_empty(),
                "{ext} should be highlighted, got no spans"
            );
        }
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
        highlight_raw_rows(&mut rows, LanguageHint::Extension("unknown_ext_xyz"));
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
        highlight_raw_rows(&mut rows, LanguageHint::Extension("rs"));

        for (i, row) in rows.iter().enumerate() {
            assert!(!row.spans.is_empty(), "row {i} should be highlighted");
            assert!(
                row.spans
                    .iter()
                    .all(|s| s.bucket == Some(SyntaxBucket::Comment)),
                "row {i} should be entirely comment-bucketed, got {:?}",
                row.spans.iter().map(|s| s.bucket).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn selected_palette_changes_the_highlight_colour() {
        use super::super::{VisualRow, VisualRowKind};
        let make = || VisualRow {
            kind: VisualRowKind::Plain,
            line_no_left: String::new(),
            line_no_right: String::new(),
            content: "let x = 1;".to_owned(),
            header_context: String::new(),
            spans: Vec::new(),
            word_changes: Vec::new(),
        };

        // The same highlighted row resolved under different palettes must
        // differ somewhere — proving the selection actually drives the
        // colours now that the row stores buckets, not colours.
        let mut highlighted = vec![make()];
        highlight_raw_rows(&mut highlighted, LanguageHint::Extension("rs"));
        let profile = |theme_name: &str| {
            use crate::ui::theme::{SyntaxPalette, syntax_theme_of};
            let theme = syntax_theme_of(SyntaxPalette::from_config_name(theme_name), false);
            highlighted[0]
                .spans
                .iter()
                .map(|s| (s.text.clone(), s.bucket.map(|b| theme.color(b))))
                .collect::<Vec<_>>()
        };

        let daruda = profile("daruda");
        assert_ne!(daruda, profile("one-dark"), "one-dark differs from daruda");
        assert_ne!(
            daruda,
            profile("tokyo-night"),
            "tokyo-night differs from daruda"
        );
        // Unknown / legacy names resolve to the recommended Daruda palette.
        assert_eq!(daruda, profile("base16-ocean.dark"), "legacy name → daruda");
        // Daruda carries a non-color channel on keywords (bold).
        let daruda_theme = crate::ui::theme::syntax_theme_of(Default::default(), false);
        assert!(
            highlighted[0]
                .spans
                .iter()
                .any(|s| s.text.contains("let")
                    && s.bucket.is_some_and(|b| daruda_theme.style(b).bold)),
            "daruda keyword span should be bold"
        );
    }

    /// Compiling a language's tree-sitter queries is the expensive part of
    /// highlighting, and the result is immutable, so every spelling of a
    /// language must land on one shared compilation. Without this the markdown
    /// viewer paid a full compile per fenced code block.
    #[test]
    fn a_language_compiles_its_queries_once() {
        let by_ext = build_config(LanguageHint::Extension("rs")).expect("rust via extension");
        let by_name = build_config(LanguageHint::FenceToken("rust")).expect("rust via fence");
        let by_short = build_config(LanguageHint::FenceToken("rs")).expect("rust via short fence");
        assert!(
            std::sync::Arc::ptr_eq(&by_ext, &by_name),
            "`rs` and `rust` compiled separately instead of sharing one config"
        );
        assert!(
            std::sync::Arc::ptr_eq(&by_ext, &by_short),
            "the same spelling compiled twice"
        );

        // A different language must not collide onto the same entry.
        let java = build_config(LanguageHint::Extension("java")).expect("java");
        assert!(
            !std::sync::Arc::ptr_eq(&by_ext, &java),
            "java and rust share a cache entry"
        );
    }

    /// Reproduction for the "raw file viewer shows no syntax colour" bug.
    ///
    /// The diff path (above) uses daruda's own `tree_sitter_highlight`
    /// pipeline and works. The *raw* editor path instead drives
    /// gpui_component's built-in `SyntaxHighlighter` (`code_editor("rust")`
    /// → `set_value` → `update_highlighter` → `styles`). This exercises that
    /// exact path in isolation: build the highlighter the way the editor
    /// does and assert it yields coloured spans for rust source. If this
    /// fails while the diff test passes, the bug is in the gpui_component
    /// highlighter path, not in the language data / registry.
    #[test]
    fn gpui_raw_editor_highlighter_colours_rust() {
        let code = "fn main() {\n    let x = 1;\n}\n";
        let rope = gpui_component::Rope::from_str(code);

        let mut highlighter = gpui_component::highlighter::SyntaxHighlighter::new("rust");
        highlighter.update(None, &rope);

        let theme = gpui_component::highlighter::HighlightTheme::default_dark();
        let styles = highlighter.styles(&(0..code.len()), &theme);

        let colored = styles.iter().filter(|(_, s)| s.color.is_some()).count();
        assert!(
            colored > 0,
            "gpui_component SyntaxHighlighter produced no coloured spans for \
             rust (raw editor path is broken): {styles:?}"
        );
    }

    /// End-to-end reproduction of the reported bug: a `.java` file opened
    /// in the **raw** file viewer showed no colour while `.rs` did. Drives
    /// the exact chain the pane builds — resolve the extension, hand the
    /// name to gpui_component's editor highlighter — for both, so a
    /// regression in either the resolver or the registry fails here.
    #[test]
    fn raw_editor_highlighter_colours_java_like_rust() {
        let cases = [
            ("java", "class A {\n    int x = 1;\n}\n"),
            ("rs", "fn main() {\n    let x = 1;\n}\n"),
        ];
        for (ext, code) in cases {
            let language = crate::ui::highlighter::language_for_extension(ext);
            let mut highlighter = gpui_component::highlighter::SyntaxHighlighter::new(&language);
            highlighter.update(None, &gpui_component::Rope::from_str(code));

            let theme = gpui_component::highlighter::HighlightTheme::default_dark();
            let styles = highlighter.styles(&(0..code.len()), &theme);
            let colored = styles.iter().filter(|(_, s)| s.color.is_some()).count();
            assert!(
                colored > 0,
                ".{ext} resolved to {language:?} but the raw editor path \
                 produced no coloured spans"
            );
        }

        // The defect itself: `.java` used to resolve to the empty string and
        // the pane opened it as `PLAIN_LANGUAGE`, whose query is empty. Pin
        // that down so the assertions above stay meaningful.
        let (_, java) = cases[0];
        let mut plain = gpui_component::highlighter::SyntaxHighlighter::new(
            crate::ui::highlighter::PLAIN_LANGUAGE,
        );
        plain.update(None, &gpui_component::Rope::from_str(java));
        let theme = gpui_component::highlighter::HighlightTheme::default_dark();
        assert!(
            plain
                .styles(&(0..java.len()), &theme)
                .iter()
                .all(|(_, s)| s.color.is_none()),
            "the plain language must not colour anything"
        );
    }

    /// Same as above but with the *daruda* highlight theme the live app
    /// installs (`apply_daruda_palette` seeds `style.syntax` from
    /// `editor_syntax_colors_of`), not the upstream `default_dark`. Rules out
    /// "the theme's `style(name)` mapping returns no colour for daruda's
    /// SyntaxColors" as the cause.
    #[test]
    fn gpui_raw_editor_highlighter_colours_rust_with_daruda_theme() {
        use crate::ui::theme::syntax;

        let code = "fn main() {\n    let x = 1;\n}\n";
        let rope = gpui_component::Rope::from_str(code);

        let mut highlighter = gpui_component::highlighter::SyntaxHighlighter::new("rust");
        highlighter.update(None, &rope);

        let mut theme = (*gpui_component::highlighter::HighlightTheme::default_dark()).clone();
        theme.style.syntax = syntax::editor_syntax_colors_of(syntax::SyntaxPalette::Daruda, false);

        let styles = highlighter.styles(&(0..code.len()), &theme);
        let colored = styles.iter().filter(|(_, s)| s.color.is_some()).count();
        assert!(
            colored > 0,
            "daruda highlight theme yields no coloured spans: {styles:?}"
        );
    }
}

//! Literal-text search over the current viewport (GPUI-free).
//!
//! We keep this module platform-free so it can run headless in unit
//! tests. The renderer (`element.rs`) translates `MatchRange`s into
//! background overlay quads at paint time. This mirrors Alacritty's
//! approach in `alacritty_terminal/src/term/search.rs` but without the
//! regex automaton — Phase 1 is literal-only.

use regex::{Regex, RegexBuilder};
use unicode_width::UnicodeWidthChar as _;
use unicode_width::UnicodeWidthStr as _;

use crate::TerminalSession;
use crate::session::FindOptions;

/// A single match located somewhere in the screen (scrollback +
/// viewport) coordinate space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchRange {
    /// 0-indexed row in the screen coordinate space (not the viewport
    /// — use `viewport_row_offset` to translate when rendering).
    pub row: u32,
    /// 1-indexed inclusive column range.
    pub start_col: u16,
    pub end_col: u16,
}

/// Case handling for literal search. ASCII-only fold — matches
/// Alacritty's simple folding in `search.rs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Case {
    Sensitive,
    Insensitive,
}

/// Literal or regex search state shared between paint and event paths.
#[derive(Clone, Debug, Default)]
pub struct SearchState {
    pub(super) query: String,
    pub(super) case_insensitive: bool,
    pub(super) is_regex: bool,
    /// True when `is_regex` but the pattern failed to compile.
    pub(super) regex_error: bool,
    pub(super) matches: Vec<MatchRange>,
    pub(super) focused: Option<usize>,
    /// Byte offset of the search-bar input caret inside `query`.
    pub(super) cursor_byte: usize,
}

impl SearchState {
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn is_regex(&self) -> bool {
        self.is_regex
    }

    pub fn regex_error(&self) -> bool {
        self.regex_error
    }

    pub fn case_insensitive(&self) -> bool {
        self.case_insensitive
    }

    pub fn match_count(&self) -> usize {
        self.matches.len()
    }

    pub fn focused_index(&self) -> Option<usize> {
        self.focused
    }
}

/// Combined output of [`scan_search_matches`]: a list of visual-row
/// match ranges plus a `regex_error` flag the search bar surfaces when
/// the user typed `is_regex = true` and the pattern failed to compile.
pub(super) struct ScanResult {
    pub matches: Vec<MatchRange>,
    pub regex_error: bool,
}

/// Single entry point that produces the full match list for the
/// search-bar overlay. Walks the scrollback portion of the unified
/// frame via [`crate::session::LineBuffer::find_matches`] (so multi-line
/// patterns can match across hard newlines and wrap boundaries) and
/// then scans the live viewport rows one at a time. Viewport rows
/// continue to use the per-row scan because the live grid lacks the
/// cross-row continuation state `FindContext` carries.
pub(super) fn scan_search_matches(
    session: &TerminalSession,
    query: &str,
    case_insensitive: bool,
    is_regex: bool,
) -> ScanResult {
    if query.is_empty() {
        return ScanResult {
            matches: Vec::new(),
            regex_error: false,
        };
    }
    let case = if case_insensitive {
        Case::Insensitive
    } else {
        Case::Sensitive
    };

    // Validate the regex up-front so we can surface a single
    // regex_error flag rather than relying on FindContext's silent
    // fall-back to its Invalid state.
    let viewport_regex = if is_regex {
        match compile_regex(query, case) {
            Some(re) => Some(re),
            None => {
                return ScanResult {
                    matches: Vec::new(),
                    regex_error: true,
                };
            }
        }
    } else {
        None
    };

    let cell_cols = session.cols();
    let lb_rows = session.line_buffer().wrapped_row_count(cell_cols);
    let total_rows = session.total_rows();

    let mut matches: Vec<MatchRange> = Vec::new();

    // Scrollback portion via FindContext (cross-line aware). Cell
    // columns are already 1-indexed inclusive in the triple returned
    // from `find_matches`, so we forward them directly.
    let opts = FindOptions {
        case_sensitive: !case_insensitive,
        regex: is_regex,
        forward: true,
    };
    for (row, start_col, end_col) in session.line_buffer().find_matches(query, opts, cell_cols) {
        matches.push(MatchRange {
            row,
            start_col,
            end_col,
        });
    }

    // Viewport portion via the existing per-row scanner. The live grid
    // does not carry cross-row continuation state, so multi-line
    // patterns can only match in scrollback for now.
    if total_rows > lb_rows {
        let mut viewport_lines: Vec<String> = Vec::with_capacity((total_rows - lb_rows) as usize);
        for y in lb_rows..total_rows {
            let line = match session.dump_screen_row(y) {
                Ok(s) => s.strip_suffix('\n').unwrap_or(s.as_str()).to_string(),
                Err(_) => String::new(),
            };
            viewport_lines.push(line);
        }
        if let Some(re) = viewport_regex.as_ref() {
            matches.extend(find_regex_matches_from(&viewport_lines, re, lb_rows));
        } else {
            matches.extend(find_literal_matches_from(
                &viewport_lines,
                query,
                case,
                lb_rows,
            ));
        }
    }

    ScanResult {
        matches,
        regex_error: false,
    }
}

/// Compile `pattern` as a regex honoring case handling. Mirrors
/// `RegexBuilder::case_insensitive` usage in Alacritty's
/// `term/search.rs`. Returns `None` on invalid patterns — callers are
/// expected to fall back to literal search.
pub(super) fn compile_regex(pattern: &str, case: Case) -> Option<Regex> {
    if pattern.is_empty() {
        return None;
    }
    RegexBuilder::new(pattern)
        .case_insensitive(matches!(case, Case::Insensitive))
        .build()
        .ok()
}

/// Regex-based counterpart to `find_literal_matches`. Overlapping hits
/// are collapsed the same way — the iterator returned by `find_iter`
/// is already non-overlapping.
#[cfg(test)]
pub(super) fn find_regex_matches(viewport_lines: &[String], re: &Regex) -> Vec<MatchRange> {
    find_regex_matches_from(viewport_lines, re, 0)
}

pub(super) fn find_regex_matches_from(
    lines: &[String],
    re: &Regex,
    row_offset: u32,
) -> Vec<MatchRange> {
    let mut out = Vec::new();
    for (idx, line) in lines.iter().enumerate() {
        for m in re.find_iter(line) {
            if m.is_empty() {
                continue;
            }
            let start_col = column_at_byte(line, m.start());
            let matched = &line[m.range()];
            let width = matched.width() as u16;
            let end_col = start_col.saturating_add(width.saturating_sub(1));
            out.push(MatchRange {
                row: row_offset + idx as u32,
                start_col,
                end_col,
            });
        }
    }
    out
}

fn ascii_fold(s: &str, case: Case) -> String {
    match case {
        Case::Sensitive => s.to_string(),
        Case::Insensitive => s.to_ascii_lowercase(),
    }
}

/// Scan `viewport_lines` for every non-overlapping occurrence of
/// `needle`. Empty needle returns an empty vector.
///
/// Match ranges are produced in viewport order (top-to-bottom,
/// left-to-right). Overlapping matches (`"aa"` in `"aaa"`) are
/// resolved by advancing past each hit (Alacritty-style), producing
/// one match for `"aaa"`.
#[cfg(test)]
pub(super) fn find_literal_matches(
    viewport_lines: &[String],
    needle: &str,
    case: Case,
) -> Vec<MatchRange> {
    find_literal_matches_from(viewport_lines, needle, case, 0)
}

/// Search variant that labels matches with absolute screen rows
/// starting from `row_offset`. Used by the scrollback-aware scan.
pub(super) fn find_literal_matches_from(
    lines: &[String],
    needle: &str,
    case: Case,
    row_offset: u32,
) -> Vec<MatchRange> {
    if needle.is_empty() {
        return Vec::new();
    }
    let needle_folded = ascii_fold(needle, case);
    let needle_width = needle.width() as u16;
    let mut out = Vec::new();

    for (idx, line) in lines.iter().enumerate() {
        let hay = ascii_fold(line, case);
        let mut search_start = 0usize;
        while let Some(rel) = hay[search_start..].find(&needle_folded) {
            let byte_start = search_start + rel;
            let byte_end = byte_start + needle_folded.len();
            let start_col = column_at_byte(line, byte_start);
            let end_col = start_col.saturating_add(needle_width.saturating_sub(1));
            out.push(MatchRange {
                row: row_offset + idx as u32,
                start_col,
                end_col,
            });
            search_start = byte_end;
        }
    }

    out
}

fn column_at_byte(line: &str, byte_index: usize) -> u16 {
    let clamped = byte_index.min(line.len());
    let mut col: u16 = 1;
    for (idx, ch) in line.char_indices() {
        if idx >= clamped {
            return col;
        }
        col = col.saturating_add(ch.width().unwrap_or(0) as u16);
    }
    col
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(s: &str) -> Vec<String> {
        vec![s.to_string()]
    }

    #[test]
    fn empty_pattern_returns_no_matches() {
        assert!(find_literal_matches(&line("anything"), "", Case::Sensitive).is_empty());
    }

    #[test]
    fn finds_single_occurrence_case_sensitive() {
        let out = find_literal_matches(&line("Hello World"), "World", Case::Sensitive);
        assert_eq!(
            out,
            vec![MatchRange {
                row: 0,
                start_col: 7,
                end_col: 11
            }]
        );
    }

    #[test]
    fn case_insensitive_matches_regardless_of_case() {
        let out = find_literal_matches(&line("Hello World"), "world", Case::Insensitive);
        assert_eq!(out.len(), 1);
        let out_no = find_literal_matches(&line("Hello World"), "world", Case::Sensitive);
        assert!(out_no.is_empty());
    }

    #[test]
    fn finds_multiple_occurrences_in_one_line() {
        let out = find_literal_matches(&line("foo bar foo baz foo"), "foo", Case::Sensitive);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].start_col, 1);
        assert_eq!(out[1].start_col, 9);
        assert_eq!(out[2].start_col, 17);
    }

    #[test]
    fn multiple_matches_in_same_line_have_distinct_start_cols() {
        // Regression: search_step cycling relied on (row, start_col)
        // being unique per match. Same-row duplicates must each expose
        // their own column so focus preservation does not collapse them.
        let out = find_literal_matches(&line("aa bb aa bb aa"), "aa", Case::Sensitive);
        let keys: Vec<(u32, u16)> = out.iter().map(|m| (m.row, m.start_col)).collect();
        assert_eq!(keys, vec![(0, 1), (0, 7), (0, 13)]);
        // Every key distinct (no collapse).
        let mut sorted = keys.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), keys.len());
    }

    #[test]
    fn finds_across_multiple_lines_in_order() {
        let lines = vec![
            "foo".to_string(),
            "bar foo".to_string(),
            "foo foo".to_string(),
        ];
        let out = find_literal_matches(&lines, "foo", Case::Sensitive);
        let rows: Vec<u32> = out.iter().map(|m| m.row).collect();
        assert_eq!(rows, vec![0, 1, 2, 2]);
    }

    #[test]
    fn overlapping_aa_in_aaa_advances_past_each_hit() {
        // 'aa' in 'aaa' — the non-overlapping strategy finds exactly one
        // match at col 1, then advances to col 3 leaving no room.
        let out = find_literal_matches(&line("aaa"), "aa", Case::Sensitive);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].start_col, 1);
        assert_eq!(out[0].end_col, 2);
    }

    #[test]
    fn handles_cjk_width_correctly() {
        // Korean chars are display width 2. "한글" starts at col 1,
        // occupies cols 1-4 (2 chars × width 2). The needle "글" is at
        // col 3-4.
        let out = find_literal_matches(&line("한글 테스트"), "글", Case::Sensitive);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].start_col, 3);
        assert_eq!(out[0].end_col, 4);
    }

    #[test]
    fn empty_viewport_returns_no_matches() {
        let empty: Vec<String> = Vec::new();
        assert!(find_literal_matches(&empty, "x", Case::Sensitive).is_empty());
    }

    #[test]
    fn no_match_returns_empty() {
        assert!(find_literal_matches(&line("hello"), "xyz", Case::Sensitive).is_empty());
    }

    #[test]
    fn match_at_end_of_line_end_col_is_correct() {
        let out = find_literal_matches(&line("ab"), "b", Case::Sensitive);
        assert_eq!(
            out,
            vec![MatchRange {
                row: 0,
                start_col: 2,
                end_col: 2
            }]
        );
    }

    // Regex search ----------------------------------------------------

    #[test]
    fn compile_regex_rejects_empty_pattern() {
        assert!(compile_regex("", Case::Sensitive).is_none());
    }

    #[test]
    fn compile_regex_rejects_invalid_pattern() {
        assert!(compile_regex("(unclosed", Case::Sensitive).is_none());
    }

    #[test]
    fn regex_finds_word_boundary_matches() {
        let re = compile_regex(r"\bfoo\b", Case::Sensitive).unwrap();
        let lines = vec![
            "foo bar".to_string(),
            "foobar".to_string(),
            "(foo)".to_string(),
        ];
        let out = find_regex_matches(&lines, &re);
        let rows: Vec<u32> = out.iter().map(|m| m.row).collect();
        // Row 0 matches "foo"; row 1 does not (word-internal); row 2
        // matches (parentheses are non-word).
        assert_eq!(rows, vec![0, 2]);
    }

    #[test]
    fn regex_case_insensitive_flag_respected() {
        let re = compile_regex("FOO", Case::Insensitive).unwrap();
        let out = find_regex_matches(&line("foo Foo FOO"), &re);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn regex_character_class_matches() {
        let re = compile_regex(r"\d{3}", Case::Sensitive).unwrap();
        let out = find_regex_matches(&line("abc 123 xyz 456"), &re);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].start_col, 5);
        assert_eq!(out[1].start_col, 13);
    }

    #[test]
    fn regex_skips_zero_width_matches() {
        // `(?:)` can match at every position but we must not emit
        // infinite results.
        let re = compile_regex(r"a*", Case::Sensitive).unwrap();
        let out = find_regex_matches(&line("bbb"), &re);
        // Every zero-width `a*` match must be skipped.
        assert!(out.iter().all(|m| m.start_col <= m.end_col));
    }
}

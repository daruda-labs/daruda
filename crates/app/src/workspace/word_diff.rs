//! Word-level diff computation for the diff viewer.
//!
//! Scans parsed hunks for consecutive Removed/Added blocks and annotates them
//! with intra-line change ranges using `similar`.
//!
//! Pairing strategy: consecutive Removed lines and the consecutive Added lines
//! immediately following them are treated as a section pair. Only equal-count
//! sections (N:N) receive word diff; N≠M sections and lines over
//! `WORD_DIFF_LINE_LIMIT` bytes are left without intra-line highlights.
//!
//! Tokenisation: runs of alphanumeric/`_` characters form one token each;
//! every other character (whitespace, operator, punctuation) is its own token.
//! Adjacent changed tokens are coalesced into contiguous byte ranges.
//!
//! GPUI-free — safe to call on `background_executor`.

use similar::{ChangeTag, TextDiff};

use super::pane_file_view::{DiffHunk, DiffLine, WordChange};

/// Maximum line byte length for which word diff is computed. Lines longer than
/// this are left without intra-line highlights (performance guard).
const WORD_DIFF_LINE_LIMIT: usize = 300;

/// Scan every hunk for consecutive Removed/Added blocks and annotate them
/// with word-level change ranges in-place.
///
/// Pairing rule (mirrors GitButler):
/// - Collect a run of consecutive `Removed` lines, then a run of consecutive
///   `Added` lines immediately following.
/// - Only pair when both runs have the **same length** (N:N). N≠M blocks are
///   skipped — the line counts differ too much for meaningful pairing.
/// - Lines longer than `WORD_DIFF_LINE_LIMIT` bytes are skipped individually.
pub(in crate::workspace) fn apply_word_diff(hunks: &mut [DiffHunk]) {
    for hunk in hunks.iter_mut() {
        let mut i = 0;
        while i < hunk.lines.len() {
            // Skip until a Removed line.
            if !matches!(hunk.lines[i], DiffLine::Removed { .. }) {
                i += 1;
                continue;
            }

            // Collect a consecutive run of Removed lines.
            let removed_start = i;
            while i < hunk.lines.len() && matches!(hunk.lines[i], DiffLine::Removed { .. }) {
                i += 1;
            }
            let removed_end = i;

            // Must be immediately followed by Added lines.
            if i >= hunk.lines.len() || !matches!(hunk.lines[i], DiffLine::Added { .. }) {
                continue;
            }

            // Collect a consecutive run of Added lines.
            let added_start = i;
            while i < hunk.lines.len() && matches!(hunk.lines[i], DiffLine::Added { .. }) {
                i += 1;
            }
            let added_end = i;

            let removed_count = removed_end - removed_start;
            let added_count = added_end - added_start;

            // Skip N≠M blocks — line counts too different to pair meaningfully.
            if removed_count != added_count {
                continue;
            }

            for j in 0..removed_count {
                let old = match &hunk.lines[removed_start + j] {
                    DiffLine::Removed { content, .. } => content.clone(),
                    _ => unreachable!(),
                };
                let new = match &hunk.lines[added_start + j] {
                    DiffLine::Added { content, .. } => content.clone(),
                    _ => unreachable!(),
                };

                // Skip very long lines to keep diffing responsive.
                if old.len() > WORD_DIFF_LINE_LIMIT || new.len() > WORD_DIFF_LINE_LIMIT {
                    continue;
                }

                let (old_changes, new_changes) = compute_word_diff(&old, &new);

                if let DiffLine::Removed { word_changes, .. } = &mut hunk.lines[removed_start + j] {
                    *word_changes = old_changes;
                }
                if let DiffLine::Added { word_changes, .. } = &mut hunk.lines[added_start + j] {
                    *word_changes = new_changes;
                }
            }
        }
    }
}

/// Split a string into tokens for word-level diffing.
///
/// Each token is either a run of word characters (letters, digits, `_`) or a
/// single non-word character (whitespace, punctuation, operator, …). This
/// gives natural code-word boundaries without requiring Unicode segmentation.
fn tokenize(s: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut start = 0usize;
    let mut in_word = false;

    for (i, ch) in s.char_indices() {
        let is_word_ch = ch.is_alphanumeric() || ch == '_';
        if in_word && !is_word_ch {
            tokens.push(&s[start..i]);
            start = i;
            in_word = false;
        } else if !in_word && is_word_ch {
            if start < i {
                // Each non-word character is its own single token.
                for (j, c) in s[start..i].char_indices() {
                    let abs = start + j;
                    tokens.push(&s[abs..abs + c.len_utf8()]);
                }
            }
            start = i;
            in_word = true;
        } else if !in_word {
            // Accumulate each non-word char as its own token on the next transition.
        }
    }
    // Flush the final segment.
    if start < s.len() {
        if in_word {
            tokens.push(&s[start..]);
        } else {
            for (j, c) in s[start..].char_indices() {
                let abs = start + j;
                tokens.push(&s[abs..abs + c.len_utf8()]);
            }
        }
    }
    tokens
}

/// Compute word-level diff between two strings.
/// Returns `(old_changes, new_changes)` where each `WordChange` is a byte
/// range within its respective string that should be highlighted.
/// Adjacent changed tokens are coalesced into a single range.
pub(in crate::workspace) fn compute_word_diff(
    old: &str,
    new: &str,
) -> (Vec<WordChange>, Vec<WordChange>) {
    let old_tokens = tokenize(old);
    let new_tokens = tokenize(new);

    let diff = TextDiff::from_slices(&old_tokens, &new_tokens);

    let mut old_changes: Vec<WordChange> = Vec::new();
    let mut new_changes: Vec<WordChange> = Vec::new();
    let mut old_pos = 0usize;
    let mut new_pos = 0usize;

    for change in diff.iter_all_changes() {
        let token = change.value();
        let bytes = token.len();
        match change.tag() {
            ChangeTag::Equal => {
                old_pos += bytes;
                new_pos += bytes;
            }
            ChangeTag::Delete => {
                match old_changes.last_mut() {
                    Some(last) if last.end == old_pos => last.end += bytes,
                    _ => old_changes.push(WordChange {
                        start: old_pos,
                        end: old_pos + bytes,
                    }),
                }
                old_pos += bytes;
            }
            ChangeTag::Insert => {
                match new_changes.last_mut() {
                    Some(last) if last.end == new_pos => last.end += bytes,
                    _ => new_changes.push(WordChange {
                        start: new_pos,
                        end: new_pos + bytes,
                    }),
                }
                new_pos += bytes;
            }
        }
    }

    (old_changes, new_changes)
}

// ----------------------------------------------------------------
// Tests
// ----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_strings_no_changes() {
        let (old, new) = compute_word_diff("hello", "hello");
        assert!(old.is_empty());
        assert!(new.is_empty());
    }

    #[test]
    fn fully_different_strings() {
        let (old, new) = compute_word_diff("aaa", "bbb");
        assert_eq!(old.len(), 1);
        assert_eq!(old[0].start, 0);
        assert_eq!(old[0].end, 3);
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].start, 0);
        assert_eq!(new[0].end, 3);
    }

    #[test]
    fn prefix_change() {
        // "foo_bar" → "baz_bar" — prefix "foo" replaced by "baz"
        let (old, new) = compute_word_diff("foo_bar", "baz_bar");
        assert!(!old.is_empty());
        // Both "foo" in old and "baz" in new should be in the change ranges.
        let old_text: &str = &"foo_bar"[old[0].start..old[0].end];
        let new_text: &str = &"baz_bar"[new[0].start..new[0].end];
        assert!(old_text.contains('f') || old_text.contains('o'));
        assert!(new_text.contains('b') || new_text.contains('a'));
    }

    #[test]
    fn apply_word_diff_marks_adjacent_pair() {
        use super::super::pane_file_view::parse_diff_hunks;
        let diff = "@@ -1,2 +1,2 @@\n-foo_old\n+foo_new\n";
        let mut hunks = parse_diff_hunks(diff);
        apply_word_diff(&mut hunks);
        let removed = &hunks[0].lines[0];
        let added = &hunks[0].lines[1];
        let has_any = match (removed, added) {
            (
                DiffLine::Removed {
                    word_changes: r, ..
                },
                DiffLine::Added {
                    word_changes: a, ..
                },
            ) => !r.is_empty() || !a.is_empty(),
            _ => false,
        };
        assert!(has_any, "expected word changes on the Removed/Added pair");
    }

    #[test]
    fn apply_word_diff_skips_non_adjacent() {
        use super::super::pane_file_view::parse_diff_hunks;
        // Context line between Removed and Added — no pairing.
        let diff = "@@ -1,3 +1,3 @@\n-foo\n ctx\n+bar\n";
        let mut hunks = parse_diff_hunks(diff);
        apply_word_diff(&mut hunks);
        let removed = &hunks[0].lines[0];
        match removed {
            DiffLine::Removed { word_changes, .. } => {
                assert!(
                    word_changes.is_empty(),
                    "non-adjacent pair should not be word-diffed"
                );
            }
            _ => panic!("expected Removed"),
        }
    }

    // ---- Multi-line block pairing ----

    #[test]
    fn apply_word_diff_pairs_multi_line_block_correctly() {
        use super::super::pane_file_view::parse_diff_hunks;
        // 2 removed + 2 added: removed[0]↔added[0], removed[1]↔added[1].
        // The old 1:1 scan would pair removed[1]↔added[0] (wrong).
        let diff =
            "@@ -1,2 +1,2 @@\n-fn foo(x: i32)\n-fn bar(a: i32)\n+fn foo(y: i32)\n+fn bar(b: i32)\n";
        let mut hunks = parse_diff_hunks(diff);
        apply_word_diff(&mut hunks);

        // removed[0] = "fn foo(x: i32)" — only "x" should be highlighted.
        // removed[1] = "fn bar(a: i32)" — only "a" should be highlighted.
        let (r0_changes, r1_changes, a0_changes, a1_changes) = match (
            &hunks[0].lines[0],
            &hunks[0].lines[1],
            &hunks[0].lines[2],
            &hunks[0].lines[3],
        ) {
            (
                DiffLine::Removed {
                    word_changes: r0, ..
                },
                DiffLine::Removed {
                    word_changes: r1, ..
                },
                DiffLine::Added {
                    word_changes: a0, ..
                },
                DiffLine::Added {
                    word_changes: a1, ..
                },
            ) => (r0, r1, a0, a1),
            _ => panic!("unexpected line types"),
        };

        // Both removed lines and both added lines must have word changes.
        assert!(
            !r0_changes.is_empty(),
            "removed[0] should have word changes"
        );
        assert!(
            !r1_changes.is_empty(),
            "removed[1] should have word changes"
        );
        assert!(!a0_changes.is_empty(), "added[0] should have word changes");
        assert!(!a1_changes.is_empty(), "added[1] should have word changes");

        // Verify correct pairing: removed[0] ("x") pairs with added[0] ("y"),
        // not with added[1] ("b"). Check by confirming the highlighted byte
        // range on removed[0] covers only "x" (byte 7 in "fn foo(x: i32)").
        let old0 = "fn foo(x: i32)";
        let highlighted: &str = &old0[r0_changes[0].start..r0_changes[0].end];
        assert!(
            highlighted.contains('x'),
            "removed[0] highlight should cover 'x', got {:?}",
            highlighted
        );
    }

    #[test]
    fn apply_word_diff_skips_unequal_block() {
        use super::super::pane_file_view::parse_diff_hunks;
        // 2 removed + 1 added — N≠M, no word diff on either side.
        let diff = "@@ -1,2 +1,1 @@\n-foo\n-bar\n+baz\n";
        let mut hunks = parse_diff_hunks(diff);
        apply_word_diff(&mut hunks);

        for line in &hunks[0].lines {
            match line {
                DiffLine::Removed { word_changes, .. } | DiffLine::Added { word_changes, .. } => {
                    assert!(
                        word_changes.is_empty(),
                        "N≠M block should not be word-diffed"
                    );
                }
                _ => {}
            }
        }
    }

    #[test]
    fn apply_word_diff_skips_long_lines() {
        use super::super::pane_file_view::parse_diff_hunks;
        let long = "x".repeat(WORD_DIFF_LINE_LIMIT + 1);
        let diff = format!("@@ -1,2 +1,2 @@\n-{long}\n+{long}z\n");
        let mut hunks = parse_diff_hunks(&diff);
        apply_word_diff(&mut hunks);

        for line in &hunks[0].lines {
            match line {
                DiffLine::Removed { word_changes, .. } | DiffLine::Added { word_changes, .. } => {
                    assert!(
                        word_changes.is_empty(),
                        "lines over the length limit should not be word-diffed"
                    );
                }
                _ => {}
            }
        }
    }
}

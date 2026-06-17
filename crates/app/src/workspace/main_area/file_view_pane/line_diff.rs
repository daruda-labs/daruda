//! In-app line diff via `imara-diff` (Histogram).
//!
//! Produces a unified-diff text (`@@ -a,b +c,d @@` hunks with ` `/`+`/`-`
//! line prefixes and N context lines) from the old and new file contents,
//! so the existing [`super::parse_diff_hunks`] consumes it unchanged. This
//! lets the viewer compute diffs itself with the Histogram algorithm —
//! which favours human-readable hunks over Myers' minimal edit script —
//! instead of relying on `git diff`'s output.
//!
//! GPUI-free; safe to call on `background_executor`.

use imara_diff::{Algorithm, BasicLineDiffPrinter, Diff, InternedInput, UnifiedDiffConfig};

/// Compute a unified diff between `old` and `new` using the Histogram
/// algorithm, with the default 3 lines of context. The result is in the
/// same format `git diff` emits (minus the file header), ready for
/// [`super::parse_diff_hunks`].
pub(in crate::workspace) fn unified_diff_text(old: &str, new: &str) -> String {
    let input = InternedInput::new(old, new);
    let mut diff = Diff::compute(Algorithm::Histogram, &input);
    // Slider postprocessing picks human-friendly hunk boundaries; always
    // wanted when the diff is shown to a person.
    diff.postprocess_lines(&input);
    diff.unified_diff(
        &BasicLineDiffPrinter(&input.interner),
        UnifiedDiffConfig::default(),
        &input,
    )
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::super::{DiffLine, parse_diff_hunks};
    use super::unified_diff_text;

    fn count_added_removed(text: &str) -> (usize, usize, Vec<String>, Vec<String>) {
        let hunks = parse_diff_hunks(text);
        let (mut added, mut removed) = (0, 0);
        let (mut add_txt, mut rem_txt) = (Vec::new(), Vec::new());
        for h in &hunks {
            for l in &h.lines {
                match l {
                    DiffLine::Added { content, .. } => {
                        added += 1;
                        add_txt.push(content.clone());
                    }
                    DiffLine::Removed { content, .. } => {
                        removed += 1;
                        rem_txt.push(content.clone());
                    }
                    _ => {}
                }
            }
        }
        (added, removed, add_txt, rem_txt)
    }

    #[test]
    fn identical_inputs_produce_no_hunks() {
        let s = "fn a() {}\nlet x = 1;\n";
        let text = unified_diff_text(s, s);
        assert!(parse_diff_hunks(&text).is_empty());
    }

    #[test]
    fn single_line_modification_round_trips_through_parser() {
        let old = "fn a() {}\nlet x = 1;\nfn b() {}\n";
        let new = "fn a() {}\nlet y = 2;\nfn b() {}\n";
        let text = unified_diff_text(old, new);
        let (added, removed, add_txt, rem_txt) = count_added_removed(&text);
        assert_eq!(removed, 1, "one line removed");
        assert_eq!(added, 1, "one line added");
        assert_eq!(rem_txt, vec!["let x = 1;".to_owned()]);
        assert_eq!(add_txt, vec!["let y = 2;".to_owned()]);
    }

    #[test]
    fn pure_insertion_has_no_removals() {
        let old = "a\nb\n";
        let new = "a\nb\nc\n";
        let text = unified_diff_text(old, new);
        let (added, removed, add_txt, _) = count_added_removed(&text);
        assert_eq!(removed, 0);
        assert_eq!(added, 1);
        assert_eq!(add_txt, vec!["c".to_owned()]);
    }

    #[test]
    fn unequal_block_emits_valid_hunk() {
        // 2 removed → 3 added; parser must still produce a coherent hunk.
        let old = "x\ny\n";
        let new = "p\nq\nr\n";
        let text = unified_diff_text(old, new);
        let (added, removed, _, _) = count_added_removed(&text);
        assert_eq!(removed, 2);
        assert_eq!(added, 3);
    }
}

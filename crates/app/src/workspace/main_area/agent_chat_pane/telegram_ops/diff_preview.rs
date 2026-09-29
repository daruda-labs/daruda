//! A permission request's file changes, shortened for a phone: the changed
//! lines of each diff, `-` removed and `+` added, under the file's name. The
//! whole change can be far larger than a message, so a line and a character
//! budget stop it, and a closing line says how much was left out.

use daruda_acp::DiffView;

/// Changed lines a preview shows before it stops.
const PREVIEW_LINES: usize = 12;
/// Characters a preview spends before it stops.
const PREVIEW_CHARS: usize = 900;
/// Characters one line keeps; a minified line would spend the whole budget.
const LINE_CHARS: usize = 120;

/// The preview for `diffs`, or `None` when they change nothing.
/// `more` names how many changed lines were left out.
pub(super) fn diff_preview(diffs: &[DiffView], more: impl Fn(usize) -> String) -> Option<String> {
    let mut out = String::new();
    let mut shown = 0usize;
    let mut left_out = 0usize;
    for diff in diffs {
        let changed = changed_lines(diff);
        if changed.is_empty() {
            continue;
        }
        let name = diff
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| diff.path.display().to_string());
        let mut named = false;
        for line in changed {
            let full = shown >= PREVIEW_LINES || out.chars().count() >= PREVIEW_CHARS;
            if full {
                left_out += 1;
                continue;
            }
            if !named {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&name);
                named = true;
            }
            out.push('\n');
            out.push_str(&clip(&line));
            shown += 1;
        }
    }
    if out.is_empty() {
        return None;
    }
    if left_out > 0 {
        out.push('\n');
        out.push_str(&more(left_out));
    }
    Some(out)
}

/// The lines `diff` removes and adds, in order, each with its sign.
fn changed_lines(diff: &DiffView) -> Vec<String> {
    let old = diff.old_text.as_deref().unwrap_or("");
    similar::TextDiff::from_lines(old, &diff.new_text)
        .iter_all_changes()
        .filter_map(|change| {
            let sign = match change.tag() {
                similar::ChangeTag::Delete => '-',
                similar::ChangeTag::Insert => '+',
                similar::ChangeTag::Equal => return None,
            };
            Some(format!("{sign} {}", change.value().trim_end_matches(['\n', '\r'])))
        })
        .collect()
}

fn clip(line: &str) -> String {
    if line.chars().count() <= LINE_CHARS {
        return line.to_string();
    }
    let mut kept: String = line.chars().take(LINE_CHARS).collect();
    kept.push('…');
    kept
}

/// Escape administrative text before it shares a Markdown tail with the diff.
/// Escaping every ASCII punctuation mark is deliberately conservative: the
/// parser removes valid backslash escapes, while paths and commands stay text.
pub(super) fn escape_markdown_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii_punctuation() {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// Put a preview in a fenced `diff` block. The fence is longer than any run
/// of backticks in the preview, so file content cannot close it early.
pub(super) fn fenced_diff(preview: &str) -> String {
    let mut run = 0usize;
    let mut longest = 0usize;
    for ch in preview.chars() {
        if ch == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}diff\n{preview}\n{fence}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn diff(path: &str, old: Option<&str>, new: &str) -> DiffView {
        DiffView {
            path: PathBuf::from(path),
            old_text: old.map(str::to_string),
            new_text: new.to_string(),
        }
    }

    fn more(n: usize) -> String {
        format!("({n} more)")
    }

    #[test]
    fn an_edit_shows_only_the_lines_it_changes_under_the_file_name() {
        let preview = diff_preview(
            &[diff("/repo/src/x.rs", Some("a\nb\nc\n"), "a\nB\nc\n")],
            more,
        );
        assert_eq!(preview.as_deref(), Some("x.rs\n- b\n+ B"));
    }

    #[test]
    fn a_new_file_is_all_additions() {
        let preview = diff_preview(&[diff("new.txt", None, "one\ntwo\n")], more);
        assert_eq!(preview.as_deref(), Some("new.txt\n+ one\n+ two"));
    }

    #[test]
    fn a_change_that_changes_nothing_has_no_preview() {
        assert_eq!(diff_preview(&[diff("same.rs", Some("a\n"), "a\n")], more), None);
        assert_eq!(diff_preview(&[], more), None);
    }

    #[test]
    fn a_large_change_stops_at_the_budget_and_says_how_much_is_left() {
        let new: String = (0..40).map(|i| format!("line {i}\n")).collect();
        let preview = diff_preview(&[diff("big.rs", None, &new)], more).unwrap();
        assert_eq!(preview.lines().filter(|l| l.starts_with('+')).count(), PREVIEW_LINES);
        assert!(preview.ends_with(&more(40 - PREVIEW_LINES)), "{preview}");
    }

    #[test]
    fn a_long_line_is_clipped() {
        let long = "x".repeat(500);
        let preview = diff_preview(&[diff("min.js", None, &long)], more).unwrap();
        let line = preview.lines().nth(1).unwrap();
        assert_eq!(line.chars().count(), LINE_CHARS + 1);
        assert!(line.ends_with('…'));
    }

    #[test]
    fn each_file_is_named_once_above_its_lines() {
        let preview = diff_preview(
            &[diff("a.rs", None, "1\n"), diff("b.rs", Some("2\n"), "")],
            more,
        );
        assert_eq!(preview.as_deref(), Some("a.rs\n+ 1\nb.rs\n- 2"));
    }

    #[test]
    fn administrative_text_is_literal_when_the_tail_is_markdown() {
        assert_eq!(
            escape_markdown_text("Edit a_b.rs\nrm -rf *.log"),
            "Edit a\\_b\\.rs\nrm \\-rf \\*\\.log"
        );
    }

    #[test]
    fn a_backtick_run_in_content_cannot_close_the_diff_block() {
        let block = fenced_diff("x.rs\n+ ```");
        assert!(block.starts_with("````diff\n"), "{block}");
        assert!(block.ends_with("\n````"), "{block}");
    }
}

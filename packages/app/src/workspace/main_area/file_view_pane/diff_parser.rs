//! Unified-diff parser — converts `git diff` text into the typed
//! `DiffHunk` / `DiffLine` shape consumed by the diff renderer and the
//! word-level change detector.
//!
//! GPUI-free; safe to run on `cx.background_executor`.

use super::{HighlightedSpan, WordChange};

pub(in crate::workspace) struct DiffHunk {
    /// The `@@ -old +new @@` portion of the hunk header.
    pub header: String,
    /// Trailing context text after the closing `@@` (e.g. `fn foo() {`).
    /// Empty when the hunk header has no trailing context.
    pub header_context: String,
    #[allow(dead_code)]
    pub old_start: usize,
    #[allow(dead_code)]
    pub new_start: usize,
    pub lines: Vec<DiffLine>,
}

pub(in crate::workspace) enum DiffLine {
    Context {
        old_no: usize,
        new_no: usize,
        content: String,
        spans: Vec<HighlightedSpan>,
    },
    Added {
        new_no: usize,
        content: String,
        spans: Vec<HighlightedSpan>,
        word_changes: Vec<WordChange>,
    },
    Removed {
        old_no: usize,
        content: String,
        spans: Vec<HighlightedSpan>,
        word_changes: Vec<WordChange>,
    },
    NoNewline,
}

/// Parse unified diff text (output of `git diff`) into hunks.
pub(in crate::workspace) fn parse_diff_hunks(diff_text: &str) -> Vec<DiffHunk> {
    let mut hunks: Vec<DiffHunk> = Vec::new();
    let mut old_line = 0usize;
    let mut new_line = 0usize;

    for raw_line in diff_text.lines() {
        // Skip git header lines.
        if raw_line.starts_with("diff --git")
            || raw_line.starts_with("index ")
            || raw_line.starts_with("--- ")
            || raw_line.starts_with("+++ ")
        {
            continue;
        }

        if raw_line.starts_with("@@") {
            let (old_start, new_start) = parse_hunk_coords(raw_line);
            let (header, header_context) = split_hunk_header(raw_line);
            old_line = old_start;
            new_line = new_start;
            hunks.push(DiffHunk {
                header,
                header_context,
                old_start,
                new_start,
                lines: Vec::new(),
            });
            continue;
        }

        let Some(hunk) = hunks.last_mut() else {
            continue;
        };

        if let Some(content) = raw_line.strip_prefix('+') {
            hunk.lines.push(DiffLine::Added {
                new_no: new_line,
                content: content.to_owned(),
                spans: Vec::new(),
                word_changes: Vec::new(),
            });
            new_line += 1;
        } else if let Some(content) = raw_line.strip_prefix('-') {
            hunk.lines.push(DiffLine::Removed {
                old_no: old_line,
                content: content.to_owned(),
                spans: Vec::new(),
                word_changes: Vec::new(),
            });
            old_line += 1;
        } else if raw_line.starts_with('\\') {
            hunk.lines.push(DiffLine::NoNewline);
        } else {
            // Context line (leading space or bare empty line).
            let content = raw_line.strip_prefix(' ').unwrap_or("").to_owned();
            hunk.lines.push(DiffLine::Context {
                old_no: old_line,
                new_no: new_line,
                content,
                spans: Vec::new(),
            });
            old_line += 1;
            new_line += 1;
        }
    }

    hunks
}

/// Split `@@ -old +new @@ trailing context` into `(header, context)`.
/// `header` contains everything up to and including the closing `@@`.
/// `context` is the trailing text trimmed of surrounding whitespace.
fn split_hunk_header(raw: &str) -> (String, String) {
    // The format is `@@ ... @@ [optional context]`.
    // Find the second `@@` by searching from position 2 (after the opening @@).
    if let Some(rel) = raw[2..].find("@@") {
        let split = 2 + rel + 2; // byte position right after the closing @@
        let header = raw[..split].trim_end().to_owned();
        let context = raw[split..].trim().to_owned();
        (header, context)
    } else {
        (raw.to_owned(), String::new())
    }
}

/// Extract old_start and new_start from `@@ -old_start,... +new_start,... @@`.
fn parse_hunk_coords(header: &str) -> (usize, usize) {
    let mut old_start = 1usize;
    let mut new_start = 1usize;
    for part in header.split_whitespace() {
        if let Some(old) = part.strip_prefix('-') {
            old_start = old
                .split(',')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1);
        } else if let Some(new) = part.strip_prefix('+') {
            new_start = new
                .split(',')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1);
        }
    }
    (old_start, new_start)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunk_header_helpers_cases() {
        for (raw, expected_header, expected_context, expected_old, expected_new) in [
            ("@@ -1,6 +1,8 @@", "@@ -1,6 +1,8 @@", "", 1, 1),
            (
                "@@ -10,4 +12,6 @@ fn foo() {",
                "@@ -10,4 +12,6 @@",
                "fn foo() {",
                10,
                12,
            ),
        ] {
            let (header, context) = split_hunk_header(raw);
            assert_eq!(header, expected_header);
            assert_eq!(context, expected_context);

            let (old, new) = parse_hunk_coords(raw);
            assert_eq!((old, new), (expected_old, expected_new));
        }
    }
}

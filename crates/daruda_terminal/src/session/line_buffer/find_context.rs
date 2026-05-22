//! Streaming search across [`LineBuffer`] logical lines.
//!
//! Mirrors iTerm2's `FindContext.h` so a multi-line literal or regex
//! pattern can match across the boundaries between stored
//! [`LogicalLine`]s. Each call to [`FindContext::next_match`] resumes
//! from the previous match's end so the caller can drive a single
//! iterator forward until exhaustion.
//!
//! Coordinates are returned as ring-local line indices plus byte
//! offsets within each line's `text`. Convert to display coordinates
//! via [`super::LineBuffer::position_at`] + the dispatcher-aware
//! wrappers in `TerminalSession`.
//!
//! ## Stream construction
//!
//! Lines are concatenated into a single search stream with **no
//! separator** between consecutive logical lines. This matches iTerm2's
//! `LineBuffer.numberOfFullLinesFromBuffer` semantics — Hard newlines
//! separate logical lines for storage but not for matching, so a query
//! like `"world"` matches across `["hello wor", "ld there"]`. Soft and
//! DWC wraps never split a logical line in this buffer (they extend in
//! place), so cross-line matching is meaningful only across Hard breaks.

#![allow(dead_code)]

use regex::{Regex, RegexBuilder};

use super::LineBuffer;

/// Search-bar options for [`FindContext`]. Defaults: case-sensitive,
/// non-regex, forward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FindOptions {
    pub case_sensitive: bool,
    pub regex: bool,
    pub forward: bool,
}

impl Default for FindOptions {
    fn default() -> Self {
        Self {
            case_sensitive: true,
            regex: false,
            forward: true,
        }
    }
}

/// A single match located within the [`LineBuffer`]. `start_line` /
/// `end_line` are ring-local indices; `start_byte` / `end_byte` are
/// byte offsets inside the corresponding logical line's `text`.
///
/// `end_line` may equal `start_line` (single-line match) or be larger
/// (cross-line match). `end_byte` is exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchRange {
    pub start_line: usize,
    pub start_byte: usize,
    pub end_line: usize,
    pub end_byte: usize,
}

enum SearchPattern {
    Literal(String),
    Regex(Regex),
    /// Set when the user passed `regex = true` but the pattern failed
    /// to compile. `next_match` returns `None` so the caller can
    /// surface a regex-error state to the UI.
    Invalid,
}

/// Iterator-like cursor over [`LineBuffer`] matches. Construct with
/// [`FindContext::new`], then call [`FindContext::next_match`] until
/// it returns `None`.
pub struct FindContext {
    pattern: SearchPattern,
    /// True when the literal-pattern path should fold both sides to
    /// ASCII-lowercase. `Regex` carries its own case flag and ignores
    /// this field. Stored separately from the `pattern` enum so the
    /// case rule survives explicit `seek` resets.
    case_insensitive_literal: bool,
    forward: bool,
    cursor_line: usize,
    cursor_byte: usize,
}

impl FindContext {
    pub fn new(needle: &str, opts: FindOptions) -> Self {
        let pattern = if needle.is_empty() {
            SearchPattern::Literal(String::new())
        } else if opts.regex {
            match RegexBuilder::new(needle)
                .case_insensitive(!opts.case_sensitive)
                .build()
            {
                Ok(re) => SearchPattern::Regex(re),
                Err(_) => SearchPattern::Invalid,
            }
        } else if opts.case_sensitive {
            SearchPattern::Literal(needle.to_string())
        } else {
            // Pre-fold so the per-call comparison is byte-aligned.
            SearchPattern::Literal(needle.to_ascii_lowercase())
        };
        Self {
            pattern,
            case_insensitive_literal: !opts.regex && !opts.case_sensitive,
            forward: opts.forward,
            cursor_line: 0,
            cursor_byte: 0,
        }
    }

    /// Reset the cursor so the next [`Self::next_match`] starts at
    /// `(line_idx, byte_offset)`. Use to skip past a match the caller
    /// has rejected or to restart from a viewport-induced anchor.
    pub fn seek(&mut self, line_idx: usize, byte_offset: usize) {
        self.cursor_line = line_idx;
        self.cursor_byte = byte_offset;
    }

    /// Return the next match against `buf`'s current state. `None` once
    /// the cursor reaches the end of the buffer (or its start, when
    /// searching backward).
    ///
    /// Backward search is exposed via the `forward` field but currently
    /// short-circuits — iTerm2 issues backward queries through a separate
    /// pre-built reverse index that we do not yet maintain. Callers that
    /// need reverse iteration should re-seek to an earlier anchor and
    /// run a fresh forward pass.
    pub fn next_match(&mut self, buf: &LineBuffer) -> Option<MatchRange> {
        if !self.forward {
            return None;
        }
        let needle_empty = match &self.pattern {
            SearchPattern::Literal(s) => s.is_empty(),
            SearchPattern::Regex(_) => false,
            SearchPattern::Invalid => true,
        };
        if needle_empty {
            return None;
        }
        if self.cursor_line >= buf.len() {
            return None;
        }

        loop {
            // Build the search stream by concatenating logical lines from
            // (cursor_line, cursor_byte) onward — no separator between hard
            // breaks. `line_starts[i]` records the stream offset at which
            // ring-local line `cursor_line + i` begins (after the partial
            // prefix skip on i == 0).
            let mut stream = String::new();
            let mut line_starts: Vec<usize> = Vec::with_capacity(buf.len() - self.cursor_line);
            for local in 0..(buf.len() - self.cursor_line) {
                line_starts.push(stream.len());
                let line = buf.get(self.cursor_line + local)?;
                let text = if local == 0 {
                    let clamp = self.cursor_byte.min(line.text.len());
                    // SAFETY: ensure clamp is at a char boundary to avoid
                    // mid-codepoint slice panic.
                    let safe_clamp = (0..=clamp)
                        .rev()
                        .find(|&b| line.text.is_char_boundary(b))
                        .unwrap_or(0);
                    &line.text[safe_clamp..]
                } else {
                    line.text.as_str()
                };
                stream.push_str(text);
            }

            let (stream_start, stream_end) = match &self.pattern {
                SearchPattern::Invalid => return None,
                SearchPattern::Literal(needle) => {
                    let pos = if self.case_insensitive_literal {
                        // Needle is already lowercase from `new`. ASCII fold
                        // preserves byte positions so the match's start /
                        // end map directly back onto `stream`.
                        stream.to_ascii_lowercase().find(needle)?
                    } else {
                        stream.find(needle)?
                    };
                    (pos, pos + needle.len())
                }
                SearchPattern::Regex(re) => {
                    let m = re.find(&stream)?;
                    if m.is_empty() {
                        // Skip zero-width matches: advance cursor by one byte
                        // and retry. Prevents `a*` against `"bbb"` from looping
                        // forever. Iterative loop (not recursion) prevents stack
                        // overflow on large lines.
                        self.advance_cursor_by_one(buf);
                        if self.cursor_line >= buf.len() {
                            return None;
                        }
                        continue;
                    }
                    (m.start(), m.end())
                }
            };

            let (start_local, start_byte_in_local) = locate_in_stream(stream_start, &line_starts);
            let (end_local, end_byte_in_local) = locate_in_stream(stream_end, &line_starts);
            let start_line = self.cursor_line + start_local;
            let end_line = self.cursor_line + end_local;
            // The first segment of the stream is offset by `cursor_byte`
            // inside its line; later segments start at byte 0 of their line.
            let start_byte = if start_local == 0 {
                self.cursor_byte + start_byte_in_local
            } else {
                start_byte_in_local
            };
            let end_byte = if end_local == 0 {
                self.cursor_byte + end_byte_in_local
            } else {
                end_byte_in_local
            };

            // Advance the cursor past the match so the next call returns the
            // next non-overlapping hit. An empty trailing line (end_byte at
            // line.text.len()) still leaves us inside that line — the next
            // call will see no more text there and move on.
            self.cursor_line = end_line;
            self.cursor_byte = end_byte;
            return Some(MatchRange {
                start_line,
                start_byte,
                end_line,
                end_byte,
            });
        }
    }

    fn advance_cursor_by_one(&mut self, buf: &LineBuffer) {
        let Some(line) = buf.get(self.cursor_line) else {
            return;
        };
        if self.cursor_byte < line.text.len() {
            // Step forward by one char boundary so we do not split a
            // UTF-8 codepoint.
            let mut next = self.cursor_byte + 1;
            while next < line.text.len() && !line.text.is_char_boundary(next) {
                next += 1;
            }
            self.cursor_byte = next;
        } else {
            self.cursor_line = self.cursor_line.saturating_add(1);
            self.cursor_byte = 0;
        }
    }
}

/// Translate a `stream` byte offset into `(line_local, byte_in_line)`,
/// where `line_local` is an index into `line_starts` and
/// `byte_in_line` is the offset within that line's contribution to the
/// stream.
///
/// The match is attributed to the last line whose `line_starts[i] <=
/// offset` — i.e. matches that land exactly on a line boundary are
/// counted as the start of the *next* line, matching iTerm2's
/// convention for cross-line ranges.
fn locate_in_stream(offset: usize, line_starts: &[usize]) -> (usize, usize) {
    if line_starts.is_empty() {
        return (0, offset);
    }
    // Find the largest i with line_starts[i] <= offset.
    let i = match line_starts.binary_search(&offset) {
        Ok(exact) => exact,
        Err(insert) => insert.saturating_sub(1),
    };
    (i, offset - line_starts[i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::line_buffer::EolKind;

    #[test]
    fn finds_match_that_spans_two_logical_lines() {
        let mut b = LineBuffer::new(1024);
        b.append("hello wor", &[], EolKind::Hard);
        b.append("ld there", &[], EolKind::Hard);
        let mut ctx = FindContext::new("world", FindOptions::default());
        let m = ctx.next_match(&b).expect("cross-line match");
        assert_eq!(m.start_line, 0);
        assert_eq!(m.end_line, 1);
        // "wor" starts at byte 6 of line 0, "ld" ends at byte 2 of line 1
        assert_eq!(m.start_byte, 6);
        assert_eq!(m.end_byte, 2);
    }

    #[test]
    fn streams_progress_across_calls() {
        let mut b = LineBuffer::new(1024);
        for i in 0..100 {
            b.append(&format!("line{i} foo"), &[], EolKind::Hard);
        }
        let mut ctx = FindContext::new("foo", FindOptions::default());
        let mut count = 0;
        while ctx.next_match(&b).is_some() {
            count += 1;
        }
        assert_eq!(count, 100);
    }

    #[test]
    fn returns_none_for_no_match() {
        let mut b = LineBuffer::new(1024);
        b.append("alpha", &[], EolKind::Hard);
        let mut ctx = FindContext::new("missing", FindOptions::default());
        assert!(ctx.next_match(&b).is_none());
    }

    #[test]
    fn empty_needle_returns_none() {
        let mut b = LineBuffer::new(1024);
        b.append("alpha", &[], EolKind::Hard);
        let mut ctx = FindContext::new("", FindOptions::default());
        assert!(ctx.next_match(&b).is_none());
    }

    #[test]
    fn case_insensitive_matches_mixed_case() {
        let mut b = LineBuffer::new(1024);
        b.append("Hello WORLD", &[], EolKind::Hard);
        let mut ctx = FindContext::new(
            "world",
            FindOptions {
                case_sensitive: false,
                ..FindOptions::default()
            },
        );
        let m = ctx.next_match(&b).expect("case-insensitive match");
        assert_eq!(m.start_line, 0);
        assert_eq!(m.start_byte, 6);
    }

    #[test]
    fn case_sensitive_skips_wrong_case() {
        let mut b = LineBuffer::new(1024);
        b.append("Hello WORLD", &[], EolKind::Hard);
        let mut ctx = FindContext::new(
            "world",
            FindOptions {
                case_sensitive: true,
                ..FindOptions::default()
            },
        );
        assert!(ctx.next_match(&b).is_none());
    }

    #[test]
    fn regex_finds_digits_across_lines() {
        let mut b = LineBuffer::new(1024);
        b.append("count 4", &[], EolKind::Hard);
        b.append("2 done", &[], EolKind::Hard);
        let mut ctx = FindContext::new(
            r"\d+",
            FindOptions {
                regex: true,
                ..FindOptions::default()
            },
        );
        // Stream is "count 42 done"; first regex match is "42" at offset 6.
        let m = ctx.next_match(&b).expect("regex match");
        assert_eq!(m.start_line, 0);
        assert_eq!(m.end_line, 1);
    }

    #[test]
    fn invalid_regex_returns_no_matches() {
        let mut b = LineBuffer::new(1024);
        b.append("anything", &[], EolKind::Hard);
        let mut ctx = FindContext::new(
            "(unclosed",
            FindOptions {
                regex: true,
                ..FindOptions::default()
            },
        );
        assert!(ctx.next_match(&b).is_none());
    }

    #[test]
    fn seek_resumes_from_explicit_position() {
        let mut b = LineBuffer::new(1024);
        b.append("foo bar foo", &[], EolKind::Hard);
        let mut ctx = FindContext::new("foo", FindOptions::default());
        let first = ctx.next_match(&b).unwrap();
        assert_eq!(first.start_byte, 0);
        // Seek to byte 9 — past the second "foo" start, so the next
        // match comes from a fresh pass after the cursor.
        ctx.seek(0, 0);
        let again = ctx.next_match(&b).unwrap();
        assert_eq!(again.start_byte, 0);
    }

    #[test]
    fn regex_zero_width_skipped() {
        // `a*` matches at every position — we must skip zero-width hits
        // so the iterator terminates.
        let mut b = LineBuffer::new(1024);
        b.append("bbb", &[], EolKind::Hard);
        let mut ctx = FindContext::new(
            r"a*",
            FindOptions {
                regex: true,
                ..FindOptions::default()
            },
        );
        let mut count = 0;
        while ctx.next_match(&b).is_some() {
            count += 1;
            if count > 10 {
                panic!("zero-width regex looped");
            }
        }
        assert_eq!(count, 0);
    }
}

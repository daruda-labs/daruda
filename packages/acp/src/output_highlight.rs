//! Reclassify a tool's text output when it is one file's verbatim contents, and
//! undo the wrapping the adapter and the tool put around those bytes.
//!
//! The Claude ACP adapter wraps every tool text result in a *language-less*
//! ``` ``` ``` fence (`markdownEscape`), and a `Read` body arrives in `cat -n`
//! format (`NNN\t<code>` / `NNN→<code>`). Neither belongs to the file: the fence
//! is transport, the numbers are the tool's own gutter. Both are stripped here
//! and the block is retyped [`ToolOutputBlock::SourceText`] carrying the
//! language the read target's extension implies.
//!
//! The language rides as a field rather than a fence tag because an adapter that
//! sends the file *unfenced* needs exactly the same treatment — so neither the
//! classification nor the language may depend on a fence being present.
//!
//! Pure + GPUI-free: `&str` in, `String`/enum out, unit-tested in isolation.

use serde_json::Value;

use crate::model::{ToolKindView, ToolOutputBlock};

/// How a tool's text output has to be rendered.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum TextOutputKind {
    /// One file's verbatim contents. `language` is what the read target's
    /// extension implies, `None` when it is absent or unrecognized.
    Source { language: Option<&'static str> },
    /// Markdown, the ACP spec's default for tool text.
    Markdown,
}

/// Raw-input key naming the single file a tool read.
///
/// Deliberately narrower than [`crate::mapping`]'s display-only summary
/// allowlist: `path` is also the natural input for directory/list-style tools
/// that adapters can classify as `Read`, so it is not enough proof that the
/// output is one file's contents.
const READ_FILE_PATH_KEY: &str = "file_path";

/// Classify a tool's *text* output.
///
/// [`TextOutputKind::Source`] needs both a file-reading tool and a path in its
/// input, and the file path is the load-bearing half: `ToolKindView::Read`
/// classifies by intent, so a directory listing or a glob lands there too and
/// that output is not one file's contents. Command output, search results, and
/// fetches stay markdown.
pub(crate) fn classify_text_output(
    kind: ToolKindView,
    raw_input: &Option<Value>,
) -> TextOutputKind {
    if kind != ToolKindView::Read {
        return TextOutputKind::Markdown;
    }
    let Some(path) = read_path(raw_input) else {
        return TextOutputKind::Markdown;
    };
    TextOutputKind::Source {
        language: language_of(path),
    }
}

/// The file path a tool's raw input names, if [`READ_FILE_PATH_KEY`] carries a
/// non-empty string.
fn read_path(raw_input: &Option<Value>) -> Option<&str> {
    let object = raw_input.as_ref()?.as_object()?;
    let value = object.get(READ_FILE_PATH_KEY)?.as_str()?;
    (!value.is_empty()).then_some(value)
}

/// The source language `path`'s extension implies.
///
/// Extension → language is shared with the app so this crate and the file
/// viewer agree on what a given extension is. Whether the host can actually
/// colour that language is the host's call, not this crate's — an unknown name
/// renders un-highlighted, the same as none.
fn language_of(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.').map(|(_, e)| e)?;
    daruda_core::language::from_extension(ext)
}

/// Retype `block` as [`ToolOutputBlock::SourceText`] when it is a `Text` block,
/// stripping the adapter's fence and the tool's line-number prefixes. Other
/// block kinds are left alone: only markdown-escaped text carries that wrapping,
/// and a `RawText` block came from a terminal sideband rather than a file.
pub(crate) fn retype_as_source(block: &mut ToolOutputBlock, language: Option<&'static str>) {
    let ToolOutputBlock::Text {
        text,
        truncated_from,
    } = block
    else {
        return;
    };
    *block = ToolOutputBlock::SourceText {
        text: normalize_source_output(text, truncated_from.is_some()),
        language: language.map(str::to_string),
        truncated_from: *truncated_from,
    };
}

/// Drop a single enclosing code fence and any `cat -n` line-number prefixes, so
/// what is left is the file's own bytes.
///
/// `truncated` waives the closing fence: `bounded_text` cuts from the tail, so
/// the largest reads arrive with their terminator already gone.
fn normalize_source_output(text: &str, truncated: bool) -> String {
    strip_line_numbers(unwrap_fence(text, truncated).unwrap_or(text))
}

/// Backticks a code fence needs at minimum, per CommonMark.
const MIN_FENCE_TICKS: usize = 3;

/// The body inside one enclosing bare code fence, or `None` when `text` is not
/// that shape.
///
/// Deliberately strict, because mis-reading a fence deletes the file's own first
/// and last lines: the opener must be nothing but at least [`MIN_FENCE_TICKS`]
/// backticks (the adapter never tags it), the closer at least as long, and no
/// body line may reach the opener's run — so a file that merely *contains* a
/// fence pair of its own is left whole, while an escalated wrapper (four
/// backticks around a body holding three) still unwraps.
///
/// The app mirrors these rules in
/// `agent_chat_pane::output_editor`'s `single_fenced_block`, which classifies
/// the blocks that stay markdown; that one also has to read a language tag,
/// which cannot appear here.
fn unwrap_fence(text: &str, truncated: bool) -> Option<&str> {
    let (open, rest) = text.split_once('\n')?;
    let ticks = backtick_run(open);
    if ticks < MIN_FENCE_TICKS || ticks != open.trim_end().len() {
        return None;
    }
    let body = match rest.rsplit_once('\n') {
        Some((body, close)) if is_closing_fence(close, ticks) => body,
        _ if truncated => rest,
        _ => return None,
    };
    (!body.lines().any(|line| backtick_run(line) >= ticks)).then_some(body)
}

/// Length of `line`'s leading backtick run.
fn backtick_run(line: &str) -> usize {
    line.bytes().take_while(|b| *b == b'`').count()
}

/// A line closing a fence opened with `open_ticks` backticks: nothing but
/// backticks, and at least as long as the opener. Trailing whitespace is
/// tolerated — CommonMark allows it on a closer, as does a CRLF line ending.
fn is_closing_fence(line: &str, open_ticks: usize) -> bool {
    let run = backtick_run(line);
    run >= open_ticks && run == line.trim_end().len()
}

/// Strip a `cat -n`-style line-number prefix from every body line, but only
/// when *every* non-blank line carries one — so genuine source (no prefixes)
/// and mixed content are left untouched.
fn strip_line_numbers(body: &str) -> String {
    let lines: Vec<&str> = body.split('\n').collect();
    let mut any_numbered = false;
    for line in &lines {
        if line.trim().is_empty() {
            continue;
        }
        if line_number_prefix_len(line).is_none() {
            return body.to_string();
        }
        any_numbered = true;
    }
    if !any_numbered {
        return body.to_string();
    }
    lines
        .iter()
        .map(|line| match line_number_prefix_len(line) {
            Some(n) => &line[n..],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Byte length of a leading `\s*\d+[\t→]` line-number prefix, if present.
fn line_number_prefix_len(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i] == b' ' {
        i += 1;
    }
    let digits_start = i;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == digits_start {
        return None;
    }
    if line[i..].starts_with('\t') {
        return Some(i + 1);
    }
    if line[i..].starts_with('→') {
        return Some(i + '→'.len_utf8());
    }
    None
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn source_language(kind: ToolKindView, raw_input: Value) -> Option<&'static str> {
        match classify_text_output(kind, &Some(raw_input)) {
            TextOutputKind::Source { language } => language,
            TextOutputKind::Markdown => panic!("expected Source"),
        }
    }

    #[test]
    fn a_read_with_a_file_path_is_source_in_the_extension_s_language() {
        assert_eq!(
            source_language(ToolKindView::Read, json!({"file_path": "src/main.rs"})),
            Some("rust")
        );
        assert_eq!(
            source_language(ToolKindView::Read, json!({"file_path": "a/b.py"})),
            Some("python")
        );
    }

    #[test]
    fn a_read_of_an_unknown_extension_is_still_source_without_a_language() {
        assert_eq!(
            source_language(ToolKindView::Read, json!({"file_path": "x.unknownext"})),
            None
        );
        assert_eq!(
            source_language(ToolKindView::Read, json!({"file_path": "LICENSE"})),
            None
        );
    }

    #[test]
    fn output_without_a_read_target_stays_markdown() {
        // Not a file-reading tool, even with a path in its input.
        assert_eq!(
            classify_text_output(ToolKindView::Execute, &Some(json!({"file_path": "x.rs"}))),
            TextOutputKind::Markdown
        );
        // `Read` classifies by intent, so a listing/glob lands there with no
        // single file to call source.
        assert_eq!(
            classify_text_output(ToolKindView::Read, &Some(json!({"pattern": "*.rs"}))),
            TextOutputKind::Markdown
        );
        assert_eq!(
            classify_text_output(ToolKindView::Read, &Some(json!({"path": "src"}))),
            TextOutputKind::Markdown
        );
        assert_eq!(
            classify_text_output(ToolKindView::Read, &Some(json!({"file_path": ""}))),
            TextOutputKind::Markdown
        );
        assert_eq!(
            classify_text_output(ToolKindView::Read, &None),
            TextOutputKind::Markdown
        );
    }

    /// The block `retype_as_source` produces for a Rust read, so every case
    /// below asserts on the shape the host actually receives.
    fn retyped(text: &str, truncated_from: Option<usize>) -> ToolOutputBlock {
        let mut block = ToolOutputBlock::Text {
            text: text.to_string(),
            truncated_from,
        };
        retype_as_source(&mut block, Some("rust"));
        block
    }

    /// Just the normalized body of [`retyped`].
    fn body_of(text: &str, truncated_from: Option<usize>) -> String {
        let ToolOutputBlock::SourceText { text, .. } = retyped(text, truncated_from) else {
            panic!("a Text block must retype to SourceText");
        };
        text
    }

    #[test]
    fn a_fenced_line_numbered_read_loses_both_the_fence_and_the_numbers() {
        assert_eq!(
            body_of("```\n1\tfn main() {}\n2\t// end\n```", None),
            "fn main() {}\n// end"
        );
    }

    #[test]
    fn an_unfenced_line_numbered_read_still_loses_its_numbers() {
        // The measured shape an adapter that does not markdown-escape sends —
        // and the one whose `cat -n` gutter used to survive to the render.
        assert_eq!(
            body_of("   1\tfn main() {}\n   2\tlet x = 1;", None),
            "fn main() {}\nlet x = 1;"
        );
    }

    #[test]
    fn an_unfenced_read_is_kept_verbatim() {
        assert_eq!(
            body_of("fn main() {}\nlet x = 1;", None),
            "fn main() {}\nlet x = 1;"
        );
    }

    #[test]
    fn arrow_line_numbers_are_stripped_too() {
        assert_eq!(
            body_of("  12\u{2192}let x = 1;\n  13\u{2192}let y = 2;", None),
            "let x = 1;\nlet y = 2;"
        );
    }

    #[test]
    fn a_partly_numbered_body_is_kept_verbatim() {
        // One line lacks a prefix → strip nothing rather than mangle the rest.
        assert_eq!(
            body_of("```\n1\tnumbered\nplain line\n```", None),
            "1\tnumbered\nplain line"
        );
    }

    #[test]
    fn a_truncated_read_unwraps_without_a_closing_fence() {
        // The byte cap cuts from the tail, so the terminator is already gone.
        assert_eq!(
            body_of("```\n1\tfn main() {}\n2\tlet x =", Some(999_999)),
            "fn main() {}\nlet x ="
        );
    }

    #[test]
    fn an_escalated_wrapper_unwraps_but_a_file_s_own_fence_pair_does_not() {
        // Four backticks around a body holding three: the wrapper is the fence.
        assert_eq!(
            body_of("````\n```sh\nls\n```\n````", None),
            "```sh\nls\n```"
        );
        // A markdown file that merely opens and closes with a fence of the same
        // run stays whole — unwrapping would delete two of its own lines.
        let own_fences = "```\ncode\n```\nprose\n```\nmore\n```";
        assert_eq!(body_of(own_fences, None), own_fences);
    }

    #[test]
    fn a_read_carries_its_language_and_truncation_through() {
        assert_eq!(
            retyped("fn main() {}", Some(70_000)),
            ToolOutputBlock::SourceText {
                text: "fn main() {}".to_string(),
                language: Some("rust".to_string()),
                truncated_from: Some(70_000),
            }
        );
    }

    #[test]
    fn a_non_text_block_is_left_alone() {
        let mut block = ToolOutputBlock::RawText {
            text: "  1\tnot a file\n".to_string(),
            truncated_from: None,
        };
        let before = block.clone();
        retype_as_source(&mut block, Some("rust"));
        assert_eq!(block, before);
    }
}

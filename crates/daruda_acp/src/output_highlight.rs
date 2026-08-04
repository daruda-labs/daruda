//! Rewrite a tool call's text output so its fenced code block syntax-highlights.
//!
//! The Claude ACP adapter wraps every tool text result in a *language-less*
//! ``` ``` ``` fence (`markdownEscape`). A client markdown renderer only
//! highlights a fenced block when the fence carries a recognized language
//! token, so tool output renders as flat monospace. Here we infer a language
//! from the tool (the read target's file extension) and inject it into the
//! fence. When the fenced body is line-numbered — the Read tool's `cat -n`
//! format, `NNN\t<code>` / `NNN→<code>` — the numeric prefixes are stripped so
//! the body is valid source the highlighter can parse cleanly.
//!
//! Pure + GPUI-free: `&str` in, `String`/`Option` out, unit-tested in isolation.

use serde_json::Value;

use crate::model::ToolKindView;

/// Infer a fenced-code language token for a tool's *text* output.
///
/// Only file-reading tools (`Read`) map to a language, taken from the target
/// file's extension. Command output, search results, and fetches aren't a
/// single source language, so they return `None` (left as an un-highlighted
/// block).
pub(crate) fn output_language(
    kind: ToolKindView,
    raw_input: &Option<Value>,
) -> Option<&'static str> {
    if kind != ToolKindView::Read {
        return None;
    }
    let path = raw_input.as_ref()?.get("file_path")?.as_str()?;
    let ext = path.rsplit_once('.').map(|(_, e)| e)?;
    // Shared with the app so the fence token and the file viewer agree on
    // what a given extension is. Whether the client can actually colour
    // that language is the client's call, not this crate's — an unknown
    // token renders as an untagged block, the same as emitting none.
    daruda_core::language::from_extension(ext)
}

/// Rewrite one tool-output text block so its leading bare ``` ``` ``` fence
/// carries `lang` and (if line-numbered) has its numeric prefixes stripped.
///
/// No-op unless `text` is exactly the adapter's `markdownEscape` shape — a
/// leading line of only backticks and a matching trailing fence line — so a
/// block that already tags its language, or plain prose, is left untouched.
pub(crate) fn rewrite_fenced_output(text: &str, lang: &str) -> String {
    let Some((open, rest)) = text.split_once('\n') else {
        return text.to_string();
    };
    if !is_bare_backtick_fence(open) {
        return text.to_string();
    }
    let Some((body, close)) = rest.rsplit_once('\n') else {
        return text.to_string();
    };
    if !is_bare_backtick_fence(close) {
        return text.to_string();
    }
    let body = strip_line_numbers(body);
    format!("{open}{lang}\n{body}\n{close}")
}

/// A fence line made of nothing but backticks (`` ``` ``, `` ```` ``, …).
///
/// Deliberately looser than the app's mirror of this parsing
/// (`workspace::main_area::agent_chat_pane::output_editor`'s `fence_open` /
/// `is_closing_fence`): here the fence is already known to be the adapter's
/// bare one, so no minimum run or tag handling is needed; that one classifies
/// arbitrary text and must reject ambiguous shapes.
fn is_bare_backtick_fence(line: &str) -> bool {
    !line.is_empty() && line.bytes().all(|b| b == b'`')
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

    #[test]
    fn output_language_reads_extension_for_read() {
        assert_eq!(
            output_language(
                ToolKindView::Read,
                &Some(json!({"file_path": "src/main.rs"}))
            ),
            Some("rust")
        );
        assert_eq!(
            output_language(ToolKindView::Read, &Some(json!({"file_path": "a/b.py"}))),
            Some("python")
        );
    }

    #[test]
    fn output_language_none_for_non_read_or_unknown() {
        // Not a Read tool → no language even with a file_path.
        assert_eq!(
            output_language(ToolKindView::Execute, &Some(json!({"file_path": "x.rs"}))),
            None
        );
        // Read but unknown extension.
        assert_eq!(
            output_language(
                ToolKindView::Read,
                &Some(json!({"file_path": "x.unknownext"}))
            ),
            None
        );
        // Read but no file_path / no raw input.
        assert_eq!(output_language(ToolKindView::Read, &Some(json!({}))), None);
        assert_eq!(output_language(ToolKindView::Read, &None), None);
    }

    #[test]
    fn rewrite_injects_language_and_strips_line_numbers() {
        // Adapter shape: bare fence + `cat -n` body + bare fence.
        let text = "```\n1\tfn main() {}\n2\t// end\n```";
        assert_eq!(
            rewrite_fenced_output(text, "rust"),
            "```rust\nfn main() {}\n// end\n```"
        );
    }

    #[test]
    fn rewrite_injects_language_without_stripping_clean_body() {
        // Body that isn't line-numbered keeps its content verbatim.
        let text = "```\nfn main() {}\n```";
        assert_eq!(
            rewrite_fenced_output(text, "rust"),
            "```rust\nfn main() {}\n```"
        );
    }

    #[test]
    fn rewrite_handles_arrow_line_numbers() {
        let text = "```\n  12→let x = 1;\n  13→let y = 2;\n```";
        assert_eq!(
            rewrite_fenced_output(text, "rust"),
            "```rust\nlet x = 1;\nlet y = 2;\n```"
        );
    }

    #[test]
    fn rewrite_leaves_non_fenced_text_untouched() {
        assert_eq!(rewrite_fenced_output("just prose", "rust"), "just prose");
        // A fence that already tags a language is not the adapter's bare shape.
        let tagged = "```rust\nfn main() {}\n```";
        assert_eq!(rewrite_fenced_output(tagged, "python"), tagged);
    }

    #[test]
    fn rewrite_does_not_strip_mixed_content() {
        // One line lacks a number prefix → strip nothing (avoid mangling).
        let text = "```\n1\tnumbered\nplain line\n```";
        assert_eq!(
            rewrite_fenced_output(text, "text"),
            "```text\n1\tnumbered\nplain line\n```"
        );
    }
}

use super::*;

fn text_block(text: &str) -> ToolOutputBlock {
    ToolOutputBlock::Text {
        text: text.to_string(),
        truncated_from: None,
    }
}

fn raw_block(text: &str) -> ToolOutputBlock {
    ToolOutputBlock::RawText {
        text: text.to_string(),
        truncated_from: None,
    }
}

/// A block `daruda_acp`'s byte cap cut from the tail: the marker is set and
/// the closing fence is gone with it.
fn truncated_text_block(text: &str) -> ToolOutputBlock {
    ToolOutputBlock::Text {
        text: text.to_string(),
        truncated_from: Some(text.len() * 2),
    }
}

fn source_block(text: &str, language: Option<&str>) -> ToolOutputBlock {
    ToolOutputBlock::SourceText {
        text: text.to_string(),
        language: language.map(str::to_string),
        truncated_from: None,
    }
}

#[test]
fn a_file_s_contents_are_verbatim_in_the_language_the_block_names() {
    // `daruda_acp` already undid the fence and the `cat -n` gutter, so nothing
    // about the body may gate the embed — not even a fence pair the file itself
    // holds, which is exactly what a markdown read looks like.
    let block = source_block("# Doc\n```sh\nls\n```", Some("markdown"));
    let src = output_editor_source(&block).expect("a file's contents are verbatim by definition");
    assert_eq!(src.text, "# Doc\n```sh\nls\n```");
    assert_eq!(src.language, Some("markdown"));
}

#[test]
fn a_file_of_an_unknown_extension_is_still_verbatim() {
    let block = source_block("some contents", None);
    let src = output_editor_source(&block).expect("an unknown language still embeds");
    assert_eq!(src.text, "some contents");
    assert_eq!(src.language, None);
}

#[test]
fn raw_shell_bytes_are_verbatim_and_untagged() {
    let block = raw_block("# not a heading\n*not* emphasis");
    let src = output_editor_source(&block).expect("raw output is verbatim by definition");
    assert_eq!(src.text, "# not a heading\n*not* emphasis");
    assert_eq!(src.language, None);
}

#[test]
fn bare_fence_yields_its_body_without_a_language() {
    let block = text_block("```\nabc\ndef\n```");
    let src = output_editor_source(&block).expect("the adapter's bare-fence shape qualifies");
    assert_eq!(src.text, "abc\ndef");
    assert_eq!(src.language, None);
}

#[test]
fn tagged_fence_yields_its_language() {
    let block = text_block("```rust\nfn main() {}\n```");
    let src = output_editor_source(&block).expect("a tagged fence qualifies");
    assert_eq!(src.text, "fn main() {}");
    assert_eq!(src.language, Some("rust"));
}

#[test]
fn mermaid_fence_keeps_its_diagram_renderer() {
    assert!(
        output_editor_source(&text_block("```mermaid\nflowchart TD\n  a --> b\n```")).is_none()
    );
}

#[test]
fn prose_and_multi_fence_blocks_keep_markdown() {
    assert!(output_editor_source(&text_block("just output")).is_none());
    // Prose wrapped around a fence is markdown, not one verbatim body.
    assert!(output_editor_source(&text_block("see below:\n```\nx\n```")).is_none());
    assert!(output_editor_source(&text_block("```\none\n```\n```\ntwo\n```")).is_none());
}

#[test]
fn a_nested_fence_in_the_body_disqualifies_the_block() {
    // Reading a markdown file: the body carries fences of its own, so the
    // outer shape is ambiguous and the block stays with markdown.
    assert!(output_editor_source(&text_block("```\n# Doc\n```sh\nls\n```\n```")).is_none());
}

#[test]
fn an_escalated_wrapper_keeps_a_shorter_run_as_body() {
    // Reading a markdown file whose wrapper escalated to four backticks so
    // the three-backtick fences inside stay content.
    let block = text_block("````md\n# Doc\n```sh\nls\n```\n````");
    let src = output_editor_source(&block).expect("the escalated wrapper is one fence");
    assert_eq!(src.text, "# Doc\n```sh\nls\n```");
    assert_eq!(src.language, Some("md"));
}

#[test]
fn a_run_too_short_to_be_a_code_fence_is_not_one() {
    // CommonMark would render neither as a code block, so neither is
    // verbatim output.
    assert!(output_editor_source(&text_block("`\nfoo\n`")).is_none());
    assert!(output_editor_source(&text_block("``\nfoo\n``")).is_none());
}

#[test]
fn a_closer_shorter_than_the_opener_does_not_close_the_block() {
    assert!(output_editor_source(&text_block("````\nfoo\n```")).is_none());
}

#[test]
fn a_closer_padded_with_whitespace_still_closes_the_block() {
    // CommonMark allows trailing whitespace on a closing fence, and a CRLF body
    // leaves a `\r` there — neither may push the block onto the markdown path.
    for closer in ["```  ", "```\t", "```\r"] {
        let block = text_block(&format!("```\nabc\n{closer}"));
        let src = output_editor_source(&block).expect("a padded closer is still a closer");
        assert_eq!(src.text, "abc");
    }
}

#[test]
fn a_truncated_block_qualifies_without_its_closing_fence() {
    // The byte cap cut the terminator; the body is everything after the
    // opening fence line.
    let block = truncated_text_block("```\nabc\ndef");
    let src = output_editor_source(&block).expect("a truncated tail cannot carry a closer");
    assert_eq!(src.text, "abc\ndef");
    assert_eq!(src.language, None);
}

#[test]
fn a_truncated_tagged_block_keeps_its_language() {
    let block = truncated_text_block("```rust\nfn main() {}");
    let src = output_editor_source(&block).expect("a truncated tagged fence qualifies");
    assert_eq!(src.text, "fn main() {}");
    assert_eq!(src.language, Some("rust"));
}

#[test]
fn an_untruncated_block_still_needs_its_closing_fence() {
    // Nothing was cut, so a missing terminator means the block is not the
    // adapter's single-fence shape.
    assert!(output_editor_source(&text_block("```\nabc\ndef")).is_none());
}

#[test]
fn a_truncated_mermaid_block_keeps_its_diagram_renderer() {
    assert!(output_editor_source(&truncated_text_block("```mermaid\nflowchart TD")).is_none());
}

#[test]
fn non_text_blocks_never_use_an_editor() {
    assert!(
        output_editor_source(&ToolOutputBlock::Image {
            data: "AAAA".into(),
            mime: "image/png".into(),
        })
        .is_none()
    );
    assert!(
        output_editor_source(&ToolOutputBlock::Media {
            mime: "audio/wav".into(),
            byte_len: 12,
        })
        .is_none()
    );
    assert!(
        output_editor_source(&ToolOutputBlock::ResourceLink {
            uri: "file:///tmp/a".into(),
            name: "a".into(),
        })
        .is_none()
    );
}

#[test]
fn fingerprint_tracks_text_and_language() {
    let grown = OutputEditorSource {
        text: "a\nb",
        language: None,
    };
    let partial = OutputEditorSource {
        text: "a",
        language: None,
    };
    let tagged = OutputEditorSource {
        text: "a",
        language: Some("rust"),
    };
    assert_ne!(
        output_source_fingerprint(&partial),
        output_source_fingerprint(&grown)
    );
    assert_ne!(
        output_source_fingerprint(&partial),
        output_source_fingerprint(&tagged)
    );
    assert_eq!(
        output_source_fingerprint(&partial),
        output_source_fingerprint(&OutputEditorSource {
            text: "a",
            language: None,
        })
    );
}

/// The strip below the last row, shared by every case.
fn strip() -> f32 {
    theme::SCROLLBAR_W + theme::SCROLLBAR_MARGIN_R
}

#[test]
fn a_short_embed_is_exactly_its_content_plus_the_strip() {
    assert_eq!(
        bounded_embed_height(3),
        px(3.0 * theme::AGENT_CHAT_EMBED_ROW_H + strip())
    );
}

#[test]
fn the_last_uncapped_row_count_still_measures_its_content() {
    let rows = (theme::AGENT_CHAT_EMBED_MAX_H / theme::AGENT_CHAT_EMBED_ROW_H) as usize;
    assert_eq!(
        bounded_embed_height(rows),
        px(theme::AGENT_CHAT_EMBED_MAX_H + strip())
    );
    assert_eq!(
        bounded_embed_height(rows),
        px(rows as f32 * theme::AGENT_CHAT_EMBED_ROW_H + strip())
    );
}

#[test]
fn a_long_embed_is_pinned_to_the_cap() {
    // The bound is what makes `InputState` shape only the visible rows, so
    // it must not grow with the output.
    assert_eq!(
        bounded_embed_height(11_000),
        px(theme::AGENT_CHAT_EMBED_MAX_H + strip())
    );
}

#[test]
fn key_is_per_tool_call_and_block() {
    assert_eq!(output_editor_key("call_1", 0), "call_1#0");
    assert_ne!(
        output_editor_key("call_1", 0),
        output_editor_key("call_1", 1)
    );
}

#[test]
fn a_trailing_terminator_is_not_a_row() {
    // Shell output ends with a newline terminating its last line; counting the
    // empty remainder as a row made every embed one row too tall.
    assert_eq!(without_trailing_terminator("a\nb\nc\n".into()), "a\nb\nc");
    assert_eq!(without_trailing_terminator("a\nb\nc\r\n".into()), "a\nb\nc");
}

#[test]
fn only_the_terminator_goes_and_a_blank_last_line_stays() {
    // Two newlines mean a genuine blank final line — dropping both would eat
    // content, not a terminator.
    assert_eq!(without_trailing_terminator("a\n\n".into()), "a\n");
    assert_eq!(without_trailing_terminator("a\nb".into()), "a\nb");
    assert_eq!(without_trailing_terminator(String::new()), "");
}

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
fn verbatim_output_sources_cover_source_and_raw_blocks() {
    // `daruda_acp` already undid the fence and the `cat -n` gutter, so nothing
    // about the body may gate the embed, even if the file contents hold fences.
    let block = source_block("# Doc\n```sh\nls\n```", Some("markdown"));
    let src = output_editor_source(&block).expect("a file's contents are verbatim by definition");
    assert_eq!(src.text, "# Doc\n```sh\nls\n```");
    assert_eq!(src.language, Some("markdown"));

    let block = source_block("some contents", None);
    let src = output_editor_source(&block).expect("an unknown language still embeds");
    assert_eq!(src.text, "some contents");
    assert_eq!(src.language, None);

    let block = raw_block("# not a heading\n*not* emphasis");
    let src = output_editor_source(&block).expect("raw output is verbatim by definition");
    assert_eq!(src.text, "# not a heading\n*not* emphasis");
    assert_eq!(src.language, None);
}

#[test]
fn fenced_text_blocks_that_embed_as_editor_cases() {
    for (block, expected_text, expected_language, label) in [
        (
            text_block("```\nabc\ndef\n```"),
            "abc\ndef",
            None,
            "bare fence",
        ),
        (
            text_block("```rust\nfn main() {}\n```"),
            "fn main() {}",
            Some("rust"),
            "tagged fence",
        ),
        (
            text_block("````md\n# Doc\n```sh\nls\n```\n````"),
            "# Doc\n```sh\nls\n```",
            Some("md"),
            "escalated wrapper",
        ),
        (
            truncated_text_block("```\nabc\ndef"),
            "abc\ndef",
            None,
            "truncated bare fence",
        ),
        (
            truncated_text_block("```rust\nfn main() {}"),
            "fn main() {}",
            Some("rust"),
            "truncated tagged fence",
        ),
    ] {
        let src = output_editor_source(&block).unwrap_or_else(|| panic!("{label} should embed"));
        assert_eq!(src.text, expected_text, "{label}");
        assert_eq!(src.language, expected_language, "{label}");
    }

    // CommonMark allows trailing whitespace on a closing fence, and a CRLF body
    // leaves a `\r` there. Neither may push the block onto the markdown path.
    for closer in ["```  ", "```\t", "```\r"] {
        let block = text_block(&format!("```\nabc\n{closer}"));
        let src = output_editor_source(&block).expect("a padded closer is still a closer");
        assert_eq!(src.text, "abc");
    }
}

#[test]
fn blocks_that_must_stay_on_markdown_or_media_path() {
    for (block, label) in [
        (
            text_block("```mermaid\nflowchart TD\n  a --> b\n```"),
            "mermaid fence",
        ),
        (text_block("just output"), "plain prose"),
        (
            text_block("see below:\n```\nx\n```"),
            "prose wrapped around a fence",
        ),
        (
            text_block("```\none\n```\n```\ntwo\n```"),
            "multiple fences",
        ),
        (
            text_block("```\n# Doc\n```sh\nls\n```\n```"),
            "nested body fence",
        ),
        (text_block("`\nfoo\n`"), "single-backtick run"),
        (text_block("``\nfoo\n``"), "too-short run"),
        (text_block("````\nfoo\n```"), "shorter closer"),
        (text_block("```\nabc\ndef"), "untruncated missing closer"),
        (
            truncated_text_block("```mermaid\nflowchart TD"),
            "truncated mermaid",
        ),
    ] {
        assert!(output_editor_source(&block).is_none(), "{label}");
    }

    for block in [
        ToolOutputBlock::Image {
            data: "AAAA".into(),
            mime: "image/png".into(),
        },
        ToolOutputBlock::Media {
            mime: "audio/wav".into(),
            byte_len: 12,
        },
        ToolOutputBlock::ResourceLink {
            uri: "file:///tmp/a".into(),
            name: "a".into(),
        },
    ] {
        assert!(output_editor_source(&block).is_none());
    }
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
fn bounded_embed_height_cases() {
    assert_eq!(
        bounded_embed_height(3),
        px(3.0 * theme::AGENT_CHAT_EMBED_ROW_H + strip())
    );

    let rows = (theme::AGENT_CHAT_EMBED_MAX_H / theme::AGENT_CHAT_EMBED_ROW_H) as usize;
    assert_eq!(
        bounded_embed_height(rows),
        px(theme::AGENT_CHAT_EMBED_MAX_H + strip())
    );
    assert_eq!(
        bounded_embed_height(rows),
        px(rows as f32 * theme::AGENT_CHAT_EMBED_ROW_H + strip())
    );

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
fn without_trailing_terminator_cases() {
    // Shell output ends with a newline terminating its last line; counting the
    // empty remainder as a row made every embed one row too tall.
    assert_eq!(without_trailing_terminator("a\nb\nc\n".into()), "a\nb\nc");
    assert_eq!(without_trailing_terminator("a\nb\nc\r\n".into()), "a\nb\nc");

    // Two newlines mean a genuine blank final line — dropping both would eat
    // content, not a terminator.
    assert_eq!(without_trailing_terminator("a\n\n".into()), "a\n");
    assert_eq!(without_trailing_terminator("a\nb".into()), "a\nb");
    assert_eq!(without_trailing_terminator(String::new()), "");
}

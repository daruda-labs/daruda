use super::*;

#[test]
fn bounded_text_truncates_overlong_text_at_the_cap() {
    let long_text = "a".repeat(MAX_TOOL_OUTPUT_TEXT_BYTES + 1000);
    let original_len = long_text.len();
    let ToolOutputBlock::Text {
        text,
        truncated_from,
    } = bounded_text(long_text)
    else {
        panic!("expected a Text block");
    };
    assert_eq!(text.len(), MAX_TOOL_OUTPUT_TEXT_BYTES);
    assert_eq!(truncated_from, Some(original_len));
}

#[test]
fn bounded_text_leaves_short_text_unchanged() {
    let short_text = "hello world".to_string();
    let ToolOutputBlock::Text {
        text,
        truncated_from,
    } = bounded_text(short_text.clone())
    else {
        panic!("expected a Text block");
    };
    assert_eq!(text, short_text);
    assert_eq!(truncated_from, None);
}

#[test]
fn bounded_text_never_splits_a_multibyte_char_at_the_cap_boundary() {
    // Build a string whose byte length lands exactly one byte past the cap
    // with a multibyte (3-byte) char straddling the boundary, so a naive
    // byte-index truncate would split it and panic / produce invalid UTF-8.
    let filler_len = MAX_TOOL_OUTPUT_TEXT_BYTES - 1;
    let mut s = "a".repeat(filler_len);
    // "€" is 3 bytes in UTF-8; its first byte lands exactly at the cap.
    s.push('€');
    assert_eq!(s.len(), MAX_TOOL_OUTPUT_TEXT_BYTES + 2);

    let ToolOutputBlock::Text {
        text,
        truncated_from,
    } = bounded_text(s)
    else {
        panic!("expected a Text block");
    };
    // The truncated text must be valid UTF-8 (guaranteed by type) and must
    // not include a partial "€" — the boundary walk backs off to `filler_len`.
    assert_eq!(text.len(), filler_len);
    assert!(text.chars().all(|c| c == 'a'));
    assert_eq!(truncated_from, Some(MAX_TOOL_OUTPUT_TEXT_BYTES + 2));
}

#[test]
fn raw_output_bare_string_is_raw_text_and_bounded() {
    let long = "x".repeat(MAX_TOOL_OUTPUT_TEXT_BYTES + 10);
    let blocks = raw_output_blocks(&serde_json::Value::String(long.clone()));
    let [
        ToolOutputBlock::RawText {
            text,
            truncated_from,
        },
    ] = blocks.as_slice()
    else {
        panic!("expected one Text block, got {blocks:?}");
    };
    assert_eq!(text.len(), MAX_TOOL_OUTPUT_TEXT_BYTES);
    assert_eq!(*truncated_from, Some(long.len()));
}

#[test]
fn raw_output_pretty_json_is_raw_text_and_bounded() {
    // A structured object with no `formatted_output` falls back to
    // pretty-printed raw text — verify that path is bounded too.
    let big_value: String = "v".repeat(MAX_TOOL_OUTPUT_TEXT_BYTES + 10);
    let raw = serde_json::json!({ "result": big_value });
    let blocks = raw_output_blocks(&raw);
    let [
        ToolOutputBlock::RawText {
            text,
            truncated_from,
        },
    ] = blocks.as_slice()
    else {
        panic!("expected one Text block, got {blocks:?}");
    };
    assert!(text.len() <= MAX_TOOL_OUTPUT_TEXT_BYTES);
    assert!(truncated_from.is_some());
}

#[test]
fn raw_output_image_array_produces_image_block() {
    // Anthropic's raw content-block shape: an array with one image block,
    // base64 data nested under `source.data` / `source.media_type`.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(ToolCall::new("c1", "generate image").raw_output(
            serde_json::json!([
                {
                    "type": "image",
                    "source": { "type": "base64", "data": "iVBORw0KGgo=", "media_type": "image/png" },
                }
            ]),
        )),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    assert_eq!(
        tc.output,
        vec![ToolOutputBlock::Image {
            data: "iVBORw0KGgo=".to_string(),
            mime: "image/png".to_string(),
        }]
    );
}

#[test]
fn raw_output_audio_array_produces_media_block() {
    let data = "QUJDRA==";
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "transcribe").raw_output(serde_json::json!([
                {
                    "type": "audio",
                    "source": { "type": "base64", "data": data, "media_type": "audio/mp3" },
                }
            ])),
        ),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    assert_eq!(
        tc.output,
        vec![ToolOutputBlock::Media {
            mime: "audio/mp3".to_string(),
            byte_len: est_decoded_len(data),
        }]
    );
}

#[test]
fn raw_output_image_then_text_preserves_order() {
    // A mixed array (image + text) must yield both blocks, in order — not
    // just the first recognized one.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(ToolCall::new("c1", "mixed").raw_output(serde_json::json!([
            {
                "type": "image",
                "source": { "data": "abcd", "media_type": "image/jpeg" },
            },
            { "type": "text", "text": "caption" },
        ]))),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    assert_eq!(
        tc.output,
        vec![
            ToolOutputBlock::Image {
                data: "abcd".to_string(),
                mime: "image/jpeg".to_string(),
            },
            ToolOutputBlock::Text {
                text: "caption".to_string(),
                truncated_from: None,
            },
        ]
    );
}

#[test]
fn raw_output_array_of_unrecognized_objects_falls_back_to_pretty_json_raw_text() {
    // No element has a recognized `type` — the whole array must still
    // surface as pretty JSON, not be silently dropped. It is raw text so the
    // app uses the bounded output editor.
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("c1", "weird")
                .raw_output(serde_json::json!([{ "foo": "bar" }, { "baz": 1 }])),
        ),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    let [ToolOutputBlock::RawText { text, .. }] = tc.output.as_slice() else {
        panic!("expected one raw text block, got {:?}", tc.output);
    };
    assert!(text.contains("\"foo\""), "pretty JSON, got: {text}");
}

#[test]
fn split_content_maps_image_content_block() {
    use agent_client_protocol::schema::v1::{Content, ImageContent};
    let content = vec![ToolCallContent::Content(Content::new(ContentBlock::Image(
        ImageContent::new("b64data", "image/png"),
    )))];
    let (_, output) = split(&content);
    assert_eq!(
        output,
        vec![ToolOutputBlock::Image {
            data: "b64data".to_string(),
            mime: "image/png".to_string(),
        }]
    );
}

#[test]
fn split_content_maps_audio_content_block_to_media() {
    use agent_client_protocol::schema::v1::{AudioContent, Content};
    let data = "b64audio";
    let content = vec![ToolCallContent::Content(Content::new(ContentBlock::Audio(
        AudioContent::new(data, "audio/wav"),
    )))];
    let (_, output) = split(&content);
    assert_eq!(
        output,
        vec![ToolOutputBlock::Media {
            mime: "audio/wav".to_string(),
            byte_len: est_decoded_len(data),
        }]
    );
}

#[test]
fn split_content_maps_embedded_blob_resource_to_media() {
    use agent_client_protocol::schema::v1::{
        BlobResourceContents, Content, EmbeddedResource, EmbeddedResourceResource,
    };
    let blob = "b64blob";
    let content = vec![ToolCallContent::Content(Content::new(
        ContentBlock::Resource(EmbeddedResource::new(
            EmbeddedResourceResource::BlobResourceContents(
                BlobResourceContents::new(blob, "file:///tmp/x.bin")
                    .mime_type("application/octet-stream"),
            ),
        )),
    ))];
    let (_, output) = split(&content);
    assert_eq!(
        output,
        vec![ToolOutputBlock::Media {
            mime: "application/octet-stream".to_string(),
            byte_len: est_decoded_len(blob),
        }]
    );
}

#[test]
fn split_content_maps_embedded_text_resource_to_bounded_text() {
    use agent_client_protocol::schema::v1::{
        Content, EmbeddedResource, EmbeddedResourceResource, TextResourceContents,
    };
    let content = vec![ToolCallContent::Content(Content::new(
        ContentBlock::Resource(EmbeddedResource::new(
            EmbeddedResourceResource::TextResourceContents(TextResourceContents::new(
                "hello resource",
                "file:///tmp/x.txt",
            )),
        )),
    ))];
    let (_, output) = split(&content);
    assert_eq!(
        output,
        vec![ToolOutputBlock::Text {
            text: "hello resource".to_string(),
            truncated_from: None,
        }]
    );
}

#[test]
fn split_content_drops_whitespace_only_text_resource() {
    use agent_client_protocol::schema::v1::{
        Content, EmbeddedResource, EmbeddedResourceResource, TextResourceContents,
    };
    let content = vec![ToolCallContent::Content(Content::new(
        ContentBlock::Resource(EmbeddedResource::new(
            EmbeddedResourceResource::TextResourceContents(TextResourceContents::new(
                "   \n",
                "file:///tmp/x.txt",
            )),
        )),
    ))];
    let (_, output) = split(&content);
    assert!(
        output.is_empty(),
        "whitespace-only embedded resource text is dropped, not an empty block"
    );
}

#[test]
fn image_content_block_over_cap_falls_back_to_media() {
    use agent_client_protocol::schema::v1::{Content, ImageContent};
    let data = "A".repeat(MAX_TOOL_OUTPUT_IMAGE_BYTES + 1);
    let content = vec![ToolCallContent::Content(Content::new(ContentBlock::Image(
        ImageContent::new(data.clone(), "image/png"),
    )))];
    let (_, output) = split(&content);
    assert_eq!(
        output,
        vec![ToolOutputBlock::Media {
            mime: "image/png".to_string(),
            byte_len: est_decoded_len(&data),
        }],
        "an oversized image must fall back to a Media descriptor, not an Image block"
    );
}

#[test]
fn image_content_block_under_cap_stays_image() {
    use agent_client_protocol::schema::v1::{Content, ImageContent};
    let data = "A".repeat(1024);
    let content = vec![ToolCallContent::Content(Content::new(ContentBlock::Image(
        ImageContent::new(data.clone(), "image/png"),
    )))];
    let (_, output) = split(&content);
    assert_eq!(
        output,
        vec![ToolOutputBlock::Image {
            data,
            mime: "image/png".to_string(),
        }]
    );
}

#[test]
fn raw_output_oversized_image_falls_back_to_media() {
    // The raw-content path (Anthropic's array shape) must honor the same
    // cap as the typed `ContentBlock::Image` path.
    let data = "B".repeat(MAX_TOOL_OUTPUT_IMAGE_BYTES + 1);
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(ToolCall::new("c1", "generate image").raw_output(
            serde_json::json!([
                {
                    "type": "image",
                    "source": { "data": data, "media_type": "image/png" },
                }
            ]),
        )),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call");
    };
    assert_eq!(
        tc.output,
        vec![ToolOutputBlock::Media {
            mime: "image/png".to_string(),
            byte_len: est_decoded_len(&data),
        }]
    );
}

#[test]
fn raw_output_array_with_only_empty_text_falls_back_to_pretty_json_raw_text() {
    // The one element parses as a recognized "text" type but with empty
    // text — `raw_content_block` drops empty text (mirroring the
    // `ContentBlock::Text` guard in `output_block_of`), so the array has no
    // recognized blocks and falls through to the pretty-JSON fallback
    // rather than surfacing an empty `Text` block. It is raw text so the app
    // uses the bounded output editor.
    let raw = serde_json::json!([{ "type": "text", "text": "" }]);
    let blocks = raw_output_blocks(&raw);
    let [ToolOutputBlock::RawText { text, .. }] = blocks.as_slice() else {
        panic!("expected one pretty-JSON raw text block, got {blocks:?}");
    };
    assert!(
        text.contains("\"type\""),
        "pretty JSON of the raw array, got: {text}"
    );
}

#[test]
fn raw_output_array_partial_recognition_keeps_recognized_and_drops_junk() {
    // One recognized `text` element plus one element with no `type` at
    // all: the junk element is silently dropped and only the recognized
    // block survives — documents the "partially recognized array" behavior
    // distinct from the "nothing recognized" pretty-JSON fallback above.
    let raw = serde_json::json!([{ "type": "text", "text": "hi" }, { "foo": "bar" }]);
    let blocks = raw_output_blocks(&raw);
    assert_eq!(
        blocks,
        vec![ToolOutputBlock::Text {
            text: "hi".to_string(),
            truncated_from: None,
        }]
    );
}

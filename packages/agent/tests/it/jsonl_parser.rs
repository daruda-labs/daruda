//! Tests for the public `daruda_agent::jsonl::parser` API.

use daruda_agent::jsonl::parser::*;

#[test]
fn user_message_string_content_is_not_tool_result() {
    let json = r#"{"role":"user","content":"hello"}"#;
    let m: UserMessage = serde_json::from_str(json).unwrap();
    assert_eq!(m.content, "hello");
    assert!(!m.is_tool_result);
}

#[test]
fn user_message_array_with_tool_result() {
    let json = r#"{"role":"user","content":[
            {"type":"tool_result","content":"output"}
        ]}"#;
    let m: UserMessage = serde_json::from_str(json).unwrap();
    assert!(m.is_tool_result);
    assert_eq!(m.content, "output");
}

#[test]
fn assistant_with_tool_use_and_result_pair() {
    let entry: SessionEntry = serde_json::from_str(
        r#"{
                "type":"assistant",
                "uuid":"u1",
                "timestamp":"2026-05-01T00:00:00Z",
                "message":{
                    "model":"claude-x",
                    "id":"m1",
                    "role":"assistant",
                    "content":[
                        {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}},
                        {"type":"tool_result","tool_use_id":"t1","content":"file"}
                    ],
                    "stop_reason":"end_turn"
                }
            }"#,
    )
    .unwrap();
    match entry {
        SessionEntry::Assistant { message, .. } => {
            assert_eq!(message.content.len(), 2);
            assert_eq!(message.stop_reason.as_deref(), Some("end_turn"));
        }
        _ => panic!("expected assistant"),
    }
}

#[test]
fn unknown_entry_type_falls_through() {
    let entry: SessionEntry = serde_json::from_str(r#"{"type":"future-entry-kind"}"#).unwrap();
    assert!(matches!(entry, SessionEntry::Unknown));
}

#[test]
fn parse_jsonl_skips_malformed_lines() {
    let lines = vec![
        r#"{"type":"user","uuid":"u1","timestamp":"t","message":{"role":"user","content":"hi"}}"#
            .to_string(),
        r#"{ malformed"#.to_string(),
        r#"{"type":"summary","summary":"s","leafUuid":"l"}"#.to_string(),
    ];
    let entries = parse_jsonl_entries(&lines);
    assert_eq!(entries.len(), 2);
}

#[test]
fn tool_result_with_inner_text_block_array_concatenates_text() {
    // Real Claude Code JSONL nests `content` two levels deep when a
    // tool returns multiple blocks. Each inner block of `type: text`
    // contributes its `text` field to the joined content; non-text
    // inner blocks are ignored.
    let json = r#"{"role":"user","content":[
            {"type":"tool_result","content":[
                {"type":"text","text":"line one"},
                {"type":"text","text":"line two"}
            ]}
        ]}"#;
    let m: UserMessage = serde_json::from_str(json).unwrap();
    assert!(m.is_tool_result);
    assert_eq!(m.content, "line one\nline two");
}

#[test]
fn tool_result_with_multiple_blocks_marks_is_tool_result() {
    // Multiple top-level tool_result blocks (e.g. parallel tool
    // calls). All of them flag is_tool_result and their text is
    // joined into a single content string.
    let json = r#"{"role":"user","content":[
            {"type":"tool_result","content":"a"},
            {"type":"tool_result","content":"b"}
        ]}"#;
    let m: UserMessage = serde_json::from_str(json).unwrap();
    assert!(m.is_tool_result);
    assert_eq!(m.content, "a\nb");
}

#[test]
fn mixed_text_and_tool_result_blocks_joined_in_order() {
    // A user message can intermix a plain text block with a tool
    // result reply (rare but allowed). The text block's content is
    // joined with the tool result's content, and is_tool_result
    // remains true so the FSM doesn't treat this as a new prompt.
    let json = r#"{"role":"user","content":[
            {"type":"text","text":"context"},
            {"type":"tool_result","content":"result"}
        ]}"#;
    let m: UserMessage = serde_json::from_str(json).unwrap();
    assert!(m.is_tool_result);
    assert_eq!(m.content, "context\nresult");
}

#[test]
fn tool_result_without_content_field_emits_placeholder() {
    // An empty tool_result block (no `content` key) is still a
    // tool result in semantics — surface the synthetic placeholder
    // so logs don't show a blank line.
    let json = r#"{"role":"user","content":[
            {"type":"tool_result"}
        ]}"#;
    let m: UserMessage = serde_json::from_str(json).unwrap();
    assert!(m.is_tool_result);
    assert_eq!(m.content, "[tool result]");
}

#[test]
fn user_message_with_image_only_collects_images_and_empty_content() {
    let json = r#"{"role":"user","content":[
            {"type":"image","source":{"media_type":"image/jpeg","data":"BASE64"}}
        ]}"#;
    let m: UserMessage = serde_json::from_str(json).unwrap();
    assert!(!m.is_tool_result);
    assert_eq!(m.content, "");
    assert_eq!(m.images.len(), 1);
    assert_eq!(m.images[0].media_type, "image/jpeg");
    assert_eq!(m.images[0].data, "BASE64");
}

#[test]
fn user_message_image_with_default_media_type() {
    // Missing media_type falls back to image/png (Claude Code's
    // historical default).
    let json = r#"{"role":"user","content":[
            {"type":"image","source":{"data":"X"}}
        ]}"#;
    let m: UserMessage = serde_json::from_str(json).unwrap();
    assert_eq!(m.images.len(), 1);
    assert_eq!(m.images[0].media_type, "image/png");
}

#[test]
fn user_message_text_and_image_combined() {
    let json = r#"{"role":"user","content":[
            {"type":"text","text":"see screenshot"},
            {"type":"image","source":{"media_type":"image/png","data":"D"}}
        ]}"#;
    let m: UserMessage = serde_json::from_str(json).unwrap();
    assert!(!m.is_tool_result);
    assert_eq!(m.content, "see screenshot");
    assert_eq!(m.images.len(), 1);
}

#[test]
fn user_message_unknown_block_type_silently_skipped() {
    // Forward-compat: a block type Claude Code adds in the future
    // must not break the parser. The block is ignored; the rest of
    // the message parses normally.
    let json = r#"{"role":"user","content":[
            {"type":"future_block_kind","payload":42},
            {"type":"text","text":"still here"}
        ]}"#;
    let m: UserMessage = serde_json::from_str(json).unwrap();
    assert!(!m.is_tool_result);
    assert_eq!(m.content, "still here");
}

#[test]
fn user_message_empty_array_yields_empty_content() {
    let json = r#"{"role":"user","content":[]}"#;
    let m: UserMessage = serde_json::from_str(json).unwrap();
    assert!(!m.is_tool_result);
    assert_eq!(m.content, "");
    assert!(m.images.is_empty());
}

#[test]
fn user_message_role_field_is_preserved() {
    // The role field is currently set by the producer ("user"); the
    // parser must surface whatever shipped on disk, not a default.
    let json = r#"{"role":"system","content":"x"}"#;
    let m: UserMessage = serde_json::from_str(json).unwrap();
    assert_eq!(m.role, "system");
}

#[test]
fn user_message_missing_role_defaults_to_user() {
    // Defensive default — parser shouldn't fail on a message without
    // a role (some legacy lines lacked it).
    let json = r#"{"content":"x"}"#;
    let m: UserMessage = serde_json::from_str(json).unwrap();
    assert_eq!(m.role, "user");
}

#[test]
fn parses_sample_jsonl_from_real_project() {
    // Sanity-check against a real-world JSONL line shape — the
    // session-id, cwd, gitBranch fields all show up at the top
    // level (camelCase).
    let line = r#"{
            "type":"user",
            "uuid":"u1",
            "timestamp":"2026-05-01T12:00:00Z",
            "sessionId":"abc",
            "cwd":"/Users/x/proj",
            "gitBranch":"main",
            "message":{"role":"user","content":"hi"}
        }"#;
    let entry: SessionEntry = serde_json::from_str(line).unwrap();
    match entry {
        SessionEntry::User { base, .. } => {
            assert_eq!(base.session_id.as_deref(), Some("abc"));
            assert_eq!(base.cwd.unwrap().to_str().unwrap(), "/Users/x/proj");
            assert_eq!(base.git_branch.as_deref(), Some("main"));
        }
        _ => panic!("expected user"),
    }
}

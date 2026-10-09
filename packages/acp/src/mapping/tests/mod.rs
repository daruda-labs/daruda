mod lifecycle;
mod messages;
mod output;
mod permissions;
mod tool_calls;

use super::content::*;
use super::permissions::{RAW_INPUT_SUMMARY_MAX_CHARS, summarize_raw_input};
use super::*;
use crate::model::CommandExit;
use agent_client_protocol::schema::v1::{
    Content, ContentChunk, Diff, ImageContent, ResourceLink, Terminal, TextContent,
    ToolCallUpdateFields,
};

/// [`split_content`] as a plain `(diffs, output)` pair — the shape the
/// content-only tests care about.
fn split(content: &[ToolCallContent]) -> (Vec<DiffView>, Vec<ToolOutputBlock>) {
    let split = split_content(content);
    (split.diffs, split.output)
}

/// A settled Bash tool call whose result is the adapter's content-less
/// terminal handle, with `meta` standing in for the update's `_meta`.
fn terminal_result(meta: Option<serde_json::Value>) -> SessionUpdate {
    let mut tc = ToolCall::new("c1", "ls -la")
        .kind(ToolKind::Execute)
        .status(ToolCallStatus::Completed)
        .content(vec![ToolCallContent::Terminal(Terminal::new("c1"))]);
    if let Some(m) = meta {
        tc = tc.meta(meta_map(m));
    }
    SessionUpdate::ToolCall(tc)
}

fn only_tool_call(items: &[ChatItem]) -> &ToolCallItem {
    match items.first() {
        Some(ChatItem::ToolCall(tc)) => tc,
        other => panic!("expected a tool call, got {other:?}"),
    }
}

/// The literal wire capture from claude-agent-acp **0.64.2**, taken through
/// `examples/acp_spike` with `_meta.terminal_output` advertised. Replayed
/// verbatim rather than hand-transcribed: the defect this pins — reading the
/// sideband only when `content` is present — survived a suite of hand-written
/// tests because every one of them modelled the shape from adapter source,
/// and the source comment names three notifications where the wire sends
/// four (the second refines title/rawInput and carries content but no status).
const CAPTURED_BASH_TURN: [&str; 4] = [
    r#"{"_meta":{"claudeCode":{"toolName":"Bash"},"terminal_info":{"terminal_id":"toolu_01"}},"toolCallId":"toolu_01","sessionUpdate":"tool_call","rawInput":{},"status":"pending","title":"Terminal","kind":"execute","content":[{"type":"terminal","terminalId":"toolu_01"}]}"#,
    r#"{"_meta":{"claudeCode":{"toolName":"Bash","title":"List crates"}},"toolCallId":"toolu_01","sessionUpdate":"tool_call_update","rawInput":{"command":"ls -la crates | head -5"},"title":"ls -la crates | head -5","kind":"execute","content":[{"type":"terminal","terminalId":"toolu_01"}]}"#,
    r#"{"_meta":{"terminal_output":{"terminal_id":"toolu_01","data":"total 24\n---\ndrwxr-xr-x@  5 woo  staff   160 Jul 31 14:17 app"}},"toolCallId":"toolu_01","sessionUpdate":"tool_call_update"}"#,
    r#"{"_meta":{"claudeCode":{"toolName":"Bash"},"terminal_exit":{"terminal_id":"toolu_01","exit_code":0,"signal":null}},"toolCallId":"toolu_01","sessionUpdate":"tool_call_update","status":"completed","rawOutput":"total 24\n---\ndrwxr-xr-x@  5 woo  staff   160 Jul 31 14:17 app","content":[{"type":"terminal","terminalId":"toolu_01"}]}"#,
];

/// Build a `Meta` map from a JSON object literal — the schema type is
/// `serde_json::Map<String, Value>`, not `Value`, so builders taking `Meta`
/// need the unwrapped map.
fn meta_map(v: serde_json::Value) -> agent_client_protocol::schema::v1::Meta {
    v.as_object()
        .expect("test fixture must be a JSON object")
        .clone()
}

fn text_chunk(s: &str) -> ContentChunk {
    ContentChunk::new(ContentBlock::Text(TextContent::new(s.to_string())))
}

fn text_chunk_id(s: &str, id: &str) -> ContentChunk {
    ContentChunk::new(ContentBlock::Text(TextContent::new(s.to_string()))).message_id(id)
}

/// Every assistant message in arrival order — what the renderer turns into
/// one markdown document each.
fn assistant_texts(items: &[ChatItem]) -> Vec<&str> {
    items
        .iter()
        .filter_map(|i| match i {
            ChatItem::AssistantText { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn subagent_child(parent: &str, status: ToolStatusView) -> ChatItem {
    ChatItem::ToolCall(ToolCallItem {
        id: format!("{parent}-child"),
        title: "child".to_string(),
        kind: ToolKindView::Read,
        tool_name: None,
        status,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: None,
        locations: Vec::new(),
        parent_tool_id: Some(parent.to_string()),
        exit: None,
    })
}

/// Drive a `Read` tool call whose only output block is `body`, and return the
/// output blocks it mapped to.
fn read_output_blocks(path: &str, body: &str) -> Vec<ToolOutputBlock> {
    read_output_blocks_with_raw_input(serde_json::json!({"file_path": path}), body)
}

/// Drive a `Read` tool call with explicit raw input, and return the output
/// blocks it mapped to.
fn read_output_blocks_with_raw_input(
    raw_input: serde_json::Value,
    body: &str,
) -> Vec<ToolOutputBlock> {
    let mut items = Vec::new();
    apply_update(
        &mut items,
        &SessionUpdate::ToolCall(
            ToolCall::new("t1", "Read")
                .kind(ToolKind::Read)
                .raw_input(raw_input),
        ),
    );
    let mut fields = ToolCallUpdateFields::default();
    fields.status = Some(ToolCallStatus::Completed);
    fields.content = Some(vec![ToolCallContent::Content(Content::new(
        ContentBlock::Text(TextContent::new(body.to_string())),
    ))]);
    apply_update(
        &mut items,
        &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("t1", fields)),
    );
    let ChatItem::ToolCall(tc) = &items[0] else {
        panic!("expected a tool call item");
    };
    tc.output.clone()
}

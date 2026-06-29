//! Fold ACP protocol traffic into the [`crate::model`] chat item list.
//!
//! Pure functions over `&mut Vec<ChatItem>` so they unit-test without any
//! connection or executor. MVP handles the streaming/tool/permission updates
//! that drive the conversation view; plan, slash-command, mode, config, info,
//! and usage updates are intentionally ignored.

use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, PermissionOption, PermissionOptionKind, RequestPermissionRequest,
    SessionUpdate, ToolCall, ToolCallContent, ToolCallStatus, ToolCallUpdate, ToolKind,
};

use crate::model::{
    ChatItem, DiffView, PermissionChoice, PermissionItem, PermissionKindView, ToolCallItem,
    ToolKindView, ToolStatusView,
};

/// Apply one `session/update` notification to the chat item list.
pub fn apply_update(items: &mut Vec<ChatItem>, update: &SessionUpdate) {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            append_streaming(
                items,
                &text_of(&chunk.content),
                msg_id(chunk),
                StreamKind::Assistant,
            );
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            append_streaming(
                items,
                &text_of(&chunk.content),
                msg_id(chunk),
                StreamKind::Thinking,
            );
        }
        SessionUpdate::UserMessageChunk(chunk) => {
            items.push(ChatItem::UserText(text_of(&chunk.content)));
        }
        SessionUpdate::ToolCall(tool_call) => upsert_tool_call(items, tool_call),
        SessionUpdate::ToolCallUpdate(update) => apply_tool_call_update(items, update),
        // MVP: plan / available-commands / mode / config / info / usage updates
        // carry no conversation content we render yet.
        _ => {}
    }
}

/// Build a pending permission card from a `session/request_permission` request.
pub fn permission_item(request: &RequestPermissionRequest) -> ChatItem {
    ChatItem::Permission(PermissionItem {
        tool_title: request.tool_call.fields.title.clone(),
        options: request.options.iter().map(choice_of).collect(),
        resolved: None,
    })
}

/// Clear the streaming flag on **every** assistant/thinking item — called when
/// a prompt turn completes so the view drops any "typing" affordance.
///
/// Clearing only the tail is wrong: a streamed text block followed by a tool
/// call (the common "let me look" → tool pattern) is no longer the last item,
/// so its `streaming` flag would stay `true` forever. That makes the turn's
/// rollup read `Running` (a perpetually blinking dot) and keeps the turn
/// expanded (`is_active`) long after it ended — and since most turns interleave
/// text and tools, *every* past turn would blink in lockstep. Settle them all.
pub fn finalize_streaming(items: &mut [ChatItem]) {
    for item in items.iter_mut() {
        if let ChatItem::AssistantText { streaming, .. } | ChatItem::Thinking { streaming, .. } =
            item
        {
            *streaming = false;
        }
    }
}

/// Settle every still-running tool call as [`ToolStatusView::Cancelled`].
///
/// Called when the user stops a turn so `Pending` / `InProgress` tool cards
/// stop reading as live (the tool-group rollup keys its blinking ● off
/// `InProgress`). The counterpart to [`finalize_streaming`] for tool calls;
/// terminal `Completed` / `Failed` calls keep their status.
pub fn cancel_pending_tools(items: &mut [ChatItem]) {
    for item in items.iter_mut() {
        if let ChatItem::ToolCall(tc) = item
            && matches!(
                tc.status,
                ToolStatusView::Pending | ToolStatusView::InProgress
            )
        {
            tc.status = ToolStatusView::Cancelled;
        }
    }
}

#[derive(Clone, Copy)]
enum StreamKind {
    Assistant,
    Thinking,
}

/// Append streamed text, extending the trailing item only when it is the same
/// still-streaming kind **and** the same message (matching `message_id`, or both
/// absent); otherwise a new message has started, so finalize the previous
/// streaming block and start a fresh item. A change in `message_id` therefore
/// splits two agent messages into separate items even with no tool call between
/// them — the protocol's "a change in messageId indicates a new message"
/// (`ContentChunk::message_id`). When the agent omits `message_id`, chunks merge
/// by adjacency (legacy behaviour). Empty chunks (the agent emits a leading
/// empty chunk per message) start the item without text.
fn append_streaming(
    items: &mut Vec<ChatItem>,
    text: &str,
    message_id: Option<String>,
    kind: StreamKind,
) {
    if let Some(last) = items.last_mut() {
        match (last, kind) {
            (
                ChatItem::AssistantText {
                    text: prev,
                    streaming: true,
                    message_id: mid,
                },
                StreamKind::Assistant,
            ) if *mid == message_id => {
                prev.push_str(text);
                return;
            }
            (
                ChatItem::Thinking {
                    text: prev,
                    streaming: true,
                    message_id: mid,
                },
                StreamKind::Thinking,
            ) if *mid == message_id => {
                prev.push_str(text);
                return;
            }
            _ => {}
        }
    }
    // A new message (different id, or a kind switch) begins: the previous
    // streaming block, if any, is now complete.
    finalize_streaming(items);
    match kind {
        StreamKind::Assistant => items.push(ChatItem::AssistantText {
            text: text.to_string(),
            streaming: true,
            message_id,
        }),
        StreamKind::Thinking => items.push(ChatItem::Thinking {
            text: text.to_string(),
            streaming: true,
            message_id,
        }),
    }
}

fn upsert_tool_call(items: &mut Vec<ChatItem>, tool_call: &ToolCall) {
    let id = tool_call.tool_call_id.0.to_string();
    let (diffs, output) = split_content(&tool_call.content);
    let item = ToolCallItem {
        id: id.clone(),
        title: tool_call.title.clone(),
        kind: kind_of(&tool_call.kind),
        status: status_of(&tool_call.status),
        diffs,
        output,
        raw_input: tool_call.raw_input.clone(),
    };
    match find_tool_call(items, &id) {
        Some(existing) => *existing = item,
        None => items.push(ChatItem::ToolCall(item)),
    }
}

fn apply_tool_call_update(items: &mut [ChatItem], update: &ToolCallUpdate) {
    let id = update.tool_call_id.0.to_string();
    let Some(item) = find_tool_call(items, &id) else {
        return;
    };
    let fields = &update.fields;
    if let Some(kind) = &fields.kind {
        item.kind = kind_of(kind);
    }
    if let Some(status) = &fields.status {
        item.status = status_of(status);
    }
    if let Some(title) = &fields.title {
        item.title = title.clone();
    }
    if let Some(content) = &fields.content {
        let (diffs, output) = split_content(content);
        item.diffs = diffs;
        item.output = output;
    }
    if fields.raw_input.is_some() {
        item.raw_input = fields.raw_input.clone();
    }
}

fn find_tool_call<'a>(items: &'a mut [ChatItem], id: &str) -> Option<&'a mut ToolCallItem> {
    items.iter_mut().rev().find_map(|item| match item {
        ChatItem::ToolCall(tc) if tc.id == id => Some(tc),
        _ => None,
    })
}

/// Partition tool-call content into diffs and plain-text output, dropping
/// embedded terminals (not rendered in the MVP).
fn split_content(content: &[ToolCallContent]) -> (Vec<DiffView>, Vec<String>) {
    let mut diffs = Vec::new();
    let mut output = Vec::new();
    for block in content {
        match block {
            ToolCallContent::Diff(diff) => diffs.push(DiffView {
                path: diff.path.clone(),
                old_text: diff.old_text.clone(),
                new_text: diff.new_text.clone(),
            }),
            ToolCallContent::Content(c) => {
                let text = text_of(&c.content);
                if !text.is_empty() {
                    output.push(text);
                }
            }
            // Embedded terminals and any future content kinds are not rendered.
            _ => {}
        }
    }
    (diffs, output)
}

/// The owned `message_id` of a streamed content chunk, if the agent supplied
/// one. `MessageId` wraps an `Arc<str>`; we copy it into a `String` so the
/// GPUI-free model carries no protocol type.
fn msg_id(chunk: &ContentChunk) -> Option<String> {
    chunk.message_id.as_ref().map(|m| m.0.to_string())
}

/// Extract renderable text from a content block. Non-text blocks (image,
/// audio, resource) collapse to empty for the MVP text view.
fn text_of(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(t) => t.text.clone(),
        _ => String::new(),
    }
}

fn choice_of(option: &PermissionOption) -> PermissionChoice {
    PermissionChoice {
        option_id: option.option_id.0.to_string(),
        name: option.name.clone(),
        kind: match option.kind {
            PermissionOptionKind::AllowOnce => PermissionKindView::AllowOnce,
            PermissionOptionKind::AllowAlways => PermissionKindView::AllowAlways,
            PermissionOptionKind::RejectOnce => PermissionKindView::RejectOnce,
            PermissionOptionKind::RejectAlways => PermissionKindView::RejectAlways,
            // Unknown future kind: render as a one-time reject (deny by default).
            _ => PermissionKindView::RejectOnce,
        },
    }
}

fn status_of(status: &ToolCallStatus) -> ToolStatusView {
    match status {
        ToolCallStatus::Pending => ToolStatusView::Pending,
        ToolCallStatus::InProgress => ToolStatusView::InProgress,
        ToolCallStatus::Completed => ToolStatusView::Completed,
        ToolCallStatus::Failed => ToolStatusView::Failed,
        _ => ToolStatusView::Pending,
    }
}

fn kind_of(kind: &ToolKind) -> ToolKindView {
    match kind {
        ToolKind::Read => ToolKindView::Read,
        ToolKind::Edit => ToolKindView::Edit,
        ToolKind::Delete => ToolKindView::Delete,
        ToolKind::Move => ToolKindView::Move,
        ToolKind::Search => ToolKindView::Search,
        ToolKind::Execute => ToolKindView::Execute,
        ToolKind::Think => ToolKindView::Think,
        ToolKind::Fetch => ToolKindView::Fetch,
        ToolKind::SwitchMode => ToolKindView::SwitchMode,
        ToolKind::Other => ToolKindView::Other,
        _ => ToolKindView::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::{
        ContentChunk, Diff, TextContent, ToolCallUpdateFields,
    };

    fn text_chunk(s: &str) -> ContentChunk {
        ContentChunk::new(ContentBlock::Text(TextContent::new(s.to_string())))
    }

    fn text_chunk_id(s: &str, id: &str) -> ContentChunk {
        ContentChunk::new(ContentBlock::Text(TextContent::new(s.to_string()))).message_id(id)
    }

    #[test]
    fn agent_chunks_accumulate_into_one_streaming_item() {
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::AgentMessageChunk(text_chunk("2 + 2 ")),
        );
        apply_update(
            &mut items,
            &SessionUpdate::AgentMessageChunk(text_chunk("is 4.")),
        );
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ChatItem::AssistantText {
                text: "2 + 2 is 4.".to_string(),
                streaming: true,
                message_id: None,
            }
        );
    }

    #[test]
    fn chunks_with_same_message_id_merge() {
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::AgentMessageChunk(text_chunk_id("Hello ", "m1")),
        );
        apply_update(
            &mut items,
            &SessionUpdate::AgentMessageChunk(text_chunk_id("world", "m1")),
        );
        assert_eq!(items.len(), 1);
        assert_eq!(
            items[0],
            ChatItem::AssistantText {
                text: "Hello world".to_string(),
                streaming: true,
                message_id: Some("m1".to_string()),
            }
        );
    }

    #[test]
    fn message_id_change_splits_and_finalizes_previous() {
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::AgentMessageChunk(text_chunk_id("first", "m1")),
        );
        // A new messageId means a new message started — the previous one is done.
        apply_update(
            &mut items,
            &SessionUpdate::AgentMessageChunk(text_chunk_id("second", "m2")),
        );
        assert_eq!(items.len(), 2, "different messageId starts a new item");
        assert_eq!(
            items[0],
            ChatItem::AssistantText {
                text: "first".to_string(),
                streaming: false, // finalized when the next message began
                message_id: Some("m1".to_string()),
            }
        );
        assert_eq!(
            items[1],
            ChatItem::AssistantText {
                text: "second".to_string(),
                streaming: true,
                message_id: Some("m2".to_string()),
            }
        );
    }

    #[test]
    fn thought_chunk_becomes_thinking_item() {
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::AgentThoughtChunk(text_chunk("hmm")),
        );
        assert_eq!(
            items[0],
            ChatItem::Thinking {
                text: "hmm".to_string(),
                streaming: true,
                message_id: None,
            }
        );
    }

    #[test]
    fn thinking_is_finalized_when_assistant_text_follows() {
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::AgentThoughtChunk(text_chunk("reasoning")),
        );
        apply_update(
            &mut items,
            &SessionUpdate::AgentMessageChunk(text_chunk("answer")),
        );
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0],
            ChatItem::Thinking {
                text: "reasoning".to_string(),
                streaming: false, // kind switch finalizes the thinking block
                message_id: None,
            }
        );
    }

    #[test]
    fn finalize_clears_streaming_flag() {
        let mut items = vec![ChatItem::AssistantText {
            text: "done".to_string(),
            streaming: true,
            message_id: None,
        }];
        finalize_streaming(&mut items);
        assert_eq!(
            items[0],
            ChatItem::AssistantText {
                text: "done".to_string(),
                streaming: false,
                message_id: None,
            }
        );
    }

    #[test]
    fn finalize_settles_every_streaming_block_not_just_the_last() {
        // Real shape: the agent streams "let me look", then starts a tool call,
        // then streams a conclusion. The first text block is no longer the tail,
        // so clearing only the last item would leave it `streaming: true` —
        // making the turn's rollup blink and stay expanded after the turn ends.
        let mut items = vec![
            ChatItem::AssistantText {
                text: "let me look".to_string(),
                streaming: true,
                message_id: Some("m1".to_string()),
            },
            ChatItem::ToolCall(ToolCallItem {
                id: "t1".to_string(),
                title: "Read".to_string(),
                kind: ToolKindView::Read,
                status: ToolStatusView::Completed,
                diffs: Vec::new(),
                output: Vec::new(),
                raw_input: None,
            }),
            ChatItem::AssistantText {
                text: "done".to_string(),
                streaming: true,
                message_id: Some("m2".to_string()),
            },
        ];
        finalize_streaming(&mut items);
        assert!(
            !items.iter().any(|i| matches!(
                i,
                ChatItem::AssistantText {
                    streaming: true,
                    ..
                } | ChatItem::Thinking {
                    streaming: true,
                    ..
                }
            )),
            "every streamed block settles when the turn ends, not just the tail"
        );
    }

    #[test]
    fn cancel_pending_tools_settles_only_unfinished_calls() {
        let tool = |id: &str, status| {
            ChatItem::ToolCall(ToolCallItem {
                id: id.to_string(),
                title: id.to_string(),
                kind: ToolKindView::Read,
                status,
                diffs: Vec::new(),
                output: Vec::new(),
                raw_input: None,
            })
        };
        let mut items = vec![
            tool("pending", ToolStatusView::Pending),
            tool("running", ToolStatusView::InProgress),
            tool("done", ToolStatusView::Completed),
            tool("failed", ToolStatusView::Failed),
        ];
        cancel_pending_tools(&mut items);
        let status = |ix: usize| match &items[ix] {
            ChatItem::ToolCall(tc) => tc.status,
            _ => panic!("expected a tool call"),
        };
        assert_eq!(status(0), ToolStatusView::Cancelled, "pending → cancelled");
        assert_eq!(
            status(1),
            ToolStatusView::Cancelled,
            "in-progress → cancelled"
        );
        assert_eq!(
            status(2),
            ToolStatusView::Completed,
            "completed is terminal"
        );
        assert_eq!(status(3), ToolStatusView::Failed, "failed is terminal");
    }

    #[test]
    fn tool_call_then_update_completes_in_place_with_diff() {
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(ToolCall::new("t1", "Write").kind(ToolKind::Edit)),
        );

        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        fields.content = Some(vec![ToolCallContent::Diff(Diff::new("/tmp/x.txt", "hi\n"))]);
        apply_update(
            &mut items,
            &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("t1", fields)),
        );

        assert_eq!(items.len(), 1, "update must mutate in place, not append");
        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call item");
        };
        assert_eq!(tc.status, ToolStatusView::Completed);
        assert_eq!(tc.diffs.len(), 1);
        assert_eq!(tc.diffs[0].new_text, "hi\n");
    }

    #[test]
    fn permission_request_maps_options_in_order() {
        let mut tool_fields = ToolCallUpdateFields::default();
        tool_fields.title = Some("Write /tmp/x".to_string());
        let request = RequestPermissionRequest::new(
            "s1",
            ToolCallUpdate::new("t1", tool_fields),
            vec![
                PermissionOption::new(
                    "allow_always",
                    "Always Allow",
                    PermissionOptionKind::AllowAlways,
                ),
                PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
            ],
        );

        let ChatItem::Permission(card) = permission_item(&request) else {
            panic!("expected permission item");
        };
        assert_eq!(card.tool_title.as_deref(), Some("Write /tmp/x"));
        assert_eq!(card.options.len(), 2);
        assert_eq!(card.options[0].kind, PermissionKindView::AllowAlways);
        assert_eq!(card.options[1].option_id, "reject");
        assert_eq!(card.resolved, None);
    }
}

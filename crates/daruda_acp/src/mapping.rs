//! Fold ACP protocol traffic into the [`crate::model`] chat item list.
//!
//! Pure functions over `&mut Vec<ChatItem>` so they unit-test without any
//! connection or executor. MVP handles the streaming/tool/permission updates
//! that drive the conversation view; plan, slash-command, mode, config, info,
//! and usage updates are intentionally ignored.

use agent_client_protocol::schema::v1::{
    ContentBlock, PermissionOption, PermissionOptionKind, RequestPermissionRequest, SessionUpdate,
    ToolCall, ToolCallContent, ToolCallStatus, ToolCallUpdate, ToolKind,
};

use crate::model::{
    ChatItem, DiffView, PermissionChoice, PermissionItem, PermissionKindView, ToolCallItem,
    ToolKindView, ToolStatusView,
};

/// Apply one `session/update` notification to the chat item list.
pub fn apply_update(items: &mut Vec<ChatItem>, update: &SessionUpdate) {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            append_streaming(items, &text_of(&chunk.content), StreamKind::Assistant);
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            append_streaming(items, &text_of(&chunk.content), StreamKind::Thinking);
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

/// Stop the streaming flag on the trailing assistant/thinking item — called
/// when a prompt turn completes so the view drops any "typing" affordance.
pub fn finalize_streaming(items: &mut [ChatItem]) {
    if let Some(ChatItem::AssistantText { streaming, .. } | ChatItem::Thinking { streaming, .. }) =
        items.last_mut()
    {
        *streaming = false;
    }
}

#[derive(Clone, Copy)]
enum StreamKind {
    Assistant,
    Thinking,
}

/// Append streamed text, extending the trailing item when it is the same
/// still-streaming kind, otherwise starting a new one. Empty chunks (the agent
/// emits a leading empty chunk per message) start the item without text.
fn append_streaming(items: &mut Vec<ChatItem>, text: &str, kind: StreamKind) {
    match (items.last_mut(), kind) {
        (
            Some(ChatItem::AssistantText {
                text: prev,
                streaming: true,
            }),
            StreamKind::Assistant,
        ) => {
            prev.push_str(text);
        }
        (
            Some(ChatItem::Thinking {
                text: prev,
                streaming: true,
            }),
            StreamKind::Thinking,
        ) => {
            prev.push_str(text);
        }
        (_, StreamKind::Assistant) => items.push(ChatItem::AssistantText {
            text: text.to_string(),
            streaming: true,
        }),
        (_, StreamKind::Thinking) => items.push(ChatItem::Thinking {
            text: text.to_string(),
            streaming: true,
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
                streaming: true
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
                streaming: true
            }
        );
    }

    #[test]
    fn finalize_clears_streaming_flag() {
        let mut items = vec![ChatItem::AssistantText {
            text: "done".to_string(),
            streaming: true,
        }];
        finalize_streaming(&mut items);
        assert_eq!(
            items[0],
            ChatItem::AssistantText {
                text: "done".to_string(),
                streaming: false
            }
        );
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

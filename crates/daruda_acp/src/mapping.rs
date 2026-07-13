//! Fold ACP protocol traffic into the [`crate::model`] chat item list.
//!
//! Pure functions over `&mut Vec<ChatItem>` so they unit-test without any
//! connection or executor. MVP handles the streaming/tool/permission updates
//! that drive the conversation view; plan, slash-command, mode, config, info,
//! and usage updates are intentionally ignored.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, PermissionOption, PermissionOptionKind, RequestPermissionRequest,
    SessionUpdate, ToolCall, ToolCallContent, ToolCallStatus, ToolCallUpdate, ToolKind,
};

use crate::model::{
    ChatItem, DiffView, PermissionChoice, PermissionItem, PermissionKindView, ToolCallItem,
    ToolKindView, ToolOutputBlock, ToolStatusView,
};

/// What an applied `session/update` touched, so the host can gate its expensive
/// per-event reconciles instead of rescanning the whole conversation on every
/// event: diff editors are rebuilt only when a tool call changed, and mermaid
/// diagrams re-rasterized only when message text changed. Keeping the protocol
/// match here (rather than exposing `SessionUpdate` variants to the host) holds
/// the "host never touches protocol types" boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpdateEffect {
    /// A tool call was inserted or updated — its diffs may have changed.
    pub touched_tool: bool,
    /// Assistant / thinking / user message text changed — it may carry a
    /// ` ```mermaid ` fence to rasterize.
    pub touched_text: bool,
}

/// Apply one `session/update` notification to the chat item list, reporting what
/// it touched via [`UpdateEffect`] so the host can gate its reconciles.
pub fn apply_update(items: &mut Vec<ChatItem>, update: &SessionUpdate) -> UpdateEffect {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            append_streaming(
                items,
                &text_of(&chunk.content),
                msg_id(chunk),
                StreamKind::Assistant,
            );
            UpdateEffect {
                touched_text: true,
                ..UpdateEffect::default()
            }
        }
        SessionUpdate::AgentThoughtChunk(chunk) => {
            append_streaming(
                items,
                &text_of(&chunk.content),
                msg_id(chunk),
                StreamKind::Thinking,
            );
            UpdateEffect {
                touched_text: true,
                ..UpdateEffect::default()
            }
        }
        SessionUpdate::UserMessageChunk(chunk) => {
            append_user_chunk(items, &text_of(&chunk.content));
            UpdateEffect {
                touched_text: true,
                ..UpdateEffect::default()
            }
        }
        SessionUpdate::ToolCall(tool_call) => {
            upsert_tool_call(items, tool_call);
            UpdateEffect {
                touched_tool: true,
                ..UpdateEffect::default()
            }
        }
        SessionUpdate::ToolCallUpdate(update) => {
            apply_tool_call_update(items, update);
            UpdateEffect {
                touched_tool: true,
                ..UpdateEffect::default()
            }
        }
        // MVP: plan / available-commands / mode / config / info / usage updates
        // carry no conversation content we render yet.
        _ => UpdateEffect::default(),
    }
}

/// Build a pending permission card from a `session/request_permission` request.
pub fn permission_item(request: &RequestPermissionRequest) -> ChatItem {
    ChatItem::Permission(PermissionItem {
        tool_title: request.tool_call.fields.title.clone(),
        raw_input_summary: summarize_raw_input(request.tool_call.fields.raw_input.as_ref()),
        options: request.options.iter().map(choice_of).collect(),
        resolved: None,
    })
}

/// Keys (in priority order) checked when summarizing a tool's `raw_input`
/// into one short, safe line for [`permission_item`] — an allowlist, not a
/// generic dump: a bulk-content field (Edit's `old_string`/`new_string`,
/// Write's `content`) is diff-shaped data exactly like
/// `ToolCallContent::Diff`, and just as wrong to surface in a one-line
/// summary as a full diff would be, so only short, single-value fields are
/// read.
const RAW_INPUT_SUMMARY_KEYS: &[(&str, &str)] = &[
    ("command", "command"),
    ("file_path", "file"),
    ("path", "file"),
    ("pattern", "pattern"),
    ("query", "query"),
    ("url", "url"),
];

/// Max chars kept from a matched raw-input value. A well-formed value for
/// any of [`RAW_INPUT_SUMMARY_KEYS`] is short (a path, a command, a query),
/// but nothing in the protocol bounds what an adapter actually sends.
const RAW_INPUT_SUMMARY_MAX_CHARS: usize = 300;

/// Summarize `raw_input` into one `"<label>: <value>"` line — `None` when
/// `raw_input` is absent, isn't a JSON object, or none of
/// [`RAW_INPUT_SUMMARY_KEYS`] is present as a non-empty string.
fn summarize_raw_input(raw_input: Option<&serde_json::Value>) -> Option<String> {
    let object = raw_input?.as_object()?;
    let (label, value) = RAW_INPUT_SUMMARY_KEYS.iter().find_map(|(key, label)| {
        let value = object.get(*key)?.as_str()?;
        (!value.is_empty()).then_some((*label, value))
    })?;
    let char_count = value.chars().count();
    let value = if char_count > RAW_INPUT_SUMMARY_MAX_CHARS {
        let truncated: String = value.chars().take(RAW_INPUT_SUMMARY_MAX_CHARS).collect();
        format!("{truncated}…")
    } else {
        value.to_string()
    };
    Some(format!("{label}: {value}"))
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

/// The `tool_call_id` a `session/update` targets, if it is a tool-call event
/// (a `ToolCall` insert or a `ToolCallUpdate`); `None` for every other update
/// kind. The host uses this to find the `ChatItem::ToolCall` that `apply_update`
/// just mutated and bump its parent subagent's last-activity timestamp — without
/// itself reaching into protocol types (the host consumes only the render model).
pub fn touched_tool_id(update: &SessionUpdate) -> Option<String> {
    match update {
        SessionUpdate::ToolCall(tc) => Some(tc.tool_call_id.0.to_string()),
        SessionUpdate::ToolCallUpdate(u) => Some(u.tool_call_id.0.to_string()),
        _ => None,
    }
}

/// Aggregate run-state over the background subagents in a conversation.
pub struct SubagentActivity {
    /// Number of distinct subagents (parent Task/Agent tool calls).
    pub total: usize,
    /// Subagents that are no longer running.
    pub settled: usize,
    /// At least one subagent is still running.
    pub any_running: bool,
}

/// Derive subagent run-state. A *subagent* is a tool call whose id is
/// referenced by some other tool's `parent_tool_id` (its parent Task/Agent
/// call). A subagent P is *running* while it has a live child
/// (`parent_tool_id == P && status.is_live()`) OR its last child activity was
/// within `quiescence` of `now`. The quiescence window bridges the gaps
/// between a subagent's sequential child tool calls: the parent's own status
/// completes early and there is no clean terminal signal, so "no live child
/// right now" does not mean the run ended.
pub fn subagent_activity(
    items: &[ChatItem],
    last_activity: &HashMap<String, Instant>,
    now: Instant,
    quiescence: Duration,
) -> SubagentActivity {
    // Single pass over `items` collects both the set of every parent seen and
    // the subset whose child tool is currently live — O(N) instead of scanning
    // `items` once per parent (the old O(P·N) `has_live_child` per parent).
    let mut parents: HashSet<&str> = HashSet::new();
    let mut live_parents: HashSet<&str> = HashSet::new();
    for it in items {
        if let ChatItem::ToolCall(tc) = it
            && let Some(parent) = tc.parent_tool_id.as_deref()
        {
            parents.insert(parent);
            if tc.status.is_live() {
                live_parents.insert(parent);
            }
        }
    }

    let recent = |parent: &str| {
        last_activity
            .get(parent)
            .is_some_and(|t| now.saturating_duration_since(*t) < quiescence)
    };

    let total = parents.len();
    let running_count = parents
        .iter()
        .filter(|parent| live_parents.contains(*parent) || recent(parent))
        .count();

    SubagentActivity {
        total,
        settled: total - running_count,
        any_running: running_count > 0,
    }
}

/// Fold a `UserMessageChunk` into the item list.
///
/// The host echoes the user's prompt locally on send (a `UserText` pushed
/// straight into `items`), so an adapter that *also* replays that prompt back
/// as a `UserMessageChunk` would double the bubble. Defensive dedup: skip a
/// chunk that exactly repeats the trailing user text — the local echo is the
/// authoritative copy of a user turn. This can only ever collapse an
/// agent-originated replay of the message already on screen; two genuine
/// consecutive user turns are pushed as separate local echoes and never travel
/// through this path, so they are unaffected. In the observed adapter the
/// replay does not occur, but the guard keeps a future/other adapter from
/// duplicating the turn.
///
/// WORKAROUND: also drop a chunk that is a harness-injected task-notification
/// blob (`<task-notification>…`) rather than real conversation text. The
/// Claude Agent SDK persists background-subagent completions as synthetic
/// `role: "user"` transcript entries; the claude-agent-acp adapter's *live*
/// path deliberately skips any such string-content user message
/// ("these seem to be messages we don't want in the feed" —
/// `acp-agent.js`'s live `session/update` loop), but its `session/load`
/// *replay* path (`replaySessionHistory`) has no equivalent guard — it only
/// strips `<command-*>`/`<local-command-*>` markers, not this one — so a
/// restored pane replays every past task-notification verbatim, including an
/// embedded `<system-reminder>`, straight into the user bubble. Root cause is
/// in the adapter (an npm dependency we don't vendor), so this is a
/// host-side filter until upstream carries the fix.
fn append_user_chunk(items: &mut Vec<ChatItem>, text: &str) {
    if text.trim_start().starts_with("<task-notification>") {
        return;
    }
    if matches!(items.last(), Some(ChatItem::UserText(prev)) if prev == text) {
        return;
    }
    items.push(ChatItem::UserText(text.to_string()));
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

/// The parent tool-call id the Claude adapter stamps on a subagent's inner tool
/// calls: `_meta.claudeCode.parentToolUseId`. The adapter flattens subagent
/// activity into the single session, so this is the only link from a child call
/// back to the `Task`/`Agent` call that spawned it. `None` for a top-level call
/// (no such meta) or any other adapter that doesn't set it.
fn parent_tool_id_from_meta(
    meta: &Option<agent_client_protocol::schema::v1::Meta>,
) -> Option<String> {
    meta.as_ref()?
        .get("claudeCode")?
        .get("parentToolUseId")?
        .as_str()
        .map(str::to_owned)
}

fn upsert_tool_call(items: &mut Vec<ChatItem>, tool_call: &ToolCall) {
    let id = tool_call.tool_call_id.0.to_string();
    let (diffs, output) = split_content(&tool_call.content);
    let mut item = ToolCallItem {
        id: id.clone(),
        title: tool_call.title.clone(),
        kind: kind_of(&tool_call.kind),
        status: status_of(&tool_call.status),
        diffs,
        output,
        raw_input: tool_call.raw_input.clone(),
        parent_tool_id: parent_tool_id_from_meta(&tool_call.meta),
    };
    highlight_tool_output(&mut item);
    match find_tool_call(items, &id) {
        Some(existing) => *existing = item,
        None => items.push(ChatItem::ToolCall(item)),
    }
}

/// Rewrite the tool's text output so its fenced code block syntax-highlights:
/// the ACP adapter wraps results in a language-less ``` fence, so we inject the
/// language inferred from the tool (the read target's file extension) and strip
/// any `cat -n` line-number prefixes. No-op for tools without an inferable
/// language and idempotent (an already-tagged fence is left untouched), so it
/// is safe to run after every tool-call insert or update.
fn highlight_tool_output(item: &mut ToolCallItem) {
    let Some(lang) = crate::output_highlight::output_language(item.kind, &item.raw_input) else {
        return;
    };
    for block in &mut item.output {
        if let ToolOutputBlock::Text(text) = block {
            *text = crate::output_highlight::rewrite_fenced_output(text, lang);
        }
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
    // Run last: kind / raw_input (source of the language) and output text are
    // both current by now, and the rewrite is idempotent.
    highlight_tool_output(item);
}

fn find_tool_call<'a>(items: &'a mut [ChatItem], id: &str) -> Option<&'a mut ToolCallItem> {
    items.iter_mut().rev().find_map(|item| match item {
        ChatItem::ToolCall(tc) if tc.id == id => Some(tc),
        _ => None,
    })
}

/// Partition tool-call content into diffs and typed output blocks. Embedded
/// terminals, images, audio, and embedded resources are not rendered yet and
/// are dropped.
fn split_content(content: &[ToolCallContent]) -> (Vec<DiffView>, Vec<ToolOutputBlock>) {
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
                if let Some(out) = output_block_of(&c.content) {
                    output.push(out);
                }
            }
            // Embedded terminals and any future content kinds are not rendered.
            _ => {}
        }
    }
    (diffs, output)
}

/// Map a content block to a renderable output block. `None` for empty text and
/// for kinds we don't render yet (image, audio, embedded resource).
fn output_block_of(block: &ContentBlock) -> Option<ToolOutputBlock> {
    match block {
        ContentBlock::Text(t) if !t.text.is_empty() => Some(ToolOutputBlock::Text(t.text.clone())),
        ContentBlock::ResourceLink(rl) => Some(ToolOutputBlock::ResourceLink {
            uri: rl.uri.clone(),
            // Prefer the human title; the `name` field is always present.
            name: rl.title.clone().unwrap_or_else(|| rl.name.clone()),
        }),
        _ => None,
    }
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
        Content, ContentChunk, Diff, ResourceLink, TextContent, ToolCallUpdateFields,
    };

    #[test]
    fn parent_tool_id_read_from_claude_meta() {
        let obj = |v: serde_json::Value| Some(v.as_object().unwrap().clone());
        // The Claude adapter stamps the parent id here on a subagent's inner call.
        assert_eq!(
            parent_tool_id_from_meta(&obj(
                serde_json::json!({"claudeCode": {"parentToolUseId": "toolu_parent"}})
            )),
            Some("toolu_parent".to_owned())
        );
        // No meta, or meta without the parent key → top-level call.
        assert_eq!(parent_tool_id_from_meta(&None), None);
        assert_eq!(
            parent_tool_id_from_meta(&obj(
                serde_json::json!({"claudeCode": {"toolName": "Bash"}})
            )),
            None
        );
    }

    #[test]
    fn split_content_types_output_blocks() {
        let content = vec![
            ToolCallContent::Content(Content::new(ContentBlock::Text(TextContent::new("hello")))),
            // Empty text is dropped, not carried as an empty block.
            ToolCallContent::Content(Content::new(ContentBlock::Text(TextContent::new("")))),
            ToolCallContent::Content(Content::new(ContentBlock::ResourceLink(ResourceLink::new(
                "file.rs",
                "file:///tmp/file.rs",
            )))),
            ToolCallContent::Diff(Diff::new("/tmp/x.txt", "hi\n")),
        ];
        let (diffs, output) = split_content(&content);
        assert_eq!(diffs.len(), 1);
        assert_eq!(
            output,
            vec![
                ToolOutputBlock::Text("hello".to_string()),
                ToolOutputBlock::ResourceLink {
                    uri: "file:///tmp/file.rs".to_string(),
                    name: "file.rs".to_string(),
                },
            ]
        );
    }

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
    fn user_message_chunk_deduped_against_local_echo() {
        // The host echoed the prompt locally; the adapter then replays the same
        // text as a UserMessageChunk. The replay must not double the bubble.
        let mut items = vec![ChatItem::UserText("run the tests".to_string())];
        apply_update(
            &mut items,
            &SessionUpdate::UserMessageChunk(text_chunk("run the tests")),
        );
        assert_eq!(
            items.len(),
            1,
            "an exact replay of the echoed prompt is dropped"
        );
    }

    #[test]
    fn distinct_user_message_chunk_is_kept() {
        // A user chunk that does not repeat the trailing text is a real new
        // message and is appended (the guard only drops exact duplicates).
        let mut items = vec![ChatItem::UserText("first".to_string())];
        apply_update(
            &mut items,
            &SessionUpdate::UserMessageChunk(text_chunk("second")),
        );
        assert_eq!(items.len(), 2);
        assert_eq!(items[1], ChatItem::UserText("second".to_string()));
    }

    #[test]
    fn user_message_chunk_appends_when_no_trailing_user_text() {
        // No preceding echo (e.g. the trailing item is agent text): the chunk is
        // a genuine user message and is pushed.
        let mut items = vec![ChatItem::AssistantText {
            text: "hi".to_string(),
            streaming: false,
            message_id: None,
        }];
        apply_update(
            &mut items,
            &SessionUpdate::UserMessageChunk(text_chunk("a real prompt")),
        );
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn user_message_chunk_drops_a_replayed_task_notification() {
        // session/load replays a background-subagent task-notification the SDK
        // persisted as a synthetic `role: "user"` transcript entry (the
        // adapter's live path already skips these; its replay path doesn't —
        // see append_user_chunk's WORKAROUND doc). It must not surface as a
        // user chat bubble.
        let mut items = vec![ChatItem::AssistantText {
            text: "hi".to_string(),
            streaming: false,
            message_id: None,
        }];
        apply_update(
            &mut items,
            &SessionUpdate::UserMessageChunk(text_chunk(
                "<task-notification>\n<task-id>abc</task-id>\n\
                 <system-reminder>do not mention this</system-reminder>\n\
                 </task-notification>",
            )),
        );
        assert_eq!(items.len(), 1, "the task-notification blob is dropped");
    }

    #[test]
    fn user_message_chunk_keeps_text_that_merely_mentions_task_notification() {
        // Only a chunk that *is* the wrapper (after leading whitespace) is
        // dropped; genuine prose that happens to reference the tag survives.
        let mut items = vec![ChatItem::AssistantText {
            text: "hi".to_string(),
            streaming: false,
            message_id: None,
        }];
        apply_update(
            &mut items,
            &SessionUpdate::UserMessageChunk(text_chunk("what does <task-notification> mean?")),
        );
        assert_eq!(items.len(), 2);
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
                parent_tool_id: None,
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
                parent_tool_id: None,
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

    fn subagent_child(parent: &str, status: ToolStatusView) -> ChatItem {
        ChatItem::ToolCall(ToolCallItem {
            id: format!("{parent}-child"),
            title: "child".to_string(),
            kind: ToolKindView::Read,
            status,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: Some(parent.to_string()),
        })
    }

    #[test]
    fn subagent_activity_counts_live_child_as_running() {
        let items = [subagent_child("p1", ToolStatusView::InProgress)];
        let last_activity = HashMap::new();
        let activity = subagent_activity(
            &items,
            &last_activity,
            Instant::now(),
            Duration::from_secs(8),
        );
        assert_eq!(activity.total, 1);
        assert_eq!(activity.settled, 0);
        assert!(activity.any_running);
    }

    #[test]
    fn subagent_activity_treats_recent_activity_as_running() {
        let items = [subagent_child("p1", ToolStatusView::Completed)];
        let now = Instant::now();
        let mut last_activity = HashMap::new();
        last_activity.insert("p1".to_string(), now - Duration::from_secs(2));
        let activity = subagent_activity(&items, &last_activity, now, Duration::from_secs(8));
        assert_eq!(activity.total, 1);
        assert_eq!(activity.settled, 0, "recent activity keeps it running");
        assert!(activity.any_running);
    }

    #[test]
    fn subagent_activity_settles_after_quiescence_elapses() {
        let items = [subagent_child("p1", ToolStatusView::Completed)];
        let now = Instant::now();
        let mut last_activity = HashMap::new();
        last_activity.insert("p1".to_string(), now - Duration::from_secs(20));
        let activity = subagent_activity(&items, &last_activity, now, Duration::from_secs(8));
        assert_eq!(activity.total, 1);
        assert_eq!(activity.settled, 1);
        assert!(!activity.any_running);
    }

    #[test]
    fn subagent_activity_boundary_is_exclusive_at_quiescence() {
        // Exactly `quiescence` old is the `<` boundary: `now - t == quiescence`
        // is NOT within the window (the check is strict `<`), so a subagent whose
        // only child is settled and whose last activity lands exactly on the
        // boundary is treated as settled, not running.
        let items = [subagent_child("p1", ToolStatusView::Completed)];
        let quiescence = Duration::from_secs(8);
        let now = Instant::now();
        let mut last_activity = HashMap::new();
        last_activity.insert("p1".to_string(), now - quiescence);
        let activity = subagent_activity(&items, &last_activity, now, quiescence);
        assert_eq!(activity.total, 1);
        assert_eq!(
            activity.settled, 1,
            "activity exactly quiescence old is outside the window"
        );
        assert!(!activity.any_running);
    }

    #[test]
    fn subagent_activity_mixes_running_and_settled_parents() {
        let now = Instant::now();
        let items = [
            subagent_child("running", ToolStatusView::InProgress),
            subagent_child("quiesced", ToolStatusView::Completed),
        ];
        let mut last_activity = HashMap::new();
        last_activity.insert("quiesced".to_string(), now - Duration::from_secs(20));
        let activity = subagent_activity(&items, &last_activity, now, Duration::from_secs(8));
        assert_eq!(activity.total, 2);
        assert_eq!(activity.settled, 1);
        assert!(activity.any_running);
    }

    #[test]
    fn subagent_activity_empty_items_is_all_zero() {
        let last_activity = HashMap::new();
        let activity =
            subagent_activity(&[], &last_activity, Instant::now(), Duration::from_secs(8));
        assert_eq!(activity.total, 0);
        assert_eq!(activity.settled, 0);
        assert!(!activity.any_running);
    }

    #[test]
    fn subagent_activity_counts_untracked_settled_parent() {
        // A parent id with no live child and no last_activity entry at all
        // (never recorded, or the map predates it) is settled, not running.
        let items = [subagent_child("p1", ToolStatusView::Completed)];
        let last_activity = HashMap::new();
        let activity = subagent_activity(
            &items,
            &last_activity,
            Instant::now(),
            Duration::from_secs(8),
        );
        assert_eq!(activity.total, 1);
        assert_eq!(activity.settled, 1);
        assert!(!activity.any_running);
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
    fn touched_tool_id_extracts_only_tool_call_events() {
        // A `ToolCall` insert and a `ToolCallUpdate` both carry the target id.
        assert_eq!(
            touched_tool_id(&SessionUpdate::ToolCall(ToolCall::new("t1", "Read"))),
            Some("t1".to_string())
        );
        assert_eq!(
            touched_tool_id(&SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "t2",
                ToolCallUpdateFields::default()
            ))),
            Some("t2".to_string())
        );
        // A text chunk is not a tool-call event → no id.
        assert_eq!(
            touched_tool_id(&SessionUpdate::AgentMessageChunk(text_chunk("hi"))),
            None
        );
    }

    #[test]
    fn read_output_gets_language_injected_and_line_numbers_stripped() {
        let mut items = Vec::new();
        // A Read tool call carrying its target path in raw_input.
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("t1", "Read src/main.rs")
                    .kind(ToolKind::Read)
                    .raw_input(serde_json::json!({"file_path": "src/main.rs"})),
            ),
        );
        // The adapter delivers the file as a language-less, line-numbered fence.
        let mut fields = ToolCallUpdateFields::default();
        fields.status = Some(ToolCallStatus::Completed);
        fields.content = Some(vec![ToolCallContent::Content(Content::new(
            ContentBlock::Text(TextContent::new("```\n1\tfn main() {}\n2\t// end\n```")),
        ))]);
        apply_update(
            &mut items,
            &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("t1", fields)),
        );

        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call item");
        };
        assert_eq!(
            tc.output,
            vec![ToolOutputBlock::Text(
                "```rust\nfn main() {}\n// end\n```".to_string()
            )]
        );
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

    #[test]
    fn permission_request_summarizes_a_known_raw_input_field() {
        let mut tool_fields = ToolCallUpdateFields::default();
        tool_fields.title = Some("Run npm install".to_string());
        tool_fields.raw_input = Some(serde_json::json!({"command": "npm install"}));
        let request = RequestPermissionRequest::new(
            "s1",
            ToolCallUpdate::new("t1", tool_fields),
            vec![PermissionOption::new(
                "allow",
                "Allow",
                PermissionOptionKind::AllowOnce,
            )],
        );

        let ChatItem::Permission(card) = permission_item(&request) else {
            panic!("expected permission item");
        };
        assert_eq!(
            card.raw_input_summary.as_deref(),
            Some("command: npm install")
        );
    }

    #[test]
    fn summarize_raw_input_picks_the_first_matching_key_in_priority_order() {
        // `command` outranks `file_path` when both happen to be present.
        assert_eq!(
            summarize_raw_input(Some(
                &serde_json::json!({"file_path": "/tmp/x", "command": "cat /tmp/x"})
            )),
            Some("command: cat /tmp/x".to_string())
        );
    }

    #[test]
    fn summarize_raw_input_truncates_an_overlong_value() {
        let long_path = "a".repeat(RAW_INPUT_SUMMARY_MAX_CHARS + 50);
        let summary =
            summarize_raw_input(Some(&serde_json::json!({"file_path": long_path}))).unwrap();
        assert_eq!(
            summary.chars().count(),
            "file: ".len() + RAW_INPUT_SUMMARY_MAX_CHARS + 1 // +1 for the "…" marker
        );
        assert!(summary.ends_with('…'));
    }

    #[test]
    fn summarize_raw_input_is_none_without_a_known_key() {
        assert_eq!(
            summarize_raw_input(Some(&serde_json::json!({"subagent_type": "reviewer"}))),
            None
        );
    }

    #[test]
    fn summarize_raw_input_is_none_for_an_empty_matched_value() {
        assert_eq!(
            summarize_raw_input(Some(&serde_json::json!({"command": ""}))),
            None
        );
    }

    #[test]
    fn summarize_raw_input_is_none_without_raw_input() {
        assert_eq!(summarize_raw_input(None), None);
    }
}

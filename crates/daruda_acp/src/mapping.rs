//! Fold ACP protocol traffic into the [`crate::model`] chat item list.
//!
//! Pure functions over `&mut Vec<ChatItem>` so they unit-test without any
//! connection or executor. MVP handles the streaming/tool/permission updates
//! that drive the conversation view; plan, slash-command, mode, config, info,
//! and usage updates are intentionally ignored.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, EmbeddedResourceResource, PermissionOption, PermissionOptionKind,
    RequestPermissionRequest, SessionUpdate, ToolCall, ToolCallContent, ToolCallStatus,
    ToolCallUpdate, ToolKind,
};

use crate::adapter::{AcpAdapter, DefaultAdapter, MessagePhase};
use crate::model::{
    ChatItem, DiffView, PermissionChoice, PermissionItem, PermissionKindView, ToolCallItem,
    ToolKindView, ToolOutputBlock, ToolStatusView,
};
use crate::output_highlight::TextOutputKind;

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
    /// A settled tool call carried an embedded terminal block whose output this
    /// build could not recover from any channel, leaving the card empty. This
    /// crate has no logger of its own (it depends on `daruda_core` only), so the
    /// host logs it — see [`crate::adapter::AcpAdapter::sideband_output`] for
    /// the channel that normally recovers it.
    pub dropped_terminal_output: bool,
}

/// Apply one `session/update` notification to the chat item list, reporting what
/// it touched via [`UpdateEffect`] so the host can gate its reconciles.
pub fn apply_update(items: &mut Vec<ChatItem>, update: &SessionUpdate) -> UpdateEffect {
    apply_update_with(items, update, &DefaultAdapter)
}

/// [`apply_update`] with an explicit per-agent strategy (see [`crate::adapter`]).
/// The host selects the adapter once per session and passes it on every update;
/// the plain [`apply_update`] is sugar that uses [`DefaultAdapter`].
pub fn apply_update_with(
    items: &mut Vec<ChatItem>,
    update: &SessionUpdate,
    adapter: &dyn AcpAdapter,
) -> UpdateEffect {
    match update {
        SessionUpdate::AgentMessageChunk(chunk) => {
            append_streaming(
                items,
                &text_of(&chunk.content),
                msg_id(chunk),
                StreamKind::Assistant,
                adapter.message_phase(&chunk.meta),
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
                // Thinking is never a message role; no captured adapter labels
                // a thought chunk, so the phase field it would set is unused.
                MessagePhase::Answer,
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
        SessionUpdate::ToolCall(tool_call) => UpdateEffect {
            touched_tool: true,
            dropped_terminal_output: upsert_tool_call(items, tool_call, adapter),
            ..UpdateEffect::default()
        },
        SessionUpdate::ToolCallUpdate(update) => UpdateEffect {
            touched_tool: true,
            dropped_terminal_output: apply_tool_call_update(items, update, adapter),
            ..UpdateEffect::default()
        },
        // MVP: plan / available-commands / mode / config / info / usage updates
        // carry no conversation content we render yet.
        _ => UpdateEffect::default(),
    }
}

/// Build a pending permission card from a `session/request_permission` request.
/// `items` is the conversation's current item list — used to look up a
/// `raw_input` already recorded for this tool call id (see
/// [`existing_raw_input`]) in preference to the request's own copy, which
/// some adapters (codex-acp) re-quote for the permission prompt and can
/// mangle in the process.
pub fn permission_item(
    id: u64,
    request: &RequestPermissionRequest,
    items: &[ChatItem],
) -> ChatItem {
    let own_raw_input = request.tool_call.fields.raw_input.as_ref();
    let raw_input = existing_raw_input(items, &request.tool_call.tool_call_id.0).or(own_raw_input);
    ChatItem::Permission(PermissionItem {
        id,
        tool_title: request.tool_call.fields.title.clone(),
        raw_input_summary: summarize_raw_input(raw_input),
        options: request.options.iter().map(choice_of).collect(),
        resolved: None,
    })
}

/// The `raw_input` already recorded on a `ChatItem::ToolCall` matching
/// `tool_call_id`, if the item list has one. A prior `tool_call` (insert)
/// event for the same id — which arrives before the permission request in
/// every observed adapter — carries the pristine value; the permission
/// request's own copy is a separate, adapter-reconstructed echo that isn't
/// guaranteed to match it byte-for-byte.
fn existing_raw_input<'a>(
    items: &'a [ChatItem],
    tool_call_id: &str,
) -> Option<&'a serde_json::Value> {
    items.iter().rev().find_map(|item| match item {
        ChatItem::ToolCall(tc) if tc.id == tool_call_id => tc.raw_input.as_ref(),
        _ => None,
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
    phase: MessagePhase,
) {
    if let Some(last) = items.last_mut() {
        match (last, kind) {
            (
                ChatItem::AssistantText {
                    text: prev,
                    streaming: true,
                    message_id: mid,
                    // Not matched on: the role was fixed when this message
                    // started, so a chunk that restates it differently is an
                    // adapter contradicting itself, not a new message.
                    phase: _,
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
            phase,
        }),
        StreamKind::Thinking => items.push(ChatItem::Thinking {
            text: text.to_string(),
            streaming: true,
            message_id,
        }),
    }
}

/// Insert or replace a tool call. Returns whether its terminal content was
/// dropped unrecovered — see [`UpdateEffect::dropped_terminal_output`].
fn upsert_tool_call(
    items: &mut Vec<ChatItem>,
    tool_call: &ToolCall,
    adapter: &dyn AcpAdapter,
) -> bool {
    let id = tool_call.tool_call_id.0.to_string();
    let SplitContent {
        diffs,
        output: blocks,
        saw_terminal,
    } = split_content(&tool_call.content);
    let sideband = adapter.sideband_output(&tool_call.meta);
    let unreported_terminal = saw_terminal && sideband.is_none();
    let kind = kind_of(&tool_call.kind);
    let mut output = Vec::new();
    fold_output(
        &mut output,
        Some(ContentBody {
            blocks,
            terminal_handle: saw_terminal,
        }),
        sideband,
        &tool_call.raw_output,
    );
    let mut item = ToolCallItem {
        id: id.clone(),
        title: tool_call.title.clone(),
        kind,
        tool_name: adapter.tool_name(&tool_call.meta),
        status: status_of(&tool_call.status),
        diffs,
        output,
        raw_input: tool_call.raw_input.clone(),
        parent_tool_id: adapter.parent_tool_id(&tool_call.meta),
        exit: adapter.command_exit(&tool_call.raw_output, &tool_call.meta),
    };
    classify_source_output(&mut item);
    strip_redundant_edit_output(&mut item);
    let dropped = unreported_terminal && lost_output(&item);
    match find_tool_call(items, &id) {
        Some(existing) => *existing = item,
        None => items.push(ChatItem::ToolCall(item)),
    }
    dropped
}

/// Whether a dropped terminal block actually cost the user something: the call
/// has settled and no other channel filled the card. While the call is still
/// live the output simply hasn't arrived yet (every adapter's *first* Bash
/// event is a content-less handle), and codex's completion refills the card
/// from `raw_output` — neither is a loss worth logging.
fn lost_output(item: &ToolCallItem) -> bool {
    !item.status.is_live() && item.output.is_empty()
}

/// Retype a file read's text output as [`ToolOutputBlock::SourceText`]: the
/// bytes are one file's contents, not markdown, so the adapter's escaping fence
/// and the tool's `cat -n` gutter come off and the language its path implies
/// rides along as a field. No-op for every other tool, and idempotent (a retyped
/// block is no longer `Text`), so it is safe to run after every tool-call insert
/// or update.
fn classify_source_output(item: &mut ToolCallItem) {
    let TextOutputKind::Source { language } =
        crate::output_highlight::classify_text_output(item.kind, &item.raw_input)
    else {
        return;
    };
    for block in &mut item.output {
        crate::output_highlight::retype_as_source(block, language);
    }
}

/// Drop a *successful* edit's textual output. On completion an Edit/Write call
/// carries nothing beyond what the diff and the "Done" badge already say:
/// claude-agent-acp's only channel there is a self-referential `raw_output`
/// confirmation meant for the model's own context (e.g. "...no need to Read it
/// back"), and codex-acp reports none at all. A failed edit is left untouched —
/// that status's `output` carries the real rejection/error text via `content`,
/// which has no diff-shaped substitute. Idempotent, so safe to run after every
/// insert or update.
fn strip_redundant_edit_output(item: &mut ToolCallItem) {
    if item.kind == ToolKindView::Edit && item.status == ToolStatusView::Completed {
        item.output.clear();
    }
}

/// Fold a `ToolCallUpdate` into the matching tool call. Returns whether its
/// terminal content was dropped unrecovered — see
/// [`UpdateEffect::dropped_terminal_output`].
fn apply_tool_call_update(
    items: &mut [ChatItem],
    update: &ToolCallUpdate,
    adapter: &dyn AcpAdapter,
) -> bool {
    let id = update.tool_call_id.0.to_string();
    let Some(item) = find_tool_call(items, &id) else {
        return false;
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
    // Read unconditionally, outside the `content` guard: claude-agent-acp ships
    // the captured bytes on a notification of their own that carries neither
    // `content` nor `status` (`dist/acp-agent.js`, 0.62.0), so gating this on
    // `content` would never see it.
    let sideband = adapter.sideband_output(&update.meta);
    // `None` when the update carried no `content` field at all — distinct from a
    // present-but-empty one, which `fold_output` must treat as a replacement.
    let body = fields.content.as_ref().map(|content| {
        let split = split_content(content);
        item.diffs = split.diffs;
        ContentBody {
            blocks: split.output,
            terminal_handle: split.saw_terminal,
        }
    });
    if fields.raw_input.is_some() {
        item.raw_input = fields.raw_input.clone();
    }
    let unreported_terminal =
        body.as_ref().is_some_and(|b| b.terminal_handle) && sideband.is_none();
    fold_output(&mut item.output, body, sideband, &fields.raw_output);
    // Only overwrite on a reported value — an intermediate status-only update
    // carries no exit channel and must not blank out one recorded earlier.
    if let Some(exit) = adapter.command_exit(&fields.raw_output, &update.meta) {
        item.exit = Some(exit);
    }
    // Run last: kind / raw_input (source of the language) and output text are
    // both current by now, and the retype is idempotent.
    classify_source_output(item);
    strip_redundant_edit_output(item);
    unreported_terminal && lost_output(item)
}

fn find_tool_call<'a>(items: &'a mut [ChatItem], id: &str) -> Option<&'a mut ToolCallItem> {
    items.iter_mut().rev().find_map(|item| match item {
        ChatItem::ToolCall(tc) if tc.id == id => Some(tc),
        _ => None,
    })
}

/// What one pass over a tool call's `content` yielded.
struct SplitContent {
    diffs: Vec<DiffView>,
    output: Vec<ToolOutputBlock>,
    /// An embedded terminal block was present. It renders nothing by itself, so
    /// the caller has to recover the bytes from another channel.
    saw_terminal: bool,
}

/// Partition tool-call content into diffs and typed output blocks.
///
/// An embedded terminal block carries no content of its own — it is a handle to
/// a terminal this client never created (daruda implements no `terminal/*`
/// method) — so it only sets [`SplitContent::saw_terminal`]; recovering its
/// bytes is [`fold_output`]'s job, since they arrive on a different
/// notification than the handle. Any future content kind [`output_block_of`]
/// doesn't recognize is dropped silently.
fn split_content(content: &[ToolCallContent]) -> SplitContent {
    let mut diffs = Vec::new();
    let mut output = Vec::new();
    let mut saw_terminal = false;
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
            ToolCallContent::Terminal(_) => saw_terminal = true,
            // Any future content kind is not rendered.
            _ => {}
        }
    }
    SplitContent {
        diffs,
        output,
        saw_terminal,
    }
}

/// Fold one event's output channels into a tool call's body, in priority order:
/// typed `content` blocks, then the adapter's `_meta` output sideband, then
/// `raw_output`. Only the first channel that carries something applies, and a
/// channel that carries nothing leaves the body as it stands.
///
/// The "leaves it as it stands" half is what makes claude-agent-acp's shell
/// lifecycle work: the captured bytes and the completion (`terminal_exit` +
/// an unfenced `rawOutput`) arrive as two separate `tool_call_update`s, so
/// overwriting from the second would both blank the recovered [`ToolOutputBlock::RawText`]
/// and push raw shell bytes through the markdown renderer.
/// The part of a `content` field [`fold_output`] folds: what it rendered to,
/// and whether a terminal handle was among it.
struct ContentBody {
    blocks: Vec<ToolOutputBlock>,
    terminal_handle: bool,
}

fn fold_output(
    output: &mut Vec<ToolOutputBlock>,
    content: Option<ContentBody>,
    sideband: Option<String>,
    raw_output: &Option<serde_json::Value>,
) {
    // `content` is replace semantics (the schema calls the field "Replace the
    // content collection"), so a present-but-unrenderable one — an empty array,
    // or diffs only — must clear a body an earlier update filled. The single
    // exception is a bare terminal handle: its bytes ride a content-less
    // notification of their own, so replacing on it would blank them.
    if let Some(body) = content
        && !(body.blocks.is_empty() && body.terminal_handle)
    {
        *output = body.blocks;
    }
    if output.is_empty()
        && let Some(blocks) = sideband
            .map(bounded_raw_text_blocks)
            .filter(|blocks| !blocks.is_empty())
    {
        *output = blocks;
    }
    push_raw_output_fallback(output, raw_output);
}

/// Max bytes of tool-output text carried into the render model. Larger text is
/// truncated at a char boundary so expanding a tool card can't feed megabytes
/// to the markdown renderer (the expand-freeze bug). Not user-tunable yet.
const MAX_TOOL_OUTPUT_TEXT_BYTES: usize = 64 * 1024;

/// Max bytes of a tool-output image's base64 `data` carried into the render
/// model as an `Image` block. Same trust-boundary rationale as
/// [`MAX_TOOL_OUTPUT_TEXT_BYTES`]: nothing in the protocol bounds what an
/// adapter sends, and unlike text, an oversized image can't be truncated in
/// place (a truncated base64 string is either invalid or decodes to a corrupt
/// image) — so a payload over this cap is remapped to a `Media` descriptor
/// (via [`media_block`]) instead, which is cheap to carry, rather than an
/// `Image` block, which is retained, cloned, decoded, and re-hashed every
/// frame. 8 MiB of base64 (~6 MB decoded) is generous for a normal
/// screenshot/PNG; only pathological payloads fall back.
const MAX_TOOL_OUTPUT_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// Cap `s` at a UTF-8 char boundary when it exceeds
/// `MAX_TOOL_OUTPUT_TEXT_BYTES`, returning the kept text plus the original byte
/// length when it was cut (so the renderer can show a marker). Shared by both
/// text block kinds so one cap governs everything the model carries.
fn bounded(s: String) -> (String, Option<usize>) {
    let cap = MAX_TOOL_OUTPUT_TEXT_BYTES;
    if s.len() <= cap {
        return (s, None);
    }
    let original_len = s.len();
    let boundary = s.floor_char_boundary(cap);
    let mut text = s;
    text.truncate(boundary);
    (text, Some(original_len))
}

/// Build a bounded markdown `Text` block.
fn bounded_text(s: String) -> ToolOutputBlock {
    let (text, truncated_from) = bounded(s);
    ToolOutputBlock::Text {
        text,
        truncated_from,
    }
}

/// Build a bounded `RawText` block — verbatim shell output, never markdown.
fn bounded_raw_text(s: String) -> ToolOutputBlock {
    let (text, truncated_from) = bounded(s);
    ToolOutputBlock::RawText {
        text,
        truncated_from,
    }
}

/// A bounded `RawText` block wrapped in a `Vec`, or an empty `Vec` when the
/// command printed nothing, so a silent command adds no empty block.
fn bounded_raw_text_blocks(text: String) -> Vec<ToolOutputBlock> {
    if text.trim().is_empty() {
        Vec::new()
    } else {
        vec![bounded_raw_text(text)]
    }
}

/// Map a content block to a renderable output block. `None` for empty text and
/// for any future content kind the protocol adds that this build doesn't know
/// about (`ContentBlock` is `#[non_exhaustive]`).
fn output_block_of(block: &ContentBlock) -> Option<ToolOutputBlock> {
    match block {
        ContentBlock::Text(t) if !t.text.is_empty() => Some(bounded_text(t.text.clone())),
        ContentBlock::Image(img) => Some(if img.data.len() > MAX_TOOL_OUTPUT_IMAGE_BYTES {
            media_block(img.mime_type.clone(), &img.data)
        } else {
            ToolOutputBlock::Image {
                data: img.data.clone(),
                mime: img.mime_type.clone(),
            }
        }),
        ContentBlock::Audio(a) => Some(media_block(a.mime_type.clone(), &a.data)),
        ContentBlock::ResourceLink(rl) => Some(ToolOutputBlock::ResourceLink {
            uri: rl.uri.clone(),
            // Prefer the human title; the `name` field is always present.
            name: rl.title.clone().unwrap_or_else(|| rl.name.clone()),
        }),
        ContentBlock::Resource(er) => match &er.resource {
            EmbeddedResourceResource::TextResourceContents(t) if !t.text.trim().is_empty() => {
                Some(bounded_text(t.text.clone()))
            }
            EmbeddedResourceResource::TextResourceContents(_) => None,
            EmbeddedResourceResource::BlobResourceContents(b) => Some(media_block(
                b.mime_type.clone().unwrap_or_default(),
                &b.blob,
            )),
            // `EmbeddedResourceResource` is `#[non_exhaustive]`.
            #[allow(unreachable_patterns)]
            _ => None,
        },
        _ => None,
    }
}

/// Estimated decoded byte size of a base64 string (ignoring padding/whitespace),
/// good enough for a human-readable descriptor. `daruda_acp` carries no base64
/// dependency, so this is arithmetic only — the real decode (and image
/// rasterization) happens at the app render boundary.
fn est_decoded_len(b64: &str) -> usize {
    b64.len() / 4 * 3
}

/// Build a `Media` descriptor block — the shared shape for every non-rendered
/// binary payload (audio, embedded blob, and an image over
/// [`MAX_TOOL_OUTPUT_IMAGE_BYTES`]), so every call site computes `byte_len` the
/// same way and orders the fields the same way.
fn media_block(mime: String, data: &str) -> ToolOutputBlock {
    ToolOutputBlock::Media {
        mime,
        byte_len: est_decoded_len(data),
    }
}

/// Append fallback output blocks derived from a tool call's `raw_output`, but
/// only when no higher-priority channel produced renderable output (see
/// [`fold_output`]). Adapters that report results through an embedded terminal
/// with no output sideband, or *only* in `raw_output` — codex-acp streams a
/// shell command's output through a terminal and repeats it in `raw_output`, and
/// its MCP calls carry results solely there — would otherwise render an empty
/// card. Claude-style adapters embed the same content as `content` blocks, so
/// their `output` is already non-empty and this is a no-op (no duplication).
fn push_raw_output_fallback(
    output: &mut Vec<ToolOutputBlock>,
    raw_output: &Option<serde_json::Value>,
) {
    if !output.is_empty() {
        return;
    }
    if let Some(raw) = raw_output {
        output.extend(raw_output_blocks(raw));
    }
}

/// Render a `raw_output` value as output blocks. codex-acp's command execution
/// stores the human-facing output as a `formatted_output` string, surfaced as
/// [`ToolOutputBlock::RawText`] (highest priority — codex-acp relies on it):
/// those are shell bytes, not markdown, so they must render verbatim. Some
/// adapters use the same shape but call the field `output`; treat that as the
/// same human-facing stream rather than showing the JSON wrapper. A bare string
/// is also raw output. A JSON **array** is Anthropic's raw content-block shape
/// (e.g. `[{"type":"image",...}, {"type":"text",...}]`) — each element is
/// parsed via [`raw_content_block`], in order, and recognized elements (image /
/// audio / text / embedded resource) are kept while unrecognized ones are
/// skipped; if *none* of the array's elements are recognized the whole array
/// falls back to pretty JSON below (so a genuinely unrelated array isn't
/// silently dropped). A bare JSON object that is itself one recognized content
/// block parses to that one block. Anything else (a structured object such as an
/// MCP `{ result, error }`, or an array/object with nothing recognizable)
/// renders as pretty JSON, still as raw text so the app shows it in the bounded
/// editor embed instead of the markdown path. Returns an empty `Vec` when there
/// is no visible text.
fn raw_output_blocks(raw: &serde_json::Value) -> Vec<ToolOutputBlock> {
    if let Some(s) = raw
        .get("formatted_output")
        .and_then(serde_json::Value::as_str)
    {
        // codex's command-execution shape (`{ formatted_output, exit_code }` —
        // the same key `DefaultAdapter::command_exit` requires before it badges
        // an exit), so these are shell bytes: a `#` or `---` the command printed
        // is literal, not a heading or a horizontal rule.
        return bounded_raw_text_blocks(s.to_string());
    }
    if let Some(s) = raw.get("output").and_then(serde_json::Value::as_str) {
        return bounded_raw_text_blocks(s.to_string());
    }
    match raw {
        serde_json::Value::String(s) => return bounded_raw_text_blocks(s.clone()),
        serde_json::Value::Array(elements) => {
            let recognized: Vec<ToolOutputBlock> =
                elements.iter().filter_map(raw_content_block).collect();
            if !recognized.is_empty() {
                return recognized;
            }
            // No element recognized — fall through to the pretty-JSON fallback
            // below rather than silently dropping the whole array.
        }
        _ => {
            if let Some(block) = raw_content_block(raw) {
                return vec![block];
            }
        }
    }
    match serde_json::to_string_pretty(raw) {
        Ok(text) => bounded_raw_text_blocks(text),
        Err(_) => Vec::new(),
    }
}

/// Parse one raw JSON value as a single content block — either one element of
/// a `rawOutput` array, or a bare `rawOutput` object that is itself a content
/// block. Recognizes Anthropic's raw shape: `type` discriminates
/// `"text"` / `"image"` / `"audio"` / `"resource"`, with image/audio payload
/// nested under `source.data` / `source.media_type` (Anthropic's own shape) or
/// falling back to a flat `data` / `media_type` / `mime_type` (defensive, in
/// case another adapter sends the flatter ACP-like shape instead). Returns
/// `None` for a value with no recognized `type`, or a recognized `type` missing
/// its required payload field.
fn raw_content_block(el: &serde_json::Value) -> Option<ToolOutputBlock> {
    let ty = el.get("type").and_then(serde_json::Value::as_str)?;
    match ty {
        "text" => {
            let text = el.get("text").and_then(serde_json::Value::as_str)?;
            (!text.trim().is_empty()).then(|| bounded_text(text.to_string()))
        }
        "image" => {
            let data = raw_media_data(el)?;
            let mime = raw_media_mime(el);
            Some(if data.len() > MAX_TOOL_OUTPUT_IMAGE_BYTES {
                media_block(mime, &data)
            } else {
                ToolOutputBlock::Image { data, mime }
            })
        }
        "audio" => {
            let data = raw_media_data(el)?;
            Some(media_block(raw_media_mime(el), &data))
        }
        "resource" => {
            let resource = el.get("resource")?;
            if let Some(blob) = resource.get("blob").and_then(serde_json::Value::as_str) {
                let mime = resource
                    .get("mime_type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Some(media_block(mime, blob))
            } else {
                let text = resource.get("text").and_then(serde_json::Value::as_str)?;
                (!text.trim().is_empty()).then(|| bounded_text(text.to_string()))
            }
        }
        _ => None,
    }
}

/// The base64 payload of a raw image/audio content-block element: Anthropic's
/// nested `source.data`, falling back to a flat `data`.
fn raw_media_data(el: &serde_json::Value) -> Option<String> {
    el.get("source")
        .and_then(|s| s.get("data"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| el.get("data").and_then(serde_json::Value::as_str))
        .map(str::to_string)
}

/// The MIME type of a raw image/audio content-block element: Anthropic's
/// nested `source.media_type`, falling back to a flat `media_type` or
/// `mime_type`. Empty string when none is present — the source omitted it.
fn raw_media_mime(el: &serde_json::Value) -> String {
    el.get("source")
        .and_then(|s| s.get("media_type"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| el.get("media_type").and_then(serde_json::Value::as_str))
        .or_else(|| el.get("mime_type").and_then(serde_json::Value::as_str))
        .unwrap_or_default()
        .to_string()
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

/// Public because a consumer may fold `session/update`s without building chat
/// items — the flow engine records what tools a node used and keeps no payload.
/// What a protocol value means stays this module's answer either way.
pub fn status_of(status: &ToolCallStatus) -> ToolStatusView {
    match status {
        ToolCallStatus::Pending => ToolStatusView::Pending,
        ToolCallStatus::InProgress => ToolStatusView::InProgress,
        ToolCallStatus::Completed => ToolStatusView::Completed,
        ToolCallStatus::Failed => ToolStatusView::Failed,
        _ => ToolStatusView::Pending,
    }
}

/// Public for the same reason as [`status_of`].
pub fn kind_of(kind: &ToolKind) -> ToolKindView {
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
    use crate::model::CommandExit;
    use agent_client_protocol::schema::v1::{
        Content, ContentChunk, Diff, ImageContent, ResourceLink, Terminal, TextContent,
        ToolCallUpdateFields,
    };

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
        let (diffs, output) = split(&content);
        assert_eq!(diffs.len(), 1);
        assert_eq!(
            output,
            vec![
                ToolOutputBlock::Text {
                    text: "hello".to_string(),
                    truncated_from: None,
                },
                ToolOutputBlock::ResourceLink {
                    uri: "file:///tmp/file.rs".to_string(),
                    name: "file.rs".to_string(),
                },
            ]
        );
    }

    #[test]
    fn apply_update_with_routes_parent_id_through_adapter() {
        // The mapper must consult the injected strategy for the parent id, not a
        // hardcoded meta read — proven by a stub that returns a fixed sentinel
        // regardless of the (empty) meta.
        struct StubAdapter;
        impl AcpAdapter for StubAdapter {
            fn message_phase(
                &self,
                _meta: &Option<agent_client_protocol::schema::v1::Meta>,
            ) -> crate::adapter::MessagePhase {
                crate::adapter::MessagePhase::Answer
            }

            fn parent_tool_id(
                &self,
                _meta: &Option<agent_client_protocol::schema::v1::Meta>,
            ) -> Option<String> {
                Some("stub-parent".to_owned())
            }

            fn tool_name(
                &self,
                _meta: &Option<agent_client_protocol::schema::v1::Meta>,
            ) -> Option<String> {
                Some("stub-tool".to_owned())
            }

            fn command_exit(
                &self,
                _raw_output: &Option<serde_json::Value>,
                _meta: &Option<agent_client_protocol::schema::v1::Meta>,
            ) -> Option<crate::model::CommandExit> {
                None
            }

            fn sideband_output(
                &self,
                _meta: &Option<agent_client_protocol::schema::v1::Meta>,
            ) -> Option<String> {
                None
            }
        }
        let mut items = Vec::new();
        apply_update_with(
            &mut items,
            &SessionUpdate::ToolCall(ToolCall::new("c1", "x")),
            &StubAdapter,
        );
        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call");
        };
        assert_eq!(tc.parent_tool_id, Some("stub-parent".to_owned()));
        // The tool name must likewise come from the injected strategy, not a
        // hardcoded meta read.
        assert_eq!(tc.tool_name, Some("stub-tool".to_owned()));
    }

    #[test]
    fn codex_command_output_surfaces_from_raw_output() {
        // codex-acp reports a shell command's output only in `raw_output`
        // (`{ formatted_output, exit_code }`); its `content` is an embedded
        // terminal block we drop. The output must still fill the card body.
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("c1", "ls -la")
                    .kind(ToolKind::Execute)
                    .content(vec![ToolCallContent::Terminal(Terminal::new("term-1"))]),
            ),
        );
        // The in-progress insert carries no renderable text yet.
        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call");
        };
        assert!(tc.output.is_empty());

        apply_update(
            &mut items,
            &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "c1",
                ToolCallUpdateFields::new()
                    .status(ToolCallStatus::Completed)
                    .raw_output(serde_json::json!({
                        "formatted_output": "total 0\ndrwxr-xr-x  2 me  staff",
                        "exit_code": 0,
                    })),
            )),
        );
        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call");
        };
        // Regression anchor: exit-status recovery must not change output-block
        // recovery, which already worked before this field existed.
        assert_eq!(
            tc.output,
            vec![ToolOutputBlock::RawText {
                text: "total 0\ndrwxr-xr-x  2 me  staff".to_string(),
                truncated_from: None,
            }]
        );
        assert_eq!(
            tc.exit,
            Some(CommandExit {
                code: Some(0),
                signal: None
            })
        );
    }

    #[test]
    fn successful_edit_drops_raw_output_confirmation() {
        // Mirrors a captured claude-agent-acp wire session: the initial insert
        // carries only the diff, and completion carries no `content` at all —
        // just a self-referential `rawOutput` confirmation string with nothing
        // the diff and the "Done" badge don't already say.
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("c1", "Edit /tmp/x.txt")
                    .kind(ToolKind::Edit)
                    .content(vec![ToolCallContent::Diff(Diff::new("/tmp/x.txt", "hi\n"))]),
            ),
        );
        apply_update(
            &mut items,
            &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "c1",
                ToolCallUpdateFields::new()
                    .status(ToolCallStatus::Completed)
                    .raw_output(serde_json::json!(
                        "The file /tmp/x.txt has been updated successfully. \
                         (file state is current in your context — no need to Read it back)"
                    )),
            )),
        );
        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call");
        };
        assert!(tc.output.is_empty());
        assert_eq!(tc.diffs.len(), 1);
    }

    #[test]
    fn failed_edit_keeps_its_error_output() {
        // A rejected/failed edit reports the real reason via `content`, which
        // has no diff-shaped substitute — must survive the success-only strip.
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("c1", "Edit /tmp/x.txt")
                    .kind(ToolKind::Edit)
                    .content(vec![ToolCallContent::Diff(Diff::new("/tmp/x.txt", "hi\n"))]),
            ),
        );
        apply_update(
            &mut items,
            &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "c1",
                ToolCallUpdateFields::new()
                    .status(ToolCallStatus::Failed)
                    .content(vec![ToolCallContent::Content(Content::new(
                        ContentBlock::Text(TextContent::new("The user rejected this edit.")),
                    ))]),
            )),
        );
        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call");
        };
        assert_eq!(
            tc.output,
            vec![ToolOutputBlock::Text {
                text: "The user rejected this edit.".to_string(),
                truncated_from: None,
            }]
        );
    }

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

    #[test]
    fn terminal_sideband_output_fills_the_card() {
        // claude-agent-acp, once `_meta.terminal_output` is advertised, replaces
        // a Bash result's content with a bare terminal handle and ships the
        // captured bytes on `_meta`. They must land as raw (unfenced) text.
        let mut items = Vec::new();
        let effect = apply_update(
            &mut items,
            &terminal_result(Some(serde_json::json!({
                "terminal_output": { "terminal_id": "c1", "data": "total 0\ndrwxr-xr-x  2 me" }
            }))),
        );
        assert_eq!(
            only_tool_call(&items).output,
            vec![ToolOutputBlock::RawText {
                text: "total 0\ndrwxr-xr-x  2 me".to_string(),
                truncated_from: None,
            }]
        );
        assert!(
            !effect.dropped_terminal_output,
            "nothing was dropped — the sideband carried the output"
        );
    }

    #[test]
    fn terminal_block_without_a_sideband_is_dropped_and_flagged() {
        let mut items = Vec::new();
        let effect = apply_update(&mut items, &terminal_result(None));
        assert!(
            only_tool_call(&items).output.is_empty(),
            "with no channel to read, the terminal block renders nothing"
        );
        assert!(
            effect.dropped_terminal_output,
            "the host must be told the card was left empty"
        );
    }

    #[test]
    fn a_reshaped_sideband_payload_is_dropped_and_flagged_not_panicked() {
        // The `_meta` shape is derived from adapter source, not a stable
        // contract: a renamed inner key degrades to the pre-existing drop.
        let mut items = Vec::new();
        let effect = apply_update(
            &mut items,
            &terminal_result(Some(
                serde_json::json!({ "terminal_output": { "output": "renamed" } }),
            )),
        );
        assert!(only_tool_call(&items).output.is_empty());
        assert!(effect.dropped_terminal_output);
    }

    #[test]
    fn a_live_terminal_block_is_not_flagged_as_dropped() {
        // Every adapter's *first* Bash event is a content-less handle with no
        // output yet. Flagging that would warn on every healthy command.
        let mut items = Vec::new();
        let effect = apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("c1", "ls -la")
                    .kind(ToolKind::Execute)
                    .content(vec![ToolCallContent::Terminal(Terminal::new("c1"))]),
            ),
        );
        assert!(only_tool_call(&items).output.is_empty());
        assert!(
            !effect.dropped_terminal_output,
            "a still-running call hasn't lost anything yet"
        );
    }

    #[test]
    fn terminal_sideband_output_arrives_on_a_tool_call_update() {
        // The real sequence: content-less handle on the insert, bytes on the
        // completion update's `_meta`.
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(ToolCall::new("c1", "ls").kind(ToolKind::Execute)),
        );
        let effect = apply_update(
            &mut items,
            &SessionUpdate::ToolCallUpdate(
                ToolCallUpdate::new(
                    "c1",
                    ToolCallUpdateFields::new()
                        .status(ToolCallStatus::Completed)
                        .content(vec![ToolCallContent::Terminal(Terminal::new("c1"))]),
                )
                .meta(meta_map(serde_json::json!({
                    "terminal_output": { "data": "hello" },
                    "terminal_exit": { "exit_code": 0 },
                }))),
            ),
        );
        let tc = only_tool_call(&items);
        assert_eq!(
            tc.output,
            vec![ToolOutputBlock::RawText {
                text: "hello".to_string(),
                truncated_from: None,
            }]
        );
        assert_eq!(
            tc.exit,
            Some(CommandExit {
                code: Some(0),
                signal: None
            }),
            "the exit sideband keeps working alongside the output one"
        );
        assert!(!effect.dropped_terminal_output);
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

    #[test]
    fn the_captured_bash_turn_recovers_its_output_and_exit() {
        let mut items = Vec::new();
        let effects: Vec<UpdateEffect> = CAPTURED_BASH_TURN
            .iter()
            .map(|line| {
                let update: SessionUpdate =
                    serde_json::from_str(line).expect("captured notification deserializes");
                apply_update(&mut items, &update)
            })
            .collect();

        let tc = only_tool_call(&items);
        // `---` is in the payload on purpose: routed through the markdown path it
        // would become a horizontal rule, so this asserts the bytes stay verbatim.
        assert_eq!(
            tc.output,
            vec![ToolOutputBlock::RawText {
                text: "total 24\n---\ndrwxr-xr-x@  5 woo  staff   160 Jul 31 14:17 app".to_string(),
                truncated_from: None,
            }],
            "the content-less third notification carries the bytes, and the \
             completion behind it must neither blank them nor refill from rawOutput"
        );
        assert_eq!(
            tc.exit,
            Some(CommandExit {
                code: Some(0),
                signal: None
            })
        );
        assert!(
            effects.iter().all(|e| !e.dropped_terminal_output),
            "a healthy command must not burn the once-per-pane warning"
        );
    }

    #[test]
    fn a_present_but_empty_content_clears_a_body_an_earlier_update_filled() {
        // `content` is replace semantics (schema: "Replace the content
        // collection"), so an update that explicitly sends nothing renderable
        // must not leave stale output on screen. Only a bare terminal handle is
        // exempt — its bytes ride their own notification.
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(ToolCall::new("c1", "run").kind(ToolKind::Execute).content(
                vec![ToolCallContent::Content(Content::new(ContentBlock::Text(
                    TextContent::new("stale"),
                )))],
            )),
        );
        assert!(!only_tool_call(&items).output.is_empty());

        apply_update(
            &mut items,
            &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "c1",
                ToolCallUpdateFields::new().content(Vec::new()),
            )),
        );
        assert!(
            only_tool_call(&items).output.is_empty(),
            "an explicit empty content replaces, it does not preserve"
        );
    }

    #[test]
    fn a_codex_shell_failure_badges_its_exit_whatever_kind_it_was_labelled() {
        // Wire-captured (`acp-wire-codex-acp.log`): codex labels a failing `ls`
        // as `Read` — it classifies by intent, not by mechanism — and the
        // completion update carries no `kind` at all. Gating the exit on
        // `Execute` therefore blanks the badge for every codex command that is
        // not literally a shell invocation.
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(ToolCall::new("c1", "List files").kind(ToolKind::Read)),
        );
        apply_update(
            &mut items,
            &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "c1",
                ToolCallUpdateFields::new()
                    .status(ToolCallStatus::Failed)
                    .raw_output(serde_json::json!({
                        "formatted_output": "ls: /nope: No such file or directory\n",
                        "exit_code": 1,
                    })),
            )),
        );
        assert_eq!(
            only_tool_call(&items).exit,
            Some(CommandExit {
                code: Some(1),
                signal: None
            }),
            "a shell result is identified by its channel shape, not the tool kind"
        );
    }

    #[test]
    fn a_stray_exit_code_without_command_output_is_not_a_command_exit() {
        // `raw_output` also carries MCP and other free-form results. Only the
        // `{ formatted_output, exit_code }` pair is codex's command shape, so an
        // `exit_code` on its own must not badge a tool that never ran a shell.
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("c1", "fetch")
                    .kind(ToolKind::Fetch)
                    .raw_output(serde_json::json!({ "exit_code": 1, "result": "ok" })),
            ),
        );
        assert_eq!(only_tool_call(&items).exit, None);
    }

    #[test]
    fn a_later_empty_terminal_exit_does_not_erase_a_recorded_exit() {
        // `dist/tools.js` builds `terminal_exit` from `bashResult.return_code`;
        // when that is absent the key is dropped and only `signal: null` ships.
        // The renderer reads `{None, None}` as "no exit", so letting it through
        // would silently drop an `Exit 1` badge recorded a moment earlier.
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(ToolCall::new("c1", "false").kind(ToolKind::Execute)),
        );
        apply_update(
            &mut items,
            &SessionUpdate::ToolCallUpdate(
                ToolCallUpdate::new("c1", ToolCallUpdateFields::new()).meta(meta_map(
                    serde_json::json!({ "terminal_exit": { "exit_code": 1, "signal": null } }),
                )),
            ),
        );
        apply_update(
            &mut items,
            &SessionUpdate::ToolCallUpdate(
                ToolCallUpdate::new(
                    "c1",
                    ToolCallUpdateFields::new().status(ToolCallStatus::Failed),
                )
                .meta(meta_map(serde_json::json!({
                    "terminal_exit": { "terminal_id": "c1", "signal": null }
                }))),
            ),
        );
        assert_eq!(
            only_tool_call(&items).exit,
            Some(CommandExit {
                code: Some(1),
                signal: None
            }),
            "an exit report with no usable field must not overwrite the recorded one"
        );
    }

    #[test]
    fn oversized_sideband_output_is_capped_and_records_its_original_length() {
        let original_len = MAX_TOOL_OUTPUT_TEXT_BYTES + 4096;
        let huge = "x".repeat(original_len);
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &terminal_result(Some(
                serde_json::json!({ "terminal_output": { "data": huge } }),
            )),
        );
        let [
            ToolOutputBlock::RawText {
                text,
                truncated_from,
            },
        ] = only_tool_call(&items).output.as_slice()
        else {
            panic!("expected one raw text block");
        };
        assert_eq!(text.len(), MAX_TOOL_OUTPUT_TEXT_BYTES);
        assert_eq!(
            *truncated_from,
            Some(original_len),
            "the sideband reuses the shared cap and its truncation marker"
        );
    }

    #[test]
    fn a_silent_command_adds_no_empty_block_and_is_not_flagged() {
        let mut items = Vec::new();
        let effect = apply_update(
            &mut items,
            &terminal_result(Some(
                serde_json::json!({ "terminal_output": { "data": "  \n" } }),
            )),
        );
        assert!(only_tool_call(&items).output.is_empty());
        assert!(
            !effect.dropped_terminal_output,
            "the sideband reported an empty command — nothing was lost"
        );
    }

    #[test]
    fn image_output_keeps_the_content_path_and_ignores_the_sideband() {
        // The adapter routes image results through normal content blocks rather
        // than the terminal handle. The sideband must not touch that path even
        // when the update happens to carry a `terminal_output` meta.
        let mut items = Vec::new();
        let effect = apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("c1", "screenshot")
                    .kind(ToolKind::Execute)
                    .status(ToolCallStatus::Completed)
                    .content(vec![ToolCallContent::Content(Content::new(
                        ContentBlock::Image(ImageContent::new("BASE64", "image/png")),
                    ))])
                    .meta(meta_map(
                        serde_json::json!({ "terminal_output": { "data": "leaked" } }),
                    )),
            ),
        );
        assert_eq!(
            only_tool_call(&items).output,
            vec![ToolOutputBlock::Image {
                data: "BASE64".to_string(),
                mime: "image/png".to_string(),
            }]
        );
        assert!(!effect.dropped_terminal_output);
    }

    #[test]
    fn codex_terminal_then_raw_output_is_never_flagged_as_dropped() {
        // Regression anchor: codex's shell path (terminal handle on the insert,
        // output in `raw_output` on the completion) is unchanged by the
        // sideband, and must not produce a spurious drop warning either.
        let mut items = Vec::new();
        let insert = apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("c1", "ls -la")
                    .kind(ToolKind::Execute)
                    .content(vec![ToolCallContent::Terminal(Terminal::new("term-1"))]),
            ),
        );
        let complete = apply_update(
            &mut items,
            &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "c1",
                ToolCallUpdateFields::new()
                    .status(ToolCallStatus::Completed)
                    .content(vec![ToolCallContent::Terminal(Terminal::new("term-1"))])
                    .raw_output(serde_json::json!({
                        "formatted_output": "total 0",
                        "exit_code": 0,
                    })),
            )),
        );
        assert_eq!(
            only_tool_call(&items).output,
            vec![ToolOutputBlock::RawText {
                text: "total 0".to_string(),
                truncated_from: None,
            }],
            "codex still recovers its output through raw_output, verbatim"
        );
        assert!(!insert.dropped_terminal_output);
        assert!(
            !complete.dropped_terminal_output,
            "raw_output refilled the card, so nothing was lost"
        );
    }

    /// Build a `Meta` map from a JSON object literal — the schema type is
    /// `serde_json::Map<String, Value>`, not `Value`, so builders taking `Meta`
    /// need the unwrapped map.
    fn meta_map(v: serde_json::Value) -> agent_client_protocol::schema::v1::Meta {
        v.as_object()
            .expect("test fixture must be a JSON object")
            .clone()
    }

    #[test]
    fn command_exit_reads_codex_exit_code_through_the_full_pipeline() {
        // End-to-end through apply_update (DefaultAdapter), not just the
        // adapter unit test — proves the mapping wiring, not only the parse.
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("c1", "run tests")
                    .kind(ToolKind::Execute)
                    .raw_output(serde_json::json!({ "formatted_output": "FAIL", "exit_code": 1 })),
            ),
        );
        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call");
        };
        assert_eq!(
            tc.exit,
            Some(CommandExit {
                code: Some(1),
                signal: None
            })
        );
    }

    #[test]
    fn command_exit_reads_claude_terminal_exit_meta_through_the_full_pipeline() {
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("c1", "run tests")
                    .kind(ToolKind::Execute)
                    .meta(meta_map(
                        serde_json::json!({ "terminal_exit": { "exit_code": 2, "signal": null } }),
                    )),
            ),
        );
        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call");
        };
        assert_eq!(
            tc.exit,
            Some(CommandExit {
                code: Some(2),
                signal: None
            })
        );
    }

    #[test]
    fn command_exit_signal_only_has_no_code() {
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("c1", "run tests")
                    .kind(ToolKind::Execute)
                    .meta(meta_map(
                        serde_json::json!({ "terminal_exit": { "signal": "SIGTERM" } }),
                    )),
            ),
        );
        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call");
        };
        assert_eq!(
            tc.exit,
            Some(CommandExit {
                code: None,
                signal: Some("SIGTERM".to_string())
            })
        );
    }

    #[test]
    fn command_exit_is_none_when_neither_channel_reports() {
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(ToolCall::new("c1", "run tests").kind(ToolKind::Execute)),
        );
        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call");
        };
        assert_eq!(tc.exit, None);
    }

    #[test]
    fn command_exit_is_none_for_a_non_execute_tool() {
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("t1", "Read src/main.rs")
                    .kind(ToolKind::Read)
                    .raw_input(serde_json::json!({"file_path": "src/main.rs"})),
            ),
        );
        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call");
        };
        assert_eq!(tc.exit, None);
    }

    #[test]
    fn command_exit_arrives_on_a_tool_call_update_without_blanking_on_a_later_status_only_update() {
        // The completion update carries the exit status; a later status-only
        // update (no raw_output, no meta) must not wipe it back to None.
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(ToolCall::new("c1", "ls").kind(ToolKind::Execute)),
        );
        apply_update(
            &mut items,
            &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "c1",
                ToolCallUpdateFields::new()
                    .status(ToolCallStatus::Completed)
                    .raw_output(serde_json::json!({ "formatted_output": "ok\n", "exit_code": 0 })),
            )),
        );
        apply_update(
            &mut items,
            &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                "c1",
                ToolCallUpdateFields::new().title("ls (renamed)"),
            )),
        );
        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call");
        };
        assert_eq!(
            tc.exit,
            Some(CommandExit {
                code: Some(0),
                signal: None
            }),
            "a later update with no exit channel must not clear the recorded exit"
        );
    }

    #[test]
    fn raw_output_does_not_duplicate_text_content() {
        // Claude-style adapters embed the result as a `content` text block *and*
        // repeat it in `raw_output`. The text block wins — no duplicate block.
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("c1", "read")
                    .content(vec![ToolCallContent::Content(Content::new(
                        ContentBlock::Text(TextContent::new("real output")),
                    ))])
                    .raw_output(serde_json::json!({ "formatted_output": "dup" })),
            ),
        );
        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call");
        };
        assert_eq!(
            tc.output,
            vec![ToolOutputBlock::Text {
                text: "real output".to_string(),
                truncated_from: None,
            }]
        );
    }

    #[test]
    fn raw_output_object_pretty_printed_as_raw_text_when_no_output_field() {
        // An MCP tool call returns structured `raw_output` (`{ result, error }`)
        // with no content — fall back to pretty-printed raw text so it renders in
        // the bounded output editor rather than the markdown path.
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("m1", "mcp.server.tool")
                    .kind(ToolKind::Other)
                    .raw_output(serde_json::json!({ "result": { "ok": true }, "error": null })),
            ),
        );
        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call");
        };
        let [ToolOutputBlock::RawText { text, .. }] = tc.output.as_slice() else {
            panic!("expected one raw text block, got {:?}", tc.output);
        };
        assert!(text.contains("\"result\""), "pretty JSON, got: {text}");
        assert!(text.contains("\"ok\": true"), "pretty JSON, got: {text}");
    }

    #[test]
    fn raw_output_output_field_is_promoted_to_raw_text() {
        // Some adapters put the user-facing stream under `output` instead of
        // codex's `formatted_output`. The wrapper is transport, not content.
        let printed = "Chunk ID: abc\nOutput:\n# literal heading\n";
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("c1", "Read")
                    .kind(ToolKind::Read)
                    .raw_output(serde_json::json!({ "output": printed })),
            ),
        );
        assert_eq!(
            only_tool_call(&items).output,
            vec![ToolOutputBlock::RawText {
                text: printed.to_string(),
                truncated_from: None,
            }]
        );
    }

    #[test]
    fn formatted_output_keeps_markdown_significant_bytes_verbatim() {
        // A command may print `#`, `---` or `**` — as shell bytes those are
        // literal, so the block must be `RawText` carrying the input unchanged.
        let printed = "# heading\n---\n**bold**\n";
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("c1", "cat notes.md")
                    .kind(ToolKind::Execute)
                    .raw_output(serde_json::json!({
                        "formatted_output": printed,
                        "exit_code": 0,
                    })),
            ),
        );
        assert_eq!(
            only_tool_call(&items).output,
            vec![ToolOutputBlock::RawText {
                text: printed.to_string(),
                truncated_from: None,
            }]
        );
    }

    #[test]
    fn blank_raw_output_adds_no_block() {
        // A whitespace-only `formatted_output` (a command that printed nothing)
        // must not add an empty output block.
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("c1", "noop")
                    .kind(ToolKind::Execute)
                    .raw_output(serde_json::json!({ "formatted_output": "  \n ", "exit_code": 0 })),
            ),
        );
        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call");
        };
        assert!(tc.output.is_empty());
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
                phase: Default::default(),
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
                phase: Default::default(),
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
                phase: Default::default(),
            }
        );
        assert_eq!(
            items[1],
            ChatItem::AssistantText {
                text: "second".to_string(),
                streaming: true,
                message_id: Some("m2".to_string()),
                phase: Default::default(),
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
            phase: Default::default(),
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
            phase: Default::default(),
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
            phase: Default::default(),
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
            phase: Default::default(),
        }];
        finalize_streaming(&mut items);
        assert_eq!(
            items[0],
            ChatItem::AssistantText {
                text: "done".to_string(),
                streaming: false,
                message_id: None,
                phase: Default::default(),
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
                phase: Default::default(),
            },
            ChatItem::ToolCall(ToolCallItem {
                id: "t1".to_string(),
                title: "Read".to_string(),
                kind: ToolKindView::Read,
                tool_name: None,
                status: ToolStatusView::Completed,
                diffs: Vec::new(),
                output: Vec::new(),
                raw_input: None,
                parent_tool_id: None,
                exit: None,
            }),
            ChatItem::AssistantText {
                text: "done".to_string(),
                streaming: true,
                message_id: Some("m2".to_string()),
                phase: Default::default(),
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
                tool_name: None,
                status,
                diffs: Vec::new(),
                output: Vec::new(),
                raw_input: None,
                parent_tool_id: None,
                exit: None,
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
            tool_name: None,
            status,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: None,
            parent_tool_id: Some(parent.to_string()),
            exit: None,
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

    #[test]
    fn a_fenced_read_becomes_source_text_in_the_extension_s_language() {
        // The adapter delivers the file as a language-less, line-numbered fence.
        assert_eq!(
            read_output_blocks("src/main.rs", "```\n1\tfn main() {}\n2\t// end\n```"),
            vec![ToolOutputBlock::SourceText {
                text: "fn main() {}\n// end".to_string(),
                language: Some("rust".to_string()),
                truncated_from: None,
            }]
        );
    }

    #[test]
    fn an_unfenced_read_becomes_source_text_too() {
        // An adapter that does not markdown-escape: no fence to hang the
        // language off, and the `cat -n` gutter used to survive to the render.
        assert_eq!(
            read_output_blocks("src/main.rs", "   1\tfn main() {}\n   2\tlet x = 1;"),
            vec![ToolOutputBlock::SourceText {
                text: "fn main() {}\nlet x = 1;".to_string(),
                language: Some("rust".to_string()),
                truncated_from: None,
            }]
        );
    }

    #[test]
    fn a_read_of_an_unknown_extension_is_source_text_without_a_language() {
        assert_eq!(
            read_output_blocks("NOTES", "plain contents"),
            vec![ToolOutputBlock::SourceText {
                text: "plain contents".to_string(),
                language: None,
                truncated_from: None,
            }]
        );
    }

    #[test]
    fn a_read_with_only_path_raw_input_stays_markdown() {
        // `path` is too broad: directory/list-style tools naturally use it and
        // may still be classified as `Read`, so only `file_path` proves this is
        // one file's source text.
        assert_eq!(
            read_output_blocks_with_raw_input(
                serde_json::json!({"path": "src"}),
                "```\na.rs\nb.rs\n```",
            ),
            vec![ToolOutputBlock::Text {
                text: "```\na.rs\nb.rs\n```".to_string(),
                truncated_from: None,
            }]
        );
    }

    #[test]
    fn a_shell_command_s_text_output_stays_markdown() {
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::ToolCall(
                ToolCall::new("c1", "ls")
                    .kind(ToolKind::Execute)
                    .raw_input(serde_json::json!({"command": "ls"})),
            ),
        );
        let mut fields = ToolCallUpdateFields::default();
        fields.content = Some(vec![ToolCallContent::Content(Content::new(
            ContentBlock::Text(TextContent::new("```\na.rs\nb.rs\n```")),
        ))]);
        apply_update(
            &mut items,
            &SessionUpdate::ToolCallUpdate(ToolCallUpdate::new("c1", fields)),
        );
        let ChatItem::ToolCall(tc) = &items[0] else {
            panic!("expected a tool call item");
        };
        assert_eq!(
            tc.output,
            vec![ToolOutputBlock::Text {
                text: "```\na.rs\nb.rs\n```".to_string(),
                truncated_from: None,
            }]
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

        let ChatItem::Permission(card) = permission_item(7, &request, &[]) else {
            panic!("expected permission item");
        };
        assert_eq!(card.tool_title.as_deref(), Some("Write /tmp/x"));
        assert_eq!(card.options.len(), 2);
        assert_eq!(card.options[0].kind, PermissionKindView::AllowAlways);
        assert_eq!(card.options[1].option_id, "reject");
        assert_eq!(card.resolved, None);
    }

    #[test]
    fn permission_item_carries_the_request_id() {
        // The card records the daruda-internal request id so the host can
        // correlate a specific card to its park when several permissions are
        // outstanding at once (parallel tool calls). Without this, the host
        // can only track one at a time and mis-routes the rest.
        let request = RequestPermissionRequest::new(
            "s1",
            ToolCallUpdate::new("t1", ToolCallUpdateFields::default()),
            vec![PermissionOption::new(
                "allow_once",
                "Allow",
                PermissionOptionKind::AllowOnce,
            )],
        );

        let ChatItem::Permission(card) = permission_item(42, &request, &[]) else {
            panic!("expected permission item");
        };
        assert_eq!(card.id, 42);
    }

    #[test]
    fn permission_request_prefers_raw_input_from_a_matching_existing_tool_call() {
        // codex-acp sometimes re-quotes `rawInput.command` for the permission
        // request and mangles embedded quotes, even though a clean copy
        // already arrived via the tool_call insert for the same id. Prefer
        // the item list's already-clean copy over the request's own.
        let items = vec![ChatItem::ToolCall(ToolCallItem {
            id: "t1".to_string(),
            title: "perl -0pi -e ...".to_string(),
            kind: ToolKindView::Execute,
            tool_name: None,
            status: ToolStatusView::InProgress,
            diffs: Vec::new(),
            output: Vec::new(),
            raw_input: Some(serde_json::json!({"command": "perl -0pi -e 's/clean/'"})),
            parent_tool_id: None,
            exit: None,
        })];

        let mut tool_fields = ToolCallUpdateFields::default();
        tool_fields.raw_input =
            Some(serde_json::json!({"command": "perl -0pi -e 's/GARBLED\"'!--/'"}));
        let request = RequestPermissionRequest::new(
            "s1",
            ToolCallUpdate::new("t1", tool_fields),
            vec![PermissionOption::new(
                "allow",
                "Allow",
                PermissionOptionKind::AllowOnce,
            )],
        );

        let ChatItem::Permission(card) = permission_item(1, &request, &items) else {
            panic!("expected permission item");
        };
        assert_eq!(
            card.raw_input_summary.as_deref(),
            Some("command: perl -0pi -e 's/clean/'")
        );
    }

    #[test]
    fn permission_request_falls_back_to_its_own_raw_input_without_a_matching_tool_call() {
        // No matching ToolCall in items (e.g. Claude-style: permission
        // arrives before any tool_call event for that id) — use the
        // request's own raw_input, same as before this change.
        let mut tool_fields = ToolCallUpdateFields::default();
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

        let ChatItem::Permission(card) = permission_item(1, &request, &[]) else {
            panic!("expected permission item");
        };
        assert_eq!(
            card.raw_input_summary.as_deref(),
            Some("command: npm install")
        );
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

        let ChatItem::Permission(card) = permission_item(1, &request, &[]) else {
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
            &SessionUpdate::ToolCall(ToolCall::new("c1", "transcribe").raw_output(
                serde_json::json!([
                    {
                        "type": "audio",
                        "source": { "type": "base64", "data": data, "media_type": "audio/mp3" },
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
}

#[cfg(test)]
mod phase_mapping_tests {
    use super::*;
    use crate::adapter::{CodexAdapter, MessagePhase};
    use agent_client_protocol::schema::v1::{ContentChunk, TextContent};

    fn chunk(text: &str, id: &str, phase: &str) -> ContentChunk {
        let mut c = ContentChunk::new(ContentBlock::Text(TextContent::new(text.to_string())))
            .message_id(id);
        c.meta = Some(
            serde_json::json!({"codex": {"phase": phase}})
                .as_object()
                .unwrap()
                .clone(),
        );
        c
    }

    fn phase_of(item: &ChatItem) -> MessagePhase {
        match item {
            ChatItem::AssistantText { phase, .. } => *phase,
            other => panic!("expected assistant text, got {other:?}"),
        }
    }

    #[test]
    fn a_labelled_message_carries_its_phase_into_the_model() {
        let mut items = Vec::new();
        apply_update_with(
            &mut items,
            &SessionUpdate::AgentMessageChunk(chunk("looking", "msg_1", "commentary")),
            &CodexAdapter,
        );
        apply_update_with(
            &mut items,
            &SessionUpdate::AgentMessageChunk(chunk("done", "msg_2", "final_answer")),
            &CodexAdapter,
        );
        assert_eq!(phase_of(&items[0]), MessagePhase::Commentary);
        assert_eq!(phase_of(&items[1]), MessagePhase::Answer);
    }

    /// A message's role is fixed when it starts. Letting a later chunk restate
    /// it would make "the role changed mid-message" representable, and the only
    /// way to reach it is an adapter contradicting itself.
    #[test]
    fn a_later_chunk_of_the_same_message_cannot_change_its_phase() {
        let mut items = Vec::new();
        apply_update_with(
            &mut items,
            &SessionUpdate::AgentMessageChunk(chunk("look", "msg_1", "commentary")),
            &CodexAdapter,
        );
        apply_update_with(
            &mut items,
            &SessionUpdate::AgentMessageChunk(chunk("ing", "msg_1", "final_answer")),
            &CodexAdapter,
        );
        assert_eq!(items.len(), 1, "same message id, so one item");
        assert_eq!(phase_of(&items[0]), MessagePhase::Commentary);
    }

    /// An agent that labels nothing produces exactly what it did before.
    #[test]
    fn an_unlabelled_message_is_an_answer() {
        let mut items = Vec::new();
        apply_update(
            &mut items,
            &SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new("hi".to_string()),
            ))),
        );
        assert_eq!(phase_of(&items[0]), MessagePhase::Answer);
    }
}

//! Fold ACP protocol traffic into the [`crate::model`] chat item list.
//!
//! Pure functions over `&mut Vec<ChatItem>` so they unit-test without any
//! connection or executor. MVP handles the streaming/tool/permission updates
//! that drive the conversation view; plan, slash-command, mode, config, info,
//! and usage updates are intentionally ignored.

mod content;
mod messages;
mod permissions;
mod tools;

use messages::{StreamKind, append_streaming, append_user_chunk, msg_id, text_of};
pub use permissions::permission_item;
use tools::{apply_tool_call_update, upsert_tool_call};
pub use tools::{kind_of, status_of};

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use agent_client_protocol::schema::v1::{
    ContentBlock, ContentChunk, EmbeddedResourceResource, PermissionOption, PermissionOptionKind,
    RequestPermissionRequest, SessionUpdate, ToolCall, ToolCallContent, ToolCallStatus,
    ToolCallUpdate, ToolKind,
};

use crate::adapter::{AcpAdapter, DefaultAdapter, MessagePhase};
use crate::model::{
    ChatItem, DiffView, PermissionChoice, PermissionItem, PermissionKindView, PlanEntryView,
    PlanStatus, ToolCallItem, ToolKindView, ToolOutputBlock, ToolStatusView,
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

/// Settle every still-running plan entry as [`PlanStatus::Cancelled`].
///
/// The plan is a second live-flagged store beside `items`, and the agent has no
/// terminal signal for it: a run that is stopped (or that ends without a final
/// all-`Completed` `PlanChanged`) leaves its current step `InProgress` forever,
/// which the host renders as a pulsing dot. `Pending` entries are left alone —
/// the run never reached them, which is already a settled fact.
pub fn cancel_pending_plan_entries(plan: &mut [PlanEntryView]) {
    for entry in plan.iter_mut() {
        if entry.status == PlanStatus::InProgress {
            entry.status = PlanStatus::Cancelled;
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

#[cfg(test)]
mod tests;

#[cfg(test)]
mod phase_mapping_tests;

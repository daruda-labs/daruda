//! Tool call insertion, updates, and output classification.

use super::content::{ContentBody, SplitContent, fold_output, split_content};
use super::*;

/// Insert or replace a tool call. Returns whether its terminal content was
/// dropped unrecovered — see [`UpdateEffect::dropped_terminal_output`].
pub(super) fn upsert_tool_call(
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
        locations: tool_call.locations.iter().map(|l| l.path.clone()).collect(),
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
pub(super) fn lost_output(item: &ToolCallItem) -> bool {
    !item.status.is_live() && item.output.is_empty()
}

/// Retype a file read's text output as [`ToolOutputBlock::SourceText`]: the
/// bytes are one file's contents, not markdown, so the adapter's escaping fence
/// and the tool's `cat -n` gutter come off and the language its path implies
/// rides along as a field. No-op for every other tool, and idempotent (a retyped
/// block is no longer `Text`), so it is safe to run after every tool-call insert
/// or update.
pub(super) fn classify_source_output(item: &mut ToolCallItem) {
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
pub(super) fn strip_redundant_edit_output(item: &mut ToolCallItem) {
    if item.kind == ToolKindView::Edit && item.status == ToolStatusView::Completed {
        item.output.clear();
    }
}

/// Fold a `ToolCallUpdate` into the matching tool call. Returns whether its
/// terminal content was dropped unrecovered — see
/// [`UpdateEffect::dropped_terminal_output`].
pub(super) fn apply_tool_call_update(
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

pub(super) fn find_tool_call<'a>(
    items: &'a mut [ChatItem],
    id: &str,
) -> Option<&'a mut ToolCallItem> {
    items.iter_mut().rev().find_map(|item| match item {
        ChatItem::ToolCall(tc) if tc.id == id => Some(tc),
        _ => None,
    })
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

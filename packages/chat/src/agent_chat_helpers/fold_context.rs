//! Fold context resolution: the newest-turn boundary, each fold key's turn
//! position and activity, and the row a fold toggle must remeasure.

use super::{is_active, response_run};
use crate::fold::{FoldContext, FoldKey};
use crate::tool_hierarchy::ToolHierarchy;
use crate::tool_status::{LiveSubagentUnits, tool_or_subtree_live};
use crate::transcript_structure::{TranscriptStructure, run_active};
use daruda_transcript::fold_mode::TurnPosition;

/// Cached start index of the newest turn.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct TurnBoundary(usize);

impl TurnBoundary {
    pub(crate) fn of(items: &[daruda_acp::ChatItem]) -> Self {
        use daruda_acp::ChatItem;
        Self(
            items
                .iter()
                .rposition(|item| matches!(item, ChatItem::UserText(_)))
                .unwrap_or(0),
        )
    }

    pub(crate) fn at(self, ix: usize) -> TurnPosition {
        if ix >= self.0 {
            TurnPosition::Last
        } else {
            TurnPosition::Past
        }
    }
}

/// Resolve a key's activity and turn position from the conversation.
pub(crate) fn fold_context(
    key: &FoldKey,
    items: &[daruda_acp::ChatItem],
    live_units: &LiveSubagentUnits,
) -> FoldContext {
    match fold_key_index(key, items) {
        Some(ix) => fold_context_at(key, ix, items, TurnBoundary::of(items), live_units),
        None => FoldContext::new(TurnPosition::Past, false),
    }
}

/// Resolve fold context when the item index and turn boundary are already known.
pub(crate) fn fold_context_at(
    key: &FoldKey,
    ix: usize,
    items: &[daruda_acp::ChatItem],
    boundary: TurnBoundary,
    live_units: &LiveSubagentUnits,
) -> FoldContext {
    let context = FoldContext::new(boundary.at(ix), fold_active_at(key, ix, items, live_units));
    match (key, items.get(ix)) {
        (FoldKey::Tool(_), Some(daruda_acp::ChatItem::ToolCall(tc))) => {
            context.with_tool_category(daruda_transcript::tool_category::classify_tool(tc))
        }
        _ => context,
    }
}

fn fold_key_index(key: &FoldKey, items: &[daruda_acp::ChatItem]) -> Option<usize> {
    match key {
        FoldKey::Assistant(ix)
        | FoldKey::Compaction(ix)
        | FoldKey::Thinking(ix)
        | FoldKey::ThinkingGroup(ix)
        | FoldKey::Response(ix)
        | FoldKey::Tail(ix)
        | FoldKey::Filtered(ix) => Some(*ix),
        FoldKey::Tool(id)
        | FoldKey::Subagent(id)
        | FoldKey::ToolRawInput(id)
        | FoldKey::ToolGroup(id)
        | FoldKey::ToolGroupTail(id)
        | FoldKey::SubagentTail(id) => tool_item_index(items, id),
        FoldKey::Diff(diff_key) => {
            let tool_id = diff_key.split('#').next().unwrap_or(diff_key.as_str());
            tool_item_index(items, tool_id)
        }
    }
}

#[cfg(test)]
pub(crate) fn fold_turn(key: &FoldKey, items: &[daruda_acp::ChatItem]) -> TurnPosition {
    match fold_key_index(key, items) {
        Some(ix) => TurnBoundary::of(items).at(ix),
        None => TurnPosition::Past,
    }
}

fn tool_item_index(items: &[daruda_acp::ChatItem], tool_id: &str) -> Option<usize> {
    use daruda_acp::ChatItem;
    items
        .iter()
        .position(|item| matches!(item, ChatItem::ToolCall(tc) if tc.id == tool_id))
}

#[cfg(test)]
pub(crate) fn fold_active(key: &FoldKey, items: &[daruda_acp::ChatItem]) -> bool {
    match fold_key_index(key, items) {
        Some(ix) => fold_active_at(key, ix, items, &LiveSubagentUnits::of(items)),
        None => false,
    }
}

/// What a `running` rule reads for `key` — the input to
/// [`BlockRule::WhileRunning`](daruda_transcript::fold_mode::BlockRule::WhileRunning).
fn fold_active_at(
    key: &FoldKey,
    ix: usize,
    items: &[daruda_acp::ChatItem],
    live_units: &LiveSubagentUnits,
) -> bool {
    use daruda_acp::ChatItem;
    match key {
        FoldKey::Assistant(_) | FoldKey::Thinking(_) => {
            items.get(ix).map(is_active).unwrap_or(false)
        }
        // Everything a call owns runs while the call does, a launch's
        // flattened children included — the same reading its badge takes.
        FoldKey::Tool(_) | FoldKey::Subagent(_) | FoldKey::Diff(_) | FoldKey::ToolRawInput(_) => {
            matches!(
                items.get(ix),
                Some(ChatItem::ToolCall(tc)) if tool_or_subtree_live(tc, live_units)
            )
        }
        // Keyed by the response's own first item (`rows::project` passes
        // `run.start`), so the run includes that item. Asked through
        // [`agent_run`] rather than rescanned here: a stop marker closes the
        // run it cut, and a scan that stopped only at the next prompt read the
        // turn after a Stop as part of this one.
        FoldKey::Response(_) => run_active(items, response_run(items, ix)),
        // A nested child is spanned by the run but is not a member of it, so a
        // group's liveness must not read one — otherwise a call that belongs to
        // an inner card holds the group force-expanded.
        //
        // The hierarchy is built here rather than passed in because only the
        // click path reaches this arm: the projection asks `group_active` of
        // the structure it already holds, and no render asks about a group.
        FoldKey::ToolGroup(_) => {
            let hierarchy = ToolHierarchy::build(items);
            let structure = TranscriptStructure::new(items, &hierarchy);
            structure.group_active(structure.tool_run(ix, items.len()))
        }
        FoldKey::ThinkingGroup(_) => items.get(ix..).is_some_and(|rest| {
            rest.iter()
                .take_while(|item| matches!(item, ChatItem::Thinking { .. }))
                .any(is_active)
        }),
        FoldKey::Tail(_)
        | FoldKey::Compaction(_)
        | FoldKey::ToolGroupTail(_)
        | FoldKey::SubagentTail(_)
        | FoldKey::Filtered(_) => false,
    }
}

/// The `self.rows` index a fold toggle on `key` must remeasure so its stale
/// cached row height doesn't clip/overlap neighboring rows. `Assistant` /
/// `Thinking` are their own row, keyed directly by item index. `Tool` /
/// `Subagent` / `ToolRawInput` / `Diff` (keyed by tool-call id, `Diff` as
/// `"{tool_id}#{diff_index}"`) only ever change *their owning row's* rendered
/// height in place — never a `RenderRow::hidden` flip anywhere — so
/// `rebuild_rows`'s hidden-range diff can't see them and falls back to
/// remeasuring the tail, leaving a stale height on whichever row actually
/// changed (the diff-collapse clipping bug). `Response` / `ToolGroup` /
/// `ThinkingGroup` collapse instead hides their child rows, which the
/// hidden-range diff already catches correctly, so they resolve to `None` here.
pub(crate) fn fold_key_item_index(key: &FoldKey, items: &[daruda_acp::ChatItem]) -> Option<usize> {
    // Built only for the keys that ask a hierarchy question; nested subagent
    // children render inside their parent's card and earn no row of their own.
    let owner = |id: &str| {
        let hierarchy = ToolHierarchy::build(items);
        TranscriptStructure::new(items, &hierarchy).owning_item(id)
    };
    match key {
        FoldKey::Assistant(ix) | FoldKey::Thinking(ix) | FoldKey::Compaction(ix) => Some(*ix),
        // `SubagentTail` belongs with these: its reveal changes which child
        // cards the parent's body holds — a height change in place, with no
        // `RenderRow::hidden` flip for the diff to see.
        FoldKey::Tool(id)
        | FoldKey::Subagent(id)
        | FoldKey::ToolRawInput(id)
        | FoldKey::SubagentTail(id) => owner(id),
        FoldKey::Diff(diff_key) => owner(diff_key.split('#').next().unwrap_or(diff_key.as_str())),
        FoldKey::Response(_)
        | FoldKey::ToolGroup(_)
        | FoldKey::ThinkingGroup(_)
        | FoldKey::Tail(_)
        | FoldKey::ToolGroupTail(_)
        | FoldKey::Filtered(_) => None,
    }
}

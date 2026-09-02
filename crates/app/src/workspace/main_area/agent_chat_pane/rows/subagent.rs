//! Which of a subagent card's flattened children the card shows.
//!
//! The adapter flattens a spawned subagent's tool calls into the conversation
//! and links each to its launch by `parent_tool_id`. Those children own no
//! [`super::RenderRow`] — they render inside the launch's card — so this
//! decision cannot live in the row projection. It lives here, pure, so the
//! card's renderer only has to iterate what it returns.
//!
//! A subagent contributes only tool calls, so a card's children are one group
//! of calls with no prose to split them into steps: the step axis counts them
//! the way a tool group counts its own.

use daruda_acp::{ChatItem, ToolCallItem};

use super::tail::TailWindow;
use super::{FilterMatchIndex, LiveSubagentUnits, tool_or_subtree_live};
use crate::workspace::main_area::agent_chat_pane::tool_hierarchy::SUBAGENT_NEST_DEPTH_CAP;

/// Everything outside the child list itself that decides what a card shows.
/// Grouped so the decision reads as one call rather than eight arguments.
#[derive(Clone, Copy)]
pub(in crate::workspace) struct SubagentLens<'a> {
    pub(in crate::workspace) filter: &'a FilterMatchIndex,
    /// The run's filter disclosure is open, so rows the filter rejected are
    /// back. A card's children have no row to reject, but the disclosure still
    /// admits the whole card, descendants included.
    pub(in crate::workspace) filter_revealed: bool,
    pub(in crate::workspace) live_units: &'a LiveSubagentUnits,
    pub(in crate::workspace) tail: TailWindow,
    /// The card's own boundary is open.
    pub(in crate::workspace) revealed: bool,
}

/// One child the card renders.
pub(in crate::workspace) struct SubagentChild<'a> {
    /// Its index in the conversation — the card keys its fold state off this.
    pub(in crate::workspace) ix: usize,
    pub(in crate::workspace) call: &'a ToolCallItem,
    /// The window covers this child, so the rail ties it back to the boundary
    /// above it. Reports coverage, not the boundary's state: a live covered
    /// child is on screen through a shut boundary and is still outside the
    /// range the card is showing.
    pub(in crate::workspace) covered: bool,
}

/// What a subagent card renders for its children.
pub(in crate::workspace) struct SubagentChildren<'a> {
    /// The children to render, in conversation order.
    pub(in crate::workspace) shown: Vec<SubagentChild<'a>>,
    /// Children the boundary holds back — the count its closed label promises.
    /// Deliberately not `collected - shown`: a covered child that is live
    /// escapes onto the screen without leaving the tally, exactly as a live run
    /// does under the response's boundary one level up.
    pub(in crate::workspace) hidden: usize,
    /// Children the window keeps — the open label's count.
    pub(in crate::workspace) kept: usize,
}

impl<'a> SubagentChildren<'a> {
    pub(in crate::workspace) fn of(
        items: &'a [ChatItem],
        parent_id: &str,
        depth: usize,
        lens: SubagentLens<'_>,
    ) -> Self {
        // Past the cap the children are simply not nested — real
        // `parentToolUseId`s are unique and acyclic, but this one comes from a
        // subprocess pipe, and a cyclic ref would otherwise recurse until the
        // stack overflows.
        if depth >= SUBAGENT_NEST_DEPTH_CAP {
            return Self {
                shown: Vec::new(),
                hidden: 0,
                kept: 0,
            };
        }
        let collected: Vec<(usize, &'a ToolCallItem)> = items
            .iter()
            .enumerate()
            .filter_map(|(ix, item)| match item {
                ChatItem::ToolCall(call) if call.parent_tool_id.as_deref() == Some(parent_id) => {
                    (lens.filter_revealed || lens.filter.keeps_tool(call)).then_some((ix, call))
                }
                _ => None,
            })
            .collect();
        let count = collected.len();
        let hidden = lens.tail.hidden_steps(count);
        let shown = collected
            .into_iter()
            .enumerate()
            .filter_map(|(pos, (ix, call))| {
                let covered = lens.tail.hides(pos, count);
                let live = tool_or_subtree_live(call, lens.live_units);
                (!covered || lens.revealed || live).then_some(SubagentChild { ix, call, covered })
            })
            .collect();
        Self {
            shown,
            hidden,
            kept: count - hidden,
        }
    }

    /// Whether the card's boundary has anything to offer. `hidden` moves with
    /// the window, so the boundary comes and goes with it — unlike the row-level
    /// ones, which always occupy their slot so the list's diff stays stable.
    pub(in crate::workspace) fn offers_reveal(&self) -> bool {
        self.hidden > 0
    }
}

#[cfg(test)]
mod tests;

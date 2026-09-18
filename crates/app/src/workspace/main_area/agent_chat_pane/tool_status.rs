//! Live/settled status of a tool call, including what its flattened
//! descendants contribute.
//!
//! Separate from row projection because the question is about a call, not
//! about a row: both the projection and the fold/rollup helpers ask it, and
//! keeping it here is what lets those two stop depending on each other.

use std::collections::HashSet;

use daruda_acp::{ChatItem, ToolCallItem, ToolStatusView};

use super::tool_hierarchy::ToolHierarchy;

/// Tool ids with a live descendant, built by walking upward from live calls.
#[derive(Default)]
pub(in crate::workspace) struct LiveSubagentUnits {
    ids: HashSet<String>,
}

impl LiveSubagentUnits {
    /// The running calls that declare a parent — the only ones with ancestors
    /// to mark. Empty means there is nothing to walk, which is the common idle
    /// case (every codex session, every Task-less claude session).
    fn nested_live(items: &[ChatItem]) -> impl Iterator<Item = &str> {
        items.iter().filter_map(|it| match it {
            ChatItem::ToolCall(tc) if tc.status.is_live() && tc.parent_tool_id.is_some() => {
                Some(tc.id.as_str())
            }
            _ => None,
        })
    }

    pub(super) fn build<'a>(hierarchy: &ToolHierarchy<'a>, items: &'a [ChatItem]) -> Self {
        if Self::nested_live(items).next().is_none() {
            return Self::default();
        }
        let mut ids = HashSet::new();
        for id in Self::nested_live(items) {
            ids.extend(hierarchy.ancestors(id).map(str::to_owned));
        }
        Self { ids }
    }

    /// Test convenience: derive the hierarchy for this one call. Production
    /// shares a single hierarchy across the whole projection pass. Keeps the
    /// idle early-out ahead of the build so the cheap case stays cheap here too.
    #[cfg(test)]
    pub(in crate::workspace) fn of(items: &[ChatItem]) -> Self {
        if Self::nested_live(items).next().is_none() {
            return Self::default();
        }
        Self::build(&ToolHierarchy::build(items), items)
    }

    pub(in crate::workspace) fn contains(&self, tool_id: &str) -> bool {
        self.ids.contains(tool_id)
    }
}

/// Whether a tool or one of its flattened descendants is live.
pub(in crate::workspace) fn tool_or_subtree_live(
    tc: &ToolCallItem,
    live_units: &LiveSubagentUnits,
) -> bool {
    tc.status.is_live() || live_units.contains(&tc.id)
}

/// Subtree-aware status used by badges, filters, and rollups.
pub(in crate::workspace) fn effective_tool_status(
    tc: &ToolCallItem,
    live_units: &LiveSubagentUnits,
) -> ToolStatusView {
    if !tc.status.is_live() && live_units.contains(&tc.id) {
        ToolStatusView::InProgress
    } else {
        tc.status
    }
}

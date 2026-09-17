//! What expand-all and collapse-all are allowed to touch.
//!
//! Derived from a projection rather than from `items` alone: only the row walk
//! knows which prose renders inline (no fold of its own) and which runs earned
//! a bar. Lives beside the projection for that reason — asking the question
//! from `agent_chat_helpers` is what made the two modules depend on each other.

use std::collections::HashSet;

use daruda_acp::ChatItem;

use super::super::agent_chat_helpers::{diff_editor_key, renders_raw_input, tool_fold_key};
use super::super::fold::{FoldKey, FoldState};
use super::{RowKind, project};
use crate::transcript::display_filter::DisplayFilter;

/// Fold keys controlled by expand-all and collapse-all. Tail and filter reveals
/// are excluded because their chips own those states.
pub(in crate::workspace) fn collect_foldable_keys(items: &[ChatItem]) -> Vec<FoldKey> {
    let mut keys: Vec<FoldKey> = Vec::new();
    // Defaults preserve the structural header set while avoiding pane state.
    let rows = project(
        items,
        &FoldState::default(),
        false,
        &super::LiveSubagentUnits::default(),
        super::tail::StepWindow::default(),
        &DisplayFilter::default(),
    );
    // Inline assistant prose has no independent fold control.
    let inline_assistant: HashSet<usize> = rows
        .iter()
        .filter_map(|row| match row.kind {
            RowKind::AgentItem(ix) if row.indent > 0 => Some(ix),
            _ => None,
        })
        .collect();
    for row in &rows {
        match &row.kind {
            RowKind::ResponseHeader { run_start, .. } => keys.push(FoldKey::Response(*run_start)),
            RowKind::ToolGroupHeader { gid, .. } => keys.push(FoldKey::ToolGroup(gid.clone())),
            RowKind::ThinkingGroupHeader { first_ix, .. } => {
                keys.push(FoldKey::ThinkingGroup(*first_ix))
            }
            RowKind::TailMore { .. } | RowKind::ToolGroupTailMore { .. } => {}
            RowKind::User(_)
            | RowKind::Interrupted(_)
            | RowKind::AgentItem(_)
            | RowKind::ConclusionItem(_)
            | RowKind::WorkingIndicator => {}
        }
    }
    for (ix, item) in items.iter().enumerate() {
        match item {
            ChatItem::AssistantText { .. } if inline_assistant.contains(&ix) => {}
            ChatItem::AssistantText { .. } => keys.push(FoldKey::Assistant(ix)),
            ChatItem::Thinking { .. } => keys.push(FoldKey::Thinking(ix)),
            ChatItem::ToolCall(tc) => {
                keys.push(tool_fold_key(tc));
                for di in 0..tc.diffs.len() {
                    keys.push(FoldKey::Diff(diff_editor_key(&tc.id, di)));
                }
                // Mirror the renderer's raw-input gate (generic tool, no diffs,
                // has args) so expand/collapse-all covers the disclosure. The
                // "Instructions" section (`renders_subagent_instructions`) has
                // no fold key of its own — it is always visible once shown, not
                // a disclosure — so it contributes nothing here.
                if renders_raw_input(tc) {
                    keys.push(FoldKey::ToolRawInput(tc.id.clone()));
                }
            }
            ChatItem::UserText(_)
            | ChatItem::Permission(_)
            | ChatItem::Failure(_)
            | ChatItem::Interrupted => {}
        }
    }
    keys
}

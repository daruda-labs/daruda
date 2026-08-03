//! Render-row projection over the flat `items` model.
//!
//! Folding structure (turns, agent responses, tool-call groups) is derived at
//! render time by [`project`], never stored: it yields a stable sequence of
//! [`RenderRow`]s (synthetic headers interleaved with item rows, each with a
//! `hidden` flag and `indent` depth) that the virtualized `list` indexes over.
//! Folding flips `hidden` rather than removing rows, so the count is fold-stable
//! and `same_slot` lets `rebuild_rows` splice only the changed tail, preserving
//! scroll.

use daruda_acp::{ChatItem, ToolCallItem};

use super::agent_chat_helpers::{agent_run, fold_active};
use super::fold::{FoldKey, FoldState};

/// Minimum consecutive tool-call run length that earns a collapsible group. A
/// lone tool call renders as its own (block-foldable) card — wrapping one tool
/// in a "1 tool call" group bar is noise.
const TOOL_GROUP_MIN: usize = 2;

/// What a projected row represents. The payload is the row's stable *slot key*
/// (item index / tool-group id); `same_slot` compares only the key.
pub(in crate::workspace) enum RowKind {
    /// A user message — the always-visible turn anchor (never folded).
    User(usize),
    /// Synthetic header for an agent response (the run of agent items after a
    /// user message), keyed by the anchoring `UserText` index. Emitted only for
    /// a non-trivial response (≥ 1 tool or ≥ 2 blocks); `collapsed` hides the
    /// whole response.
    ResponseHeader { anchor: usize, collapsed: bool },
    /// An individual agent block (assistant / thinking / tool / permission /
    /// error), keyed by its `items` index. One block among siblings: it says
    /// nothing about the run it sits in.
    AgentItem(usize),
    /// The lone agent block that *is* an entire response — an anchored run too
    /// trivial to earn a [`RowKind::ResponseHeader`]. Since no bar exists to carry
    /// them, this row owns the response-level affordances (the status-rollup
    /// glyph). Emitted only for a single-block anchored run, so a response can
    /// never show two rollups: a leading run with no user anchor stays
    /// [`RowKind::AgentItem`] and reports nothing.
    SoloResponse(usize),
    /// Synthetic header for a consecutive run of ≥ [`TOOL_GROUP_MIN`] tool
    /// calls. `gid` is the first tool call's id (stable under append-only);
    /// `first_ix` + `count` locate the run; `collapsed` drives the chevron and
    /// summary line.
    ToolGroupHeader {
        gid: String,
        first_ix: usize,
        count: usize,
        collapsed: bool,
    },
    /// The turn's conclusion — the run's final assistant-text block when it sits
    /// under a response bar. Stays visible when the response is collapsed and
    /// carries its own fold toggle (`FoldKey::Assistant`), so it can collapse to
    /// a one-line summary independently of the process prose.
    ConclusionItem(usize),
    /// Synthetic "agent is working" indicator pinned to the last turn's tail for
    /// the whole time a turn is in flight (through tool execution and
    /// streaming). At most one exists (always the last row). Suppressed only
    /// while blocked on a permission prompt; stays visible even when the
    /// response is manually collapsed (pinned below the still-visible
    /// conclusion).
    WorkingIndicator,
}

/// One row in the projected, renderable sequence.
pub(in crate::workspace) struct RenderRow {
    pub(in crate::workspace) kind: RowKind,
    /// Collapsed under an ancestor fold → render as a zero-height `Empty` (the
    /// row stays in the sequence so the count is fold-stable).
    pub(in crate::workspace) hidden: bool,
    /// Nesting depth (left indent): 0 = top level (user / response header),
    /// 1 = inside a response, 2 = inside a tool group within a response.
    pub(in crate::workspace) indent: u8,
}

impl RenderRow {
    /// Whether two rows occupy the same logical slot — same kind + same key,
    /// ignoring `hidden` / payload fields. Drives `rebuild_rows`' LCP diff so a
    /// fold toggle splices nothing and a streamed append splices only the tail.
    pub(in crate::workspace) fn same_slot(&self, other: &Self) -> bool {
        match (&self.kind, &other.kind) {
            (RowKind::User(a), RowKind::User(b))
            | (RowKind::AgentItem(a), RowKind::AgentItem(b))
            | (RowKind::SoloResponse(a), RowKind::SoloResponse(b))
            | (
                RowKind::ResponseHeader { anchor: a, .. },
                RowKind::ResponseHeader { anchor: b, .. },
            ) => a == b,
            (RowKind::ToolGroupHeader { gid: a, .. }, RowKind::ToolGroupHeader { gid: b, .. }) => {
                a == b
            }
            (RowKind::ConclusionItem(a), RowKind::ConclusionItem(b)) => a == b,
            // At most one indicator exists (always the tail), so any two occupy
            // the same logical slot.
            (RowKind::WorkingIndicator, RowKind::WorkingIndicator) => true,
            _ => false,
        }
    }
}

/// Threshold for a response to earn its own collapsible bar: at least one tool
/// call, or ≥ this many agent blocks. A lone short reply renders inline (no
/// response bar on every trivial answer).
const RESPONSE_MIN_BLOCKS: usize = 2;

/// Project the flat `items` into renderable rows, deciding each row's
/// visibility from `fold`. Structure (derived, not stored):
/// - A **turn** is a `UserText` anchor plus the run of agent items up to the
///   next `UserText`.
/// - A **non-trivial response** (≥ 1 tool or ≥ [`RESPONSE_MIN_BLOCKS`] blocks)
///   gets a collapsible `ResponseHeader`; a trivial one renders inline.
/// - Inside a response, a run of ≥ [`TOOL_GROUP_MIN`] tool calls gets a
///   collapsible `ToolGroupHeader`. Nesting deepens `indent`.
///
/// Defaults via `ExpandedWhileActive`: the last/streaming turn stays expanded,
/// settled past responses and tool groups collapse. Pure and total.
///
/// `awaiting_response` (the view's `activity_state() == Working` flag, which
/// includes background-tool activity as well as an in-flight prompt) drives a
/// trailing [`RowKind::WorkingIndicator`] pinned to the last turn's tail for the
/// whole turn. Suppressed only when blocked on a permission prompt; stays
/// visible when the response is manually collapsed, pinned below the conclusion.
pub(in crate::workspace) fn project(
    items: &[ChatItem],
    fold: &FoldState,
    awaiting_response: bool,
) -> Vec<RenderRow> {
    let mut rows = Vec::with_capacity(items.len() + 4);
    let mut i = 0;
    while i < items.len() {
        // A turn = optional leading user message + the agent run until the next
        // user message.
        let anchor = match &items[i] {
            ChatItem::UserText(_) => {
                rows.push(RenderRow {
                    kind: RowKind::User(i),
                    hidden: false,
                    indent: 0,
                });
                let a = i;
                i += 1;
                Some(a)
            }
            // Leading agent items with no preceding user message: render the
            // run at top level with no response header.
            _ => None,
        };

        let run = agent_run(items, i);
        i = run.end;
        let is_last_turn = i >= items.len();

        let tools = run
            .clone()
            .filter(|&k| matches!(items[k], ChatItem::ToolCall(_)))
            .count();
        let non_trivial = anchor.is_some() && (tools >= 1 || run.len() >= RESPONSE_MIN_BLOCKS);
        // The run's final assistant-text block is the turn's conclusion: it
        // stays visible when the response is collapsed (see `project_run`).
        let conclusion_ix = run
            .clone()
            .rev()
            .find(|&k| matches!(items[k], ChatItem::AssistantText { .. }));

        // Run indent, reused to place the trailing working indicator at the
        // same nesting as the conclusion.
        let run_indent = if let (true, Some(a)) = (non_trivial, anchor) {
            let collapsed = !fold.is_expanded(
                &FoldKey::Response(a),
                fold_active(&FoldKey::Response(a), items),
            );
            rows.push(RenderRow {
                kind: RowKind::ResponseHeader {
                    anchor: a,
                    collapsed,
                },
                hidden: false,
                indent: 0,
            });
            project_run(
                items,
                fold,
                run.clone(),
                1,
                collapsed,
                conclusion_ix,
                false,
                &mut rows,
            );
            1u8
        } else {
            // A single block under a user anchor is the whole response, so it
            // carries the rollup no bar exists to show. Any other bar-less run —
            // a leading run with no anchor, or (were `RESPONSE_MIN_BLOCKS` to
            // rise) a multi-block trivial one — is blocks among siblings, and a
            // per-block rollup there would report a verdict over one item while
            // a sibling tool is still running or failed.
            let solo = anchor.is_some() && run.len() == 1;
            project_run(
                items,
                fold,
                run.clone(),
                0,
                false,
                conclusion_ix,
                solo,
                &mut rows,
            );
            0u8
        };

        // While a turn is in flight (and not blocked on a permission prompt),
        // pin the working indicator to the last turn's tail so the signal stays
        // present through tool execution and streaming. Stays visible even under
        // a manually-collapsed response (at the run's indent, below the
        // conclusion).
        if awaiting_response && is_last_turn {
            rows.push(RenderRow {
                kind: RowKind::WorkingIndicator,
                hidden: false,
                indent: run_indent,
            });
        }
    }
    rows
}

/// Project one agent run into rows at `base_indent`, grouping consecutive
/// tool-call runs. `response_collapsed` hides every row in the run; a settled
/// tool group additionally hides its own members. `conclusion_ix`, when set, is
/// the run's final assistant-text item — it stays visible even while the
/// response is collapsed, so a folded turn reads as "question → conclusion".
/// `solo_response` marks a bar-less run whose single block stands for the whole
/// response ([`RowKind::SoloResponse`]).
#[allow(clippy::too_many_arguments)]
fn project_run(
    items: &[ChatItem],
    fold: &FoldState,
    run: std::ops::Range<usize>,
    base_indent: u8,
    response_collapsed: bool,
    conclusion_ix: Option<usize>,
    solo_response: bool,
    rows: &mut Vec<RenderRow>,
) {
    let mut k = run.start;
    while k < run.end {
        // A subagent's inner tool call renders nested inside its parent's card
        // (see `tool_card`), so it earns no row of its own — skip it. The Claude
        // adapter links a child to its parent via `parent_tool_id`.
        if matches!(&items[k], ChatItem::ToolCall(tc) if is_nested_child(items, tc)) {
            k += 1;
            continue;
        }
        if matches!(items[k], ChatItem::ToolCall(_)) {
            let gstart = k;
            k += 1;
            // Extend over consecutive *top-level* tool calls; a nested child
            // belongs to the preceding parent, so it ends the run.
            while k < run.end
                && matches!(&items[k], ChatItem::ToolCall(t) if !is_nested_child(items, t))
            {
                k += 1;
            }
            let grun = gstart..k;
            // A group with a still-running member (or live subagent descendant)
            // keeps its header surfaced (still folded) under a collapsed
            // response, so a folded in-flight turn shows *what* is running;
            // members stay hidden until the group is expanded.
            let group_live = grun.clone().any(
                |j| matches!(&items[j], ChatItem::ToolCall(tc) if tool_or_subtree_live(items, tc)),
            );
            if grun.len() >= TOOL_GROUP_MIN {
                let gid = tool_id(&items[gstart]);
                let group_key = FoldKey::ToolGroup(gid.clone());
                let group_collapsed = !fold.is_expanded(&group_key, fold_active(&group_key, items));
                rows.push(RenderRow {
                    kind: RowKind::ToolGroupHeader {
                        gid,
                        first_ix: gstart,
                        count: grun.len(),
                        collapsed: group_collapsed,
                    },
                    hidden: response_collapsed && !group_live,
                    indent: base_indent,
                });
                for j in grun {
                    rows.push(RenderRow {
                        kind: RowKind::AgentItem(j),
                        hidden: response_collapsed || group_collapsed,
                        indent: base_indent + 1,
                    });
                }
            } else {
                rows.push(RenderRow {
                    kind: RowKind::AgentItem(gstart),
                    hidden: response_collapsed && !group_live,
                    indent: base_indent,
                });
            }
        } else {
            // Two block kinds stay visible when the response is collapsed; every
            // other block folds away:
            //   - the conclusion (run's last assistant text), and
            //   - a still-pending permission — it is actionable, so folding away
            //     its Allow/Reject buttons would strand the user.
            // The conclusion under a response bar becomes a `ConclusionItem`
            // (its own fold toggle); at top level a plain assistant block
            // already carries one.
            let is_conclusion = Some(k) == conclusion_ix;
            let pending_permission =
                matches!(&items[k], ChatItem::Permission(c) if c.resolved.is_none());
            let force_visible = is_conclusion || pending_permission;
            let kind = if is_conclusion && base_indent > 0 {
                RowKind::ConclusionItem(k)
            } else if solo_response {
                RowKind::SoloResponse(k)
            } else {
                RowKind::AgentItem(k)
            };
            rows.push(RenderRow {
                kind,
                hidden: response_collapsed && !force_visible,
                indent: base_indent,
            });
            k += 1;
        }
    }
}

/// Whether `tc` is a subagent's inner call whose parent is present in `items`
/// (such a child renders nested inside its parent's card, so it earns no row).
/// A child with a dangling `parent_tool_id` (no matching parent) is nested by
/// nobody, so it is treated as top-level and keeps its row rather than vanishing.
fn is_nested_child(items: &[ChatItem], tc: &ToolCallItem) -> bool {
    let Some(pid) = tc.parent_tool_id.as_deref() else {
        return false;
    };
    items
        .iter()
        .any(|it| matches!(it, ChatItem::ToolCall(p) if p.id == pid))
}

/// Recursion cap for walking flattened subagent nesting (`parent_tool_id`
/// links). Real nesting is one or two levels; the cap only fires on a malformed
/// / cyclic id from the adapter, bounding the stack. Shared by the render's card
/// nesting and `subagent_subtree_live`.
pub(in crate::workspace) const SUBAGENT_NEST_DEPTH_CAP: usize = 8;

/// Whether a tool call is still working as a unit — its own status is live, or a
/// flattened descendant is still running (a subagent parent the adapter marked
/// `Completed` when its SDK call returned). Keeps a running tool / group visible
/// under a collapsed response and its card badge reading in-progress.
pub(in crate::workspace) fn tool_or_subtree_live(items: &[ChatItem], tc: &ToolCallItem) -> bool {
    tc.status.is_live() || subagent_subtree_live(items, &tc.id, SUBAGENT_NEST_DEPTH_CAP)
}

/// Whether the subagent unit rooted at `parent_id` is still working — some
/// transitive, depth-bounded descendant linked via `parent_tool_id` is live.
/// `remaining_depth` bounds the walk so a malformed / cyclic id can't recurse
/// without bound. Lets a subagent parent the adapter marked `Completed` still
/// read as running while its flattened children stream in and run afterward.
pub(in crate::workspace) fn subagent_subtree_live(
    items: &[ChatItem],
    parent_id: &str,
    remaining_depth: usize,
) -> bool {
    if remaining_depth == 0 {
        return false;
    }
    items.iter().any(|it| {
        matches!(it,
            ChatItem::ToolCall(c)
                if c.parent_tool_id.as_deref() == Some(parent_id)
                    && (c.status.is_live()
                        || subagent_subtree_live(items, &c.id, remaining_depth - 1)))
    })
}

/// First tool call's id for a `ToolCall` item (the group's stable key); empty
/// for any other item (never called that way).
fn tool_id(item: &ChatItem) -> String {
    match item {
        ChatItem::ToolCall(tc) => tc.id.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests;

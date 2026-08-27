//! Projects the flat chat model into stable virtual-list rows. Folding changes
//! `hidden` flags instead of removing rows so scroll positions remain stable.

pub(in crate::workspace) mod step;
pub(in crate::workspace) mod tail;

use std::collections::{HashMap, HashSet};

use daruda_acp::{ChatItem, ToolCallItem, ToolStatusView};

use super::agent_chat_helpers::{TurnBoundary, agent_run, fold_context_at};
use super::display_filter::DisplayFilter;
use super::fold::{FoldKey, FoldState};
use tail::TailWindow;

const TOOL_GROUP_MIN: usize = 2;

/// Projected row kinds keyed by stable item or group identity.
pub(in crate::workspace) enum RowKind {
    User(usize),
    ResponseHeader {
        anchor: usize,
        collapsed: bool,
    },
    AgentItem(usize),
    SoloResponse(usize),
    StepHeader {
        first_ix: usize,
        tool_count: usize,
        collapsed: bool,
    },
    TailMore {
        run_start: usize,
        hidden_steps: usize,
        collapsed: bool,
    },
    FilteredAway {
        run_start: usize,
        count: usize,
        collapsed: bool,
    },
    ToolGroupHeader {
        gid: String,
        first_ix: usize,
        count: usize,
        collapsed: bool,
    },
    ConclusionItem(usize),
    WorkingIndicator,
}

pub(in crate::workspace) struct RenderRow {
    pub(in crate::workspace) kind: RowKind,
    /// Hidden rows stay in the sequence to preserve slot stability.
    pub(in crate::workspace) hidden: bool,
    pub(in crate::workspace) indent: u8,
}

/// Filter matches plus ancestors needed to reach matching nested tools.
#[derive(Default)]
pub(in crate::workspace) struct FilterMatchIndex {
    filter: DisplayFilter,
    tool_ids: HashSet<String>,
}

impl FilterMatchIndex {
    pub(in crate::workspace) fn build(
        items: &[ChatItem],
        filter: DisplayFilter,
        live_units: &LiveSubagentUnits,
    ) -> Self {
        let parent_of: HashMap<&str, &str> = items
            .iter()
            .filter_map(|item| match item {
                ChatItem::ToolCall(tc) => Some((tc.id.as_str(), tc.parent_tool_id.as_deref()?)),
                _ => None,
            })
            .collect();
        let mut tool_ids = HashSet::new();
        for tc in items.iter().filter_map(|item| match item {
            ChatItem::ToolCall(tc)
                if filter.matches_tool(tc, effective_tool_status(tc, live_units)) =>
            {
                Some(tc)
            }
            _ => None,
        }) {
            let mut current = Some(tc.id.as_str());
            for _ in 0..=SUBAGENT_NEST_DEPTH_CAP {
                let Some(id) = current else { break };
                tool_ids.insert(id.to_owned());
                current = parent_of.get(id).copied();
            }
        }
        Self { filter, tool_ids }
    }

    pub(in crate::workspace) fn matches(&self, item: &ChatItem) -> bool {
        match item {
            ChatItem::ToolCall(tc) => self.keeps_tool(tc),
            _ => self.filter.matches(item),
        }
    }

    pub(in crate::workspace) fn keeps_tool(&self, tc: &ToolCallItem) -> bool {
        self.tool_ids.contains(&tc.id)
    }
}

impl RenderRow {
    /// Compare stable row identity, ignoring visibility and payload changes.
    pub(in crate::workspace) fn same_slot(&self, other: &Self) -> bool {
        match (&self.kind, &other.kind) {
            (RowKind::User(a), RowKind::User(b))
            | (RowKind::AgentItem(a), RowKind::AgentItem(b))
            | (RowKind::SoloResponse(a), RowKind::SoloResponse(b))
            | (
                RowKind::ResponseHeader { anchor: a, .. },
                RowKind::ResponseHeader { anchor: b, .. },
            )
            | (RowKind::StepHeader { first_ix: a, .. }, RowKind::StepHeader { first_ix: b, .. })
            | (RowKind::TailMore { run_start: a, .. }, RowKind::TailMore { run_start: b, .. })
            | (
                RowKind::FilteredAway { run_start: a, .. },
                RowKind::FilteredAway { run_start: b, .. },
            ) => a == b,
            (RowKind::ToolGroupHeader { gid: a, .. }, RowKind::ToolGroupHeader { gid: b, .. }) => {
                a == b
            }
            (RowKind::ConclusionItem(a), RowKind::ConclusionItem(b)) => a == b,
            (RowKind::WorkingIndicator, RowKind::WorkingIndicator) => true,
            _ => false,
        }
    }
}

const RESPONSE_MIN_BLOCKS: usize = 2;

/// Project chat items into stable rows.
pub(in crate::workspace) fn project(
    items: &[ChatItem],
    fold: &FoldState,
    awaiting_response: bool,
    live_units: &LiveSubagentUnits,
    tail: TailWindow,
    filter: &DisplayFilter,
) -> Vec<RenderRow> {
    let filter = FilterMatchIndex::build(items, *filter, live_units);
    project_with_filter_index(items, fold, awaiting_response, live_units, tail, &filter)
}

/// [`project`] with a caller-owned filter index shared with nested cards.
pub(in crate::workspace) fn project_with_filter_index(
    items: &[ChatItem],
    fold: &FoldState,
    awaiting_response: bool,
    live_units: &LiveSubagentUnits,
    tail: TailWindow,
    filter: &FilterMatchIndex,
) -> Vec<RenderRow> {
    // Only children whose parent is present are nested.
    let tool_ids: HashSet<&str> = items
        .iter()
        .filter_map(|it| match it {
            ChatItem::ToolCall(tc) => Some(tc.id.as_str()),
            _ => None,
        })
        .collect();
    let boundary = TurnBoundary::of(items);
    let mut rows = Vec::with_capacity(items.len() + 4);
    let mut i = 0;
    while i < items.len() {
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
        let conclusion_ix = run
            .clone()
            .rev()
            .find(|&k| matches!(items[k], ChatItem::AssistantText { .. }));

        let run_indent = if let (true, Some(a)) = (non_trivial, anchor) {
            let key = FoldKey::Response(a);
            let collapsed = !fold.is_expanded(&key, fold_context_at(&key, a, items, boundary));
            rows.push(RenderRow {
                kind: RowKind::ResponseHeader {
                    anchor: a,
                    collapsed,
                },
                hidden: false,
                indent: 0,
            });
            project_run(
                RunContext {
                    items,
                    fold,
                    boundary,
                    run: run.clone(),
                    base_indent: 1,
                    response_collapsed: collapsed,
                    conclusion_ix,
                    solo_response: false,
                    tool_ids: &tool_ids,
                    live_units,
                    tail,
                    filter,
                },
                &mut rows,
            );
            1u8
        } else {
            // Only an anchored single-block run represents a whole response.
            let solo = anchor.is_some() && run.len() == 1;
            project_run(
                RunContext {
                    items,
                    fold,
                    boundary,
                    run: run.clone(),
                    base_indent: 0,
                    response_collapsed: false,
                    conclusion_ix,
                    solo_response: solo,
                    tool_ids: &tool_ids,
                    live_units,
                    tail,
                    filter,
                },
                &mut rows,
            );
            0u8
        };

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

struct RunContext<'a> {
    items: &'a [ChatItem],
    fold: &'a FoldState,
    boundary: TurnBoundary,
    run: std::ops::Range<usize>,
    base_indent: u8,
    response_collapsed: bool,
    conclusion_ix: Option<usize>,
    solo_response: bool,
    tool_ids: &'a HashSet<&'a str>,
    live_units: &'a LiveSubagentUnits,
    tail: TailWindow,
    filter: &'a FilterMatchIndex,
}

struct RunRows<'a> {
    rows: &'a mut Vec<RenderRow>,
    /// Index of the `FilteredAway` row to back-patch.
    placeholder: usize,
    revealed: bool,
}

impl RunRows<'_> {
    fn push(&mut self, kind: RowKind, structural: bool, filtered: bool, indent: u8) {
        if filtered
            && !structural
            && let RowKind::FilteredAway { count, .. } = &mut self.rows[self.placeholder].kind
        {
            *count += 1;
        }
        self.rows.push(RenderRow {
            kind,
            hidden: structural || (filtered && !self.revealed),
            indent,
        });
    }

    fn finish(self, response_collapsed: bool) {
        let covered = match &self.rows[self.placeholder].kind {
            RowKind::FilteredAway { count, .. } => *count,
            _ => 0,
        };
        self.rows[self.placeholder].hidden = response_collapsed || covered == 0;
    }
}

fn project_run(ctx: RunContext<'_>, rows: &mut Vec<RenderRow>) {
    let RunContext {
        items,
        fold,
        boundary,
        run,
        base_indent,
        response_collapsed,
        conclusion_ix,
        solo_response,
        tool_ids,
        live_units,
        tail,
        filter,
    } = ctx;
    if run.is_empty() {
        return;
    }
    // Keep one filter slot per run so filter changes do not splice the list.
    let filter_key = FoldKey::Filtered(run.start);
    let filter_revealed = fold.is_expanded(
        &filter_key,
        fold_context_at(&filter_key, run.start, items, boundary),
    );
    let placeholder = rows.len();
    rows.push(RenderRow {
        kind: RowKind::FilteredAway {
            run_start: run.start,
            count: 0,
            collapsed: !filter_revealed,
        },
        hidden: true,
        indent: base_indent,
    });
    let mut out = RunRows {
        rows,
        placeholder,
        revealed: filter_revealed,
    };

    let steps = step::steps(items, run.clone(), tool_ids);
    let step_kept: Vec<bool> = steps
        .iter()
        .map(|s| (s.span.start..s.span.end).any(|j| projects_a_row(items, j, tool_ids, filter)))
        .collect();
    let step_live: Vec<bool> = steps
        .iter()
        .map(|s| {
            (s.span.tool_start..s.span.end).any(
                |j| matches!(&items[j], ChatItem::ToolCall(tc) if tool_or_subtree_live(tc, live_units)),
            )
        })
        .collect();
    // Live covered steps still count so the reveal control remains reachable.
    let hidden_steps = tail.hidden_steps(steps.len());
    let tail_key = FoldKey::Tail(run.start);
    let tail_revealed = fold.is_expanded(
        &tail_key,
        fold_context_at(&tail_key, run.start, items, boundary),
    );
    if !steps.is_empty() {
        out.push(
            RowKind::TailMore {
                run_start: run.start,
                hidden_steps,
                collapsed: !tail_revealed,
            },
            response_collapsed || hidden_steps == 0,
            false,
            base_indent,
        );
    }
    let mut next_step = 0usize;
    let mut in_step: Option<(usize, bool)> = None;
    let mut k = run.start;
    while k < run.end {
        if in_step.is_some_and(|(end, _)| k >= end) {
            in_step = None;
        }
        if let Some(s) = steps.get(next_step).filter(|s| s.span.start == k) {
            let key = FoldKey::Step(k);
            let step_collapsed = !fold.is_expanded(&key, fold_context_at(&key, k, items, boundary));
            let outside_tail = !tail_revealed && tail.hides(next_step, steps.len());
            // Live step headers stay visible through enclosing folds.
            out.push(
                RowKind::StepHeader {
                    first_ix: k,
                    tool_count: s.tool_count,
                    collapsed: step_collapsed,
                },
                (response_collapsed || outside_tail) && !step_live[next_step],
                !step_kept[next_step],
                base_indent,
            );
            in_step = Some((s.span.end, step_collapsed || outside_tail));
            next_step += 1;
        }
        if matches!(&items[k], ChatItem::ToolCall(tc) if is_nested_child(tool_ids, tc)) {
            k += 1;
            continue;
        }
        let (indent, folded) = match in_step {
            Some((_, step_collapsed)) => (base_indent + 1, response_collapsed || step_collapsed),
            None => (base_indent, response_collapsed),
        };
        if matches!(items[k], ChatItem::ToolCall(_)) {
            let gstart = k;
            k += 1;
            while k < run.end
                && matches!(&items[k], ChatItem::ToolCall(t) if !is_nested_child(tool_ids, t))
            {
                k += 1;
            }
            let grun = gstart..k;
            // Live group headers stay visible through enclosing folds.
            let group_live = grun.clone().any(
                |j| matches!(&items[j], ChatItem::ToolCall(tc) if tool_or_subtree_live(tc, live_units)),
            );
            if grun.len() >= TOOL_GROUP_MIN {
                let gid = tool_id(&items[gstart]);
                let group_key = FoldKey::ToolGroup(gid.clone());
                let group_collapsed = !fold.is_expanded(
                    &group_key,
                    fold_context_at(&group_key, gstart, items, boundary),
                );
                let group_kept = grun.clone().any(|j| filter.matches(&items[j]));
                out.push(
                    RowKind::ToolGroupHeader {
                        gid,
                        first_ix: gstart,
                        count: grun.len(),
                        collapsed: group_collapsed,
                    },
                    folded && !group_live,
                    !group_kept,
                    indent,
                );
                for j in grun {
                    out.push(
                        RowKind::AgentItem(j),
                        folded || group_collapsed,
                        !filter.matches(&items[j]),
                        indent + 1,
                    );
                }
            } else {
                out.push(
                    RowKind::AgentItem(gstart),
                    folded && !group_live,
                    !filter.matches(&items[gstart]),
                    indent,
                );
            }
        } else {
            // Conclusions and actionable permissions escape enclosing folds.
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
            out.push(
                kind,
                folded && !force_visible,
                !filter.matches(&items[k]),
                indent,
            );
            k += 1;
        }
    }
    out.finish(response_collapsed);
}

fn projects_a_row(
    items: &[ChatItem],
    ix: usize,
    tool_ids: &HashSet<&str>,
    filter: &FilterMatchIndex,
) -> bool {
    !matches!(&items[ix], ChatItem::ToolCall(tc) if is_nested_child(tool_ids, tc))
        && filter.matches(&items[ix])
}

fn is_tool_call(item: &ChatItem) -> bool {
    matches!(item, ChatItem::ToolCall(_))
}

/// A dangling parent id remains top-level so the tool cannot disappear.
fn is_nested_child(tool_ids: &HashSet<&str>, tc: &ToolCallItem) -> bool {
    tc.parent_tool_id
        .as_deref()
        .is_some_and(|pid| tool_ids.contains(pid))
}

/// Bounds malformed or cyclic subagent parent links.
pub(in crate::workspace) const SUBAGENT_NEST_DEPTH_CAP: usize = 8;

/// Tool ids with a live descendant, built by walking upward from live calls.
#[derive(Default)]
pub(in crate::workspace) struct LiveSubagentUnits {
    ids: HashSet<String>,
}

impl LiveSubagentUnits {
    pub(in crate::workspace) fn build(items: &[ChatItem]) -> Self {
        let live = || {
            items.iter().filter_map(|it| match it {
                ChatItem::ToolCall(tc) if tc.status.is_live() => tc.parent_tool_id.as_deref(),
                _ => None,
            })
        };
        if live().next().is_none() {
            return Self::default();
        }
        let parent_of: HashMap<&str, &str> = items
            .iter()
            .filter_map(|it| match it {
                ChatItem::ToolCall(tc) => Some((tc.id.as_str(), tc.parent_tool_id.as_deref()?)),
                _ => None,
            })
            .collect();
        let mut ids = HashSet::new();
        for parent in live() {
            let mut cur = Some(parent);
            for _ in 0..SUBAGENT_NEST_DEPTH_CAP {
                let Some(id) = cur else { break };
                ids.insert(id.to_owned());
                cur = parent_of.get(id).copied();
            }
        }
        Self { ids }
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

fn tool_id(item: &ChatItem) -> String {
    match item {
        ChatItem::ToolCall(tc) => tc.id.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests;

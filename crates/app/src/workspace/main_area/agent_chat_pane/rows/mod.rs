//! Projects the flat chat model into stable virtual-list rows. Folding changes
//! `hidden` flags instead of removing rows so scroll positions remain stable.

pub(in crate::workspace) mod step;
pub(in crate::workspace) mod tail;

use std::collections::HashSet;

use daruda_acp::{ChatItem, ToolCallItem, ToolStatusView};

use super::agent_chat_helpers::{TurnBoundary, agent_run, fold_context_at};
use super::display_filter::DisplayFilter;
use super::fold::{FoldKey, FoldState};
use super::tool_hierarchy::ToolHierarchy;
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
    /// The filter's per-run disclosure. `revealable` is what expanding this row
    /// puts on screen; `excluded` is everything the filter dropped in the run,
    /// including rows a collapsed step or response still holds — those stay
    /// hidden through the reveal, so the two numbers are not interchangeable.
    FilteredAway {
        run_start: usize,
        revealable: usize,
        excluded: usize,
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

/// Filter matches, the ancestors needed to reach a matching nested tool, and
/// every descendant of a kept tool.
#[derive(Default)]
pub(in crate::workspace) struct FilterMatchIndex {
    filter: DisplayFilter,
    tool_ids: HashSet<String>,
}

impl FilterMatchIndex {
    pub(in crate::workspace) fn build<'a>(
        hierarchy: &ToolHierarchy<'a>,
        items: &'a [ChatItem],
        filter: DisplayFilter,
    ) -> Self {
        let mut tool_ids = HashSet::new();
        for tc in items.iter().filter_map(|item| match item {
            ChatItem::ToolCall(tc) if filter.matches_tool(tc) => Some(tc),
            _ => None,
        }) {
            // A match drags its ancestors in so a nested hit stays reachable
            // through the cards it renders inside.
            tool_ids.extend(hierarchy.with_ancestors(tc.id.as_str()).map(str::to_owned));
        }
        // A kept tool keeps its whole subtree: nested children render inside
        // their parent's card and earn no row of their own, so the filter — whose
        // unit is the row — has no placeholder to count them in and no reveal to
        // bring them back. Dropping one would delete it silently.
        hierarchy.extend_with_descendants(&mut tool_ids);
        Self { filter, tool_ids }
    }

    /// Test convenience: derive the hierarchy for this one call. Production
    /// shares a single hierarchy across the whole projection pass.
    #[cfg(test)]
    pub(in crate::workspace) fn of(items: &[ChatItem], filter: DisplayFilter) -> Self {
        Self::build(&ToolHierarchy::build(items), items, filter)
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

/// Stable identity of a projected row: the key `rebuild_rows`' diff compares to
/// decide whether two projections put the same thing in the same list slot.
/// Deliberately carries no payload and no `hidden` flag — those change freely
/// within one slot.
#[derive(PartialEq, Eq)]
pub(in crate::workspace) enum RowSlot<'a> {
    User(usize),
    Response(usize),
    AgentItem(usize),
    SoloResponse(usize),
    Step(usize),
    TailMore(usize),
    FilteredAway(usize),
    ToolGroup(&'a str),
    Conclusion(usize),
    /// At most one indicator exists, so any two of them are the same slot.
    Working,
}

impl RowKind {
    /// This row's slot identity. The match is exhaustive on purpose: a new
    /// [`RowKind`] cannot compile until it declares which slot it occupies,
    /// which is what keeps the diff from splicing it on every projection.
    fn slot(&self) -> RowSlot<'_> {
        match self {
            RowKind::User(ix) => RowSlot::User(*ix),
            RowKind::ResponseHeader { anchor, .. } => RowSlot::Response(*anchor),
            RowKind::AgentItem(ix) => RowSlot::AgentItem(*ix),
            RowKind::SoloResponse(ix) => RowSlot::SoloResponse(*ix),
            RowKind::StepHeader { first_ix, .. } => RowSlot::Step(*first_ix),
            RowKind::TailMore { run_start, .. } => RowSlot::TailMore(*run_start),
            RowKind::FilteredAway { run_start, .. } => RowSlot::FilteredAway(*run_start),
            RowKind::ToolGroupHeader { gid, .. } => RowSlot::ToolGroup(gid.as_str()),
            RowKind::ConclusionItem(ix) => RowSlot::Conclusion(*ix),
            RowKind::WorkingIndicator => RowSlot::Working,
        }
    }
}

impl RenderRow {
    /// Compare stable row identity, ignoring visibility and payload changes.
    pub(in crate::workspace) fn same_slot(&self, other: &Self) -> bool {
        self.kind.slot() == other.kind.slot()
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
    let hierarchy = ToolHierarchy::build(items);
    let filter = FilterMatchIndex::build(&hierarchy, items, *filter);
    project_with_filter_index(
        items,
        &hierarchy,
        fold,
        awaiting_response,
        live_units,
        tail,
        &filter,
    )
}

/// [`project`] with a caller-owned hierarchy and filter index shared with
/// nested cards.
pub(in crate::workspace) fn project_with_filter_index<'a>(
    items: &'a [ChatItem],
    hierarchy: &'a ToolHierarchy<'a>,
    fold: &FoldState,
    awaiting_response: bool,
    live_units: &LiveSubagentUnits,
    tail: TailWindow,
    filter: &FilterMatchIndex,
) -> Vec<RenderRow> {
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
                    hierarchy,
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
                    hierarchy,
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
    hierarchy: &'a ToolHierarchy<'a>,
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
            && let RowKind::FilteredAway {
                revealable,
                excluded,
                ..
            } = &mut self.rows[self.placeholder].kind
        {
            *excluded += 1;
            // A row a fold already holds does not come back when the filter
            // placeholder expands, so it is not part of what this control
            // offers to reveal.
            if !structural {
                *revealable += 1;
            }
        }
        self.rows.push(RenderRow {
            kind,
            hidden: structural || (filtered && !self.revealed),
            indent,
        });
    }

    /// Hide the placeholder when expanding it would change nothing — a run
    /// whose filtered rows are all held by a fold offers no reveal, so it gets
    /// no control.
    fn finish(self, response_collapsed: bool) {
        let revealable = match &self.rows[self.placeholder].kind {
            RowKind::FilteredAway { revealable, .. } => *revealable,
            _ => 0,
        };
        self.rows[self.placeholder].hidden = response_collapsed || revealable == 0;
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
        hierarchy,
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
            revealable: 0,
            excluded: 0,
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

    let steps = step::steps(items, run.clone(), hierarchy);
    let step_kept: Vec<bool> = steps
        .iter()
        .map(|s| (s.span.start..s.span.end).any(|j| projects_a_row(items, j, hierarchy, filter)))
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
    let mut in_step: Option<(usize, bool, bool)> = None;
    let mut k = run.start;
    while k < run.end {
        if in_step.is_some_and(|(end, _, _)| k >= end) {
            in_step = None;
        }
        if let Some(s) = steps.get(next_step).filter(|s| s.span.start == k) {
            let key = FoldKey::Step(k);
            let step_collapsed = !fold.is_expanded(&key, fold_context_at(&key, k, items, boundary));
            let outside_tail = !tail_revealed && tail.hides(next_step, steps.len());
            if s.renders_header {
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
                in_step = Some((s.span.end, step_collapsed || outside_tail, true));
            } else {
                in_step = Some((s.span.end, outside_tail, false));
            }
            next_step += 1;
        }
        if matches!(&items[k], ChatItem::ToolCall(tc) if hierarchy.is_nested_child(tc)) {
            k += 1;
            continue;
        }
        let (indent, folded) = match in_step {
            Some((_, step_collapsed, renders_header)) => (
                base_indent + u8::from(renders_header),
                response_collapsed || step_collapsed,
            ),
            None => (base_indent, response_collapsed),
        };
        if matches!(items[k], ChatItem::ToolCall(_)) {
            let gstart = k;
            k += 1;
            while k < run.end
                && matches!(&items[k], ChatItem::ToolCall(t) if !hierarchy.is_nested_child(t))
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
    hierarchy: &ToolHierarchy<'_>,
    filter: &FilterMatchIndex,
) -> bool {
    !matches!(&items[ix], ChatItem::ToolCall(tc) if hierarchy.is_nested_child(tc))
        && filter.matches(&items[ix])
}

fn is_tool_call(item: &ChatItem) -> bool {
    matches!(item, ChatItem::ToolCall(_))
}

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

    pub(in crate::workspace) fn build<'a>(
        hierarchy: &ToolHierarchy<'a>,
        items: &'a [ChatItem],
    ) -> Self {
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

fn tool_id(item: &ChatItem) -> String {
    match item {
        ChatItem::ToolCall(tc) => tc.id.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests;

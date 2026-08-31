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

/// What the display filter dropped from one run. The two numbers are not
/// interchangeable: a row a fold already holds does not come back when the
/// reveal opens, so only `revealable` states what clicking does.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(in crate::workspace) struct FilteredAway {
    /// Rows the reveal puts on screen.
    pub(in crate::workspace) revealable: usize,
    /// Everything the filter dropped, including rows a collapsed step or
    /// response still holds.
    pub(in crate::workspace) excluded: usize,
}

/// Projected row kinds keyed by stable item or group identity.
pub(in crate::workspace) enum RowKind {
    User(usize),
    ResponseHeader {
        /// First item of the response this bar heads. Keyed off the run rather
        /// than the user turn: a restored pane can open with a run whose user
        /// turn was dropped on replay, and that run needs a bar too.
        run_start: usize,
        collapsed: bool,
        /// What the display filter took out of this response. The bar is the
        /// run's one header, so the reveal control rides here rather than on a
        /// row of its own that would read as another step.
        filtered: FilteredAway,
    },
    AgentItem(usize),
    StepHeader(StepHeaderRow),
    TailMore {
        run_start: usize,
        hidden_steps: usize,
        /// Steps the window keeps. The open label names this number, not
        /// `hidden_steps` — it states the state clicking returns to. Carried on
        /// the row rather than read off the pane's `TailWindow` so the label
        /// cannot outlive the projection it describes.
        kept_steps: usize,
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

/// The projection-owned facts needed to render one step header. Keeping the
/// span here prevents the renderer from rediscovering a potentially different
/// step boundary from the first item index.
#[derive(Clone, Copy)]
pub(in crate::workspace) struct StepHeaderRow {
    pub(in crate::workspace) span: step::StepSpan,
    pub(in crate::workspace) tool_count: usize,
    pub(in crate::workspace) collapsed: bool,
    /// This step's own prose item whose row escapes the fold and is on screen
    /// anyway. The header must not title itself from it, or the same line reads
    /// twice: once summarized, once in full.
    pub(in crate::workspace) visible_body_prose: Option<usize>,
}

pub(in crate::workspace) struct RenderRow {
    pub(in crate::workspace) kind: RowKind,
    /// Hidden rows stay in the sequence to preserve slot stability.
    pub(in crate::workspace) hidden: bool,
    pub(in crate::workspace) indent: u8,
    /// The per-run filter disclosure is open, so rows rejected by the active
    /// display filter are visible again. Header counts, rollups, and nested tool
    /// cards must all use this same answer as the row projection.
    pub(in crate::workspace) filter_revealed: bool,
    /// The row sits outside the tail window's kept range. Distinct from
    /// `indent`, which is structural nesting: this says the row does not belong
    /// to the range the pane is showing, and the renderer answers it with the
    /// rail tying the row back to the boundary above it.
    ///
    /// Deliberately *not* "the boundary revealed it": a live covered step stays
    /// surfaced through a shut boundary, and keying the mark on the boundary
    /// made that row gain and lose its rail as the boundary flipped.
    pub(in crate::workspace) outside_window: bool,
}

impl RenderRow {
    /// A row inside the tail window — the common case. The one default for
    /// `outside_window` lives here so no construction site restates it.
    pub(in crate::workspace) fn at(kind: RowKind, hidden: bool, indent: u8) -> Self {
        Self {
            kind,
            hidden,
            indent,
            filter_revealed: false,
            outside_window: false,
        }
    }

    fn with_filter_revealed(mut self, revealed: bool) -> Self {
        self.filter_revealed = revealed;
        self
    }
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
    Step(usize),
    TailMore(usize),
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
            RowKind::ResponseHeader { run_start, .. } => RowSlot::Response(*run_start),
            RowKind::AgentItem(ix) => RowSlot::AgentItem(*ix),
            RowKind::StepHeader(header) => RowSlot::Step(header.span.start),
            RowKind::TailMore { run_start, .. } => RowSlot::TailMore(*run_start),
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
    let context = ProjectionContext {
        items,
        fold,
        boundary,
        hierarchy,
        live_units,
        tail,
        filter,
    };
    let mut rows = Vec::with_capacity(items.len() + 4);
    let mut i = 0;
    while i < items.len() {
        if matches!(&items[i], ChatItem::UserText(_)) {
            rows.push(RenderRow::at(RowKind::User(i), false, 0));
            i += 1;
        }

        let run = agent_run(items, i);
        i = run.end;
        let is_last_turn = i >= items.len();

        let tools = run
            .clone()
            .filter(|&k| matches!(items[k], ChatItem::ToolCall(_)))
            .count();
        let blocks = run.clone().filter(|&k| !is_bodyless(&items[k])).count();
        let last_prose = LastProse::of(items, run.clone());
        let filter_key = FoldKey::Filtered(run.start);
        let filter_revealed = fold.is_expanded(
            &filter_key,
            fold_context_at(&filter_key, run.start, items, boundary),
        );

        // Every response that renders anything gets a bar — it is where the
        // filter's reveal control lives, and a turn without one has nowhere to
        // put it. Only a run with nothing on screen (an empty reply) is skipped.
        let renders_something = tools >= 1 || blocks >= 1;
        let run_indent = if renders_something {
            let key = FoldKey::Response(run.start);
            let collapsed =
                !fold.is_expanded(&key, fold_context_at(&key, run.start, items, boundary));
            let bar_ix = rows.len();
            rows.push(
                RenderRow::at(
                    RowKind::ResponseHeader {
                        run_start: run.start,
                        collapsed,
                        // Back-patched once the run walk knows what it dropped.
                        filtered: FilteredAway::default(),
                    },
                    false,
                    0,
                )
                .with_filter_revealed(filter_revealed),
            );
            RunProjector::new(
                context,
                RunSpec {
                    bar_ix: Some(bar_ix),
                    run: run.clone(),
                    base_indent: 1,
                    response_collapsed: collapsed,
                    last_prose,
                    sole_block: blocks == 1,
                    filter_revealed,
                },
                &mut rows,
            )
            .project();
            1u8
        } else {
            RunProjector::new(
                context,
                RunSpec {
                    bar_ix: None,
                    run: run.clone(),
                    base_indent: 0,
                    response_collapsed: false,
                    last_prose,
                    sole_block: blocks == 1,
                    filter_revealed,
                },
                &mut rows,
            )
            .project();
            0u8
        };

        if awaiting_response && is_last_turn {
            rows.push(RenderRow::at(RowKind::WorkingIndicator, false, run_indent));
        }
    }
    rows
}

#[derive(Clone, Copy)]
struct ProjectionContext<'a> {
    items: &'a [ChatItem],
    fold: &'a FoldState,
    boundary: TurnBoundary,
    hierarchy: &'a ToolHierarchy<'a>,
    live_units: &'a LiveSubagentUnits,
    tail: TailWindow,
    filter: &'a FilterMatchIndex,
}

struct RunSpec {
    /// Index of this run's response bar, when it has one. The filter tally is
    /// written back onto it once the walk knows what it dropped.
    bar_ix: Option<usize>,
    run: std::ops::Range<usize>,
    base_indent: u8,
    response_collapsed: bool,
    last_prose: Option<LastProse>,
    /// The run renders exactly one block. Folding it would leave the turn
    /// showing nothing, so its prose does not earn the conclusion's fold.
    sole_block: bool,
    filter_revealed: bool,
}

struct RunRows<'a> {
    rows: &'a mut Vec<RenderRow>,
    /// Index of the response bar to back-patch the tally into. `None` for an
    /// unanchored run, which has no bar to carry it.
    bar_ix: Option<usize>,
    /// Accumulated while the walk runs; written to the bar by `finish`.
    filtered: FilteredAway,
    revealed: bool,
    /// Applied to every row pushed while the walk is inside a step the tail
    /// window covers. Pusher state rather than a `push` parameter: it changes
    /// once per step, not once per row, and the eight call sites below would
    /// each have to restate it.
    outside_window: bool,
}

impl<'a> RunRows<'a> {
    fn new(
        rows: &'a mut Vec<RenderRow>,
        bar_ix: Option<usize>,
        item_count: usize,
        filter_revealed: bool,
    ) -> Self {
        rows.reserve(item_count + 2);
        Self {
            rows,
            bar_ix,
            filtered: FilteredAway::default(),
            revealed: filter_revealed,
            outside_window: false,
        }
    }

    fn push(&mut self, kind: RowKind, structural: bool, filtered: bool, indent: u8) {
        if filtered {
            self.filtered.excluded += 1;
            // A row a fold already holds does not come back when the reveal
            // opens, so it is not part of what the chip offers to show.
            if !structural {
                self.filtered.revealable += 1;
            }
        }
        self.rows.push(RenderRow {
            kind,
            hidden: structural || (filtered && !self.revealed),
            indent,
            filter_revealed: self.revealed,
            outside_window: self.outside_window,
        });
    }

    /// Write the run's tally onto its bar. The numbers describe the filter's
    /// cut alone; whether the bar is collapsed is the bar's own state, so the
    /// chip reads both rather than having one folded into the other here.
    fn finish(self) {
        let Some(bar_ix) = self.bar_ix else { return };
        if let RowKind::ResponseHeader { filtered, .. } = &mut self.rows[bar_ix].kind {
            *filtered = self.filtered;
        }
    }
}

/// The run's last assistant prose, and which of the two roles it plays. One
/// value with two readings rather than two lookups: only the *last* prose is
/// ever either of these, so "both at once" and "one without the other" are
/// states that should not be expressible.
///
/// A step ends at its tool run, so prose after the final tool is outside every
/// step — the response's conclusion, and it gets the chrome that names it.
/// Prose still inside a step is not a conclusion: it is what the agent said
/// before the work it is doing. Both stay on screen through an enclosing fold,
/// which is what conflating them was really buying — a collapsed step would
/// otherwise show nothing of what the agent just said.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LastProse {
    Conclusion(usize),
    InStep(usize),
}

impl LastProse {
    fn of(items: &[ChatItem], run: std::ops::Range<usize>) -> Option<Self> {
        let ix = run.clone().rev().find(|&k| {
            matches!(items[k], ChatItem::AssistantText { .. }) && !is_bodyless(&items[k])
        })?;
        Some(if (ix + 1..run.end).any(|j| is_tool_call(&items[j])) {
            Self::InStep(ix)
        } else {
            Self::Conclusion(ix)
        })
    }

    /// The item, whichever role it plays — both stay visible.
    fn ix(self) -> usize {
        match self {
            Self::Conclusion(ix) | Self::InStep(ix) => ix,
        }
    }
}

/// All display decisions derived for one work step before the row walk starts.
/// Keeping them together prevents the header, body, filter, and tail paths from
/// independently interpreting the same step.
#[derive(Clone, Copy)]
struct ProjectedStep {
    span: step::StepSpan,
    header: Option<StepHeaderRow>,
    /// Item this step's header took over, so the walk hides its row instead of
    /// repeating the line the header is already showing. `None` when the step
    /// has no header, or none it can show whole (see
    /// [`step::step_header_text`]).
    header_owned_prose: Option<usize>,
    body_collapsed: bool,
    header_structurally_hidden: bool,
    kept: bool,
    live: bool,
    /// The tail window covers this step, so its rows sit outside the range the
    /// pane is showing — whether or not the boundary is currently open.
    outside_window: bool,
}

impl ProjectedStep {
    fn active_state(self) -> ActiveStep {
        ActiveStep {
            end: self.span.end,
            header_owned_prose: self.header_owned_prose,
            body_collapsed: self.body_collapsed,
            renders_header: self.header.is_some(),
            outside_window: self.outside_window,
        }
    }
}

/// The subset of a projected step needed while walking its body items.
#[derive(Clone, Copy)]
struct ActiveStep {
    end: usize,
    header_owned_prose: Option<usize>,
    body_collapsed: bool,
    renders_header: bool,
    outside_window: bool,
}

struct RunProjector<'items, 'rows> {
    context: ProjectionContext<'items>,
    spec: RunSpec,
    output: &'rows mut Vec<RenderRow>,
}

impl<'items, 'rows> RunProjector<'items, 'rows> {
    fn new(
        context: ProjectionContext<'items>,
        spec: RunSpec,
        output: &'rows mut Vec<RenderRow>,
    ) -> Self {
        Self {
            context,
            spec,
            output,
        }
    }

    fn projected_steps(&self, tail_revealed: bool) -> Vec<ProjectedStep> {
        let ProjectionContext {
            items,
            fold,
            boundary,
            hierarchy,
            live_units,
            tail,
            filter,
        } = self.context;
        let raw_steps = step::steps(items, self.spec.run.clone(), hierarchy);
        let step_count = raw_steps.len();

        raw_steps
            .into_iter()
            .enumerate()
            .map(|(step_ix, step)| {
                let span = step.span;
                let key = FoldKey::Step(span.start);
                let step_collapsed = !fold.is_expanded(
                    &key,
                    fold_context_at(&key, span.start, items, boundary),
                );
                let covered = tail.hides(step_ix, step_count);
                let outside_tail = !tail_revealed && covered;
                let header_owned_prose = step
                    .renders_header
                    .then(|| step::step_header_text(items, &span).and_then(|text| text.expanded))
                    .flatten();
                let visible_body_prose = match self.spec.last_prose {
                    Some(LastProse::InStep(ix))
                        if (span.start..span.tool_start).contains(&ix)
                            && header_owned_prose != Some(ix)
                            && (self.spec.filter_revealed || filter.matches(&items[ix])) =>
                    {
                        Some(ix)
                    }
                    _ => None,
                };
                let kept = (span.start..span.end)
                    .any(|ix| projects_a_row(items, ix, hierarchy, filter));
                let live = (span.tool_start..span.end).any(|ix| {
                    matches!(&items[ix], ChatItem::ToolCall(tool) if tool_or_subtree_live(tool, live_units))
                });
                let header = step.renders_header.then(|| StepHeaderRow {
                    span,
                    tool_count: step.kept_tool_count(items, hierarchy, |item| {
                        self.spec.filter_revealed || filter.matches(item)
                    }),
                    collapsed: step_collapsed,
                    visible_body_prose,
                });

                ProjectedStep {
                    span,
                    header,
                    header_owned_prose,
                    body_collapsed: if step.renders_header {
                        step_collapsed || outside_tail
                    } else {
                        outside_tail
                    },
                    header_structurally_hidden: self.spec.response_collapsed || outside_tail,
                    kept,
                    live,
                    outside_window: covered,
                }
            })
            .collect()
    }

    fn project(self) {
        let items = self.context.items;
        let fold = self.context.fold;
        let boundary = self.context.boundary;
        let hierarchy = self.context.hierarchy;
        let live_units = self.context.live_units;
        let tail = self.context.tail;
        let filter = self.context.filter;
        let run = self.spec.run.clone();
        let base_indent = self.spec.base_indent;
        let response_collapsed = self.spec.response_collapsed;
        let last_prose = self.spec.last_prose;
        let sole_block = self.spec.sole_block;
        if run.is_empty() {
            return;
        }

        let tail_key = FoldKey::Tail(run.start);
        let tail_revealed = fold.is_expanded(
            &tail_key,
            fold_context_at(&tail_key, run.start, items, boundary),
        );
        let steps = self.projected_steps(tail_revealed);
        let hidden_steps = tail.hidden_steps(steps.len());
        let mut out = RunRows::new(
            self.output,
            self.spec.bar_ix,
            run.len(),
            self.spec.filter_revealed,
        );
        if !steps.is_empty() {
            out.push(
                RowKind::TailMore {
                    run_start: run.start,
                    hidden_steps,
                    kept_steps: steps.len() - hidden_steps,
                    collapsed: !tail_revealed,
                },
                response_collapsed || hidden_steps == 0,
                false,
                base_indent,
            );
        }

        let mut next_step = 0usize;
        let mut in_step: Option<ActiveStep> = None;
        let mut k = run.start;
        while k < run.end {
            if in_step.is_some_and(|step| k >= step.end) {
                in_step = None;
            }
            if let Some(step) = steps
                .get(next_step)
                .copied()
                .filter(|step| step.span.start == k)
            {
                in_step = Some(step.active_state());
                // Set before the header push so the header carries the rail as well.
                out.outside_window = step.outside_window;
                if let Some(header) = step.header {
                    out.push(
                        RowKind::StepHeader(header),
                        step.header_structurally_hidden && !step.live,
                        !step.kept,
                        base_indent,
                    );
                }
                next_step += 1;
            }
            // Every row pushed for the rest of this iteration belongs to whichever
            // step the walk is in — one assignment point, so the flag cannot drift.
            out.outside_window = in_step.is_some_and(|step| step.outside_window);
            if is_bodyless(&items[k])
                || matches!(&items[k], ChatItem::ToolCall(tool) if hierarchy.is_nested_child(tool))
            {
                k += 1;
                continue;
            }
            let (indent, folded) = match in_step {
                Some(step) => (
                    base_indent + u8::from(step.renders_header),
                    response_collapsed || step.body_collapsed,
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
                // The run's last prose and actionable permissions escape enclosing
                // folds. Only the conclusion reading of that prose earns the
                // conclusion's chrome: a preamble the agent wrote before work it is
                // still doing is not the response's answer, and a bare-chevron fold
                // announcing it as one is what read as a stray row.
                let is_last_prose = last_prose.map(LastProse::ix) == Some(k);
                let is_conclusion = last_prose == Some(LastProse::Conclusion(k));
                let pending_permission =
                    matches!(&items[k], ChatItem::Permission(c) if c.resolved.is_none());
                // A header showing an item whole owns it: the row would repeat
                // the line already on screen one row above.
                let header_owns = in_step.is_some_and(|step| step.header_owned_prose == Some(k));
                let force_visible = (is_last_prose || pending_permission) && !header_owns;
                let kind = if is_conclusion && base_indent > 0 && !sole_block {
                    RowKind::ConclusionItem(k)
                } else {
                    RowKind::AgentItem(k)
                };
                out.push(
                    kind,
                    header_owns || (folded && !force_visible),
                    !filter.matches(&items[k]),
                    indent,
                );
                k += 1;
            }
        }
        out.finish();
    }
}

fn projects_a_row(
    items: &[ChatItem],
    ix: usize,
    hierarchy: &ToolHierarchy<'_>,
    filter: &FilterMatchIndex,
) -> bool {
    !matches!(&items[ix], ChatItem::ToolCall(tc) if hierarchy.is_nested_child(tc))
        && !is_bodyless(&items[ix])
        && filter.matches(&items[ix])
}

fn is_tool_call(item: &ChatItem) -> bool {
    matches!(item, ChatItem::ToolCall(_))
}

/// A message carrying no renderable text. Two sources feed it: a message's
/// leading chunk arrives empty, and `daruda_acp` collapses a content block it
/// cannot render (image, audio, resource) to an empty string. Neither earns a
/// row, a block slot in the step / response thresholds, or the conclusion —
/// which escapes its enclosing fold and would pin a blank row over it.
pub(super) fn is_bodyless(item: &ChatItem) -> bool {
    matches!(
        item,
        ChatItem::AssistantText { text, .. } | ChatItem::Thinking { text, .. }
            if text.trim().is_empty()
    )
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

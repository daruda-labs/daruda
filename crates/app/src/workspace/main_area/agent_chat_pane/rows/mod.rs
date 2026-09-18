//! Projects the flat chat model into stable virtual-list rows. Folding changes
//! `hidden` flags instead of removing rows so scroll positions remain stable.

pub(in crate::workspace) mod subagent;
pub(in crate::workspace) mod tail;

use std::collections::HashSet;

use daruda_acp::{ChatItem, ToolCallItem};

use super::agent_chat_helpers::{TurnBoundary, fold_context_at};
use super::fold::{FoldContext, FoldKey, FoldState};
use super::tool_hierarchy::ToolHierarchy;
use super::transcript_structure::{TranscriptStructure, is_active, response_run};
// Re-exported so the many callers that reach these through `rows` keep one
// path; `tool_status` is the definition site and what breaks the cycle with
// `agent_chat_helpers`.
pub(in crate::workspace) use super::tool_status::{
    LiveSubagentUnits, effective_tool_status, tool_or_subtree_live,
};
use crate::transcript::display_filter::DisplayFilter;
use crate::transcript::tool_category::{ToolCategory, tally_categories};
use tail::{StepWindow, TailWindow};

pub(in crate::workspace) mod foldable_keys;
pub(in crate::workspace) use foldable_keys::collect_foldable_keys;

/// What the display filter dropped from one run and the reveal can put back.
///
/// The unit is a block, not a row: a group the filter empties counts once,
/// because the reveal brings back the group and its calls come with it. Folds
/// are deliberately not consulted — a group's fold flips on its own as its last
/// call settles, and letting that move the number made it climb to the group's
/// size and drop back mid-turn, reporting one cut two ways seconds apart.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(in crate::workspace) struct FilteredAway {
    /// Blocks the filter took out of this run.
    pub(in crate::workspace) revealable: usize,
}

impl FilteredAway {
    /// Whether the bar carries a reveal chip.
    ///
    /// The tally describes the filter alone, so the bar's own collapse must not
    /// be checked on top of it. The conclusion's `force_visible` escape is
    /// exactly the row that survives a collapsed response and can still be
    /// filtered out of it, leaving the turn showing nothing but its bar; a
    /// second collapse check erased the one control that leads back.
    pub(in crate::workspace) fn offers_reveal(self) -> bool {
        self.revealable > 0
    }
}

/// Whether the filter left a group anything to show. Decides what the group
/// contributes to the tally: the group itself, or the calls taken from it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GroupFilter {
    /// At least one call survives, so the group is on screen and what the
    /// reveal brings back is each call the filter took from it.
    Kept,
    /// Every call is rejected, so the group is what the reveal brings back and
    /// its calls are already covered by it.
    Emptied,
}

impl GroupFilter {
    fn of(
        calls: impl Iterator<Item = usize>,
        items: &[ChatItem],
        filter: &FilterMatchIndex,
    ) -> Self {
        if calls.into_iter().any(|j| filter.matches(&items[j])) {
            Self::Kept
        } else {
            Self::Emptied
        }
    }

    /// The header's own `filtered` term: an emptied group has no row on screen.
    fn hides_the_header(self) -> bool {
        self == Self::Emptied
    }
}

/// Projected row kinds keyed by stable item or group identity.
pub(in crate::workspace) enum RowKind {
    User(usize),
    /// The marker where a Stop cut the run above it. Top-level like
    /// [`RowKind::User`] rather than an item inside the response: it is the
    /// edge between two turns, so collapsing the response it ended must not
    /// take it off screen.
    Interrupted(usize),
    ResponseHeader {
        /// First item of the response this bar heads. Keyed off the run rather
        /// than the user turn: a restored pane can open with a run whose user
        /// turn was dropped on replay, and that run needs a bar too.
        run_start: usize,
        /// What the turn did, by tool category, most-numerous first.
        ///
        /// Computed here rather than in the renderer because the hierarchy that
        /// decides which calls are top-level already exists at this point;
        /// rebuilding it per frame for every bar on screen would put an
        /// items-sized walk on the paint path.
        ///
        /// Filter-blind and nesting-aware, which is the opposite of a group
        /// bar's rule: this bar summarizes the turn rather than disclosing a
        /// fixed set of rows, so a narrowed turn must still show what it did —
        /// and a subagent's inner calls are already counted inside the card
        /// that spawned them.
        categories: Vec<(ToolCategory, usize)>,
        collapsed: bool,
        /// What the display filter took out of this response. The bar is the
        /// run's one header, so the reveal control rides here rather than on a
        /// row of its own that would read as more transcript.
        filtered: FilteredAway,
    },
    AgentItem(usize),
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
    /// The same boundary one level in: the calls of a single tool group that
    /// the window covers. A group is one step of the response, so the
    /// response's own boundary never trims inside it — without this row an
    /// expanded run of twenty calls ignored the axis entirely.
    ToolGroupTailMore {
        /// The group's identity — its first call's id, the same value
        /// [`RowKind::ToolGroupHeader`] carries.
        gid: String,
        hidden_calls: usize,
        /// Calls the window keeps, for the open label. Same split of duties as
        /// [`RowKind::TailMore`]'s two counts.
        kept_calls: usize,
        collapsed: bool,
    },
    ToolGroupHeader {
        gid: String,
        /// The calls the bar speaks for, in transcript order. A list rather
        /// than a span: the run covers items that own no row, and the bar
        /// counts and summarizes its calls alone.
        calls: Vec<usize>,
        collapsed: bool,
    },
    /// Keyed on the run's first item rather than a message id: a thought carries
    /// no stable id of its own, so the item index is what the fold key, the row
    /// slot, and the element id all key off.
    ThinkingGroupHeader {
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
    /// The per-run filter disclosure is open, so rows rejected by the active
    /// display filter are visible again. Header counts, rollups, and nested tool
    /// cards must all use this same answer as the row projection.
    pub(in crate::workspace) filter_revealed: bool,
}

impl RenderRow {
    pub(in crate::workspace) fn at(kind: RowKind, hidden: bool, indent: u8) -> Self {
        Self {
            kind,
            hidden,
            indent,
            filter_revealed: false,
        }
    }

    fn with_filter_revealed(mut self, revealed: bool) -> Self {
        self.filter_revealed = revealed;
        self
    }
}

/// Filter matches, plus the ancestors needed to reach a matching nested tool.
///
/// Every call answers for its own category however deeply it nests, so
/// narrowing to one leaves a subagent card standing with only that category's
/// calls inside it — the launch itself is exempt in [`DisplayFilter::matches_tool`].
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
        self.keeps_id(&tc.id)
    }

    pub(in crate::workspace) fn keeps_id(&self, id: &str) -> bool {
        self.tool_ids.contains(id)
    }
}

/// Stable identity of a projected row: the key `rebuild_rows`' diff compares to
/// decide whether two projections put the same thing in the same list slot.
/// Deliberately carries no payload and no `hidden` flag — those change freely
/// within one slot.
#[derive(PartialEq, Eq)]
pub(in crate::workspace) enum RowSlot<'a> {
    User(usize),
    Interrupted(usize),
    Response(usize),
    AgentItem(usize),
    TailMore(usize),
    ToolGroup(&'a str),
    ToolGroupTail(&'a str),
    ThinkingGroup(usize),
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
            RowKind::Interrupted(ix) => RowSlot::Interrupted(*ix),
            RowKind::ResponseHeader { run_start, .. } => RowSlot::Response(*run_start),
            RowKind::AgentItem(ix) => RowSlot::AgentItem(*ix),
            RowKind::TailMore { run_start, .. } => RowSlot::TailMore(*run_start),
            RowKind::ToolGroupTailMore { gid, .. } => RowSlot::ToolGroupTail(gid.as_str()),
            RowKind::ToolGroupHeader { gid, .. } => RowSlot::ToolGroup(gid.as_str()),
            RowKind::ThinkingGroupHeader { first_ix, .. } => RowSlot::ThinkingGroup(*first_ix),
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
    tail: StepWindow,
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
    tail: StepWindow,
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
    // Indent of whatever row the projection ended on, so the working indicator
    // pins to the tail without asking which branch put it there.
    let mut tail_indent = 0u8;
    while i < items.len() {
        // The marker ends a run (`agent_run`), so it is never inside one and
        // gets its own top-level row. Handled before the user check because
        // the run that follows it starts at the next item, not at this one.
        if matches!(&items[i], ChatItem::Interrupted) {
            rows.push(RenderRow::at(RowKind::Interrupted(i), false, 0));
            i += 1;
            tail_indent = 0;
            continue;
        }
        if matches!(&items[i], ChatItem::UserText(_)) {
            rows.push(RenderRow::at(RowKind::User(i), false, 0));
            i += 1;
        }

        let run = response_run(items, i);
        i = run.end;

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
                        // Only when the turn has more than one tool run. With a
                        // single run the bar directly below says the same thing,
                        // and two bars repeating one tally reads as a rendering
                        // fault rather than as a summary — so the turn bar falls
                        // back to previewing the prose, as it does for a turn
                        // that called nothing at all.
                        categories: if TranscriptStructure::new(items, hierarchy)
                            .top_level_tool_runs(run.clone())
                            > 1
                        {
                            tally_categories(run.clone().filter_map(|k| match &items[k] {
                                ChatItem::ToolCall(tc) if !hierarchy.is_nested_child(tc) => {
                                    Some(tc)
                                }
                                _ => None,
                            }))
                        } else {
                            Vec::new()
                        },
                        collapsed,
                        // Back-patched once the run walk knows what it dropped.
                        filtered: FilteredAway::default(),
                    },
                    false,
                    0,
                )
                .with_filter_revealed(filter_revealed),
            );
            let projector = RunProjector::new(
                context,
                RunSpec {
                    bar_ix: Some(bar_ix),
                    run: run.clone(),
                    base_indent: 1,
                    response_collapsed: collapsed,
                    last_prose,
                    filter_revealed,
                },
                &mut rows,
            );
            projector.project();
            1u8
        } else {
            let projector = RunProjector::new(
                context,
                RunSpec {
                    bar_ix: None,
                    run: run.clone(),
                    base_indent: 0,
                    response_collapsed: false,
                    last_prose,
                    filter_revealed,
                },
                &mut rows,
            );
            projector.project();
            0u8
        };

        tail_indent = run_indent;
    }
    // One emission site, after the walk: the indicator marks the live tail of
    // the conversation whatever kind of row ended it. Gating it on "this run
    // consumed the last item" instead lost the row entirely once a trailing
    // Stop marker could follow the run.
    if awaiting_response {
        rows.push(RenderRow::at(RowKind::WorkingIndicator, false, tail_indent));
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
    tail: StepWindow,
    filter: &'a FilterMatchIndex,
}

impl<'a> ProjectionContext<'a> {
    /// The boundary rules over this pass's items and hierarchy. Two borrows —
    /// the hierarchy is built once per pass, never per query.
    fn structure(&self) -> TranscriptStructure<'a> {
        TranscriptStructure::new(self.items, self.hierarchy)
    }
}

struct RunSpec {
    /// Index of this run's response bar, when it has one. The filter tally is
    /// written back onto it once the walk knows what it dropped.
    bar_ix: Option<usize>,
    run: std::ops::Range<usize>,
    base_indent: u8,
    response_collapsed: bool,
    last_prose: Option<LastProse>,
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
}

impl<'a> RunRows<'a> {
    /// A group header's children, one indent deeper. Identical for every group
    /// kind, so extracting it is what keeps the tool and thinking branches from
    /// drifting on the fold and filter terms. `structural` is the enclosing
    /// fold and the group's own collapse together — the children cannot tell
    /// them apart.
    fn push_group_children(
        &mut self,
        context: ProjectionContext<'_>,
        calls: impl Iterator<Item = usize>,
        structural: bool,
        indent: u8,
        group: GroupFilter,
        window: GroupWindow,
    ) {
        let items = context.items;
        let filter = context.filter;
        for j in calls {
            let kind = RowKind::AgentItem(j);
            let filtered = !filter.matches(&items[j]);
            // A running call stays on screen through its group's shut boundary,
            // the same escape a live run gets from the response's.
            let live = matches!(
                &items[j],
                ChatItem::ToolCall(tc) if tool_or_subtree_live(tc, context.live_units)
            );
            let structural = structural || (window.withholds(j) && !live);
            match group {
                GroupFilter::Kept => self.push(kind, structural, filtered, indent + 1),
                // The header already stands for the whole cut; tallying the
                // calls under it would count the same thing twice.
                GroupFilter::Emptied => self.emit(kind, structural, filtered, indent + 1),
            }
        }
    }

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
        }
    }

    /// Push a row and, when the filter rejected it, tally it as one block the
    /// reveal brings back.
    fn push(&mut self, kind: RowKind, structural: bool, filtered: bool, indent: u8) {
        if filtered {
            self.filtered.revealable += 1;
        }
        self.emit(kind, structural, filtered, indent);
    }

    /// Push a row the tally already covers through the group header above it.
    fn emit(&mut self, kind: RowKind, structural: bool, filtered: bool, indent: u8) {
        self.rows.push(RenderRow {
            kind,
            hidden: structural || (filtered && !self.revealed),
            indent,
            filter_revealed: self.revealed,
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
/// Prose after the final tool call is the response's conclusion, and it gets
/// the chrome that names it. Prose the agent wrote before work it went on to do
/// is a preamble, not an answer. Both stay on screen through an enclosing fold,
/// which is what conflating them was really buying — a collapsed response would
/// otherwise show nothing of what the agent just said.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LastProse {
    Conclusion(usize),
    Preamble(usize),
}

impl LastProse {
    fn of(items: &[ChatItem], run: std::ops::Range<usize>) -> Option<Self> {
        let ix = run.clone().rev().find(|&k| {
            matches!(items[k], ChatItem::AssistantText { .. }) && !is_bodyless(&items[k])
        })?;
        Some(if (ix + 1..run.end).any(|j| is_tool_call(&items[j])) {
            Self::Preamble(ix)
        } else {
            Self::Conclusion(ix)
        })
    }

    /// The item, whichever role it plays — both stay visible.
    fn ix(self) -> usize {
        match self {
            Self::Conclusion(ix) | Self::Preamble(ix) => ix,
        }
    }
}

/// What the tail window makes of one sequence of units — a response's top-level
/// tool runs, or the calls of a single tool group. Both row levels ask the same
/// question of the same axis, so they share the arithmetic rather than each
/// deciding what "the last N" means.
///
/// The third level — a subagent card's children ([`subagent`]) —
/// deliberately does not come through here: its children own no row, so there
/// is no `window_start` item index to hand back and no filter cut to tally, and
/// it reads the call level's [`TailWindow`] directly instead.
///
/// `kept` is the window's population: counting every unit let one the filter
/// emptied spend a slot, so `Recent steps: 3` put however many of the last three
/// happened to survive on screen. `total` decides only whether the boundary row
/// exists at all, which must not move with the filter or the row would change
/// list slots as the filter changes.
#[derive(Clone, Copy)]
struct UnitWindow {
    total: usize,
    kept: usize,
    /// Units the boundary row holds back — `kept` minus what the window shows.
    hidden: usize,
    /// First item the window keeps: the end of the last covered unit. The window
    /// is a range, not a per-row test, so the prose a covered run was introduced
    /// by goes behind the boundary with it while the conclusion, which follows
    /// every run, never does.
    window_start: usize,
}

impl UnitWindow {
    /// `units` yields one entry per unit in transcript order: where it ends,
    /// and whether the filter leaves it anything to show. Taken as a `Clone`
    /// iterator rather than a slice so a level whose units are already a range
    /// — a group's calls — needs no allocation to describe them.
    fn of_units(
        units: impl Iterator<Item = (usize, bool)> + Clone,
        start: usize,
        tail: TailWindow,
    ) -> Self {
        let kept = units.clone().filter(|(_, kept)| *kept).count();
        // Position within the kept population. An emptied unit takes the
        // position of the next rendering one rather than a slot of its own, so
        // it cannot shift the window. Positions only rise, so the covered units
        // are a prefix and the last one's end is where the kept range begins.
        let mut total = 0;
        let mut position = 0;
        let mut window_start = start;
        for (end, unit_kept) in units {
            total += 1;
            if tail.hides(position, kept) {
                window_start = end;
            }
            position += usize::from(unit_kept);
        }
        Self {
            total,
            kept,
            hidden: tail.hidden_steps(kept),
            window_start,
        }
    }

    /// The response's own window: one unit per top-level tool run.
    fn over_tool_runs(run: std::ops::Range<usize>, context: ProjectionContext<'_>) -> Self {
        let items = context.items;
        let mut units: Vec<(usize, bool)> = Vec::new();
        let mut k = run.start;
        while k < run.end {
            if !context.structure().top_level_tool(k) {
                k += 1;
                continue;
            }
            let span = context.structure().tool_run(k, run.end);
            k = span.end;
            // The span's own calls decide, not everything it covers: an item
            // the run only passes over is not what the run has to show.
            let shows = context
                .structure()
                .group_calls(span.clone())
                .any(|j| context.filter.matches(&items[j]));
            units.push((span.end, shows));
        }
        Self::of_units(units.iter().copied(), run.start, context.tail.steps)
    }

    /// One group's window: one unit per call it holds — the items it spans that
    /// own no row are not units, so they cannot spend a slot.
    fn over_group_calls(calls: &[usize], start: usize, context: ProjectionContext<'_>) -> Self {
        Self::of_units(
            calls
                .iter()
                .map(move |&j| (j + 1, context.filter.matches(&context.items[j]))),
            start,
            context.tail.calls,
        )
    }

    /// Whether the item at `ix` sits before the window's kept range.
    fn covers(self, ix: usize) -> bool {
        ix < self.window_start
    }
}

/// What the step axis makes of one group's children.
#[derive(Clone, Copy)]
enum GroupWindow {
    /// A group the axis does not divide. Reasoning groups take this: a stretch
    /// of thoughts is one step's reasoning, not a run of steps, so the step
    /// axis has nothing to count inside it.
    Undivided,
    /// A tool group, whose calls are the units — with whether its own boundary
    /// row is open. The two travel together because a covered child's
    /// visibility is both answers at once.
    Divided { cut: UnitWindow, revealed: bool },
}

impl GroupWindow {
    /// A covered child the boundary is still holding back. Liveness is the
    /// caller's term: a running call stays surfaced through a shut boundary,
    /// exactly as a live run does one level up.
    fn withholds(self, ix: usize) -> bool {
        match self {
            Self::Undivided => false,
            Self::Divided { cut, revealed } => !revealed && cut.covers(ix),
        }
    }
}

/// Blocks the filter took from inside the run's tool cards. A nested child
/// owns no row, so the row walk cannot tally it — but the run's reveal admits
/// the whole card back, descendants included, so the number that reveal offers
/// has to name them or the cut is unreachable.
///
/// Summed over the run's *kept* top-level calls only: a call the filter dropped
/// is already one block on the tally and takes its card with it.
fn cut_inside_cards(context: ProjectionContext<'_>, run: std::ops::Range<usize>) -> usize {
    run.filter_map(|ix| match &context.items[ix] {
        ChatItem::ToolCall(tc)
            if !context.hierarchy.is_nested_child(tc) && context.filter.keeps_tool(tc) =>
        {
            Some(
                context
                    .hierarchy
                    .cut_below(tc.id.as_str(), |id| context.filter.keeps_id(id)),
            )
        }
        _ => None,
    })
    .sum()
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

    fn project(self) {
        let context = self.context;
        let items = self.context.items;
        let fold = self.context.fold;
        let boundary = self.context.boundary;
        let hierarchy = self.context.hierarchy;
        let live_units = self.context.live_units;
        let filter = self.context.filter;
        let run = self.spec.run.clone();
        let base_indent = self.spec.base_indent;
        let response_collapsed = self.spec.response_collapsed;
        let last_prose = self.spec.last_prose;
        if run.is_empty() {
            return;
        }

        let tail_key = FoldKey::Tail(run.start);
        let tail_revealed = fold.is_expanded(
            &tail_key,
            fold_context_at(&tail_key, run.start, items, boundary),
        );
        let window = UnitWindow::over_tool_runs(run.clone(), context);
        let mut out = RunRows::new(
            self.output,
            self.spec.bar_ix,
            run.len(),
            self.spec.filter_revealed,
        );
        if window.total > 0 {
            out.push(
                RowKind::TailMore {
                    run_start: run.start,
                    hidden_steps: window.hidden,
                    kept_steps: window.kept - window.hidden,
                    collapsed: !tail_revealed,
                },
                response_collapsed || window.hidden == 0,
                false,
                base_indent,
            );
        }

        let mut k = run.start;
        while k < run.end {
            if !context.structure().owns_a_row(k) {
                k += 1;
                continue;
            }
            // A live covered run stays surfaced through a shut boundary, so the
            // window's range decides the fold and the boundary's state gates it.
            let covered = k < window.window_start;
            let folded = response_collapsed || (!tail_revealed && covered);
            if TranscriptStructure::new(items, hierarchy).top_level_tool(k) {
                // Every run earns a header, one call included: a turn's shape
                // must not change with how many calls happened to land next to
                // each other, and the bar is where the run's fold and its
                // summary live. A run always holds at least the call that
                // started it, so there is no shorter case to branch on.
                let structure = TranscriptStructure::new(items, hierarchy);
                let grun = structure.tool_run(k, run.end);
                k = grun.end;
                // Resolved once: past this line the group is its calls, and
                // `grun` is only the walk's cursor. Reading the span where a
                // member was meant is what put a nested child in two tallies.
                let calls: Vec<usize> = structure.group_calls(grun.clone()).collect();
                let group = GroupFilter::of(calls.iter().copied(), items, filter);
                let group_live = run_is_live(items, calls.iter().copied(), live_units);
                {
                    let gid = tool_id(&items[grun.start]);
                    let group_key = FoldKey::ToolGroup(gid.clone());
                    // A run of one: collapsing would leave the bar standing over
                    // nothing, so its default keeps the call on screen. The bar
                    // and its fold still exist — a deliberate fold still shuts it.
                    //
                    // Liveness is read off the members this walk resolved.
                    // `fold_context_at` would rescan from `grun.start` without
                    // the hierarchy, so a nested child running inside one of
                    // these cards would read as a member.
                    let group_active = calls.iter().any(|&k| is_active(&items[k]));
                    let group_collapsed = !fold.is_expanded(
                        &group_key,
                        FoldContext::new(boundary.at(grun.start), group_active),
                    ) && (calls.len() > 1 || fold.is_overridden(&group_key));
                    let group_tail_key = FoldKey::ToolGroupTail(gid.clone());
                    let group_tail_revealed = fold.is_expanded(
                        &group_tail_key,
                        fold_context_at(&group_tail_key, grun.start, items, boundary),
                    );
                    let group_cut = UnitWindow::over_group_calls(&calls, grun.start, context);
                    out.push(
                        RowKind::ToolGroupHeader {
                            gid: gid.clone(),
                            calls: calls.clone(),
                            collapsed: group_collapsed,
                        },
                        folded && !group_live,
                        group.hides_the_header(),
                        base_indent,
                    );
                    // Sits with the children it holds back, one indent in from
                    // the header — the same relation the response's boundary
                    // has to the run's blocks.
                    out.push(
                        RowKind::ToolGroupTailMore {
                            gid,
                            hidden_calls: group_cut.hidden,
                            kept_calls: group_cut.kept - group_cut.hidden,
                            collapsed: !group_tail_revealed,
                        },
                        folded || group_collapsed || group_cut.hidden == 0,
                        false,
                        base_indent + 1,
                    );
                    out.push_group_children(
                        context,
                        calls.into_iter(),
                        folded || group_collapsed,
                        base_indent,
                        group,
                        GroupWindow::Divided {
                            cut: group_cut,
                            revealed: group_tail_revealed,
                        },
                    );
                }
            } else if matches!(&items[k], ChatItem::Thinking { .. }) {
                let gstart = k;
                k += 1;
                // An empty streaming chunk gets no row of its own, so letting it
                // join a group would render a blank child and inflate the count.
                while k < run.end
                    && matches!(&items[k], ChatItem::Thinking { .. })
                    && !is_bodyless(&items[k])
                {
                    k += 1;
                }
                let grun = gstart..k;
                {
                    let group_key = FoldKey::ThinkingGroup(gstart);
                    let group_collapsed = !fold.is_expanded(
                        &group_key,
                        fold_context_at(&group_key, gstart, items, boundary),
                    );
                    let group = GroupFilter::of(grun.clone(), items, filter);
                    out.push(
                        RowKind::ThinkingGroupHeader {
                            first_ix: gstart,
                            count: grun.len(),
                            collapsed: group_collapsed,
                        },
                        folded,
                        group.hides_the_header(),
                        base_indent,
                    );
                    out.push_group_children(
                        context,
                        grun,
                        folded || group_collapsed,
                        base_indent,
                        group,
                        GroupWindow::Undivided,
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
                let force_visible = is_last_prose || pending_permission;
                let kind = if is_conclusion && base_indent > 0 {
                    RowKind::ConclusionItem(k)
                } else {
                    RowKind::AgentItem(k)
                };
                out.push(
                    kind,
                    folded && !force_visible,
                    !filter.matches(&items[k]),
                    base_indent,
                );
                k += 1;
            }
        }
        out.filtered.revealable += cut_inside_cards(context, run.clone());
        out.finish();
    }
}

fn is_tool_call(item: &ChatItem) -> bool {
    matches!(item, ChatItem::ToolCall(_))
}

/// A message carrying no renderable text. Two sources feed it: a message's
/// leading chunk arrives empty, and `daruda_acp` collapses a content block it
/// cannot render (image, audio, resource) to an empty string. Neither earns a
/// row, a block slot in the response threshold, or the conclusion —
/// which escapes its enclosing fold and would pin a blank row over it.
pub(super) fn is_bodyless(item: &ChatItem) -> bool {
    matches!(
        item,
        ChatItem::AssistantText { text, .. } | ChatItem::Thinking { text, .. }
            if text.trim().is_empty()
    )
}

/// Whether any call in `run` is live — the run-wide reading of
/// [`tool_or_subtree_live`].
///
/// A group *header* speaks for its whole run, so it escapes an enclosing fold
/// while any member still works. A group *child* asks the per-call question
/// instead, and escapes only its own group's window rather than the fold above
/// it (see `RunRows::push_group_children`). The two granularities answer for
/// different subjects; they are not one rule stated twice.
fn run_is_live(
    items: &[ChatItem],
    calls: impl Iterator<Item = usize>,
    live_units: &LiveSubagentUnits,
) -> bool {
    calls.into_iter().any(
        |j| matches!(&items[j], ChatItem::ToolCall(tc) if tool_or_subtree_live(tc, live_units)),
    )
}

fn tool_id(item: &ChatItem) -> String {
    match item {
        ChatItem::ToolCall(tc) => tc.id.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests;

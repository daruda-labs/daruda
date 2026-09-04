//! Projects the flat chat model into stable virtual-list rows. Folding changes
//! `hidden` flags instead of removing rows so scroll positions remain stable.

pub(in crate::workspace) mod subagent;
pub(in crate::workspace) mod tail;

use std::collections::HashSet;

use daruda_acp::{ChatItem, ToolCallItem, ToolStatusView};

use super::agent_chat_helpers::{TurnBoundary, agent_run, fold_context_at};
use super::fold::{FoldKey, FoldState};
use super::tool_hierarchy::ToolHierarchy;
use crate::transcript::display_filter::DisplayFilter;
use tail::TailWindow;

/// Minimum consecutive same-kind items that earn a group header. Governs tool
/// runs and thinking runs alike, so the two thresholds cannot drift apart.
const RUN_GROUP_MIN: usize = 2;

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
    fn of(run: std::ops::Range<usize>, items: &[ChatItem], filter: &FilterMatchIndex) -> Self {
        if run.into_iter().any(|j| filter.matches(&items[j])) {
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
        first_ix: usize,
        count: usize,
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
    /// The row sits outside the tail window's kept range. Distinct from
    /// `indent`, which is structural nesting: this says the row does not belong
    /// to the range the pane is showing, and the renderer answers it with the
    /// rail tying the row back to the boundary above it.
    ///
    /// Deliberately *not* "the boundary revealed it": a live covered tool run
    /// stays surfaced through a shut boundary, and keying the mark on the
    /// boundary made that row gain and lose its rail as the boundary flipped.
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

        let run = agent_run(items, i);
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
    /// Applied to every row the walk pushes while it sits before the tail
    /// window's start — prose and thinking as much as tool calls, since the
    /// window covers a range rather than testing each row. Pusher state rather
    /// than a `push` parameter: it changes once per position, not once per row,
    /// and the call sites below would each have to restate it.
    outside_window: bool,
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
        run: std::ops::Range<usize>,
        structural: bool,
        indent: u8,
        group: GroupFilter,
        window: GroupWindow,
    ) {
        let items = context.items;
        let filter = context.filter;
        let enclosing = self.outside_window;
        for j in run {
            let kind = RowKind::AgentItem(j);
            let filtered = !filter.matches(&items[j]);
            // A running call stays on screen through its group's shut boundary,
            // the same escape a live run gets from the response's. The rail
            // still marks it, because coverage is what the rail reports.
            let live = matches!(
                &items[j],
                ChatItem::ToolCall(tc) if tool_or_subtree_live(tc, context.live_units)
            );
            self.outside_window = enclosing || window.covers(j);
            let structural = structural || (window.withholds(j) && !live);
            match group {
                GroupFilter::Kept => self.push(kind, structural, filtered, indent + 1),
                // The header already stands for the whole cut; tallying the
                // calls under it would count the same thing twice.
                GroupFilter::Emptied => self.emit(kind, structural, filtered, indent + 1),
            }
        }
        self.outside_window = enclosing;
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
            outside_window: false,
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
/// The third level — a subagent card's children ([`super::subagent`]) —
/// deliberately does not come through here: its children own no row, so there
/// is no `window_start` item index to hand back and no filter cut to tally, and
/// it reads [`TailWindow`] directly instead.
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
            if !top_level_tool(items, k, context.hierarchy) {
                k += 1;
                continue;
            }
            let span = k..tool_run_end(items, k, run.end, context.hierarchy);
            k = span.end;
            units.push((
                span.end,
                span.clone().any(|j| context.filter.matches(&items[j])),
            ));
        }
        Self::of_units(units.iter().copied(), run.start, context.tail)
    }

    /// One group's window: one unit per call it holds. A group is a contiguous
    /// range, so its units are derived on the fly.
    fn over_group_calls(group: std::ops::Range<usize>, context: ProjectionContext<'_>) -> Self {
        let start = group.start;
        Self::of_units(
            group.map(move |j| (j + 1, context.filter.matches(&context.items[j]))),
            start,
            context.tail,
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
    fn covers(self, ix: usize) -> bool {
        match self {
            Self::Undivided => false,
            Self::Divided { cut, .. } => cut.covers(ix),
        }
    }

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

/// End of the maximal stretch of consecutive top-level tool calls beginning at
/// `start`. The row walk and the window's tally both advance by this, so they
/// cannot disagree about where a run ends. A nested child renders inside its
/// parent's card, so it breaks a run rather than joining it.
fn tool_run_end(
    items: &[ChatItem],
    start: usize,
    limit: usize,
    hierarchy: &ToolHierarchy<'_>,
) -> usize {
    let mut k = start + 1;
    while k < limit && top_level_tool(items, k, hierarchy) {
        k += 1;
    }
    k
}

/// Whether this call earns a row of its own.
///
/// This is the transcript's row boundary, and **both narrowing axes stop at
/// it**: a nested subagent child never becomes a [`RenderRow`], so neither the
/// step window nor the display filter — whose unit is a projected row — can
/// reach inside the card that renders it. The other half of the rule lives in
/// [`FilterMatchIndex::build`], which keeps every descendant of a kept tool for
/// the same reason, and the card walks the hierarchy itself
/// (`render/tool.rs`). Narrowing one axis into a card while the other stays out
/// is the divergence this states out loud; `no_axis_narrows_inside_a_tool_card`
/// pins it.
fn top_level_tool(items: &[ChatItem], ix: usize, hierarchy: &ToolHierarchy<'_>) -> bool {
    matches!(&items[ix], ChatItem::ToolCall(tc) if !hierarchy.is_nested_child(tc))
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
        let sole_block = self.spec.sole_block;
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
            if is_bodyless(&items[k])
                || matches!(&items[k], ChatItem::ToolCall(tool) if hierarchy.is_nested_child(tool))
            {
                k += 1;
                continue;
            }
            // One assignment point, so the flag cannot drift. The rail marks
            // coverage itself rather than the boundary's state — a live covered
            // run stays surfaced through a shut boundary, and keying the mark on
            // the boundary made the row gain and lose it as the boundary flipped.
            out.outside_window = k < window.window_start;
            let folded = response_collapsed || (!tail_revealed && out.outside_window);
            if top_level_tool(items, k, hierarchy) {
                let grun = k..tool_run_end(items, k, run.end, hierarchy);
                k = grun.end;
                let group = GroupFilter::of(grun.clone(), items, filter);
                let group_live = run_is_live(items, grun.clone(), live_units);
                if grun.len() >= RUN_GROUP_MIN {
                    let gid = tool_id(&items[grun.start]);
                    let group_key = FoldKey::ToolGroup(gid.clone());
                    let group_collapsed = !fold.is_expanded(
                        &group_key,
                        fold_context_at(&group_key, grun.start, items, boundary),
                    );
                    let group_tail_key = FoldKey::ToolGroupTail(gid.clone());
                    let group_tail_revealed = fold.is_expanded(
                        &group_tail_key,
                        fold_context_at(&group_tail_key, grun.start, items, boundary),
                    );
                    let group_cut = UnitWindow::over_group_calls(grun.clone(), context);
                    out.push(
                        RowKind::ToolGroupHeader {
                            gid: gid.clone(),
                            first_ix: grun.start,
                            count: grun.len(),
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
                        grun,
                        folded || group_collapsed,
                        base_indent,
                        group,
                        GroupWindow::Divided {
                            cut: group_cut,
                            revealed: group_tail_revealed,
                        },
                    );
                } else {
                    out.push(
                        RowKind::AgentItem(grun.start),
                        folded && !group_live,
                        group.hides_the_header(),
                        base_indent,
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
                if grun.len() >= RUN_GROUP_MIN {
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
                } else {
                    out.push(
                        RowKind::AgentItem(gstart),
                        folded,
                        !filter.matches(&items[gstart]),
                        base_indent,
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
                let kind = if is_conclusion && base_indent > 0 && !sole_block {
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
    run: std::ops::Range<usize>,
    live_units: &LiveSubagentUnits,
) -> bool {
    run.into_iter().any(
        |j| matches!(&items[j], ChatItem::ToolCall(tc) if tool_or_subtree_live(tc, live_units)),
    )
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

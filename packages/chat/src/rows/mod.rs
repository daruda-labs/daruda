//! Projects the flat chat model into stable virtual-list rows. Folding changes
//! `hidden` flags instead of removing rows so scroll positions remain stable.

pub(super) mod subagent;
pub mod tail;

use std::collections::HashSet;

use daruda_acp::{ChatItem, ToolCallItem};

use super::agent_chat_helpers::{TurnBoundary, fold_context_at};
use super::fold::{FoldContext, FoldKey, FoldState};
use super::tool_hierarchy::ToolHierarchy;
use super::transcript_structure::{TranscriptStructure, response_run};
// Re-exported so the many callers that reach these through `rows` keep one
// path; `tool_status` is the definition site and what breaks the cycle with
// `agent_chat_helpers`.
pub(crate) use super::tool_status::LiveSubagentUnits;
pub(crate) use super::tool_status::{effective_tool_status, tool_or_subtree_live};
use daruda_transcript::display_filter::DisplayFilter;
use daruda_transcript::tool_category::{ToolCategory, tally_categories};
use tail::{StepWindow, TailWindow};

pub(super) mod foldable_keys;
pub(crate) use foldable_keys::collect_foldable_keys;
mod run_projector;
use run_projector::RunProjector;

/// What the display filter dropped from one run and the reveal can put back.
///
/// The unit is a block, not a row: a group the filter empties counts once,
/// because the reveal brings back the group and its calls come with it. Folds
/// are deliberately not consulted — a group's fold flips on its own as its last
/// call settles, and letting that move the number made it climb to the group's
/// size and drop back mid-turn, reporting one cut two ways seconds apart.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) struct FilteredAway {
    /// Blocks the filter took out of this run.
    pub(super) revealable: usize,
}

impl FilteredAway {
    /// Whether the bar carries a reveal chip.
    ///
    /// The tally describes the filter alone, so the bar's own collapse must not
    /// be checked on top of it. The conclusion's `force_visible` escape is
    /// exactly the row that survives a collapsed response and can still be
    /// filtered out of it, leaving the turn showing nothing but its bar; a
    /// second collapse check erased the one control that leads back.
    pub(super) fn offers_reveal(self) -> bool {
        self.revealable > 0
    }
}

/// Whether the filter left a group anything to show. Decides what the group
/// contributes to the tally: the group itself, or the calls taken from it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum GroupFilter {
    /// At least one member — a call or a thought — survives, so the group is
    /// on screen and what the reveal brings back is each member it took.
    Kept,
    /// Every member is rejected, so the group is what the reveal brings back
    /// and its members are already covered by it.
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
pub(crate) enum RowKind {
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
    /// A thought inside a tool group. Not an [`RowKind::AgentItem`]: the group
    /// bar already says what it is, so it drops the label and fold a
    /// free-standing thought carries.
    GroupThought(usize),
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
        /// The thoughts after the group's first call, in transcript order. Kept
        /// apart from `calls`: the bar tallies and rolls up calls, and only
        /// counts these.
        thoughts: Vec<usize>,
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

pub(crate) struct RenderRow {
    pub(crate) kind: RowKind,
    /// Hidden rows stay in the sequence to preserve slot stability.
    pub(crate) hidden: bool,
    pub(crate) indent: u8,
    /// The per-run filter disclosure is open, so rows rejected by the active
    /// display filter are visible again. Header counts, rollups, and nested tool
    /// cards must all use this same answer as the row projection.
    pub(super) filter_revealed: bool,
}

impl RenderRow {
    pub(crate) fn at(kind: RowKind, hidden: bool, indent: u8) -> Self {
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
/// Every call answers for its own category however deeply it nests, and a match
/// drags its ancestors in — so narrowing to one leaves a subagent card standing
/// with only that category's calls inside it, reachable through the card.
#[derive(Default)]
pub(super) struct FilterMatchIndex {
    filter: DisplayFilter,
    tool_ids: HashSet<String>,
}

impl FilterMatchIndex {
    pub(super) fn build<'a>(
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
    pub(crate) fn of(items: &[ChatItem], filter: DisplayFilter) -> Self {
        Self::build(&ToolHierarchy::build(items), items, filter)
    }

    pub(crate) fn matches(&self, item: &ChatItem) -> bool {
        match item {
            ChatItem::ToolCall(tc) => self.keeps_tool(tc),
            _ => self.filter.matches(item),
        }
    }

    pub(crate) fn keeps_tool(&self, tc: &ToolCallItem) -> bool {
        self.keeps_id(&tc.id)
    }

    pub(crate) fn keeps_id(&self, id: &str) -> bool {
        self.tool_ids.contains(id)
    }
}

/// Stable identity of a projected row: the key `rebuild_rows`' diff compares to
/// decide whether two projections put the same thing in the same list slot.
/// Deliberately carries no payload and no `hidden` flag — those change freely
/// within one slot.
#[derive(PartialEq, Eq)]
pub(super) enum RowSlot<'a> {
    User(usize),
    Interrupted(usize),
    Response(usize),
    AgentItem(usize),
    GroupThought(usize),
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
            RowKind::GroupThought(ix) => RowSlot::GroupThought(*ix),
            RowKind::TailMore { run_start, .. } => RowSlot::TailMore(*run_start),
            RowKind::ToolGroupTailMore { gid, .. } => RowSlot::ToolGroupTail(gid.as_str()),
            RowKind::ToolGroupHeader { gid, .. } => RowSlot::ToolGroup(gid.as_str()),
            RowKind::ThinkingGroupHeader { first_ix, .. } => RowSlot::ThinkingGroup(*first_ix),
            RowKind::ConclusionItem(ix) => RowSlot::Conclusion(*ix),
            RowKind::WorkingIndicator => RowSlot::Working,
        }
    }
}

/// The row that renders item `ix` as a block of its own, which a fold toggle on
/// that item remeasures in place. `None` for an item that owns no such row (a
/// group bar, a nested child).
pub(crate) fn item_row(rows: &[RenderRow], ix: usize) -> Option<usize> {
    rows.iter().position(|r| {
        matches!(
            r.kind,
            RowKind::AgentItem(i) | RowKind::GroupThought(i) | RowKind::ConclusionItem(i)
                if i == ix
        )
    })
}

impl RenderRow {
    /// Compare stable row identity, ignoring visibility and payload changes.
    pub(crate) fn same_slot(&self, other: &Self) -> bool {
        self.kind.slot() == other.kind.slot()
    }
}

/// Project chat items into stable rows.
pub(crate) fn project(
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
pub(super) fn project_with_filter_index<'a>(
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
        if matches!(&items[i], ChatItem::Compaction(_)) {
            rows.push(RenderRow::at(RowKind::AgentItem(i), false, 0));
            i += 1;
            tail_indent = 0;
            continue;
        }
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
        let blocks = run
            .clone()
            .filter(|&k| !is_bodyless(&items[k]) && !matches!(items[k], ChatItem::Compaction(_)))
            .count();
        let last_prose = LastProse::of(items, run.clone());
        let filter_key = FoldKey::Filtered(run.start);
        let filter_revealed = fold.is_expanded(
            &filter_key,
            fold_context_at(&filter_key, run.start, items, boundary, live_units),
        );

        // Every response that renders anything gets a bar — it is where the
        // filter's reveal control lives, and a turn without one has nowhere to
        // put it. Only a run with nothing on screen (an empty reply) is skipped.
        let renders_something = tools >= 1 || blocks >= 1;
        let run_indent = if renders_something {
            let key = FoldKey::Response(run.start);
            let collapsed = !fold.is_expanded(
                &key,
                fold_context_at(&key, run.start, items, boundary, live_units),
            );
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

#[cfg(test)]
mod census;
#[cfg(test)]
mod tests;

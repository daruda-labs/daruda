//! Row-count regression tests using captured shapes, plus the projection
//! invariants both they and an optional live-log replay are held to.
//!
//! Two kinds of claim live here. [`pinned`] fixes what a *known* conversation
//! costs, so a change that inflates the transcript has to restate the number.
//! [`invariant_violations`] fixes what *no* conversation may do, so a shape no
//! fixture was transcribed from is still covered — [`live_log`] runs it against
//! a real capture.

mod fixture;
mod live_log;
mod pinned;

use daruda_acp::ChatItem;

use crate::transcript::display_filter::DisplayFilter;
use crate::transcript::fold_mode::{FoldMode, FoldPreset};
use crate::workspace::main_area::agent_chat_pane::fold::FoldState;
use crate::workspace::main_area::agent_chat_pane::rows::tail::{StepWindow, TailWindow};
use crate::workspace::main_area::agent_chat_pane::rows::{
    LiveSubagentUnits, RenderRow, RowKind, project,
};

/// The recent-steps level the census measures at — the budgets in `pinned` and
/// the monotonicity check in [`invariant_violations`] probe the same window.
const TAIL_N: u8 = 5;

/// Projection settings used by the census.
#[derive(Clone, Copy)]
struct Lens {
    mode: FoldMode,
    tail: StepWindow,
    filter: DisplayFilter,
}

impl Lens {
    fn preset(preset: FoldPreset) -> Self {
        Self {
            mode: preset.mode(),
            tail: StepWindow::default(),
            filter: DisplayFilter::default(),
        }
    }

    /// Both levels of the recent-steps axis at once — the census measures the
    /// window as a user sets it, not one level in isolation.
    fn tail(self, tail: TailWindow) -> Self {
        Self {
            tail: StepWindow::uniform(tail),
            ..self
        }
    }

    fn filter(self, filter: DisplayFilter) -> Self {
        Self { filter, ..self }
    }
}

fn rows(items: &[ChatItem], lens: Lens) -> Vec<RenderRow> {
    let live = LiveSubagentUnits::of(items);
    project(
        items,
        &FoldState::with_mode(lens.mode),
        false,
        &live,
        lens.tail,
        &lens.filter,
    )
}

fn turn_bounds(items: &[ChatItem]) -> Vec<usize> {
    let mut out: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, it)| matches!(it, ChatItem::UserText(_)))
        .map(|(i, _)| i)
        .collect();
    out.push(items.len());
    out
}

/// Count visible rows under each preceding user anchor.
fn visible_per_turn(rows: &[RenderRow]) -> Vec<usize> {
    let mut per: Vec<usize> = Vec::new();
    for r in rows {
        if matches!(r.kind, RowKind::User(_)) {
            per.push(0);
        }
        if !r.hidden
            && let Some(last) = per.last_mut()
        {
            *last += 1;
        }
    }
    per
}

/// Count each turn as it looked while it was the newest turn.
fn per_turn_as_last(items: &[ChatItem], lens: Lens) -> Vec<usize> {
    turn_bounds(items)
        .windows(2)
        .map(|w| {
            let head = &items[..w[1]];
            visible_per_turn(&rows(head, lens)).pop().unwrap_or(0)
        })
        .collect()
}

fn per_turn_settled(items: &[ChatItem], lens: Lens) -> Vec<usize> {
    visible_per_turn(&rows(items, lens))
}

/// Total visible rows, the scalar the axis comparisons below order by.
fn visible_total(items: &[ChatItem], lens: Lens) -> usize {
    rows(items, lens).iter().filter(|r| !r.hidden).count()
}

/// Row kind as a violation message names it. `RowKind` carries no `Debug`, and
/// a failure has to say which row it found, not just where.
fn kind_label(kind: &RowKind) -> &'static str {
    match kind {
        RowKind::User(_) => "User",
        RowKind::Interrupted(_) => "Interrupted",
        RowKind::ResponseHeader { .. } => "ResponseHeader",
        RowKind::AgentItem(_) => "AgentItem",
        RowKind::TailMore { .. } => "TailMore",
        RowKind::ToolGroupTailMore { .. } => "ToolGroupTailMore",
        RowKind::ToolGroupHeader { .. } => "ToolGroupHeader",
        RowKind::ThinkingGroupHeader { .. } => "ThinkingGroupHeader",
        RowKind::ConclusionItem(_) => "ConclusionItem",
        RowKind::WorkingIndicator => "WorkingIndicator",
    }
}

/// Highest item index a row refers to, or `None` for the rows that name no item.
fn row_reach(kind: &RowKind) -> Option<usize> {
    match kind {
        RowKind::User(i)
        | RowKind::Interrupted(i)
        | RowKind::AgentItem(i)
        | RowKind::ConclusionItem(i) => Some(*i),
        RowKind::ResponseHeader { run_start, .. } | RowKind::TailMore { run_start, .. } => {
            Some(*run_start)
        }
        // A group header stands for several items, so its reach is the last of
        // them, not the first.
        RowKind::ToolGroupHeader { calls, .. } => calls.last().copied(),
        RowKind::ThinkingGroupHeader {
            first_ix, count, ..
        } => Some(first_ix + count.saturating_sub(1)),
        RowKind::ToolGroupTailMore { .. } | RowKind::WorkingIndicator => None,
    }
}

/// Properties that hold for any transcript, whatever its shape.
///
/// The pinned budgets say what a known conversation costs; these say what can
/// never happen regardless. Fixtures exercise them in CI and a captured log
/// through `live_log`, so a synthetic shape and a real one answer to the same
/// rules. Returns every violation rather than the first, so one run names the
/// whole failure.
fn invariant_violations(items: &[ChatItem]) -> Vec<String> {
    let mut out = Vec::new();

    // A row naming an item past the end would index out of bounds the moment
    // the renderer read it, so the projection must never emit one.
    for (n, row) in rows(items, Lens::preset(FoldPreset::Expanded))
        .iter()
        .enumerate()
    {
        if let Some(reach) = row_reach(&row.kind)
            && reach >= items.len()
        {
            out.push(format!(
                "row {n} ({}) reaches item {reach} of {}",
                kind_label(&row.kind),
                items.len()
            ));
        }
    }

    // The fold presets are a ladder: opening a response can only reveal rows
    // the tighter mode already hid.
    let (summary, auto, expanded) = (
        visible_total(items, Lens::preset(FoldPreset::Summary)),
        visible_total(items, Lens::preset(FoldPreset::Auto)),
        visible_total(items, Lens::preset(FoldPreset::Expanded)),
    );
    if !(summary <= auto && auto <= expanded) {
        out.push(format!(
            "fold ladder out of order: summary {summary}, auto {auto}, expanded {expanded}"
        ));
    }

    // Narrowing the recent-steps window is a cut. It may leave a boundary row
    // behind, but it can never end up showing more than the open window did.
    for preset in FoldPreset::ALL {
        let lens = Lens::preset(preset);
        let all = visible_total(items, lens.tail(TailWindow::All));
        let last = visible_total(items, lens.tail(TailWindow::last(TAIL_N)));
        if last > all {
            out.push(format!(
                "{preset:?}: tail last({TAIL_N}) shows {last} rows, more than All's {all}"
            ));
        }
    }

    // Indent is a nesting depth walked one level at a time: a row may close
    // several levels at once, but a jump inward skips a parent that never
    // rendered.
    let mut prev: Option<u8> = None;
    for (n, row) in rows(items, Lens::preset(FoldPreset::Expanded))
        .iter()
        .filter(|r| !r.hidden)
        .enumerate()
    {
        if let Some(p) = prev
            && row.indent > p + 1
        {
            out.push(format!(
                "row {n} jumps indent {p} -> {} without an intervening level",
                row.indent
            ));
        }
        prev = Some(row.indent);
    }

    out
}

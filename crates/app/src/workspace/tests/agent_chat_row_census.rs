//! Row-count regression tests using captured shapes and an optional live-log
//! replay diagnostic.

mod fixture;
mod live_log;
mod pinned;

use daruda_acp::ChatItem;

use crate::workspace::main_area::agent_chat_pane::display_filter::DisplayFilter;
use crate::workspace::main_area::agent_chat_pane::fold::FoldState;
use crate::workspace::main_area::agent_chat_pane::fold_mode::{FoldMode, FoldPreset};
use crate::workspace::main_area::agent_chat_pane::rows::tail::TailWindow;
use crate::workspace::main_area::agent_chat_pane::rows::{
    LiveSubagentUnits, RenderRow, RowKind, project,
};

/// Projection settings used by the census.
#[derive(Clone, Copy)]
struct Lens {
    mode: FoldMode,
    tail: TailWindow,
    filter: DisplayFilter,
}

impl Lens {
    fn preset(preset: FoldPreset) -> Self {
        Self {
            mode: preset.mode(),
            tail: TailWindow::All,
            filter: DisplayFilter::default(),
        }
    }

    fn tail(self, tail: TailWindow) -> Self {
        Self { tail, ..self }
    }

    fn filter(self, filter: DisplayFilter) -> Self {
        Self { filter, ..self }
    }
}

fn rows(items: &[ChatItem], lens: Lens) -> Vec<RenderRow> {
    let live = LiveSubagentUnits::build(items);
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

//! Row budgets measured from live logs and reproduced by synthetic fixtures.

use daruda_acp::ChatItem;

use super::fixture::{claude_session, codex_session};
use super::{Lens, per_turn_as_last, per_turn_settled, rows, turn_bounds, visible_per_turn};
use crate::workspace::main_area::agent_chat_pane::display_filter::DisplayFilter;
use crate::workspace::main_area::agent_chat_pane::fold::FoldState;
use crate::workspace::main_area::agent_chat_pane::fold_mode::{FoldMode, FoldPreset};
use crate::workspace::main_area::agent_chat_pane::rows::tail::TailWindow;
use crate::workspace::main_area::agent_chat_pane::rows::{LiveSubagentUnits, project};

const TAIL_N: u8 = 5;

/// Codex rows per turn under each projection mode.
///
/// An open response now shows its prose spine and one bar per tool run rather
/// than one titled row per work step, so `AUTO` and `EXPANDED` cost more rows
/// than the step layer did — that is the shape being bought. `SUMMARY` is
/// untouched: a folded response still costs its bar and its conclusion.
const CODEX_AUTO: [usize; 3] = [155, 3, 33];
const CODEX_TAIL: [usize; 3] = [18, 3, 19];
const CODEX_SUMMARY: [usize; 3] = [3, 2, 3];
const CODEX_EXPANDED: [usize; 3] = [377, 3, 57];
const CODEX_SETTLED: [usize; 3] = [3, 2, 33];

/// Claude rows per turn under each projection mode. Turn 0 is the one turn
/// with no tools and a single block; it costs one row for the response bar
/// that carries the filter's reveal chip.
const CLAUDE_AUTO: [usize; 3] = [3, 9, 47];
const CLAUDE_TAIL: [usize; 3] = [3, 9, 14];
const CLAUDE_SUMMARY: [usize; 3] = [3, 3, 3];
const CLAUDE_EXPANDED: [usize; 3] = [3, 70, 79];
const CLAUDE_SETTLED: [usize; 3] = [3, 3, 47];
const CLAUDE_EDITS_ONLY: [usize; 3] = [2, 2, 2];

/// Project through the fresh-pane defaults without a named test lens.
fn shipped_default_per_turn(items: &[ChatItem]) -> Vec<usize> {
    let live = LiveSubagentUnits::of(items);
    visible_per_turn(&project(
        items,
        &FoldState::default(),
        false,
        &live,
        TailWindow::All,
        &DisplayFilter::default(),
    ))
}

fn shipped_default_as_last(items: &[ChatItem]) -> Vec<usize> {
    turn_bounds(items)
        .windows(2)
        .map(|w| shipped_default_per_turn(&items[..w[1]]).pop().unwrap_or(0))
        .collect()
}

fn assert_within_budget(actual: Vec<usize>, budget: &[usize], label: &str) {
    assert_eq!(actual.len(), budget.len(), "{label} turn count");
    for (turn, (actual, limit)) in actual.into_iter().zip(budget).enumerate() {
        assert!(
            actual <= *limit,
            "{label} turn {turn}: {actual} rows exceeds budget {limit}"
        );
    }
}

#[test]
fn the_default_view_stays_within_the_measured_budget() {
    let auto = Lens::preset(FoldPreset::Auto);
    assert_within_budget(
        per_turn_as_last(&codex_session(), auto),
        &CODEX_AUTO,
        "codex auto",
    );
    assert_within_budget(
        per_turn_as_last(&claude_session(), auto),
        &CLAUDE_AUTO,
        "claude auto",
    );
}

#[test]
fn the_shipped_default_is_still_the_auto_budget() {
    let auto = Lens::preset(FoldPreset::Auto);
    assert_eq!(FoldMode::default(), FoldPreset::Auto.mode());
    for items in [codex_session(), claude_session()] {
        assert_eq!(
            shipped_default_as_last(&items),
            per_turn_as_last(&items, auto)
        );
        assert_eq!(
            shipped_default_per_turn(&items),
            per_turn_settled(&items, auto)
        );
    }
}

#[test]
fn the_tail_window_is_the_headline_cut() {
    let tail = Lens::preset(FoldPreset::Auto).tail(TailWindow::last(TAIL_N));
    assert_within_budget(
        per_turn_as_last(&codex_session(), tail),
        &CODEX_TAIL,
        "codex tail",
    );
    assert_within_budget(
        per_turn_as_last(&claude_session(), tail),
        &CLAUDE_TAIL,
        "claude tail",
    );
}

#[test]
fn summary_mode_folds_every_turn_to_its_conclusion() {
    let summary = Lens::preset(FoldPreset::Summary);
    assert_within_budget(
        per_turn_as_last(&codex_session(), summary),
        &CODEX_SUMMARY,
        "codex summary",
    );
    assert_within_budget(
        per_turn_as_last(&claude_session(), summary),
        &CLAUDE_SUMMARY,
        "claude summary",
    );
}

#[test]
fn expanded_mode_is_the_ceiling() {
    let expanded = Lens::preset(FoldPreset::Expanded);
    assert_eq!(per_turn_as_last(&codex_session(), expanded), CODEX_EXPANDED);
    assert_eq!(
        per_turn_as_last(&claude_session(), expanded),
        CLAUDE_EXPANDED
    );
    for ((&s, &a), &e) in CODEX_SUMMARY
        .iter()
        .zip(CODEX_AUTO.iter())
        .zip(CODEX_EXPANDED.iter())
    {
        assert!(s <= a && a <= e, "{s} <= {a} <= {e}");
    }
}

#[test]
fn a_turn_that_is_no_longer_the_newest_folds_on_its_own() {
    let auto = Lens::preset(FoldPreset::Auto);
    assert_eq!(per_turn_settled(&codex_session(), auto), CODEX_SETTLED);
    assert_eq!(per_turn_settled(&claude_session(), auto), CLAUDE_SETTLED);
}

#[test]
fn the_newest_turn_and_the_ones_behind_it_are_separate_policies() {
    let past_under_auto = per_turn_settled(&codex_session(), Lens::preset(FoldPreset::Auto));
    let newest_under_summary =
        per_turn_as_last(&codex_session(), Lens::preset(FoldPreset::Summary));
    assert_eq!(past_under_auto[0], newest_under_summary[0]);
    assert_ne!(newest_under_summary[0], CODEX_AUTO[0]);
}

#[test]
fn narrowing_by_the_tool_axis_lands_differently_on_the_two_adapters() {
    let edits =
        Lens::preset(FoldPreset::Auto).filter(DisplayFilter::from_tokens(["tools", "tool_edit"]));
    assert_eq!(
        per_turn_as_last(&claude_session(), edits),
        CLAUDE_EDITS_ONLY
    );

    let reads =
        Lens::preset(FoldPreset::Auto).filter(DisplayFilter::from_tokens(["tools", "tool_read"]));
    let codex = per_turn_as_last(&codex_session(), reads);
    assert!(
        CODEX_SUMMARY[0] < codex[0] && codex[0] < CODEX_AUTO[0],
        "narrowing costs rows without emptying the turn: {}",
        codex[0]
    );
}

#[test]
fn every_visible_row_is_accounted_for_by_a_turn() {
    let auto = Lens::preset(FoldPreset::Auto);
    for items in [codex_session(), claude_session()] {
        let total = rows(&items, auto).iter().filter(|r| !r.hidden).count();
        assert_eq!(per_turn_settled(&items, auto).iter().sum::<usize>(), total);
    }
}

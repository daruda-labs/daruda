use super::*;
use crate::transcript::display_filter::DisplayFilter;
use daruda_acp::{ToolKindView, ToolStatusView};

fn call(id: &str, parent: Option<&str>, status: ToolStatusView, kind: ToolKindView) -> ChatItem {
    ChatItem::ToolCall(ToolCallItem {
        id: id.to_owned(),
        title: format!("Tool {id}"),
        kind,
        tool_name: None,
        status,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: None,
        parent_tool_id: parent.map(str::to_owned),
        exit: None,
    })
}

/// A launch plus `n` settled children, and one child of a *different* launch so
/// every test also proves the collection is keyed by parent.
fn card_of(n: usize) -> Vec<ChatItem> {
    let mut items = vec![
        call("task", None, ToolStatusView::Completed, ToolKindView::Think),
        call(
            "other",
            None,
            ToolStatusView::Completed,
            ToolKindView::Think,
        ),
        call(
            "other-child",
            Some("other"),
            ToolStatusView::Completed,
            ToolKindView::Read,
        ),
    ];
    for i in 0..n {
        items.push(call(
            &format!("c{i}"),
            Some("task"),
            ToolStatusView::Completed,
            ToolKindView::Read,
        ));
    }
    items
}

fn lens<'a>(
    filter: &'a FilterMatchIndex,
    live: &'a LiveSubagentUnits,
    tail: TailWindow,
    revealed: bool,
) -> SubagentLens<'a> {
    SubagentLens {
        filter,
        filter_revealed: false,
        live_units: live,
        tail,
        revealed,
    }
}

fn ids<'a>(children: &'a SubagentChildren<'a>) -> Vec<&'a str> {
    children.shown.iter().map(|c| c.call.id.as_str()).collect()
}

#[test]
fn only_this_launchs_children_are_collected_in_conversation_order() {
    let items = card_of(3);
    let filter = FilterMatchIndex::of(&items, DisplayFilter::default());
    let live = LiveSubagentUnits::of(&items);
    let children = SubagentChildren::of(
        &items,
        "task",
        0,
        lens(&filter, &live, TailWindow::All, false),
    );
    assert_eq!(ids(&children), vec!["c0", "c1", "c2"]);
    assert_eq!((children.hidden, children.kept), (0, 3));
    assert!(!children.offers_reveal(), "an open window offers no reveal");
}

#[test]
fn the_window_keeps_the_last_calls_and_the_boundary_counts_the_rest() {
    let items = card_of(6);
    let filter = FilterMatchIndex::of(&items, DisplayFilter::default());
    let live = LiveSubagentUnits::of(&items);
    for kept in [1usize, 2, 5] {
        let children = SubagentChildren::of(
            &items,
            "task",
            0,
            lens(&filter, &live, TailWindow::Last(kept), false),
        );
        assert_eq!(
            ids(&children),
            (6 - kept..6).map(|i| format!("c{i}")).collect::<Vec<_>>(),
            "kept={kept}"
        );
        assert_eq!((children.hidden, children.kept), (6 - kept, kept));
        assert!(children.offers_reveal());
        assert!(
            children.shown.iter().all(|c| !c.covered),
            "nothing on screen is outside the window while the boundary is shut"
        );
    }
}

#[test]
fn opening_the_boundary_returns_every_child_still_marked_as_covered() {
    let items = card_of(5);
    let filter = FilterMatchIndex::of(&items, DisplayFilter::default());
    let live = LiveSubagentUnits::of(&items);
    let children = SubagentChildren::of(
        &items,
        "task",
        0,
        lens(&filter, &live, TailWindow::Last(2), true),
    );
    assert_eq!(ids(&children), vec!["c0", "c1", "c2", "c3", "c4"]);
    assert_eq!(
        children.shown.iter().map(|c| c.covered).collect::<Vec<_>>(),
        vec![true, true, true, false, false],
        "the reveal does not make a covered child part of the window"
    );
    assert_eq!(
        (children.hidden, children.kept),
        (3, 2),
        "the label still names the window it collapses back to"
    );
}

/// A running child escapes a shut boundary, exactly as a live run does one
/// level up — and the tally does not move for it, so the closed label can
/// promise more than it hides. Consistent with both row levels; pinned here so
/// a change to one is a visible change to all three.
#[test]
fn a_live_covered_child_is_shown_without_leaving_the_tally() {
    let mut items = card_of(4);
    items[3] = call(
        "c0",
        Some("task"),
        ToolStatusView::InProgress,
        ToolKindView::Read,
    );
    let filter = FilterMatchIndex::of(&items, DisplayFilter::default());
    let live = LiveSubagentUnits::of(&items);
    let children = SubagentChildren::of(
        &items,
        "task",
        0,
        lens(&filter, &live, TailWindow::Last(1), false),
    );
    assert_eq!(ids(&children), vec!["c0", "c3"]);
    assert!(children.shown[0].covered, "the live child is still outside");
    assert_eq!(children.hidden, 3);
}

#[test]
fn a_call_the_filter_drops_is_not_collected_unless_the_run_is_revealed() {
    let mut items = card_of(0);
    // A kept tool keeps its whole subtree, so narrow by a facet the *launch*
    // fails: then nothing under it survives either.
    items.push(call(
        "c0",
        Some("task"),
        ToolStatusView::Completed,
        ToolKindView::Read,
    ));
    let filter = FilterMatchIndex::of(&items, DisplayFilter::from_tokens(["tools", "tool_edit"]));
    let live = LiveSubagentUnits::of(&items);
    let mut narrowed = lens(&filter, &live, TailWindow::All, false);
    assert!(
        SubagentChildren::of(&items, "task", 0, narrowed)
            .shown
            .is_empty(),
        "the filter takes the whole card"
    );
    narrowed.filter_revealed = true;
    assert_eq!(
        ids(&SubagentChildren::of(&items, "task", 0, narrowed)),
        vec!["c0"],
        "the run's disclosure admits the card and its descendants"
    );
}

#[test]
fn the_nesting_cap_yields_no_children_at_all() {
    let items = card_of(4);
    let filter = FilterMatchIndex::of(&items, DisplayFilter::default());
    let live = LiveSubagentUnits::of(&items);
    let children = SubagentChildren::of(
        &items,
        "task",
        SUBAGENT_NEST_DEPTH_CAP,
        lens(&filter, &live, TailWindow::Last(2), false),
    );
    assert!(children.shown.is_empty());
    assert!(
        !children.offers_reveal(),
        "no children means no boundary to offer"
    );
}

#[test]
fn a_window_at_or_above_the_child_count_holds_nothing_back() {
    let items = card_of(3);
    let filter = FilterMatchIndex::of(&items, DisplayFilter::default());
    let live = LiveSubagentUnits::of(&items);
    for tail in [TailWindow::Last(3), TailWindow::Last(9), TailWindow::All] {
        let children = SubagentChildren::of(&items, "task", 0, lens(&filter, &live, tail, false));
        assert_eq!(ids(&children).len(), 3, "{tail:?}");
        assert_eq!((children.hidden, children.kept), (0, 3), "{tail:?}");
    }
}

#[test]
fn a_launch_with_no_children_yields_nothing() {
    let items = card_of(0);
    let filter = FilterMatchIndex::of(&items, DisplayFilter::default());
    let live = LiveSubagentUnits::of(&items);
    let children = SubagentChildren::of(
        &items,
        "task",
        0,
        lens(&filter, &live, TailWindow::Last(2), false),
    );
    assert!(children.shown.is_empty());
    assert_eq!((children.hidden, children.kept), (0, 0));
}

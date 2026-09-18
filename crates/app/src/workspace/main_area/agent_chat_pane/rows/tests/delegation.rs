//! Delegation: a launch, the calls it owns, and what nesting changes.

use super::*;

/// Background subagents run at once, so each one's children land in `items`
/// between the launches. The children own no row, so the launches are adjacent
/// on screen — and they belong to one group, with only the launches as members.
#[test]
fn consecutive_subagent_launches_form_one_group() {
    use ToolStatusView::Completed;
    let items = vec![
        ChatItem::UserText("q".into()),
        asst("delegating"),
        subagent_launch("A", Completed),
        child_of("a1", "A", Completed),
        subagent_launch("B", Completed),
        child_of("b1", "B", Completed),
        subagent_launch("C", Completed),
        child_of("c1", "C", Completed),
    ];
    let rows = project_all(&items);
    let headers: Vec<_> = rows
        .iter()
        .filter_map(|r| match &r.kind {
            RowKind::ToolGroupHeader { gid, calls, .. } => Some((gid.as_str(), calls.len())),
            _ => None,
        })
        .collect();
    assert_eq!(headers, vec![("A", 3)], "one group over the three launches");
    let calls: Vec<_> = rows
        .iter()
        .filter_map(|r| match r.kind {
            RowKind::AgentItem(ix) => Some(ix),
            _ => None,
        })
        .collect();
    assert_eq!(
        calls,
        vec![1, 2, 4, 6],
        "the prose and the launches, no child"
    );
}

#[test]
fn subagent_child_tool_calls_get_no_row() {
    use ToolStatusView::Completed;
    // A top-level Task/Agent parent plus one inner child linked by
    // `parent_tool_id`. The child renders nested inside the parent card
    // (see `tool_card`), so it must not appear as its own row — and the
    // parent + child must not collapse into a "2 tool calls" group.
    let mut child = tool("child", Completed);
    if let ChatItem::ToolCall(tc) = &mut child {
        tc.parent_tool_id = Some("parent".to_owned());
    }
    let items = [
        ChatItem::UserText("q".into()),
        tool("parent", Completed),
        child,
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::default(),
    );
    assert!(
        rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(1))),
        "the parent tool call still renders as a row (under its own group bar)"
    );
    assert!(
        !rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(2))),
        "the subagent child earns no row of its own"
    );
    assert!(
        rows.iter()
            .filter(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
            .count()
            == 1,
        "a parent + its child are not a sibling tool group"
    );
}

#[test]
fn multiple_subagent_children_all_skip_and_parent_stays_single() {
    use ToolStatusView::Completed;
    // Two inner calls under one parent: both skipped, and the parent renders
    // as a single card (not a "2 tool calls" group with its own children).
    let mut c1 = tool("c1", Completed);
    let mut c2 = tool("c2", Completed);
    for c in [&mut c1, &mut c2] {
        if let ChatItem::ToolCall(tc) = c {
            tc.parent_tool_id = Some("parent".to_owned());
        }
    }
    let items = [
        ChatItem::UserText("q".into()),
        tool("parent", Completed),
        c1,
        c2,
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::default(),
    );
    assert!(rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(1))));
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::AgentItem(2) | RowKind::AgentItem(3))),
        "both subagent children are skipped from the main flow"
    );
    assert_eq!(
        rows.iter()
            .filter(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
            .count(),
        1,
        "the parent is one run of one; its children are not siblings in it"
    );
}

#[test]
fn orphan_child_keeps_its_row() {
    use ToolStatusView::Completed;
    // A child whose `parent_tool_id` matches no tool call in `items` (a
    // dangling ref) is nested by nobody, so it must keep a top-level row
    // rather than vanish silently.
    let mut orphan = tool("orphan", Completed);
    if let ChatItem::ToolCall(tc) = &mut orphan {
        tc.parent_tool_id = Some("missing-parent".to_owned());
    }
    let items = [ChatItem::UserText("q".into()), orphan];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::default(),
    );
    assert!(
        rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(1))),
        "an orphan child (no present parent) still renders as a row"
    );
}

#[test]
fn a_matching_grandchild_keeps_its_ancestors_but_not_a_sibling_branch() {
    use ToolStatusView::Completed;
    // The match is three levels down. Its ancestors come along so the card that
    // renders it is on screen; the sibling branch, which holds no match of its
    // own, is cut like any other call of its category.
    let mut sibling = child_of("sibling", "task", Completed);
    if let ChatItem::ToolCall(tc) = &mut sibling {
        tc.kind = ToolKindView::Read;
    }
    let mut middle = child_of("middle", "task", Completed);
    if let ChatItem::ToolCall(tc) = &mut middle {
        tc.kind = ToolKindView::Read;
    }
    let mut task = tool("task", Completed);
    if let ChatItem::ToolCall(tc) = &mut task {
        tc.kind = ToolKindView::Read;
    }
    let items = [
        ChatItem::UserText("q".into()),
        task,
        middle,
        child_of("leaf", "middle", Completed), // ToolKindView::Edit
        sibling,
    ];
    let index = FilterMatchIndex::of(&items, DisplayFilter::from_tokens(["tools", "tool_edit"]));
    for id in ["task", "middle", "leaf"] {
        assert!(index.keeps_tool(&tool_of(&items, id)), "{id}");
    }
    assert!(
        !index.keeps_tool(&tool_of(&items, "sibling")),
        "a Read branch with no match under it goes with the other reads"
    );
}

#[test]
fn live_subagent_units_marks_every_ancestor_of_a_live_descendant() {
    use ToolStatusView::{Completed, InProgress};
    // parent → child(done) → grandchild(running): both ancestors are still
    // working units, so one pass must mark both.
    let items = [
        tool("task", Completed),
        child_of("child", "task", Completed),
        child_of("grand", "child", InProgress),
    ];
    let units = LiveSubagentUnits::of(&items);
    assert!(units.contains("task"));
    assert!(units.contains("child"));
    // A live node is not its own ancestor — `tool_or_subtree_live` ors in the
    // call's own status, so the set carries descendant liveness only.
    assert!(!units.contains("grand"));
}

#[test]
fn live_subagent_units_excludes_a_fully_settled_subtree() {
    use ToolStatusView::{Completed, Failed};
    let items = [
        tool("task", Completed),
        child_of("a", "task", Completed),
        child_of("b", "task", Failed),
    ];
    assert!(
        !LiveSubagentUnits::of(&items).contains("task"),
        "no live descendant → the parent unit is done"
    );
}

#[test]
fn live_subagent_units_stops_at_the_nesting_depth_cap() {
    use ToolStatusView::{Completed, InProgress};
    // A chain deeper than the cap: the live leaf marks only the `cap` nearest
    // ancestors, matching the depth-bounded recursive walk this replaces.
    let live_leaf = SUBAGENT_NEST_DEPTH_CAP + 2;
    let mut items = vec![tool("n0", Completed)];
    for d in 1..=live_leaf {
        let status = if d == live_leaf {
            InProgress
        } else {
            Completed
        };
        items.push(child_of(&format!("n{d}"), &format!("n{}", d - 1), status));
    }
    let units = LiveSubagentUnits::of(&items);
    assert!(units.contains(&format!("n{}", live_leaf - SUBAGENT_NEST_DEPTH_CAP)));
    assert!(
        !units.contains(&format!("n{}", live_leaf - SUBAGENT_NEST_DEPTH_CAP - 1)),
        "an ancestor past the cap stays unmarked"
    );
}

#[test]
fn live_subagent_units_terminates_on_a_cyclic_parent_id() {
    use ToolStatusView::{Completed, InProgress};
    // A malformed self-parent must not walk without bound.
    let items = [child_of("x", "x", Completed)];
    assert!(!LiveSubagentUnits::of(&items).contains("x"));
    // A live self-parent is its own ancestor here, so it marks itself — and
    // still terminates.
    let items = [child_of("y", "y", InProgress)];
    assert!(LiveSubagentUnits::of(&items).contains("y"));
}

#[test]
fn live_subagent_units_stays_linear_over_a_long_tool_run() {
    use ToolStatusView::Completed;
    let items: Vec<ChatItem> = (0..4000)
        .map(|i| tool(&format!("t{i}"), Completed))
        .collect();
    let started = std::time::Instant::now();
    let units = LiveSubagentUnits::of(&items);
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &units,
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::default(),
    );
    let elapsed = started.elapsed();
    assert_eq!(
        rows.len(),
        items.len() + 4,
        "the run's bar + tail row + one group header + its own tail row + every member"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "projecting 4000 items took {elapsed:?} — the quadratic scan is back"
    );
}

#[test]
fn live_subagent_units_marks_a_running_child_of_a_completed_parent() {
    use ToolStatusView::{Completed, InProgress};
    // The adapter marks the parent Task `Completed` (its SDK call returned)
    // while a flattened child keeps running — the unit is still working.
    let items = [
        tool("task", Completed),
        child_of("child", "task", InProgress),
    ];
    assert!(
        LiveSubagentUnits::of(&items).contains("task"),
        "a live child keeps the subagent parent reading as running"
    );

    let items = [tool("task", Completed)];
    assert!(
        !LiveSubagentUnits::of(&items).contains("task"),
        "a childless settled tool is not a live unit"
    );
}

/// A nested child renders inside the first call's card, so the two top-level
/// calls look adjacent on screen — and the run spans the child rather than
/// ending at it, so they read as one group. Native subagent sessions make this
/// the ordinary shape: each launch's own calls arrive before the next launch.
#[test]
fn a_nested_child_between_two_calls_leaves_them_in_one_group() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        tool("a", Completed),
        child_of("mid", "a", Completed),
        tool("b", Completed),
    ];
    assert!(
        ToolHierarchy::build(&items).is_nested_child(&tool_of(&items, "mid")),
        "the fixture says nothing unless the middle call really nests"
    );
    let one_group = vec![
        ("user", false),
        ("response", false),
        ("tail", true),
        ("group", false),
        ("grouptail", true),
        ("item", true), // settled members collapsed
        ("item", true),
    ];
    assert_eq!(kinds(&project_all(&items)), one_group);

    // The same two calls with nothing between them project identically: what
    // the reader sees is what decides the grouping.
    let adjacent = [
        ChatItem::UserText("q".into()),
        tool("a", Completed),
        tool("b", Completed),
    ];
    assert_eq!(kinds(&project_all(&adjacent)), one_group);

    // The step tally counts the same way — one run, so nothing to cover.
    let rows = project_tail(&items, StepWindow::uniform(TailWindow::Last(1)));
    match tail_row(&rows).kind {
        RowKind::TailMore {
            hidden_steps,
            kept_steps,
            ..
        } => assert_eq!((hidden_steps, kept_steps), (0, 1), "one run"),
        _ => unreachable!(),
    }
}

/// The row boundary — see [`top_level_tool`]. A nested child owns no row, so no
/// combination of the step window, the display filter and the fold mode can
/// give it one, and a card's children cannot move the row layer at all.
///
/// This is about rows only: both axes *do* narrow what the card renders
/// ([`super::subagent::SubagentChildren`]). What must not happen is a child
/// climbing out of its card into the list.
#[test]
fn no_axis_gives_a_card_child_a_row_of_its_own() {
    let nested_rows = |items: &[ChatItem], tail, filter: &DisplayFilter, preset: FoldPreset| {
        project(
            items,
            &FoldState::with_mode(preset.mode()),
            false,
            &LiveSubagentUnits::of(items),
            tail,
            filter,
        )
        .iter()
        .filter(|r| match r.kind {
            RowKind::AgentItem(ix) | RowKind::ConclusionItem(ix) => {
                matches!(&items[ix], ChatItem::ToolCall(tc) if tc.parent_tool_id.is_some())
            }
            _ => false,
        })
        .count()
    };
    for running in [false, true] {
        let items = turn_with_subagent(6, running);
        for tail in [
            StepWindow::uniform(TailWindow::All),
            StepWindow::uniform(TailWindow::Last(1)),
        ] {
            for filter in [DisplayFilter::default(), only_reads()] {
                for preset in FoldPreset::ALL {
                    assert_eq!(
                        nested_rows(&items, tail, &filter, preset),
                        0,
                        "running={running} {tail:?} {preset:?}"
                    );
                }
            }
        }
    }
    // And they contribute nothing to it: one child or six, the projection is
    // the same shape, because the card — not the row walk — renders them.
    let one = turn_with_subagent(1, false);
    let many = turn_with_subagent(6, false);
    for tail in [TailWindow::All, TailWindow::Last(1)] {
        assert_eq!(
            kinds(&project_tail(&one, StepWindow::uniform(tail))),
            kinds(&project_tail(&many, StepWindow::uniform(tail))),
            "a card's children never move the row layer: {tail:?}"
        );
    }
}

/// The launch answers to its own category row, and taking it takes the card its
/// children render inside. A category that matches a child still keeps the
/// launch, because a match has to stay reachable through the card that holds
/// it — that is the ancestor rule, not an exemption.
#[test]
fn the_agent_row_decides_whether_a_launch_is_in_the_index() {
    let items = turn_with_subagent(3, false); // children are Edit-kind
    let launch = tool_of(&items, "task");
    let kept = FilterMatchIndex::of(&items, DisplayFilter::from_tokens(["tools", "tool_agent"]));
    assert!(kept.keeps_tool(&launch), "its own row keeps it");
    for tokens in [vec!["tools", "tool_read"], vec!["tools", "tool_search"]] {
        let index = FilterMatchIndex::of(&items, DisplayFilter::from_tokens(tokens.clone()));
        assert!(!index.keeps_tool(&launch), "{tokens:?}");
    }
    let by_child = FilterMatchIndex::of(&items, DisplayFilter::from_tokens(["tools", "tool_edit"]));
    assert!(
        by_child.keeps_tool(&launch),
        "a matching child drags its card back in"
    );
}

/// The mixed case: `c1` survives as the ancestor of a Read, so the walk goes
/// *through* it and reports the Edit under it — while `c2`, rejected outright,
/// is one block whose own subtree is not counted again.
#[test]
fn the_tally_walks_through_a_rescued_child_and_stops_at_a_rejected_one() {
    let items = turn_with_nested_subagent();
    let index = FilterMatchIndex::of(&items, only_reads());
    assert!(
        index.keeps_tool(&tool_of(&items, "c1")),
        "rescued as g0's ancestor"
    );
    assert!(!index.keeps_tool(&tool_of(&items, "c2")));

    let rows = project_filtered(&items, &only_reads());
    assert_eq!(
        filtered_count(&rows) - 2, // the run's two prose rows
        2,
        "g1 under the rescued c1, and c2 itself"
    );
}

/// The chip's number and the card's contents are decided in two places —
/// `cut_below` here and [`subagent::SubagentChildren::of`] in the renderer —
/// and each restates the admission rule and the depth cap. If they drift, the
/// count is wrong in a direction no other test looks at.
#[test]
fn the_tally_counts_exactly_the_children_the_card_declines_to_render() {
    // What the renderer withholds, walked the way `tool_card` walks it.
    fn dropped_by_card(
        items: &[ChatItem],
        parent: &str,
        depth: usize,
        index: &FilterMatchIndex,
        live: &LiveSubagentUnits,
    ) -> usize {
        let declared = items
            .iter()
            .filter(|it| {
                matches!(it, ChatItem::ToolCall(tc) if tc.parent_tool_id.as_deref() == Some(parent))
            })
            .count();
        let shown = subagent::SubagentChildren::of(
            items,
            parent,
            depth,
            subagent::SubagentLens {
                filter: index,
                filter_revealed: false,
                live_units: live,
                calls: TailWindow::All,
                revealed: false,
            },
        );
        declared - shown.shown.len()
            + shown
                .shown
                .iter()
                .map(|c| dropped_by_card(items, c.call.id.as_str(), depth + 1, index, live))
                .sum::<usize>()
    }

    let items = turn_with_nested_subagent();
    let index = FilterMatchIndex::of(&items, only_reads());
    let live = LiveSubagentUnits::of(&items);
    let by_card = dropped_by_card(&items, "task", 0, &index, &live);
    assert!(by_card > 0, "the fixture actually withholds something");
    assert_eq!(
        ToolHierarchy::build(&items).cut_below("task", |id| index.keeps_id(id)),
        by_card,
        "the count the chip promises is the count the card withheld"
    );
}

/// The turn bar counts by a different rule than a group bar: it summarizes the
/// turn rather than disclosing rows, so a subagent's inner calls — already
/// counted inside the card that spawned them — must not be counted twice.
#[test]
fn the_turn_tally_drops_a_subagents_inner_calls() {
    use ToolStatusView::Completed;
    let mut child = tool("child", Completed);
    if let ChatItem::ToolCall(tc) = &mut child {
        tc.parent_tool_id = Some("parent".to_owned());
    }
    // Two runs, so the bar carries a tally at all (a one-run turn withholds it
    // — see `the_turn_tally_is_withheld_when_one_bar_below_already_says_it`).
    let items = [
        ChatItem::UserText("q".into()),
        tool("parent", Completed),
        child,
        asst("between"),
        tool("later", Completed),
        asst("done"),
    ];
    let rows = project_all(&items);
    let tally = rows
        .iter()
        .find_map(|r| match &r.kind {
            RowKind::ResponseHeader { categories, .. } => Some(categories.clone()),
            _ => None,
        })
        .expect("the turn has a bar");
    assert_eq!(
        tally.iter().map(|(_, n)| *n).sum::<usize>(),
        2,
        "the two top-level calls; the nested child is counted inside its card"
    );
}

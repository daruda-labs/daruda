//! The display filter's cut: what it removes, what it drags back in, and
//! what the reveal offers.

use super::*;

#[test]
fn a_filtered_away_parent_takes_its_children_with_it() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        tool("parent", Completed), // ToolKindView::Edit
        child_of("child", "parent", Completed),
    ];
    let index = FilterMatchIndex::of(&items, DisplayFilter::from_tokens(["tools", "tool_search"]));
    assert!(!index.keeps_tool(&tool_of(&items, "parent")));
    assert!(
        !index.keeps_tool(&tool_of(&items, "child")),
        "no card survives to render the child inside"
    );
}

/// A run the filter empties must not spend a slot in the tail window. The
/// window is what the reader asked to see, so counting runs that render nothing
/// makes `Recent steps: 3` put one run on screen and silently drop the other
/// two.
#[test]
fn the_window_counts_runs_the_filter_leaves_something_to_show() {
    use ToolStatusView::Completed;
    // Alternating runs: the read-only ones survive the filter below, the
    // edit-only ones are emptied by it entirely.
    let mut items = vec![ChatItem::UserText("q".into())];
    for i in 0..6 {
        let kind = if i % 2 == 0 {
            ToolKindView::Read
        } else {
            ToolKindView::Edit
        };
        items.push(asst(&format!("run {i}")));
        items.push(kinded_tool(&format!("t{i}a"), kind, Completed));
        items.push(kinded_tool(&format!("t{i}b"), kind, Completed));
    }
    items.push(asst("done"));

    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::Last(2)),
        &only_reads(),
    );

    let shown: Vec<usize> = rows
        .iter()
        .filter(|r| !r.hidden)
        .filter_map(|r| match &r.kind {
            RowKind::ToolGroupHeader { calls, .. } => Some(calls[0]),
            _ => None,
        })
        .collect();
    assert_eq!(
        shown.len(),
        2,
        "a window of 2 puts two runs on screen, not however many of the last \
         two raw runs happened to survive the filter: {shown:?}"
    );

    match tail_row(&rows).kind {
        RowKind::TailMore {
            hidden_steps,
            kept_steps,
            ..
        } => {
            assert_eq!(kept_steps, 2, "the label counts the same population");
            assert_eq!(
                hidden_steps, 1,
                "three runs have content and two are kept, so one is behind the boundary"
            );
        }
        _ => unreachable!(),
    }
}

/// The group's window hides the calls it covers, and the group's own boundary
/// is what puts them back — the same answer the response's boundary gives one
/// level up.
#[test]
fn opening_a_group_boundary_reveals_its_covered_calls() {
    let items = turn_of_one_group(5, false);
    let tail = StepWindow::uniform(TailWindow::Last(2));

    let shut = project_open_group(&items, tail);
    assert_eq!(
        call_visibility(&items, &shut),
        vec![false, false, false, true, true],
        "with the boundary shut, only the window's last two calls are on screen"
    );

    let mut fold = FoldState::with_mode(FoldPreset::Expanded.mode());
    fold.toggle(
        FoldKey::ToolGroupTail("g0".into()),
        FoldContext::last(false),
    );
    let open = project_open_group_under(&items, &fold, tail);
    assert_eq!(
        call_visibility(&items, &open),
        vec![true; 5],
        "the reveal puts the covered calls back"
    );
}

/// A call the filter drops must not spend a slot in the group's window, for the
/// same reason an emptied run must not spend one in the response's.
#[test]
fn the_group_window_counts_calls_the_filter_leaves_something_to_show() {
    use ToolStatusView::Completed;
    let mut items = vec![ChatItem::UserText("q".into()), asst("working")];
    for i in 0..6 {
        let kind = if i % 2 == 0 {
            ToolKindView::Read
        } else {
            ToolKindView::Edit
        };
        items.push(kinded_tool(&format!("t{i}"), kind, Completed));
    }
    items.push(asst("done"));

    let rows = project(
        &items,
        &FoldState::with_mode(FoldPreset::Expanded.mode()),
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::Last(2)),
        &only_reads(),
    );
    assert_eq!(
        group_tail_counts(&rows),
        (1, 2),
        "three calls survive the filter and two are kept, so one is behind the boundary"
    );
}

#[test]
fn revealing_the_tail_shows_every_run_again() {
    let items = turn_of_cycles(6);
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Tail(1), FoldContext::past(false));
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::Last(2)),
        &DisplayFilter::default(),
    );
    assert_eq!(group_visibility(&rows), vec![true; 6]);
    match tail_row(&rows).kind {
        RowKind::TailMore {
            hidden_steps,
            collapsed,
            ..
        } => {
            assert_eq!(hidden_steps, 4);
            assert!(!collapsed);
        }
        _ => unreachable!(),
    }
}

#[test]
fn expand_all_leaves_the_tail_and_filter_chips_in_charge() {
    use super::collect_foldable_keys;
    let items = turn_of_cycles(6);
    let mut fold = FoldState::default();
    fold.set_all(collect_foldable_keys(&items), true);

    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::Last(2)),
        &DisplayFilter::default(),
    );
    assert_eq!(
        group_visibility(&rows),
        vec![false, false, false, false, true, true]
    );
    match tail_row(&rows).kind {
        RowKind::TailMore {
            hidden_steps,
            collapsed,
            ..
        } => {
            assert_eq!(hidden_steps, 4, "the row's count matches what it hides");
            assert!(collapsed, "the row still offers the reveal");
        }
        _ => unreachable!(),
    }

    let filtered = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::All),
        &only_reads(),
    );
    assert!(filtered_count(&filtered) > 0, "the filter still takes rows");
    assert!(
        filtered_away(&filtered).revealable > 0,
        "the chip offers the reveal"
    );
}

#[test]
fn a_nested_tool_filter_keeps_an_ancestor_chain_and_drops_the_rest() {
    use ToolStatusView::Completed;

    let parent = tool("task", Completed);
    let mut child = child_of("read", "task", Completed);
    if let ChatItem::ToolCall(tc) = &mut child {
        tc.kind = ToolKindView::Read;
    }
    let items = [parent, child];
    let reads = FilterMatchIndex::of(&items, only_reads());
    let ChatItem::ToolCall(parent) = &items[0] else {
        unreachable!()
    };
    let ChatItem::ToolCall(child) = &items[1] else {
        unreachable!()
    };
    assert!(
        reads.keeps_tool(parent),
        "the parent carries the matching child"
    );
    assert!(reads.keeps_tool(child), "the matching nested child renders");

    let edits = DisplayFilter::from_tokens(["tool_edit"]);
    let edits = FilterMatchIndex::of(&items, edits);
    assert!(edits.keeps_tool(parent), "the Edit parent matches directly");
    assert!(
        !edits.keeps_tool(child),
        "the card survives, but a Read child inside it answers for its own kind"
    );
}

#[test]
fn an_empty_filter_hides_nothing_and_its_row_covers_nothing() {
    let items = live_run_turn();
    let rows = project_filtered(&items, &DisplayFilter::default());
    assert_eq!(filtered_count(&rows), 0);
    assert!(
        !filtered_away(&rows).offers_reveal(),
        "nothing was taken, so the bar carries no chip"
    );
}

#[test]
fn a_filter_hides_the_rows_it_rejects_and_counts_them() {
    let items = live_run_turn();
    let only_tools = DisplayFilter::from_tokens(["tools"]);
    let rows = project_filtered(&items, &only_tools);
    for row in &rows {
        match row.kind {
            RowKind::AgentItem(1) => assert!(row.hidden, "the run's prose is filtered"),
            RowKind::ConclusionItem(4) => {
                assert!(row.hidden, "the conclusion is prose too — the filter wins")
            }
            RowKind::AgentItem(2) | RowKind::AgentItem(3) => {
                assert!(!row.hidden, "the tools survive")
            }
            _ => {}
        }
    }
    assert!(
        filtered_away(&rows).revealable > 0,
        "the chip offers the reveal"
    );
    assert_eq!(
        filtered_count(&rows),
        2,
        "the run's prose and the conclusion"
    );
}

#[test]
fn a_run_whose_every_row_is_filtered_goes_with_them() {
    let items = live_run_turn();
    let rows = project_filtered(&items, &only_reads());
    assert!(
        rows.iter()
            .filter(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
            .all(|r| r.hidden),
        "no group survives a read-only filter over Edit-kind tools"
    );
    let visible: Vec<&'static str> = rows
        .iter()
        .filter(|r| !r.hidden)
        .map(|r| match r.kind {
            RowKind::User(_) => "user",
            RowKind::ResponseHeader { .. } => "response",
            _ => "other",
        })
        .collect();
    assert_eq!(visible, vec!["user", "response"]);
}

#[test]
fn a_group_bar_summarizing_only_filtered_calls_goes_with_them() {
    let items = live_run_turn();
    let only_prose = DisplayFilter::from_tokens(["prose"]);
    let rows = project_filtered(&items, &only_prose);
    assert!(
        rows.iter()
            .filter(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
            .all(|r| r.hidden)
    );
}

#[test]
fn revealing_the_filter_shows_what_it_covers() {
    let items = live_run_turn();
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Filtered(1), FoldContext::past(false));
    let only_tools = DisplayFilter::from_tokens(["tools"]);
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::All),
        &only_tools,
    );
    let conclusion = rows
        .iter()
        .find(|r| matches!(r.kind, RowKind::ConclusionItem(_)))
        .expect("the run's conclusion");
    assert!(!conclusion.hidden, "revealed in place");
    assert_eq!(filtered_away(&rows).revealable, 2);
}

#[test]
fn a_prompt_and_a_pending_permission_survive_every_filter() {
    let items = [
        ChatItem::UserText("q".into()),
        asst("about to write"),
        perm(false),
        tool("a", ToolStatusView::Completed),
    ];
    let only_edits = DisplayFilter::from_tokens(["tool_edit"]);
    let rows = project_filtered(&items, &only_edits);
    for row in &rows {
        match row.kind {
            RowKind::User(_) => assert!(!row.hidden, "the turn anchor always shows"),
            RowKind::AgentItem(2) => assert!(!row.hidden, "an actionable permission always shows"),
            _ => {}
        }
    }
}

#[test]
fn a_filter_and_a_fold_compose_rather_than_override_each_other() {
    let items = one_run_turn();
    let only_tools = DisplayFilter::from_tokens(["tools"]);
    let mut collapsed = FoldState::default();
    collapsed.toggle(FoldKey::Response(1), FoldContext::past(true));
    let rows = project(
        &items,
        &collapsed,
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::All),
        &only_tools,
    );
    for row in &rows {
        match row.kind {
            RowKind::AgentItem(2) | RowKind::AgentItem(3) => assert!(
                row.hidden,
                "a collapsed response hides even a tool the filter kept"
            ),
            RowKind::ConclusionItem(4) => assert!(
                row.hidden,
                "the filter outranks the conclusion's force-visible escape"
            ),
            _ => {}
        }
    }
    assert_eq!(
        filtered_away(&rows).revealable,
        2,
        "the tally reports the filter's cut — both prose blocks — under a collapsed bar"
    );
}

/// A collapsed response still shows its conclusion — that is what
/// `force_visible` buys. When the filter takes that one row too, the turn is
/// left with nothing but its bar, so the bar has to keep offering the way back.
/// The fold is already accounted for inside `revealable`, and re-checking the
/// collapse at the render site is what erased this exact case.
#[test]
fn a_collapsed_response_emptied_by_the_filter_still_offers_the_reveal() {
    let items = one_run_turn();
    let mut collapsed = FoldState::default();
    collapsed.toggle(FoldKey::Response(1), FoldContext::past(true));
    let rows = project(
        &items,
        &collapsed,
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::from_tokens(["tools"]),
    );
    assert!(
        rows.iter()
            .filter(|r| !matches!(r.kind, RowKind::User(_) | RowKind::ResponseHeader { .. }))
            .all(|r| r.hidden),
        "nothing but the bar is left on screen"
    );
    assert!(
        filtered_away(&rows).offers_reveal(),
        "so the bar still carries the chip"
    );
}

/// A group the filter empties is one unit in the tally, and none of its calls
/// stays on screen — the group is what the reveal brings back, and the calls
/// come with it.
#[test]
fn a_group_the_filter_empties_leaves_no_call_on_screen() {
    let items = one_run_turn();
    let rows = project_filtered(
        &items,
        &DisplayFilter::default().toggled(FilterFacet::Tools),
    );

    assert_eq!(
        filtered_away(&rows).revealable,
        1,
        "the group is the unit, not its two calls"
    );

    let on_screen_tools = rows
        .iter()
        .filter(|r| !r.hidden)
        .filter_map(|r| match r.kind {
            RowKind::AgentItem(ix) => Some(ix),
            _ => None,
        })
        .filter(|&ix| matches!(items[ix], ChatItem::ToolCall(_)))
        .count();
    assert_eq!(on_screen_tools, 0, "no tool card is on screen");
}

/// A group the filter empties is one unit, however many calls it holds: the
/// reveal brings back the group, and the calls come with it rather than as
/// units of their own.
#[test]
fn a_group_the_filter_empties_counts_as_one_unit() {
    for calls in [2, 5, 9] {
        for running in [true, false] {
            let rows = project_filtered(&turn_of_one_group(calls, running), &hides_edits());
            assert_eq!(
                filtered_count(&rows),
                1,
                "{calls} calls, running={running}: the group is the unit"
            );
        }
    }
}

/// A group the filter leaves something is on screen already, so what the reveal
/// brings back is the individual calls it took — and that count cannot depend on
/// whether the group happens to be folded.
#[test]
fn a_surviving_group_counts_the_calls_the_filter_took_from_it() {
    use ToolStatusView::{Completed, InProgress};
    let mixed = |running: bool| {
        vec![
            ChatItem::UserText("q".into()),
            asst("looking"),
            tool("e0", Completed),
            read_tool("r0", Completed),
            tool("e1", Completed),
            read_tool("r1", Completed),
            tool("e2", if running { InProgress } else { Completed }),
        ]
    };
    for running in [true, false] {
        let rows = project_filtered(&mixed(running), &hides_edits());
        assert_eq!(
            filtered_count(&rows),
            3,
            "running={running}: the three edit calls the filter took"
        );
    }
}

#[test]
fn changing_the_filter_keeps_every_row_in_its_slot() {
    let items = turn_of_cycles(6);
    let none = project_filtered(&items, &DisplayFilter::default());
    for facets in [
        vec![FilterFacet::Tools],
        vec![FilterFacet::Prose],
        vec![FilterFacet::Tools, FilterFacet::ToolEdit],
        vec![FilterFacet::Thinking, FilterFacet::ToolSearch],
    ] {
        let filter = facets
            .iter()
            .fold(DisplayFilter::default(), |f, facet| f.toggled(*facet));
        let rows = project_filtered(&items, &filter);
        assert_eq!(rows.len(), none.len(), "{facets:?} changes no row count");
        assert!(
            none.iter().zip(&rows).all(|(a, b)| a.same_slot(b)),
            "{facets:?} keeps every slot"
        );
    }
}

/// The reveal undoes the *filter*, not the group's own fold: the bar comes back
/// and its thoughts stay behind whichever fold state the group is in — the same
/// division of labour the tool group has.
#[test]
fn revealing_the_filter_brings_the_group_bar_back() {
    let items = [
        ChatItem::UserText("q".into()),
        think("a"),
        think("b"),
        asst("done"),
    ];
    let no_reasoning = DisplayFilter::from_tokens(["prose", "tools"]);
    let project_revealed = |fold: &FoldState| {
        project(
            &items,
            fold,
            false,
            &LiveSubagentUnits::of(&items),
            StepWindow::uniform(TailWindow::All),
            &no_reasoning,
        )
    };
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Filtered(1), FoldContext::past(false));
    let rows = project_revealed(&fold);
    assert!(!think_group_headers(&rows)[0].hidden, "the bar is back");
    for ix in [1usize, 2] {
        assert!(
            rows.iter()
                .any(|r| matches!(r.kind, RowKind::AgentItem(j) if j == ix) && r.hidden),
            "thought {ix} is still behind the settled group's own fold"
        );
    }

    fold.toggle(FoldKey::ThinkingGroup(1), FoldContext::last(false));
    let rows = project_revealed(&fold);
    for ix in [1usize, 2] {
        assert!(
            rows.iter()
                .any(|r| matches!(r.kind, RowKind::AgentItem(j) if j == ix) && !r.hidden),
            "thought {ix} shows once the group is open too"
        );
    }
}

/// The other half: what the launch does *not* protect is the work it did. Each
/// child answers for its own category the way the same call would at the top
/// level.
#[test]
fn a_subagents_own_calls_are_filtered_on_their_own_category() {
    let items = turn_with_subagent(3, false); // children are Edit-kind
    let reads = FilterMatchIndex::of(&items, only_reads());
    for i in 0..3 {
        assert!(
            !reads.keeps_tool(&tool_of(&items, &format!("c{i}"))),
            "c{i}"
        );
    }
    let edits = FilterMatchIndex::of(&items, DisplayFilter::from_tokens(["tools", "tool_edit"]));
    for i in 0..3 {
        assert!(edits.keeps_tool(&tool_of(&items, &format!("c{i}"))), "c{i}");
    }
}

/// A card's children own no row, so the row walk cannot tally them — and a cut
/// nothing counts is a cut with no reveal to undo it. The run's bar carries
/// their number instead.
#[test]
fn the_runs_tally_counts_what_the_filter_took_from_inside_a_card() {
    // Two prose rows and nothing else: the filter keeps the launch's own row,
    // so this is the whole tally a delegating run starts from.
    let keeps_the_card = DisplayFilter::from_tokens(["tools", "tool_agent"]);
    let childless = project_filtered(&turn_with_subagent(0, false), &keeps_the_card);
    assert_eq!(filtered_count(&childless), 2);

    let rows = project_filtered(&turn_with_subagent(3, false), &keeps_the_card);
    assert_eq!(
        filtered_count(&rows),
        5,
        "the same two rows plus one block per Edit child the card drops"
    );
    assert!(
        filtered_away(&rows).offers_reveal(),
        "so the reveal that brings them back is on the bar"
    );
}

/// And it stays filter-blind, unlike a group bar: a fully narrowed turn must
/// still show a trace of the work it did.
#[test]
fn the_turn_tally_ignores_what_the_filter_hides() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        tool("a", Completed),
        asst("between"),
        tool("b", Completed),
        asst("done"),
    ];
    let hide_tools = DisplayFilter::default().toggled(FilterFacet::Tools);
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::All),
        &hide_tools,
    );
    let tally = rows
        .iter()
        .find_map(|r| match &r.kind {
            RowKind::ResponseHeader { categories, .. } => Some(categories.clone()),
            _ => None,
        })
        .expect("the turn has a bar");
    assert_eq!(tally.iter().map(|(_, n)| *n).sum::<usize>(), 2);
}

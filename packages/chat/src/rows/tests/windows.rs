//! The step and call windows — what each boundary row holds back and what
//! escapes it.

use super::*;

/// The same rule with an item the run only *spans*: a bodyless chunk owns no
/// row and is not a member, so it must not be what keeps an emptied run in the
/// window's population.
#[test]
fn a_spanned_bodyless_chunk_does_not_keep_an_emptied_run_in_the_window() {
    use ToolStatusView::Completed;
    let mut items = vec![ChatItem::UserText("q".into())];
    for i in 0..6 {
        let edits = i % 2 == 1;
        let kind = if edits {
            ToolKindView::Edit
        } else {
            ToolKindView::Read
        };
        items.push(asst(&format!("run {i}")));
        items.push(kinded_tool(&format!("t{i}a"), kind, Completed));
        if edits {
            items.push(asst(""));
        }
        items.push(kinded_tool(&format!("t{i}b"), kind, Completed));
    }
    items.push(asst("done"));

    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::Last(2)),
        &DisplayFilter::from_tokens(["prose", "tool_read"]),
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
        "a window of 2 puts two runs on screen: {shown:?}"
    );
}

#[test]
fn a_window_keeps_only_the_last_runs_visible() {
    let items = turn_of_cycles(8);
    for (n, kept) in [(1usize, 1usize), (3, 3), (5, 5)] {
        let rows = project_tail(&items, StepWindow::uniform(TailWindow::Last(n)));
        let vis = group_visibility(&rows);
        assert_eq!(vis.len(), 8, "every run keeps its bar at n={n}");
        assert_eq!(
            vis,
            (0..8).map(|i| i >= 8 - kept).collect::<Vec<_>>(),
            "only the last {kept} runs show at n={n}"
        );
        match tail_row(&rows).kind {
            RowKind::TailMore { hidden_steps, .. } => assert_eq!(hidden_steps, 8 - kept),
            _ => unreachable!(),
        }
        assert!(!tail_row(&rows).hidden, "the tail row offers the reveal");
    }
}

#[test]
fn a_window_at_or_above_the_run_count_hides_nothing() {
    let items = turn_of_cycles(3);
    for tail in [TailWindow::Last(3), TailWindow::Last(10), TailWindow::All] {
        let rows = project_tail(&items, StepWindow::uniform(tail));
        assert_eq!(group_visibility(&rows), vec![true; 3], "{tail:?}");
        let row = tail_row(&rows);
        assert!(row.hidden, "nothing to reveal → the row stays zero-height");
        match row.kind {
            RowKind::TailMore { hidden_steps, .. } => assert_eq!(hidden_steps, 0),
            _ => unreachable!(),
        }
    }
}

/// The boundary row states the window it collapses back to, so the count it
/// names has to be the kept one — derived from the projection that produced the
/// row, not read off the pane's `TailWindow` after the fact.
#[test]
fn the_boundary_row_carries_the_kept_count_beside_the_hidden_one() {
    let items = turn_of_cycles(8);
    for n in [1usize, 3, 5] {
        let rows = project_tail(&items, StepWindow::uniform(TailWindow::Last(n)));
        match tail_row(&rows).kind {
            RowKind::TailMore {
                hidden_steps,
                kept_steps,
                ..
            } => {
                assert_eq!(kept_steps, n, "n={n}");
                assert_eq!(hidden_steps + kept_steps, 8, "n={n}");
            }
            _ => unreachable!(),
        }
    }
}

/// The two levels compose: the response's boundary covers a whole run, and that
/// run's group still trims its own calls underneath it. Every other in-group
/// test uses a single-run turn, where the response window covers nothing — so
/// this is the one that exercises `push_group_children` layering its window
/// under an already-folded group.
#[test]
fn the_response_window_and_a_group_window_compose() {
    let items = turn_of_cycles(2);
    let tail = StepWindow::uniform(TailWindow::Last(1));

    let shut = project_open_group(&items, tail);
    assert_eq!(
        marks(&shut),
        vec![
            ("user", false),
            ("response", false),
            ("tail", false),
            ("item", true),      // the covered run's prose
            ("group", true),     // and its group, behind the response's boundary
            ("grouptail", true), // whose own boundary is folded with it
            ("item", true),
            ("item", true),
            ("item", false), // the kept run's prose
            ("group", false),
            ("grouptail", false), // trimming the kept run to its last call
            ("item", true),
            ("item", false),
            ("item", false), // the conclusion
        ],
        "a covered run is hidden whole; the kept run is trimmed from inside"
    );

    let mut fold = FoldState::with_mode(FoldPreset::Expanded.mode());
    fold.toggle(FoldKey::Tail(1), FoldContext::last(false));
    let open = project_open_group_under(&items, &fold, tail);
    assert_eq!(
        marks(&open),
        vec![
            ("user", false),
            ("response", false),
            ("tail", false),
            ("item", false),      // the response's reveal surfaces the run
            ("group", false),     // its group
            ("grouptail", false), // and the group's own boundary with it
            ("item", true),       // which still holds this call back
            ("item", false),
            ("item", false),
            ("group", false),
            ("grouptail", false),
            ("item", true),
            ("item", false),
            ("item", false),
        ],
        "revealing a run does not reveal what its group's own window covers"
    );
}

/// The axis's whole point, one level in: a group is one step, so an open run of
/// twenty calls used to ignore `Recent steps` entirely.
#[test]
fn a_window_trims_the_calls_inside_one_group() {
    let items = turn_of_one_group(8, false);
    for kept in [1usize, 3, 5] {
        let rows = project_open_group_calls(&items, TailWindow::Last(kept));
        assert_eq!(
            call_visibility(&items, &rows),
            (0..8).map(|i| i >= 8 - kept).collect::<Vec<_>>(),
            "only the last {kept} calls of the group show"
        );
        assert_eq!(group_tail_counts(&rows), (8 - kept, kept));
        assert!(
            !group_tail_row(&rows).hidden,
            "the group's boundary offers the reveal"
        );
    }
}

#[test]
fn a_window_at_or_above_the_group_size_hides_nothing_inside_it() {
    let items = turn_of_one_group(3, false);
    for tail in [TailWindow::Last(3), TailWindow::Last(10), TailWindow::All] {
        let rows = project_open_group_calls(&items, tail);
        assert_eq!(call_visibility(&items, &rows), vec![true; 3], "{tail:?}");
        let row = group_tail_row(&rows);
        assert!(row.hidden, "nothing to reveal → the row stays zero-height");
        assert_eq!(group_tail_counts(&rows).0, 0, "{tail:?}");
    }
}

/// The row exists for every group whatever the axis says, so changing the
/// window flips `hidden` instead of splicing a row into the list — the same
/// slot-stability the response's own boundary keeps.
#[test]
fn the_group_boundary_keeps_its_slot_as_the_window_changes() {
    let items = turn_of_one_group(4, false);
    let slots = |tail| {
        project_open_group_calls(&items, tail)
            .iter()
            .position(|r| matches!(r.kind, RowKind::ToolGroupTailMore { .. }))
    };
    assert_eq!(slots(TailWindow::All), slots(TailWindow::Last(2)));
    let a = project_open_group_calls(&items, TailWindow::All);
    let b = project_open_group_calls(&items, TailWindow::Last(2));
    assert!(
        a.iter().zip(&b).all(|(x, y)| x.same_slot(y)),
        "the projection differs only in what each row says and shows"
    );
}

/// A collapsed group already shows none of its calls, so its boundary has
/// nothing to offer and must not paint over the header.
#[test]
fn a_collapsed_group_hides_its_own_boundary() {
    let items = turn_of_one_group(6, false);
    let rows = project_open_group_under(
        &items,
        &FoldState::default(),
        StepWindow::uniform(TailWindow::Last(2)),
    );
    assert!(
        group_tail_row(&rows).hidden,
        "the settled group is folded, so its boundary is too"
    );
}

/// A one-call group has a boundary like any other, but it covers nothing — the
/// row exists for slot stability and stays off screen.
#[test]
fn a_one_call_groups_boundary_withholds_nothing() {
    let items = [
        ChatItem::UserText("q".into()),
        asst("working"),
        tool("t0", ToolStatusView::Completed),
        asst("done"),
    ];
    let rows = project_open_group(&items, StepWindow::uniform(TailWindow::Last(1)));
    let boundary = rows
        .iter()
        .find(|r| matches!(r.kind, RowKind::ToolGroupTailMore { .. }))
        .expect("the group has a boundary row");
    assert!(boundary.hidden, "nothing is behind it, so it does not show");
    assert!(matches!(
        boundary.kind,
        RowKind::ToolGroupTailMore {
            hidden_calls: 0,
            ..
        }
    ));
}

/// A running call the group's window covers stays surfaced through a shut
/// boundary, exactly as a live run does under the response's — otherwise the
/// axis would hide the one call the reader is waiting on.
#[test]
fn a_live_covered_call_stays_surfaced_through_a_shut_group_boundary() {
    let mut items = turn_of_one_group(4, false);
    items[2] = tool("g0", ToolStatusView::InProgress);
    let rows = project_open_group(&items, StepWindow::uniform(TailWindow::Last(1)));
    assert_eq!(
        call_visibility(&items, &rows),
        vec![true, false, false, true],
        "the live covered call is on screen; the settled covered ones are not"
    );
}

#[test]
fn a_response_without_tool_calls_gets_no_tail_row() {
    let items = [ChatItem::UserText("hi".into()), asst("hello")];
    let rows = project_tail(&items, StepWindow::uniform(TailWindow::Last(1)));
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::TailMore { .. }))
    );
}

/// The window is a range ending at the last covered run, so a covered run takes
/// the prose that introduced it along — while the conclusion, which follows
/// every run, stays put.
#[test]
fn a_covered_run_takes_its_contents_with_it() {
    let items = turn_of_cycles(4);
    let rows = project_tail(&items, StepWindow::uniform(TailWindow::Last(1)));
    // Three of the four runs are covered, so the kept range opens at the end of
    // the third — item 10, the last cycle's prose.
    for row in &rows {
        match row.kind {
            RowKind::AgentItem(ix) if ix < 10 => {
                assert!(row.hidden, "item {ix} sits behind the boundary")
            }
            RowKind::AgentItem(ix) => assert!(
                row.hidden == matches!(items[ix], ChatItem::ToolCall(_)),
                "item {ix} is inside the window; only its group's own fold holds it"
            ),
            RowKind::ConclusionItem(ix) => {
                assert!(!row.hidden, "the conclusion at {ix} follows every run")
            }
            _ => {}
        }
    }
    assert_eq!(
        group_visibility(&rows),
        vec![false, false, false, true],
        "only the run inside the window keeps its bar"
    );
}

#[test]
fn a_collapsed_response_hides_its_tail_row_too() {
    let items = turn_of_cycles(6);
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(1), FoldContext::past(true));
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::Last(2)),
        &DisplayFilter::default(),
    );
    assert!(tail_row(&rows).hidden, "nothing of a folded turn shows");
}

#[test]
fn a_covered_run_with_a_running_tool_stays_surfaced() {
    let mut items = turn_of_cycles(4);
    items[2] = tool("t0", ToolStatusView::InProgress);
    let rows = project_tail(&items, StepWindow::uniform(TailWindow::Last(1)));
    assert_eq!(
        group_visibility(&rows),
        vec![true, false, false, true],
        "the live run keeps its bar, the settled covered ones fold"
    );
    match tail_row(&rows).kind {
        RowKind::TailMore { hidden_steps, .. } => assert_eq!(hidden_steps, 3),
        _ => unreachable!(),
    }
}

/// Keep the reveal control when every covered run is live.
#[test]
fn a_response_whose_every_covered_run_is_live_keeps_its_tail_row() {
    let mut items = turn_of_cycles(2);
    items[2] = tool("t0", ToolStatusView::InProgress);
    let live = LiveSubagentUnits::of(&items);

    let rows = project_tail(&items, StepWindow::uniform(TailWindow::Last(1)));
    assert_eq!(
        group_visibility(&rows),
        vec![true, true],
        "the covered run is live, so its bar stays surfaced"
    );
    let row = tail_row(&rows);
    assert!(!row.hidden, "the row must stay to offer the reveal");
    let RowKind::TailMore {
        run_start,
        hidden_steps,
        collapsed,
        ..
    } = row.kind
    else {
        unreachable!()
    };
    assert_eq!(hidden_steps, 1, "the window covers one run");
    assert!(collapsed, "and it has not been revealed yet");
    let covered_prose = |rows: &[RenderRow]| {
        rows.iter()
            .find(|r| matches!(r.kind, RowKind::AgentItem(1)))
            .expect("the covered range's assistant block keeps its row")
            .hidden
    };
    assert!(
        covered_prose(&rows),
        "folded away while the row is collapsed"
    );
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Tail(run_start), FoldContext::last(false));
    let revealed = project(
        &items,
        &fold,
        false,
        &live,
        StepWindow::uniform(TailWindow::Last(1)),
        &DisplayFilter::default(),
    );
    assert!(
        !covered_prose(&revealed),
        "clicking the row unfolds the range it covers"
    );
}

#[test]
fn changing_the_window_keeps_every_row_in_its_slot() {
    let items = turn_of_cycles(8);
    let all = project_tail(&items, StepWindow::uniform(TailWindow::All));
    for tail in [
        StepWindow::uniform(TailWindow::Last(1)),
        StepWindow::uniform(TailWindow::Last(3)),
        StepWindow::uniform(TailWindow::Last(10)),
    ] {
        let rows = project_tail(&items, tail);
        assert_eq!(rows.len(), all.len(), "{tail:?} changes no row count");
        assert!(
            all.iter().zip(&rows).all(|(a, b)| a.same_slot(b)),
            "{tail:?} keeps every slot"
        );
    }
}

#[test]
fn tail_rows_share_a_slot_by_their_run_start() {
    let a = RenderRow::at(
        RowKind::TailMore {
            run_start: 1,
            hidden_steps: 12,
            kept_steps: 2,
            collapsed: true,
        },
        false,
        1,
    );
    let b = RenderRow::at(
        RowKind::TailMore {
            run_start: 1,
            hidden_steps: 0,
            kept_steps: 2,
            collapsed: false,
        },
        true,
        1,
    );
    assert!(a.same_slot(&b), "same run_start → same slot");
    assert!(!a.same_slot(&RenderRow::at(
        RowKind::TailMore {
            run_start: 9,
            hidden_steps: 12,
            kept_steps: 2,
            collapsed: true,
        },
        false,
        1
    )));
    assert!(!a.same_slot(&RenderRow::at(RowKind::AgentItem(1), false, 1)));
}

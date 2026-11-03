//! Question rows: a pending one escapes every fold and the filter, a recorded
//! one escapes the filter alone.

use super::*;
use crate::tool_hierarchy::ToolHierarchy;

fn project_questions(
    items: &[ChatItem],
    fold: &FoldState,
    filter: DisplayFilter,
    pinned: &[&str],
    recorded: &[&str],
) -> Vec<RenderRow> {
    let hierarchy = ToolHierarchy::build(items);
    let index = FilterMatchIndex::build(&hierarchy, items, filter).with_questions(
        &hierarchy,
        pinned.iter().copied(),
        recorded.iter().copied(),
    );
    project_with_filter_index(
        items,
        &hierarchy,
        fold,
        false,
        &LiveSubagentUnits::of(items),
        StepWindow::uniform(TailWindow::All),
        &index,
    )
}

fn only_reads() -> DisplayFilter {
    DisplayFilter::from_tokens(["tools", "tool_read"])
}

fn collapsed_response() -> FoldState {
    let mut fold = FoldState::default();
    fold.set_all([FoldKey::Response(1)], false);
    fold
}

/// Visibility of the call row for item `ix`, which must have one.
fn call_hidden(rows: &[RenderRow], ix: usize) -> bool {
    rows.iter()
        .find(|r| matches!(r.kind, RowKind::AgentItem(i) if i == ix))
        .expect("the call owns a row")
        .hidden
}

fn group_hidden(rows: &[RenderRow]) -> bool {
    rows.iter()
        .find(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
        .expect("group header present")
        .hidden
}

#[test]
fn a_pinned_question_row_shows_through_a_collapsed_response() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        tool("t1", Completed),
        tool("ask", Completed),
    ];
    let rows = project_questions(
        &items,
        &collapsed_response(),
        DisplayFilter::default(),
        &["ask"],
        &[],
    );
    assert!(
        group_hidden(&rows),
        "the sibling's group folds; the question is not in it"
    );
    assert!(!call_hidden(&rows, 2), "the question's row is on screen");
    assert!(call_hidden(&rows, 1), "its sibling folds with the response");

    let plain = project_questions(
        &items,
        &collapsed_response(),
        DisplayFilter::default(),
        &[],
        &[],
    );
    assert!(group_hidden(&plain) && call_hidden(&plain, 2));
}

#[test]
fn a_pinned_question_row_shows_through_a_tool_filter_that_excludes_other() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        tool("ask", Completed),
        tool("t2", Completed),
    ];
    let fold = FoldState::with_mode(FoldPreset::Expanded.mode());
    let rows = project_questions(&items, &fold, only_reads(), &["ask"], &[]);
    assert!(group_hidden(&rows), "the edit's group is filtered away");
    assert!(
        !call_hidden(&rows, 1),
        "the pending question passes the filter"
    );
    assert!(call_hidden(&rows, 2), "other edits are still filtered out");
}

#[test]
fn a_recorded_question_row_survives_the_filter_but_folds_with_its_response() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        tool("ask", Completed),
        tool("t2", Completed),
    ];
    let open = FoldState::with_mode(FoldPreset::Expanded.mode());
    let rows = project_questions(&items, &open, only_reads(), &[], &["ask"]);
    assert!(
        !call_hidden(&rows, 1),
        "the record is exempt from the filter"
    );
    assert!(call_hidden(&rows, 2));

    let rows = project_questions(
        &items,
        &collapsed_response(),
        DisplayFilter::default(),
        &[],
        &["ask"],
    );
    assert!(
        group_hidden(&rows) && call_hidden(&rows, 1),
        "it folds with its response"
    );
}

#[test]
fn a_pinned_call_nested_in_a_subagent_keeps_its_parent_reachable() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        subagent_launch("task", Completed),
        child_of("ask", "task", Completed),
    ];
    let rows = project_questions(
        &items,
        &collapsed_response(),
        DisplayFilter::default(),
        &["ask"],
        &[],
    );
    assert!(!group_hidden(&rows));
    assert!(!call_hidden(&rows, 1), "the parent card is on screen");
}

#[test]
fn a_pinned_nested_call_and_its_parent_survive_a_filter_excluding_their_kinds() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        subagent_launch("task", Completed),
        child_of("ask", "task", Completed),
    ];
    let index_of = |pinned: &[&str]| {
        let hierarchy = ToolHierarchy::build(&items);
        FilterMatchIndex::build(&hierarchy, &items, only_reads()).with_questions(
            &hierarchy,
            pinned.iter().copied(),
            [],
        )
    };
    let plain = index_of(&[]);
    assert!(!plain.keeps_id("task") && !plain.keeps_id("ask"));

    let rows = project_questions(
        &items,
        &FoldState::with_mode(FoldPreset::Expanded.mode()),
        only_reads(),
        &["ask"],
        &[],
    );
    assert!(!group_hidden(&rows));
    assert!(
        !call_hidden(&rows, 1),
        "the parent stays through the filter"
    );
    let pinned = index_of(&["ask"]);
    assert!(pinned.keeps_id("ask") && pinned.pins("task"));
}

/// `[Read, AskUserQuestion, Edit]`: the read and the edit each get a group,
/// and the question sits between them at the run's item level.
fn read_ask_edit(ask: ToolStatusView) -> [ChatItem; 4] {
    use ToolStatusView::Completed;
    [
        ChatItem::UserText("q".into()),
        read_tool("r", Completed),
        tool("ask", ask),
        kinded_tool("e", ToolKindView::Edit, Completed),
    ]
}

/// The question's row, after checking it sits between the two group bars as
/// an item of the run rather than a group's child.
fn question_between_groups(rows: &[RenderRow]) -> &RenderRow {
    let headers: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
        .map(|(i, _)| i)
        .collect();
    assert_eq!(headers.len(), 2, "one group on each side of the question");
    let at = rows
        .iter()
        .position(|r| matches!(r.kind, RowKind::AgentItem(2)))
        .expect("the question owns a row");
    assert!(headers[0] < at && at < headers[1]);
    assert_eq!(
        rows[at].indent, 1,
        "the run's item level, like a permission"
    );
    &rows[at]
}

fn open_response() -> FoldState {
    let mut fold = FoldState::default();
    fold.set_all([FoldKey::Response(1)], true);
    fold
}

#[test]
fn a_recorded_question_is_not_grouped_and_shows_while_its_response_is_open() {
    let items = read_ask_edit(ToolStatusView::Completed);
    let rows = project_questions(
        &items,
        &open_response(),
        DisplayFilter::default(),
        &[],
        &["ask"],
    );
    assert!(
        rows.iter()
            .filter(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
            .all(|r| matches!(
                r.kind,
                RowKind::ToolGroupHeader {
                    collapsed: true,
                    ..
                }
            )),
        "both settled groups are shut"
    );
    assert!(
        !question_between_groups(&rows).hidden,
        "the record shows without opening a group"
    );
}

#[test]
fn a_recorded_question_folds_with_its_response() {
    let items = read_ask_edit(ToolStatusView::Completed);
    let rows = project_questions(
        &items,
        &collapsed_response(),
        DisplayFilter::default(),
        &[],
        &["ask"],
    );
    assert!(question_between_groups(&rows).hidden);
}

#[test]
fn a_pending_question_is_its_own_row() {
    let items = read_ask_edit(ToolStatusView::InProgress);
    let rows = project_questions(
        &items,
        &collapsed_response(),
        DisplayFilter::default(),
        &["ask"],
        &[],
    );
    assert!(!question_between_groups(&rows).hidden);
}

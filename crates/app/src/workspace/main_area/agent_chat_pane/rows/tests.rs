use super::*;
use crate::workspace::main_area::agent_chat_pane::display_filter::{DisplayFilter, FilterFacet};
use crate::workspace::main_area::agent_chat_pane::fold::FoldContext;
use crate::workspace::main_area::agent_chat_pane::fold_mode::FoldPreset;
use crate::workspace::main_area::agent_chat_pane::rows::tail::TailWindow;
use crate::workspace::main_area::agent_chat_pane::tool_hierarchy::SUBAGENT_NEST_DEPTH_CAP;
use daruda_acp::{
    PermissionItem, PermissionResolution, ToolCallItem, ToolKindView, ToolStatusView,
};

fn tool(id: &str, status: ToolStatusView) -> ChatItem {
    ChatItem::ToolCall(ToolCallItem {
        id: id.to_owned(),
        title: format!("Tool {id}"),
        kind: ToolKindView::Edit,
        tool_name: None,
        status,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: None,
        parent_tool_id: None,
        exit: None,
    })
}
fn perm(resolved: bool) -> ChatItem {
    ChatItem::Permission(PermissionItem {
        id: 0,
        tool_title: Some("Write /tmp/x".to_owned()),
        raw_input_summary: None,
        options: Vec::new(),
        resolved: resolved.then_some(PermissionResolution::Cancelled),
    })
}
fn asst(s: &str) -> ChatItem {
    ChatItem::AssistantText {
        text: s.to_owned(),
        streaming: false,
        message_id: None,
        phase: Default::default(),
    }
}
fn kinds(rows: &[RenderRow]) -> Vec<(&'static str, bool)> {
    rows.iter()
        .map(|r| {
            let k = match r.kind {
                RowKind::User(_) => "user",
                RowKind::ResponseHeader { .. } => "response",
                RowKind::AgentItem(_) | RowKind::ConclusionItem(_) => "item",
                RowKind::SoloResponse(_) => "solo",
                RowKind::StepHeader { .. } => "step",
                RowKind::TailMore { .. } => "tail",
                RowKind::FilteredAway { .. } => "filtered",
                RowKind::ToolGroupHeader { .. } => "group",
                RowKind::WorkingIndicator => "working",
            };
            (k, r.hidden)
        })
        .collect()
}

#[test]
fn turn_with_tools_nests_response_and_group() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        asst("working"),
        tool("a", Completed),
        tool("b", Completed),
        tool("c", Completed),
        asst("done"),
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("filtered", true),
            ("tail", true),
            ("step", false),
            ("item", true),
            ("group", true),
            ("item", true),
            ("item", true),
            ("item", true),
            ("item", false),
        ]
    );
    let indents: Vec<u8> = rows.iter().map(|r| r.indent).collect();
    assert_eq!(indents, vec![0, 0, 1, 1, 1, 2, 2, 3, 3, 3, 1]);
}

#[test]
fn trivial_response_has_no_bar() {
    let items = [ChatItem::UserText("hi".into()), asst("hello")];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert_eq!(
        kinds(&rows),
        vec![("user", false), ("filtered", true), ("solo", false)]
    );
}

#[test]
fn past_turn_collapses_current_expands() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("first".into()),
        asst("a1"),
        tool("t1", Completed),
        tool("t2", Completed),
        ChatItem::UserText("second".into()),
        asst("a2"),
        tool("t3", Completed),
        tool("t4", Completed),
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),     // first
            ("response", false), // header always shown
            ("filtered", true),
            ("tail", true),
            ("step", true),  // settled step under a collapsed response
            ("item", false), // a1 = conclusion, stays visible
            ("group", true),
            ("item", true),
            ("item", true),
            ("user", false), // second
            ("response", false),
            ("filtered", true),
            ("tail", true),
            ("step", false), // current turn's step: one row for the cycle
            ("item", false), // a2 = conclusion, never folded away
            ("group", true),
            ("item", true),
            ("item", true),
        ]
    );
}

fn two_settled_turns() -> [ChatItem; 8] {
    use ToolStatusView::Completed;
    [
        ChatItem::UserText("first".into()),
        asst("a1"),
        tool("t1", Completed),
        tool("t2", Completed),
        ChatItem::UserText("second".into()),
        asst("a2"),
        tool("t3", Completed),
        tool("t4", Completed),
    ]
}

fn project_under(items: &[ChatItem], fold: &FoldState) -> Vec<RenderRow> {
    project(
        items,
        fold,
        false,
        &LiveSubagentUnits::of(items),
        TailWindow::All,
        &DisplayFilter::default(),
    )
}

#[test]
fn auto_is_the_projection_default() {
    let items = two_settled_turns();
    let implicit = project_under(&items, &FoldState::default());
    let explicit = project_under(&items, &FoldState::with_mode(FoldPreset::Auto.mode()));
    assert_eq!(kinds(&implicit), kinds(&explicit));
    assert_eq!(
        implicit.iter().map(|r| r.indent).collect::<Vec<_>>(),
        explicit.iter().map(|r| r.indent).collect::<Vec<_>>()
    );
}

#[test]
fn summary_mode_folds_the_settled_newest_turn_like_history() {
    let items = two_settled_turns();
    let rows = project_under(&items, &FoldState::with_mode(FoldPreset::Summary.mode()));
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("filtered", true),
            ("tail", true),
            ("step", true),
            ("item", false), // a1 = conclusion
            ("group", true),
            ("item", true),
            ("item", true),
            ("user", false),
            ("response", false),
            ("filtered", true),
            ("tail", true),
            ("step", true),  // the newest turn's step now folds too
            ("item", false), // a2 = conclusion
            ("group", true),
            ("item", true),
            ("item", true),
        ]
    );
}

#[test]
fn expanded_mode_opens_past_responses_and_keeps_settled_newest_steps_open() {
    let items = two_settled_turns();
    let rows = project_under(&items, &FoldState::with_mode(FoldPreset::Expanded.mode()));
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("filtered", true),
            ("tail", true),
            ("step", false), // past response now open → its step header shows
            ("item", false), // a1
            ("group", true),
            ("item", true),
            ("item", true),
            ("user", false),
            ("response", false),
            ("filtered", true),
            ("tail", true),
            ("step", false),
            ("item", false), // a2 — the newest turn's settled step stays open
            ("group", false),
            ("item", true),
            ("item", true),
        ]
    );
}

#[test]
fn a_user_fold_survives_a_mode_switch() {
    let items = two_settled_turns();
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(4), FoldContext::last(false)); // newest turn → collapsed
    for preset in FoldPreset::ALL {
        fold.set_mode(preset.mode());
        let rows = project_under(&items, &fold);
        let newest_step = kinds(&rows)[13];
        assert_eq!(newest_step, ("step", true), "{preset:?}");
    }
}

#[test]
fn working_indicator_fills_gap_after_tool_group_settles() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        asst("planning"),
        tool("a", Completed),
        tool("b", Completed),
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        true,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("filtered", true),
            ("tail", true),
            ("step", false), // the settled cycle, folded to one row
            ("item", false), // assistant prose = conclusion, never folded away
            ("group", true), // tool group header, folded into the step
            ("item", true),  // settled members collapsed
            ("item", true),
            ("working", false), // gap indicator at the run tail
        ]
    );
    // The indicator nests inside the response (indent 1), not at top level.
    assert_eq!(rows.last().unwrap().indent, 1);
}

#[test]
fn working_indicator_present_while_streaming() {
    use ToolStatusView::Completed;
    // Even while the tail block streams, the indicator stays pinned to the
    // run's tail so the "working … + elapsed" signal stays visible through
    // streaming and tool execution.
    let items = [
        ChatItem::UserText("q".into()),
        tool("a", Completed),
        ChatItem::AssistantText {
            text: "answer".into(),
            streaming: true,
            message_id: None,
            phase: Default::default(),
        },
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        true,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert!(
        kinds(&rows).iter().any(|(k, _)| *k == "working"),
        "a working turn keeps the indicator even while streaming"
    );
}

#[test]
fn working_indicator_only_when_awaiting_response() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        asst("planning"),
        tool("a", Completed),
        tool("b", Completed),
    ];
    // Settled turn, nothing in flight → no indicator.
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert!(!kinds(&rows).iter().any(|(k, _)| *k == "working"));
}

#[test]
fn working_indicator_on_first_token_wait() {
    // Prompt sent, no agent output yet, turn in flight → indicator under the
    // user message at top level (no response bar for an empty run).
    let items = [ChatItem::UserText("q".into())];
    let rows = project(
        &items,
        &FoldState::default(),
        true,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert_eq!(kinds(&rows), vec![("user", false), ("working", false)]);
    assert_eq!(rows.last().unwrap().indent, 0);
}

#[test]
fn working_indicator_visible_when_response_collapsed() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        asst("planning"),
        tool("a", Completed),
        tool("b", Completed),
    ];
    let mut fold = FoldState::default();
    // User manually collapses the (last, in-flight) response.
    fold.toggle(FoldKey::Response(0), FoldContext::past(true));
    let rows = project(
        &items,
        &fold,
        true,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    let working = rows
        .iter()
        .find(|r| matches!(r.kind, RowKind::WorkingIndicator))
        .expect("indicator still projected");
    assert!(
        !working.hidden,
        "an in-flight turn keeps its working indicator even when the response is collapsed"
    );
    // Pinned at the run's indent (1), aligned under the still-visible conclusion.
    assert_eq!(working.indent, 1);
}

#[test]
fn conclusion_stays_visible_when_response_collapsed() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        asst("let me look"),
        tool("a", Completed),
        tool("b", Completed),
        asst("done: fixed it"),
    ];
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(0), FoldContext::past(true)); // collapse the response
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("filtered", true),
            ("tail", true),
            ("step", true), // the whole cycle folds with the response
            ("item", true), // "let me look" process → hidden
            ("group", true),
            ("item", true),
            ("item", true),
            ("item", false), // "done: fixed it" conclusion → visible
        ]
    );
}

#[test]
fn conclusion_under_a_response_is_a_separately_foldable_item() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        asst("let me look"),
        tool("a", Completed),
        tool("b", Completed),
        asst("done"),
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    // The final assistant block projects as a ConclusionItem (its own fold
    // toggle); the earlier prose stays a plain AgentItem.
    assert!(
        rows.iter()
            .any(|r| matches!(r.kind, RowKind::ConclusionItem(4))),
        "the run's last assistant text is a ConclusionItem"
    );
    assert!(
        rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(1))),
        "earlier prose stays a plain AgentItem"
    );
}

#[test]
fn trivial_reply_is_not_a_conclusion_item() {
    // A lone reply has no response bar, so it renders as the normal
    // (labeled) foldable assistant block, not a bare-chevron ConclusionItem.
    let items = [ChatItem::UserText("hi".into()), asst("hello")];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert!(
        rows.iter()
            .any(|r| matches!(r.kind, RowKind::SoloResponse(1)))
    );
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::ConclusionItem(_)))
    );
}

#[test]
fn only_the_last_assistant_message_is_the_conclusion() {
    // Two distinct agent messages (mapping split them by messageId) with no
    // tool between → only the last is the conclusion; the earlier one folds
    // into the process.
    let items = [
        ChatItem::UserText("q".into()),
        asst("first message"),
        asst("second message"),
    ];
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(0), FoldContext::past(true));
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("filtered", true),
            ("item", true),  // first message → process
            ("item", false), // last message → conclusion
        ]
    );
}

#[test]
fn conclusion_is_last_assistant_even_before_trailing_tool() {
    // The run ends with tools and no final text; the last assistant text
    // (mid-run) is still treated as the conclusion and stays visible.
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        asst("answer"),
        tool("a", Completed),
        tool("b", Completed),
    ];
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(0), FoldContext::past(true));
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("filtered", true),
            ("tail", true),
            ("step", true),  // the step absorbed the conclusion's own prose …
            ("item", false), // … but "answer" is still forced visible
            ("group", true),
            ("item", true),
            ("item", true),
        ]
    );
}

#[test]
fn no_conclusion_row_when_run_has_no_assistant_text() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        tool("a", Completed),
        tool("b", Completed),
    ];
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(0), FoldContext::past(true));
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("filtered", true),
            ("tail", true),
            ("group", true), // no assistant text → nothing stays visible
            ("item", true),
            ("item", true),
        ]
    );
}

#[test]
fn permission_visibility_tracks_actionability_when_response_collapsed() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        tool("a", Completed),
        tool("b", Completed),
        perm(false), // pending → actionable
    ];
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(0), FoldContext::past(true)); // collapse the response
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    let perm_row = rows
        .iter()
        .find(|r| matches!(r.kind, RowKind::AgentItem(3)))
        .expect("permission row present");
    assert!(
        !perm_row.hidden,
        "a pending permission is never folded away"
    );
    // The process (the tool group) is still hidden.
    assert!(
        rows.iter()
            .filter(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
            .all(|r| r.hidden),
        "the tool group still folds"
    );

    let items = [
        ChatItem::UserText("q".into()),
        tool("a", Completed),
        tool("b", Completed),
        perm(true), // resolved → no longer actionable
    ];
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(0), FoldContext::past(true));
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    let perm_row = rows
        .iter()
        .find(|r| matches!(r.kind, RowKind::AgentItem(3)))
        .expect("permission row present");
    assert!(
        perm_row.hidden,
        "a resolved permission folds with the process"
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
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert!(
        rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(1))),
        "the parent tool call still renders as a row"
    );
    assert!(
        !rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(2))),
        "the subagent child earns no row of its own"
    );
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. })),
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
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert!(rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(1))));
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::AgentItem(2) | RowKind::AgentItem(3))),
        "both subagent children are skipped from the main flow"
    );
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. })),
        "a parent + its children are not a sibling tool group"
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
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert!(
        rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(1))),
        "an orphan child (no present parent) still renders as a row"
    );
}

fn tool_of(items: &[ChatItem], id: &str) -> ToolCallItem {
    items
        .iter()
        .find_map(|it| match it {
            ChatItem::ToolCall(tc) if tc.id == id => Some(tc.clone()),
            _ => None,
        })
        .expect("tool call present")
}

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

#[test]
fn a_matching_grandchild_keeps_the_whole_subtree() {
    use ToolStatusView::Completed;
    // The match is three levels down: the ancestors come along to reach it, and
    // the sibling branch comes along because its parent card renders.
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
    for id in ["task", "middle", "leaf", "sibling"] {
        assert!(index.keeps_tool(&tool_of(&items, id)), "{id}");
    }
}

#[test]
fn a_cyclic_parent_link_does_not_hang_the_index() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        child_of("a", "b", Completed),
        child_of("b", "a", Completed),
    ];
    let index = FilterMatchIndex::of(&items, DisplayFilter::from_tokens(["tools", "tool_edit"]));
    assert!(index.keeps_tool(&tool_of(&items, "a")));
    assert!(index.keeps_tool(&tool_of(&items, "b")));
}

fn child_of(id: &str, parent: &str, status: ToolStatusView) -> ChatItem {
    let mut c = tool(id, status);
    if let ChatItem::ToolCall(tc) = &mut c {
        tc.parent_tool_id = Some(parent.to_owned());
    }
    c
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
        TailWindow::All,
        &DisplayFilter::default(),
    );
    let elapsed = started.elapsed();
    assert_eq!(
        rows.len(),
        items.len() + 3,
        "the run's filter row + tail row + one group header + every member"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "projecting 4000 items took {elapsed:?} — the quadratic scan is back"
    );
}

fn one_turn_of_cycles(cycles: usize) -> Vec<ChatItem> {
    use ToolStatusView::Completed;
    let mut items = vec![ChatItem::UserText("q".into())];
    for c in 0..cycles {
        items.push(think(&format!("why {c}")));
        items.push(think(&format!("also {c}")));
        items.push(asst(&format!("plan {c}")));
        for t in 0..4 {
            items.push(tool(&format!("t{c}-{t}"), Completed));
        }
    }
    items.push(asst("done"));
    items
}

fn project_all(items: &[ChatItem]) -> Vec<RenderRow> {
    project(
        items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(items),
        TailWindow::All,
        &DisplayFilter::default(),
    )
}

/// Guard against per-key rescans by checking growth when one turn doubles.
#[test]
fn a_long_single_turn_of_steps_stays_linear() {
    const N: usize = 250;
    let small = one_turn_of_cycles(N);
    let large = one_turn_of_cycles(N * 2);

    let steps = |rows: &[RenderRow]| {
        rows.iter()
            .filter(|r| matches!(r.kind, RowKind::StepHeader { .. }))
            .count()
    };
    assert!(
        steps(&project_all(&small)) >= N,
        "the fixture must actually build steps"
    );

    let sample = |items: &[ChatItem]| {
        let started = std::time::Instant::now();
        std::hint::black_box(project_all(items));
        started.elapsed()
    };
    // Interleave sizes and keep minima to reduce scheduler-noise sensitivity.
    let (mut t1, mut t2) = (std::time::Duration::MAX, std::time::Duration::MAX);
    for _ in 0..40 {
        t1 = t1.min(sample(&small));
        t2 = t2.min(sample(&large));
    }
    let ratio = t2.as_secs_f64() / t1.as_secs_f64();
    assert!(
        ratio < 2.6,
        "doubling a single turn cost {ratio:.2}× ({t1:?} -> {t2:?}) — \
         the per-key rescan is back"
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

#[test]
fn collapsed_response_surfaces_a_live_tool_group() {
    use ToolStatusView::{Completed, InProgress};
    // A non-trivial response (a 2-tool group) with one live member, response
    // manually collapsed: the group header pops out (still folded) so a
    // folded in-flight turn shows *what* is running; members stay hidden.
    let items = [
        ChatItem::UserText("q".into()),
        tool("t1", InProgress),
        tool("t2", Completed),
    ];
    let mut fold = FoldState::default();
    fold.set_all([FoldKey::Response(0)], false); // force the response collapsed
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    let header = rows
        .iter()
        .find(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
        .expect("group header present");
    assert!(
        !header.hidden,
        "a collapsed response still surfaces a live tool group header"
    );
    assert!(
        rows.iter()
            .filter(|r| matches!(r.kind, RowKind::AgentItem(_)))
            .all(|r| r.hidden),
        "the live group's members stay folded under the collapsed response"
    );
}

#[test]
fn collapsed_response_hides_a_settled_tool_group() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        tool("t1", Completed),
        tool("t2", Completed),
    ];
    let mut fold = FoldState::default();
    fold.set_all([FoldKey::Response(0)], false);
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    let header = rows
        .iter()
        .find(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
        .expect("group header present");
    assert!(
        header.hidden,
        "a settled tool group folds away with the collapsed response"
    );
}

#[test]
fn lone_tool_call_is_not_grouped() {
    let items = [asst("x"), tool("a", ToolStatusView::Completed), asst("y")];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert_eq!(
        kinds(&rows),
        vec![
            ("filtered", true),
            ("tail", true),
            ("step", false), // the prose + the lone call are one settled cycle
            ("item", true),  // "x"
            ("item", true),  // the tool call — a plain item, not a group
            ("item", false), // "y" = conclusion, never folded away
        ],
        "a single tool call renders as a plain item, no group header"
    );
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
    );
}

#[test]
fn in_progress_group_defaults_expanded() {
    use ToolStatusView::{Completed, InProgress};
    let items = [tool("a", Completed), tool("b", InProgress)];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    // group active (one tool in progress) → members visible.
    assert_eq!(
        kinds(&rows),
        vec![
            ("filtered", true),
            ("tail", true),
            ("group", false),
            ("item", false),
            ("item", false)
        ]
    );
}

#[test]
fn group_member_visibility_follows_fold_override() {
    use ToolStatusView::Completed;
    let items = [tool("a", Completed), tool("b", Completed)];
    let mut fold = FoldState::default();
    // Force-expand the (otherwise collapsed) settled group.
    fold.toggle(FoldKey::ToolGroup("a".into()), FoldContext::past(false));
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert_eq!(
        kinds(&rows),
        vec![
            ("filtered", true),
            ("tail", true),
            ("group", false),
            ("item", false),
            ("item", false)
        ],
        "user-expanded group shows its members"
    );
}

/// Every `RowKind` must land in its own slot family, so two different kinds
/// carrying the same index never collapse into one slot. The "a new variant
/// cannot forget to declare its identity" half is the compiler's job:
/// `RowKind::slot` matches exhaustively with no wildcard arm.
#[test]
fn every_row_kind_declares_a_distinct_slot() {
    let row = |kind| RenderRow::at(kind, false, 0);
    // One row per `RowKind` variant, all keyed on the same index / id so only
    // the variant itself can tell them apart.
    let kinds = vec![
        RowKind::User(0),
        RowKind::ResponseHeader {
            anchor: 0,
            collapsed: false,
        },
        RowKind::AgentItem(0),
        RowKind::SoloResponse(0),
        RowKind::StepHeader {
            first_ix: 0,
            tool_count: 0,
            collapsed: false,
        },
        RowKind::TailMore {
            run_start: 0,
            hidden_steps: 0,
            kept_steps: 0,
            collapsed: false,
        },
        RowKind::FilteredAway {
            run_start: 0,
            revealable: 0,
            excluded: 0,
            collapsed: false,
        },
        RowKind::ToolGroupHeader {
            gid: "g".into(),
            first_ix: 0,
            count: 0,
            collapsed: false,
        },
        RowKind::ConclusionItem(0),
        RowKind::WorkingIndicator,
    ];
    let rows: Vec<RenderRow> = kinds.into_iter().map(row).collect();
    for (i, a) in rows.iter().enumerate() {
        for (j, b) in rows.iter().enumerate() {
            assert_eq!(
                a.same_slot(b),
                i == j,
                "row kind {i} vs {j} must share a slot only with itself"
            );
        }
    }
}

#[test]
fn same_slot_compares_key_not_hidden_or_payload() {
    let a = RenderRow::at(
        RowKind::ToolGroupHeader {
            gid: "g".into(),
            first_ix: 1,
            count: 2,
            collapsed: false,
        },
        false,
        0,
    );
    let b = RenderRow::at(
        RowKind::ToolGroupHeader {
            gid: "g".into(),
            first_ix: 5,
            count: 3,
            collapsed: true,
        },
        true,
        0,
    );
    assert!(
        a.same_slot(&b),
        "same gid → same slot regardless of count/hidden"
    );

    let u0 = RenderRow::at(RowKind::User(0), false, 0);
    let u1 = RenderRow::at(RowKind::User(1), false, 0);
    assert!(!u0.same_slot(&u1));
    assert!(!u0.same_slot(&RenderRow::at(RowKind::AgentItem(0), false, 0)));
}

#[test]
fn collapsed_response_survivors_all_sit_at_the_run_indent() {
    use ToolStatusView::{Completed, InProgress};
    let items = [
        ChatItem::UserText("q".into()),
        asst("planning"),
        tool("a", Completed),
        tool("b", InProgress),
        asst("here is the result"),
    ];
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(0), FoldContext::past(true));
    let rows = project(
        &items,
        &fold,
        true,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );

    for row in rows.iter().filter(|r| !r.hidden) {
        let expected = match row.kind {
            RowKind::User(_) | RowKind::ResponseHeader { .. } => 0,
            RowKind::ToolGroupHeader { .. } => 2,
            _ => 1,
        };
        assert_eq!(
            row.indent,
            expected,
            "{:?} sits at the wrong depth under a collapsed response",
            kinds(std::slice::from_ref(row))
        );
    }
    let visible = kinds(&rows)
        .into_iter()
        .filter(|(_, hidden)| !hidden)
        .map(|(k, _)| k)
        .collect::<Vec<_>>();
    assert_eq!(
        visible,
        vec!["user", "response", "step", "group", "item", "working"]
    );
}

#[test]
fn leading_run_without_a_user_anchor_gets_no_solo_response() {
    use ToolStatusView::Failed;
    // Reachable on session restore: `append_user_chunk` drops a replayed
    // `<task-notification>` user turn (see daruda_acp::mapping), so a restored
    // pane can open with agent items and no `UserText` anchor. Such a run gets no
    // response bar, and its blocks must stay plain `AgentItem`s — a `SoloResponse`
    // here would let the assistant header report ✓ over itself while the sibling
    // tool call sitting right next to it has failed.
    let items = [asst("here is what I found"), tool("c1", Failed)];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );

    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::SoloResponse(_))),
        "an unanchored run has no block that stands for the whole response"
    );
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::ResponseHeader { .. })),
        "and no bar is emitted for it either"
    );
    assert_eq!(
        kinds(&rows),
        vec![
            ("filtered", true),
            ("tail", true),
            ("step", false),
            ("item", false),
            ("item", true)
        ]
    );
}

#[test]
fn anchored_multi_block_run_puts_the_rollup_on_the_bar_not_a_block() {
    // The complement: two blocks under an anchor *is* non-trivial, so the bar
    // carries the verdict and no block does.
    let items = [
        ChatItem::UserText("q".into()),
        asst("thinking out loud"),
        asst("done"),
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::SoloResponse(_)))
    );
    assert!(
        rows.iter()
            .any(|r| matches!(r.kind, RowKind::ResponseHeader { .. }))
    );
}

// ── Step boundaries ─────────────────────────────────────────────────────────
//
fn think(s: &str) -> ChatItem {
    ChatItem::Thinking {
        text: s.to_owned(),
        streaming: false,
        message_id: None,
    }
}

#[test]
fn a_step_absorbs_the_prose_in_front_of_its_run() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        think("why"),
        asst("here goes"),
        tool("a", Completed),
        tool("b", Completed),
        asst("done"),
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    let step = rows
        .iter()
        .find_map(|r| match r.kind {
            RowKind::StepHeader {
                first_ix,
                tool_count,
                ..
            } => Some((first_ix, tool_count)),
            _ => None,
        })
        .expect("the cycle earns a step header");
    assert_eq!(
        step,
        (1, 2),
        "the step opens at the thinking block and owns both tool calls"
    );
    assert!(
        rows.iter()
            .filter(|r| matches!(r.kind, RowKind::AgentItem(1) | RowKind::AgentItem(2)))
            .all(|r| r.hidden),
        "a settled step folds the prose it absorbed"
    );
    assert!(
        rows.iter()
            .any(|r| matches!(r.kind, RowKind::ConclusionItem(5)) && !r.hidden)
    );
}

#[test]
fn consecutive_runs_split_into_one_step_each() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        asst("first"),
        tool("a", Completed),
        tool("b", Completed),
        asst("second"),
        tool("c", Completed),
        tool("d", Completed),
        asst("done"),
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    let starts: Vec<usize> = rows
        .iter()
        .filter_map(|r| match r.kind {
            RowKind::StepHeader { first_ix, .. } => Some(first_ix),
            _ => None,
        })
        .collect();
    assert_eq!(starts, vec![1, 4], "one step per cycle, split at the prose");
}

#[test]
fn a_response_without_tools_gets_no_step() {
    let items = [
        ChatItem::UserText("q".into()),
        asst("first message"),
        asst("second message"),
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("filtered", true),
            ("item", false),
            ("item", false),
        ]
    );
}

#[test]
fn a_prose_less_lone_tool_gets_no_step() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        tool("a", Completed),
        asst("done"),
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::StepHeader { .. })),
        "one tool call is already one row"
    );
    let items = [
        ChatItem::UserText("q".into()),
        tool("a", Completed),
        tool("b", Completed),
        asst("done"),
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::StepHeader { .. }))
    );
}

#[test]
fn the_running_step_expands_while_its_settled_sibling_folds() {
    use ToolStatusView::{Completed, InProgress};
    let items = [
        ChatItem::UserText("q".into()),
        asst("first"),
        tool("a", Completed),
        tool("b", Completed),
        asst("second"),
        tool("c", InProgress),
        tool("d", Completed),
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        true,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    let collapsed = |first: usize| {
        rows.iter()
            .find_map(|r| match r.kind {
                RowKind::StepHeader {
                    first_ix,
                    collapsed,
                    ..
                } if first_ix == first => Some(collapsed),
                _ => None,
            })
            .expect("step header present")
    };
    assert!(collapsed(1), "the settled cycle folds to its header");
    assert!(!collapsed(4), "the running cycle stays open");
    assert!(
        rows.iter()
            .any(|r| matches!(r.kind, RowKind::AgentItem(5)) && !r.hidden)
    );
}

#[test]
fn a_collapsed_response_surfaces_a_live_step() {
    use ToolStatusView::{Completed, InProgress};
    let items = [
        ChatItem::UserText("q".into()),
        asst("working"),
        tool("a", InProgress),
        tool("b", Completed),
    ];
    let mut fold = FoldState::default();
    fold.set_all([FoldKey::Response(0)], false);
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    let step = rows
        .iter()
        .find(|r| matches!(r.kind, RowKind::StepHeader { .. }))
        .expect("step header present");
    assert!(!step.hidden, "a live step survives a collapsed response");

    let items = [
        ChatItem::UserText("q".into()),
        asst("working"),
        tool("a", Completed),
        tool("b", Completed),
    ];
    let mut fold = FoldState::default();
    fold.set_all([FoldKey::Response(0)], false);
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    let step = rows
        .iter()
        .find(|r| matches!(r.kind, RowKind::StepHeader { .. }))
        .expect("step header present");
    assert!(step.hidden, "a settled step folds with the response");
}

#[test]
fn a_pending_permission_survives_a_folded_step() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        asst("working"),
        perm(false),
        tool("a", Completed),
        tool("b", Completed),
        asst("done"),
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    let perm_row = rows
        .iter()
        .find(|r| matches!(r.kind, RowKind::AgentItem(2)))
        .expect("permission row present");
    assert!(
        !perm_row.hidden,
        "a pending permission is never folded away, step or no step"
    );
}

#[test]
fn step_headers_share_a_slot_by_their_first_index() {
    let a = RenderRow::at(
        RowKind::StepHeader {
            first_ix: 3,
            tool_count: 2,
            collapsed: false,
        },
        false,
        1,
    );
    let b = RenderRow::at(
        RowKind::StepHeader {
            first_ix: 3,
            tool_count: 5,
            collapsed: true,
        },
        true,
        1,
    );
    assert!(a.same_slot(&b), "same first_ix → same slot");
    assert!(!a.same_slot(&RenderRow::at(
        RowKind::StepHeader {
            first_ix: 4,
            tool_count: 2,
            collapsed: false,
        },
        false,
        1
    )));
    assert!(!a.same_slot(&RenderRow::at(RowKind::AgentItem(3), false, 1)));
}

// ── Tail window ────────────────────────────────────────────────────────────

fn turn_of_cycles(cycles: usize) -> Vec<ChatItem> {
    let mut items = vec![ChatItem::UserText("q".into())];
    for i in 0..cycles {
        items.push(asst(&format!("cycle {i}")));
        items.push(tool(&format!("t{i}"), ToolStatusView::Completed));
    }
    items.push(asst("done"));
    items
}

fn project_tail(items: &[ChatItem], tail: TailWindow) -> Vec<RenderRow> {
    project(
        items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(items),
        tail,
        &DisplayFilter::default(),
    )
}

fn step_visibility(rows: &[RenderRow]) -> Vec<bool> {
    rows.iter()
        .filter(|r| matches!(r.kind, RowKind::StepHeader { .. }))
        .map(|r| !r.hidden)
        .collect()
}

fn tail_row(rows: &[RenderRow]) -> &RenderRow {
    rows.iter()
        .find(|r| matches!(r.kind, RowKind::TailMore { .. }))
        .expect("a run with steps gets a tail row")
}

#[test]
fn a_window_keeps_only_the_last_steps_visible() {
    let items = turn_of_cycles(8);
    for (n, kept) in [(1usize, 1usize), (3, 3), (5, 5)] {
        let rows = project_tail(&items, TailWindow::Last(n));
        let vis = step_visibility(&rows);
        assert_eq!(vis.len(), 8, "every step keeps its row at n={n}");
        assert_eq!(
            vis,
            (0..8).map(|i| i >= 8 - kept).collect::<Vec<_>>(),
            "only the last {kept} steps show at n={n}"
        );
        match tail_row(&rows).kind {
            RowKind::TailMore { hidden_steps, .. } => assert_eq!(hidden_steps, 8 - kept),
            _ => unreachable!(),
        }
        assert!(!tail_row(&rows).hidden, "the tail row offers the reveal");
    }
}

#[test]
fn a_window_at_or_above_the_step_count_hides_nothing() {
    let items = turn_of_cycles(3);
    for tail in [TailWindow::Last(3), TailWindow::Last(10), TailWindow::All] {
        let rows = project_tail(&items, tail);
        assert_eq!(step_visibility(&rows), vec![true; 3], "{tail:?}");
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
        let rows = project_tail(&items, TailWindow::Last(n));
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

/// The rail marks exactly the steps the window covers, and only those — a row
/// inside the kept range never carries it, or the mark would say nothing. With
/// the boundary shut the marked rows are all hidden, so the mark costs nothing
/// until something surfaces one.
#[test]
fn only_rows_outside_the_window_carry_the_rail() {
    let items = turn_of_cycles(6);
    let tail = TailWindow::Last(2);

    let shut = project_tail(&items, tail);
    assert!(
        shut.iter()
            .filter(|r| matches!(r.kind, RowKind::StepHeader { .. }))
            .all(|r| r.outside_window == r.hidden),
        "with the boundary shut, a covered step is marked and hidden while a \
         kept one is neither"
    );

    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Tail(1), FoldContext::past(false));
    let open = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        tail,
        &DisplayFilter::default(),
    );
    let marked: Vec<bool> = open
        .iter()
        .filter(|r| matches!(r.kind, RowKind::StepHeader { .. }))
        .map(|r| r.outside_window)
        .collect();
    assert_eq!(
        marked,
        vec![true, true, true, true, false, false],
        "the four steps the window covers are marked; the two it keeps are not"
    );
    assert!(
        !tail_row(&open).outside_window,
        "the boundary itself is never one of the rows it brackets"
    );
    assert!(
        open.iter().any(|r| r.outside_window && !r.hidden),
        "the marked rows are the ones the reveal put on screen"
    );
}

/// A live step the window covers stays surfaced whether or not the boundary is
/// open — so the rail has to mark it in *both* states. Keying the mark on the
/// boundary being open instead made the same row gain and lose its rail as the
/// boundary flipped, leaving a visible row from outside the range unexplained in
/// exactly the state where nothing else accounts for it.
#[test]
fn a_live_covered_step_carries_the_rail_with_the_boundary_shut() {
    let mut items = turn_of_cycles(4);
    items[2] = tool("t0", ToolStatusView::InProgress);
    let tail = TailWindow::Last(1);

    let shut = project_tail(&items, tail);
    let surfaced: Vec<(bool, bool)> = shut
        .iter()
        .filter(|r| matches!(r.kind, RowKind::StepHeader { .. }))
        .map(|r| (!r.hidden, r.outside_window))
        .collect();
    assert_eq!(
        surfaced,
        vec![(true, true), (false, true), (false, true), (true, false)],
        "the live covered step is on screen and marked; the kept step is neither"
    );

    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Tail(1), FoldContext::past(false));
    let open = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        tail,
        &DisplayFilter::default(),
    );
    let marked: Vec<bool> = open
        .iter()
        .filter(|r| matches!(r.kind, RowKind::StepHeader { .. }))
        .map(|r| r.outside_window)
        .collect();
    assert_eq!(
        marked,
        vec![true, true, true, false],
        "opening the boundary changes which covered rows are visible, not which are outside"
    );
}

#[test]
fn a_response_without_steps_gets_no_tail_row() {
    let items = [ChatItem::UserText("hi".into()), asst("hello")];
    let rows = project_tail(&items, TailWindow::Last(1));
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::TailMore { .. }))
    );
}

#[test]
fn a_covered_step_takes_its_contents_with_it() {
    let items = turn_of_cycles(4);
    let rows = project_tail(&items, TailWindow::Last(1));
    for row in &rows {
        match row.kind {
            RowKind::AgentItem(ix) if ix < 7 => {
                assert!(row.hidden, "item {ix} sits in a covered step")
            }
            RowKind::ConclusionItem(ix) => {
                assert!(!row.hidden, "the conclusion at {ix} is in no step")
            }
            _ => {}
        }
    }
}

#[test]
fn revealing_the_tail_shows_every_step_again() {
    let items = turn_of_cycles(6);
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Tail(1), FoldContext::past(false));
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::Last(2),
        &DisplayFilter::default(),
    );
    assert_eq!(step_visibility(&rows), vec![true; 6]);
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
fn a_collapsed_response_hides_its_tail_row_too() {
    let items = turn_of_cycles(6);
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(0), FoldContext::past(true));
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::Last(2),
        &DisplayFilter::default(),
    );
    assert!(tail_row(&rows).hidden, "nothing of a folded turn shows");
}

#[test]
fn a_covered_step_with_a_running_tool_stays_surfaced() {
    let mut items = turn_of_cycles(4);
    items[2] = tool("t0", ToolStatusView::InProgress);
    let rows = project_tail(&items, TailWindow::Last(1));
    assert_eq!(
        step_visibility(&rows),
        vec![true, false, false, true],
        "the live cycle keeps its header, the settled covered ones fold"
    );
    match tail_row(&rows).kind {
        RowKind::TailMore { hidden_steps, .. } => assert_eq!(hidden_steps, 3),
        _ => unreachable!(),
    }
}

/// Keep the reveal control when all covered steps are live.
#[test]
fn a_run_whose_every_covered_step_is_live_keeps_its_tail_row() {
    let mut items = turn_of_cycles(2);
    items[2] = tool("t0", ToolStatusView::InProgress);
    let live = LiveSubagentUnits::of(&items);

    let rows = project_tail(&items, TailWindow::Last(1));
    assert_eq!(
        step_visibility(&rows),
        vec![true, true],
        "the covered step is live, so its header stays surfaced"
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
    assert_eq!(hidden_steps, 1, "the window covers one step");
    assert!(collapsed, "and it has not been revealed yet");
    let covered_prose = |rows: &[RenderRow]| {
        rows.iter()
            .find(|r| matches!(r.kind, RowKind::AgentItem(1)))
            .expect("the covered step's assistant block keeps its row")
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
        TailWindow::Last(1),
        &DisplayFilter::default(),
    );
    assert!(
        !covered_prose(&revealed),
        "clicking the row unfolds the step it covers"
    );
}

#[test]
fn expand_all_leaves_the_tail_and_filter_chips_in_charge() {
    use crate::workspace::main_area::agent_chat_pane::agent_chat_helpers::collect_foldable_keys;
    let items = turn_of_cycles(6);
    let mut fold = FoldState::default();
    fold.set_all(collect_foldable_keys(&items), true);

    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::Last(2),
        &DisplayFilter::default(),
    );
    assert_eq!(
        step_visibility(&rows),
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
        TailWindow::All,
        &only_reads(),
    );
    assert!(filtered_count(&filtered) > 0, "the filter still takes rows");
    assert!(!filter_row(&filtered).hidden, "the row offers the reveal");
}

#[test]
fn changing_the_window_keeps_every_row_in_its_slot() {
    let items = turn_of_cycles(8);
    let all = project_tail(&items, TailWindow::All);
    for tail in [
        TailWindow::Last(1),
        TailWindow::Last(3),
        TailWindow::Last(10),
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
    assert!(!a.same_slot(&RenderRow::at(
        RowKind::StepHeader {
            first_ix: 1,
            tool_count: 2,
            collapsed: true,
        },
        false,
        1
    )));
}

// ── Display filter ─────────────────────────────────────────────────────────

fn project_filtered(items: &[ChatItem], filter: &DisplayFilter) -> Vec<RenderRow> {
    project(
        items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(items),
        TailWindow::All,
        filter,
    )
}

fn filter_row(rows: &[RenderRow]) -> &RenderRow {
    rows.iter()
        .find(|r| matches!(r.kind, RowKind::FilteredAway { .. }))
        .expect("every non-empty run gets a filter row")
}

fn filtered_count(rows: &[RenderRow]) -> usize {
    match filter_row(rows).kind {
        RowKind::FilteredAway { revealable, .. } => revealable,
        _ => unreachable!(),
    }
}

fn one_step_turn() -> Vec<ChatItem> {
    vec![
        ChatItem::UserText("q".into()),
        asst("looking"),
        tool("a", ToolStatusView::Completed),
        tool("b", ToolStatusView::Completed),
        asst("done"),
    ]
}

fn live_step_turn() -> Vec<ChatItem> {
    let mut items = one_step_turn();
    items[3] = tool("b", ToolStatusView::InProgress);
    items
}

fn only_reads() -> DisplayFilter {
    DisplayFilter::default().toggled(FilterFacet::ToolRead)
}

#[test]
fn a_nested_tool_filter_keeps_matching_children_and_their_ancestors() {
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

    let edits = DisplayFilter::default().toggled(FilterFacet::ToolEdit);
    let edits = FilterMatchIndex::of(&items, edits);
    assert!(edits.keeps_tool(parent), "the Edit parent matches directly");
    assert!(
        edits.keeps_tool(child),
        "a nested child owns no row, so it renders with whatever card survives"
    );
}

#[test]
fn an_empty_filter_hides_nothing_and_its_row_covers_nothing() {
    let items = live_step_turn();
    let rows = project_filtered(&items, &DisplayFilter::default());
    assert_eq!(filtered_count(&rows), 0);
    assert!(
        filter_row(&rows).hidden,
        "nothing to stand in for → the row stays zero-height"
    );
}

#[test]
fn a_filter_hides_the_rows_it_rejects_and_counts_them() {
    let items = live_step_turn();
    let only_tools = DisplayFilter::default().toggled(FilterFacet::Tools);
    let rows = project_filtered(&items, &only_tools);
    for row in &rows {
        match row.kind {
            RowKind::AgentItem(1) => assert!(row.hidden, "the step's prose is filtered"),
            RowKind::ConclusionItem(4) => {
                assert!(row.hidden, "the conclusion is prose too — the filter wins")
            }
            RowKind::AgentItem(2) | RowKind::AgentItem(3) => {
                assert!(!row.hidden, "the tools survive")
            }
            _ => {}
        }
    }
    assert!(!filter_row(&rows).hidden, "the row offers the reveal");
    assert_eq!(
        filtered_count(&rows),
        2,
        "the step's prose and the conclusion"
    );
}

#[test]
fn a_step_whose_every_row_is_filtered_goes_with_them() {
    let items = live_step_turn();
    let rows = project_filtered(&items, &only_reads());
    assert!(
        rows.iter()
            .filter(|r| matches!(r.kind, RowKind::StepHeader { .. }))
            .all(|r| r.hidden),
        "no step survives a read-only filter over Edit-kind tools"
    );
    let visible: Vec<&'static str> = rows
        .iter()
        .filter(|r| !r.hidden)
        .map(|r| match r.kind {
            RowKind::User(_) => "user",
            RowKind::ResponseHeader { .. } => "response",
            RowKind::FilteredAway { .. } => "filtered",
            _ => "other",
        })
        .collect();
    assert_eq!(visible, vec!["user", "response", "filtered"]);
}

#[test]
fn a_group_bar_summarizing_only_filtered_calls_goes_with_them() {
    let items = live_step_turn();
    let only_prose = DisplayFilter::default().toggled(FilterFacet::Prose);
    let rows = project_filtered(&items, &only_prose);
    assert!(
        rows.iter()
            .filter(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
            .all(|r| r.hidden)
    );
}

#[test]
fn revealing_the_filter_row_shows_what_it_covers() {
    let items = live_step_turn();
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Filtered(1), FoldContext::past(false));
    let only_tools = DisplayFilter::default().toggled(FilterFacet::Tools);
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &only_tools,
    );
    let conclusion = rows
        .iter()
        .find(|r| matches!(r.kind, RowKind::ConclusionItem(_)))
        .expect("the run's conclusion");
    assert!(!conclusion.hidden, "revealed in place");
    match filter_row(&rows).kind {
        RowKind::FilteredAway {
            revealable,
            collapsed,
            ..
        } => {
            assert_eq!(revealable, 2);
            assert!(!collapsed);
        }
        _ => unreachable!(),
    }
}

#[test]
fn a_prompt_and_a_pending_permission_survive_every_filter() {
    let items = [
        ChatItem::UserText("q".into()),
        asst("about to write"),
        perm(false),
        tool("a", ToolStatusView::Completed),
    ];
    let only_edits = DisplayFilter::default().toggled(FilterFacet::ToolEdit);
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
    let items = one_step_turn();
    let only_tools = DisplayFilter::default().toggled(FilterFacet::Tools);
    let mut collapsed = FoldState::default();
    collapsed.toggle(FoldKey::Response(0), FoldContext::past(true));
    let rows = project(
        &items,
        &collapsed,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
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
    assert!(
        filter_row(&rows).hidden,
        "the filter row folds away with its response"
    );
}

#[test]
fn the_placeholder_counts_only_what_the_filter_alone_took() {
    let items = one_step_turn();
    let only_tools = DisplayFilter::default().toggled(FilterFacet::Tools);
    let mut collapsed = FoldState::default();
    collapsed.toggle(FoldKey::Response(0), FoldContext::past(true));
    let rows = project(
        &items,
        &collapsed,
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &only_tools,
    );
    assert_eq!(filtered_count(&rows), 1);
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

#[test]
fn filter_rows_share_a_slot_by_their_run_start() {
    let a = RenderRow::at(
        RowKind::FilteredAway {
            run_start: 1,
            revealable: 12,
            excluded: 12,
            collapsed: true,
        },
        false,
        1,
    );
    let b = RenderRow::at(
        RowKind::FilteredAway {
            run_start: 1,
            revealable: 0,
            excluded: 0,
            collapsed: false,
        },
        true,
        1,
    );
    assert!(a.same_slot(&b), "same run_start → same slot");
    assert!(!a.same_slot(&RenderRow::at(
        RowKind::FilteredAway {
            run_start: 9,
            revealable: 12,
            excluded: 12,
            collapsed: true,
        },
        false,
        1
    )));
    assert!(!a.same_slot(&RenderRow::at(
        RowKind::TailMore {
            run_start: 1,
            hidden_steps: 3,
            kept_steps: 2,
            collapsed: true,
        },
        false,
        1
    )));
}

#[test]
fn an_unanswered_prompt_gets_no_filter_row() {
    let items = [ChatItem::UserText("q".into())];
    let rows = project_filtered(&items, &DisplayFilter::default());
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::FilteredAway { .. }))
    );
}

#[test]
fn each_run_gets_its_own_filter_row() {
    let items = [
        ChatItem::UserText("first".into()),
        asst("a1"),
        tool("t1", ToolStatusView::Completed),
        ChatItem::UserText("second".into()),
        asst("a2"),
        tool("t2", ToolStatusView::Completed),
    ];
    let starts: Vec<usize> = project_filtered(&items, &DisplayFilter::default())
        .iter()
        .filter_map(|r| match r.kind {
            RowKind::FilteredAway { run_start, .. } => Some(run_start),
            _ => None,
        })
        .collect();
    assert_eq!(starts, vec![1, 4], "one per run, keyed by the run's start");
}

fn kinded_tool(id: &str, kind: ToolKindView, status: ToolStatusView) -> ChatItem {
    ChatItem::ToolCall(ToolCallItem {
        id: id.to_owned(),
        title: format!("Tool {id}"),
        kind,
        tool_name: None,
        status,
        diffs: Vec::new(),
        output: Vec::new(),
        raw_input: None,
        parent_tool_id: None,
        exit: None,
    })
}

/// A step header is a disclosure, so its count must say what expanding it puts
/// on screen. The shipped bug: a step of one edit and two reads read `3 tools`
/// under an Edits filter that lets exactly one card through.
#[test]
fn a_step_header_counts_only_the_tools_the_filter_keeps() {
    use ToolKindView::{Edit, Read};
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        asst("looking at the failure"),
        kinded_tool("a", Edit, Completed),
        kinded_tool("b", Read, Completed),
        kinded_tool("c", Read, Completed),
        asst("done"),
    ];
    let step_count = |fold: &FoldState, filter: &DisplayFilter| {
        project(
            &items,
            fold,
            false,
            &LiveSubagentUnits::of(&items),
            TailWindow::All,
            filter,
        )
        .iter()
        .find_map(|r| match r.kind {
            RowKind::StepHeader { tool_count, .. } => Some(tool_count),
            _ => None,
        })
        .expect("step header")
    };
    assert_eq!(
        step_count(&FoldState::default(), &DisplayFilter::default()),
        3,
        "an unfiltered pane still counts the whole step"
    );
    assert_eq!(
        step_count(
            &FoldState::default(),
            &DisplayFilter::default().toggled(FilterFacet::ToolEdit),
        ),
        1,
        "only the edit survives the filter, so only the edit is offered"
    );
    assert_eq!(
        step_count(
            &FoldState::default(),
            &DisplayFilter::default().toggled(FilterFacet::Thinking),
        ),
        0,
        "a step kept for its prose alone offers no tools at all"
    );

    let mut revealed = FoldState::default();
    revealed.toggle(FoldKey::Filtered(1), FoldContext::past(false));
    assert_eq!(
        step_count(
            &revealed,
            &DisplayFilter::default().toggled(FilterFacet::ToolEdit),
        ),
        3,
        "revealing filtered rows restores the count of the rows now on screen"
    );
}

/// `text_of` collapses a content block it cannot render to an empty string, so
/// an assistant message can arrive with no text at all. It has nothing to show.
#[test]
fn an_empty_assistant_reply_projects_no_row() {
    let items = [ChatItem::UserText("hi".into()), asst("")];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert_eq!(kinds(&rows), vec![("user", false), ("filtered", true)]);
}

/// A bodyless message must not occupy the row slot that tips a step into
/// earning a header, or an invisible item grows visible chrome.
#[test]
fn an_empty_message_earns_no_step_header() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        asst(""),
        tool("a", Completed),
    ];
    let bare = [ChatItem::UserText("q".into()), tool("a", Completed)];
    let project_all = |items: &[ChatItem]| {
        project(
            items,
            &FoldState::default(),
            false,
            &LiveSubagentUnits::of(items),
            TailWindow::All,
            &DisplayFilter::default(),
        )
    };
    assert_eq!(kinds(&project_all(&items)), kinds(&project_all(&bare)));
}

/// The conclusion escapes its enclosing fold, so an empty message taking that
/// slot leaves a blank row pinned over a collapsed step — and buries the real
/// reply that should have held it.
#[test]
fn an_empty_message_is_never_the_conclusion() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        asst("real"),
        tool("a", Completed),
        asst(""),
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    let visible_items: Vec<usize> = rows
        .iter()
        .filter(|r| !r.hidden)
        .filter_map(|r| match r.kind {
            RowKind::AgentItem(i) | RowKind::ConclusionItem(i) => Some(i),
            _ => None,
        })
        .collect();
    assert_eq!(
        visible_items,
        vec![1],
        "the real reply holds the conclusion"
    );
}

#[test]
fn an_empty_thinking_block_projects_no_row() {
    let items = [ChatItem::UserText("hi".into()), think(""), asst("done")];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert_eq!(
        kinds(&rows),
        vec![("user", false), ("filtered", true), ("solo", false)]
    );
}

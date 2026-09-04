use super::*;
use crate::transcript::display_filter::{DisplayFilter, FilterFacet};
use crate::transcript::fold_mode::FoldPreset;
use crate::workspace::main_area::agent_chat_pane::fold::FoldContext;
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
                RowKind::Interrupted(_) => "interrupted",
                RowKind::ResponseHeader { .. } => "response",
                RowKind::AgentItem(_) | RowKind::ConclusionItem(_) => "item",
                RowKind::TailMore { .. } => "tail",
                RowKind::ToolGroupTailMore { .. } => "grouptail",
                RowKind::ToolGroupHeader { .. } => "group",
                RowKind::ThinkingGroupHeader { .. } => "thinkgroup",
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
            ("tail", true),
            ("item", false),  // the prose is a plain paragraph
            ("group", false), // the calls behind it are one group
            ("grouptail", true),
            ("item", true),
            ("item", true),
            ("item", true),
            ("item", false), // the conclusion
        ]
    );
    let indents: Vec<u8> = rows.iter().map(|r| r.indent).collect();
    // The group's own tail row sits with the calls it holds back, not with
    // the header above them.
    assert_eq!(indents, vec![0, 0, 1, 1, 1, 2, 2, 2, 2, 1]);
}

/// A one-block reply with no tools still represents a whole response, so it
/// gets the same bar every other turn has. That bar is where the filter's
/// reveal control lives, and a turn without one has nowhere to put it.
#[test]
fn every_anchored_response_gets_a_bar() {
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
        vec![("user", false), ("response", false), ("item", false)]
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
            ("tail", true),
            ("item", false), // a1 = conclusion, stays visible
            ("group", true),
            ("grouptail", true),
            ("item", true),
            ("item", true),
            ("user", false), // second
            ("response", false),
            ("tail", true),
            ("item", false),  // a2 = conclusion, never folded away
            ("group", false), // the open response shows its group bar
            ("grouptail", true),
            ("item", true), // whose settled members stay folded
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
            ("tail", true),
            ("item", false), // a1 = conclusion
            ("group", true),
            ("grouptail", true),
            ("item", true),
            ("item", true),
            ("user", false),
            ("response", false),
            ("tail", true),
            ("item", false), // a2 = conclusion
            ("group", true),
            ("grouptail", true),
            ("item", true),
            ("item", true),
        ]
    );
}

#[test]
fn expanded_mode_opens_past_responses_and_the_newest_settled_groups() {
    let items = two_settled_turns();
    let rows = project_under(&items, &FoldState::with_mode(FoldPreset::Expanded.mode()));
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("tail", true),
            ("item", false),  // a1
            ("group", false), // the past response is open, its group is not
            ("grouptail", true),
            ("item", true),
            ("item", true),
            ("user", false),
            ("response", false),
            ("tail", true),
            ("item", false),  // a2
            ("group", false), // the newest turn goes one level deeper
            ("grouptail", true),
            ("item", false),
            ("item", false),
        ]
    );
}

#[test]
fn a_user_fold_survives_a_mode_switch() {
    let items = two_settled_turns();
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(5), FoldContext::last(false)); // newest turn → collapsed
    for preset in FoldPreset::ALL {
        fold.set_mode(preset.mode());
        let rows = project_under(&items, &fold);
        let newest_group = *kinds(&rows)
            .iter()
            .rfind(|(kind, _)| *kind == "group")
            .expect("the newest turn has a tool group");
        assert_eq!(newest_group, ("group", true), "{preset:?}");
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
            ("tail", true),
            ("item", false),  // assistant prose = conclusion, never folded away
            ("group", false), // the tool group's bar
            ("grouptail", true),
            ("item", true), // settled members collapsed
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
    fold.toggle(FoldKey::Response(1), FoldContext::past(true));
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
    fold.toggle(FoldKey::Response(1), FoldContext::past(true)); // collapse the response
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
            ("tail", true),
            ("item", true), // "let me look" process → hidden
            ("group", true),
            ("grouptail", true),
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
    // A lone reply is the whole response: folding it would leave the turn
    // showing nothing, so it renders as plain prose under the bar rather than
    // earning the conclusion's bare-chevron fold.
    let items = [ChatItem::UserText("hi".into()), asst("hello")];
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
    fold.toggle(FoldKey::Response(1), FoldContext::past(true));
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
            ("item", true),  // first message → process
            ("item", false), // last message → conclusion
        ]
    );
}

/// The run ends with tools and no final text, so its last prose is a preamble.
/// It stays visible through the collapsed response — a fold that hid it would
/// leave nothing of what the agent said — but it is not the conclusion.
#[test]
fn prose_before_a_trailing_tool_run_stays_visible_without_being_a_conclusion() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        asst("answer"),
        tool("a", Completed),
        tool("b", Completed),
    ];
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(1), FoldContext::past(true));
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
            ("tail", true),
            ("item", false), // "answer" is forced visible through the fold
            ("group", true),
            ("grouptail", true),
            ("item", true),
            ("item", true),
        ]
    );
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::ConclusionItem(_))),
        "no trailing prose in the run, so the response has no conclusion"
    );
}

/// The same run once the agent finishes: the answer it lands on is outside
/// every step, so it takes the conclusion role and its chrome — and the
/// preamble that held the visibility slot goes back to being ordinary prose.
#[test]
fn a_trailing_answer_takes_the_conclusion_role_from_the_preamble() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        asst("looking"),
        tool("a", Completed),
        tool("b", Completed),
        asst("answer"),
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
        rows.iter()
            .any(|r| matches!(r.kind, RowKind::ConclusionItem(4))),
        "the trailing answer is the conclusion"
    );
    assert!(
        rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(1))),
        "and the preamble is plain prose again"
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
    fold.toggle(FoldKey::Response(1), FoldContext::past(true));
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
            ("tail", true),
            ("group", true), // no assistant text → nothing stays visible
            ("grouptail", true),
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
    fold.toggle(FoldKey::Response(1), FoldContext::past(true)); // collapse the response
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
    fold.toggle(FoldKey::Response(1), FoldContext::past(true));
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

/// `force_visible` is `is_last_prose || pending_permission`, and neither term
/// consults any enclosing fold — an actionable prompt cannot be shut away by
/// one. A permission never becomes a group child, so the folds that can enclose
/// it are the response's and the tail window's boundary; both feed the one
/// `folded` term this escapes.
#[test]
fn a_pending_permission_outlives_every_fold_that_encloses_it() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        asst("working"),
        perm(false),
        tool("a", Completed),
        asst("mid"),
        tool("b", Completed),
        asst("done"),
    ];
    let visible = |rows: &[RenderRow], ix: usize| {
        !rows
            .iter()
            .find(
                |r| matches!(r.kind, RowKind::AgentItem(j) | RowKind::ConclusionItem(j) if j == ix),
            )
            .expect("row present")
            .hidden
    };

    // The response's own fold, shut over the whole run.
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(1), FoldContext::past(true));
    let rows = project_under(&items, &fold);
    assert!(
        visible(&rows, 2),
        "the prompt survives a collapsed response"
    );
    assert!(
        !visible(&rows, 1),
        "the prose beside it does not, so the response really is shut"
    );

    // The tail window's boundary, shut over the run the prompt sits in.
    let rows = project_tail(&items, TailWindow::Last(1));
    assert!(visible(&rows, 2), "the prompt survives a covered run");
    assert!(!visible(&rows, 1), "its neighbouring prose is covered");
    assert!(!visible(&rows, 3), "so is the call the window left out");
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
        items.len() + 4,
        "the run's bar + tail row + one group header + its own tail row + every member"
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
fn a_long_single_turn_of_cycles_stays_linear() {
    const N: usize = 250;
    let small = one_turn_of_cycles(N);
    let large = one_turn_of_cycles(N * 2);

    let groups = |rows: &[RenderRow]| {
        rows.iter()
            .filter(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
            .count()
    };
    assert!(
        groups(&project_all(&small)) >= N,
        "the fixture must actually build tool groups"
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
    fold.set_all([FoldKey::Response(1)], false); // force the response collapsed
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
    fold.set_all([FoldKey::Response(1)], false);
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
            ("response", false),
            ("tail", true),
            ("item", false), // "x"
            ("item", false), // the tool call — a plain item, not a group
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
            ("response", false),
            ("tail", true),
            ("group", false),
            ("grouptail", true),
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
            ("response", false),
            ("tail", true),
            ("group", false),
            ("grouptail", true),
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
            run_start: 1,
            collapsed: false,
            filtered: FilteredAway::default(),
        },
        RowKind::AgentItem(0),
        RowKind::TailMore {
            run_start: 0,
            hidden_steps: 0,
            kept_steps: 0,
            collapsed: false,
        },
        RowKind::ToolGroupHeader {
            gid: "g".into(),
            first_ix: 0,
            count: 0,
            collapsed: false,
        },
        RowKind::ThinkingGroupHeader {
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
    fold.toggle(FoldKey::Response(1), FoldContext::past(true));
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
        vec!["user", "response", "group", "item", "working"]
    );
}

#[test]
fn leading_run_without_a_user_anchor_still_gets_a_bar() {
    use ToolStatusView::Failed;
    // Reachable on session restore: `append_user_chunk` drops a replayed
    // `<task-notification>` user turn (see daruda_acp::mapping), so a restored
    // pane can open with agent items and no `UserText` anchor. It is still a
    // response, and the filter's reveal rides the bar — a run without one has
    // nowhere to put it, so the bar keys off the run rather than the user turn.
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
        rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(0))),
        "its prose stays a plain block"
    );
    assert_eq!(
        kinds(&rows),
        vec![
            ("response", false),
            ("tail", true),
            ("item", false),
            ("item", false)
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
        rows.iter()
            .any(|r| matches!(r.kind, RowKind::ResponseHeader { .. }))
    );
}

// ── Tool runs ──────────────────────────────────────────────────────────────
//
fn think(s: &str) -> ChatItem {
    ChatItem::Thinking {
        text: s.to_owned(),
        streaming: false,
        message_id: None,
    }
}

/// The approved grammar: prose is a plain paragraph and the calls behind it are
/// one group — no titled wrapper bundling the two into a single unit.
#[test]
fn prose_in_front_of_a_run_stays_a_row_of_its_own() {
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
    assert!(
        rows.iter()
            .filter(|r| matches!(r.kind, RowKind::AgentItem(1) | RowKind::AgentItem(2)))
            .all(|r| !r.hidden),
        "the thought and the prose are plain rows the response's own fold governs"
    );
    let group = rows
        .iter()
        .find_map(|r| match &r.kind {
            RowKind::ToolGroupHeader {
                first_ix, count, ..
            } => Some((*first_ix, *count)),
            _ => None,
        })
        .expect("two consecutive calls earn a group");
    assert_eq!(group, (3, 2), "the group starts at the first call");
    let indents: Vec<u8> = rows
        .iter()
        .filter(|r| matches!(r.kind, RowKind::AgentItem(1) | RowKind::AgentItem(2)))
        .map(|r| r.indent)
        .collect();
    assert_eq!(indents, vec![1, 1], "no wrapper indents the prose");
    assert!(
        rows.iter()
            .any(|r| matches!(r.kind, RowKind::ConclusionItem(5)) && !r.hidden)
    );
}

#[test]
fn consecutive_runs_each_get_their_own_group() {
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
        .filter_map(|r| match &r.kind {
            RowKind::ToolGroupHeader { first_ix, .. } => Some(*first_ix),
            _ => None,
        })
        .collect();
    assert_eq!(
        starts,
        vec![2, 5],
        "one group per run, split by the prose between them"
    );
}

#[test]
fn a_response_without_tools_gets_no_group() {
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
            ("item", false),
            ("item", false),
        ]
    );
}

/// `RUN_GROUP_MIN` is the whole rule: one call is already one row, two earn a
/// bar that says how many are behind it.
#[test]
fn a_run_earns_a_group_at_two_calls_not_one() {
    use ToolStatusView::Completed;
    let groups = |items: &[ChatItem]| {
        project(
            items,
            &FoldState::default(),
            false,
            &LiveSubagentUnits::of(items),
            TailWindow::All,
            &DisplayFilter::default(),
        )
        .iter()
        .filter(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
        .count()
    };
    assert_eq!(
        groups(&[
            ChatItem::UserText("q".into()),
            tool("a", Completed),
            asst("done"),
        ]),
        0
    );
    assert_eq!(
        groups(&[
            ChatItem::UserText("q".into()),
            tool("a", Completed),
            tool("b", Completed),
            asst("done"),
        ]),
        1
    );
}

/// `tool_run_end` stops at a nested child, so two top-level calls with one
/// between them are two 1-length runs and neither earns a group bar — even
/// though the child renders inside the first call's card, which leaves the two
/// looking adjacent on screen. The tail window's run tally advances by the same
/// `tool_run_end`, so the group walk and the tally cannot drift. Both real
/// captures (`acp-wire-codex-acp.log`, `acp-wire-claude.log`) hold zero nested
/// tool calls, so this split is unexercised in practice.
#[test]
fn a_nested_child_between_two_calls_leaves_them_two_ungrouped_runs() {
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
    assert_eq!(
        kinds(&project_all(&items)),
        vec![
            ("user", false),
            ("response", false),
            ("tail", true),
            ("item", false), // a
            ("item", false), // b, with no group bar over either
        ]
    );

    // The same two calls with nothing between them are one run of two.
    let adjacent = [
        ChatItem::UserText("q".into()),
        tool("a", Completed),
        tool("b", Completed),
    ];
    assert_eq!(
        kinds(&project_all(&adjacent)),
        vec![
            ("user", false),
            ("response", false),
            ("tail", true),
            ("group", false),
            ("grouptail", true),
            ("item", true),
            ("item", true),
        ]
    );

    // The window's tally splits the same way: with room for one run it covers
    // the first call and keeps the second.
    let rows = project_tail(&items, TailWindow::Last(1));
    match tail_row(&rows).kind {
        RowKind::TailMore {
            hidden_steps,
            kept_steps,
            ..
        } => assert_eq!((hidden_steps, kept_steps), (1, 1), "two runs, one covered"),
        _ => unreachable!(),
    }
}

#[test]
fn the_running_group_expands_while_its_settled_sibling_folds() {
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
            .find_map(|r| match &r.kind {
                RowKind::ToolGroupHeader {
                    first_ix,
                    collapsed,
                    ..
                } if *first_ix == first => Some(*collapsed),
                _ => None,
            })
            .expect("group header present")
    };
    assert!(collapsed(2), "the settled run folds to its bar");
    assert!(!collapsed(5), "the running run stays open");
    assert!(
        rows.iter()
            .any(|r| matches!(r.kind, RowKind::AgentItem(5)) && !r.hidden)
    );
}

// ── Tail window ────────────────────────────────────────────────────────────

/// One prose row plus a two-call run per cycle, so every cycle contributes one
/// tool run — the population the tail window counts.
fn turn_of_cycles(cycles: usize) -> Vec<ChatItem> {
    let mut items = vec![ChatItem::UserText("q".into())];
    for i in 0..cycles {
        items.push(asst(&format!("cycle {i}")));
        items.push(tool(&format!("t{i}"), ToolStatusView::Completed));
        items.push(tool(&format!("t{i}b"), ToolStatusView::Completed));
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

fn group_visibility(rows: &[RenderRow]) -> Vec<bool> {
    rows.iter()
        .filter(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
        .map(|r| !r.hidden)
        .collect()
}

fn tail_row(rows: &[RenderRow]) -> &RenderRow {
    rows.iter()
        .find(|r| matches!(r.kind, RowKind::TailMore { .. }))
        .expect("a run with tool calls gets a tail row")
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
        TailWindow::Last(2),
        &only_reads(),
    );

    let shown: Vec<usize> = rows
        .iter()
        .filter(|r| !r.hidden)
        .filter_map(|r| match &r.kind {
            RowKind::ToolGroupHeader { first_ix, .. } => Some(*first_ix),
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

#[test]
fn a_window_keeps_only_the_last_runs_visible() {
    let items = turn_of_cycles(8);
    for (n, kept) in [(1usize, 1usize), (3, 3), (5, 5)] {
        let rows = project_tail(&items, TailWindow::Last(n));
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
        let rows = project_tail(&items, tail);
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

/// The rail marks exactly the runs the window covers, and only those — a row
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
            .filter(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
            .all(|r| r.outside_window == r.hidden),
        "with the boundary shut, a covered run is marked and hidden while a \
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
        .filter(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
        .map(|r| r.outside_window)
        .collect();
    assert_eq!(
        marked,
        vec![true, true, true, true, false, false],
        "the four runs the window covers are marked; the two it keeps are not"
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

/// A live run the window covers stays surfaced whether or not the boundary is
/// open — so the rail has to mark it in *both* states. Keying the mark on the
/// boundary being open instead made the same row gain and lose its rail as the
/// boundary flipped, leaving a visible row from outside the range unexplained in
/// exactly the state where nothing else accounts for it.
#[test]
fn a_live_covered_run_carries_the_rail_with_the_boundary_shut() {
    let mut items = turn_of_cycles(4);
    items[2] = tool("t0", ToolStatusView::InProgress);
    let tail = TailWindow::Last(1);

    let shut = project_tail(&items, tail);
    let surfaced: Vec<(bool, bool)> = shut
        .iter()
        .filter(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
        .map(|r| (!r.hidden, r.outside_window))
        .collect();
    assert_eq!(
        surfaced,
        vec![(true, true), (false, true), (false, true), (true, false)],
        "the live covered run is on screen and marked; the kept run is neither"
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
        .filter(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
        .map(|r| r.outside_window)
        .collect();
    assert_eq!(
        marked,
        vec![true, true, true, false],
        "opening the boundary changes which covered rows are visible, not which are outside"
    );
}

/// The group's calls only earn rows once the group is open, which is what makes
/// the in-group window observable at all.
fn project_open_group(items: &[ChatItem], tail: TailWindow) -> Vec<RenderRow> {
    project_open_group_under(
        items,
        &FoldState::with_mode(FoldPreset::Expanded.mode()),
        tail,
    )
}

fn project_open_group_under(
    items: &[ChatItem],
    fold: &FoldState,
    tail: TailWindow,
) -> Vec<RenderRow> {
    project(
        items,
        fold,
        false,
        &LiveSubagentUnits::of(items),
        tail,
        &DisplayFilter::default(),
    )
}

fn group_tail_row(rows: &[RenderRow]) -> &RenderRow {
    rows.iter()
        .find(|r| matches!(r.kind, RowKind::ToolGroupTailMore { .. }))
        .expect("a tool group gets a boundary of its own")
}

fn group_tail_counts(rows: &[RenderRow]) -> (usize, usize) {
    match &group_tail_row(rows).kind {
        RowKind::ToolGroupTailMore {
            hidden_calls,
            kept_calls,
            ..
        } => (*hidden_calls, *kept_calls),
        _ => unreachable!(),
    }
}

/// Visibility of each of the group's calls, in transcript order.
fn call_visibility(items: &[ChatItem], rows: &[RenderRow]) -> Vec<bool> {
    rows.iter()
        .filter(
            |r| matches!(r.kind, RowKind::AgentItem(ix) if matches!(items[ix], ChatItem::ToolCall(_))),
        )
        .map(|r| !r.hidden)
        .collect()
}

/// Row identity, visibility and coverage together — the three answers the two
/// windows decide between them.
fn marks(rows: &[RenderRow]) -> Vec<(&'static str, bool, bool)> {
    rows.iter()
        .map(|r| {
            let kind = match r.kind {
                RowKind::User(_) => "user",
                RowKind::Interrupted(_) => "interrupted",
                RowKind::ResponseHeader { .. } => "response",
                RowKind::AgentItem(_) | RowKind::ConclusionItem(_) => "item",
                RowKind::TailMore { .. } => "tail",
                RowKind::ToolGroupTailMore { .. } => "grouptail",
                RowKind::ToolGroupHeader { .. } => "group",
                RowKind::ThinkingGroupHeader { .. } => "thinkgroup",
                RowKind::WorkingIndicator => "working",
            };
            (kind, r.hidden, r.outside_window)
        })
        .collect()
}

/// The two levels compose: the response's boundary covers a whole run, and that
/// run's group still trims its own calls underneath it. Every other in-group
/// test uses a single-run turn, where the response window covers nothing — so
/// this is the one that exercises `push_group_children` layering its window
/// under an already-folded group.
#[test]
fn the_response_window_and_a_group_window_compose() {
    let items = turn_of_cycles(2);
    let tail = TailWindow::Last(1);

    let shut = project_open_group(&items, tail);
    assert_eq!(
        marks(&shut),
        vec![
            ("user", false, false),
            ("response", false, false),
            ("tail", false, false),
            ("item", true, true),      // the covered run's prose
            ("group", true, true),     // and its group, behind the response's boundary
            ("grouptail", true, true), // whose own boundary is folded with it
            ("item", true, true),
            ("item", true, true),
            ("item", false, false), // the kept run's prose
            ("group", false, false),
            ("grouptail", false, false), // trimming the kept run to its last call
            ("item", true, true),
            ("item", false, false),
            ("item", false, false), // the conclusion
        ],
        "a covered run is hidden whole; the kept run is trimmed from inside"
    );

    let mut fold = FoldState::with_mode(FoldPreset::Expanded.mode());
    fold.toggle(FoldKey::Tail(1), FoldContext::last(false));
    let open = project_open_group_under(&items, &fold, tail);
    assert_eq!(
        marks(&open),
        vec![
            ("user", false, false),
            ("response", false, false),
            ("tail", false, false),
            ("item", false, true),  // the response's reveal surfaces the run
            ("group", false, true), // its group
            ("grouptail", false, true), // and the group's own boundary with it
            ("item", true, true),   // which still holds this call back
            ("item", false, true),
            ("item", false, false),
            ("group", false, false),
            ("grouptail", false, false),
            ("item", true, true),
            ("item", false, false),
            ("item", false, false),
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
        let rows = project_open_group(&items, TailWindow::Last(kept));
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
        let rows = project_open_group(&items, tail);
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
        project_open_group(&items, tail)
            .iter()
            .position(|r| matches!(r.kind, RowKind::ToolGroupTailMore { .. }))
    };
    assert_eq!(slots(TailWindow::All), slots(TailWindow::Last(2)));
    let a = project_open_group(&items, TailWindow::All);
    let b = project_open_group(&items, TailWindow::Last(2));
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
    let rows = project_open_group_under(&items, &FoldState::default(), TailWindow::Last(2));
    assert!(
        group_tail_row(&rows).hidden,
        "the settled group is folded, so its boundary is too"
    );
}

/// A single call is not a group, so it has no boundary of its own — the
/// response's window is the only one that can cover it.
#[test]
fn an_ungrouped_call_gets_no_group_boundary() {
    let items = [
        ChatItem::UserText("q".into()),
        asst("working"),
        tool("t0", ToolStatusView::Completed),
        asst("done"),
    ];
    let rows = project_open_group(&items, TailWindow::Last(1));
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::ToolGroupTailMore { .. }))
    );
}

/// The rail marks exactly the calls the group's window covers, and the reveal
/// is what puts them on screen — the same two-part answer the response's
/// boundary gives one level up.
#[test]
fn opening_a_group_boundary_reveals_its_covered_calls_railed() {
    let items = turn_of_one_group(5, false);
    let tail = TailWindow::Last(2);

    let shut = project_open_group(&items, tail);
    let covered: Vec<(bool, bool)> = shut
        .iter()
        .filter(
            |r| matches!(r.kind, RowKind::AgentItem(ix) if matches!(items[ix], ChatItem::ToolCall(_))),
        )
        .map(|r| (!r.hidden, r.outside_window))
        .collect();
    assert_eq!(
        covered,
        vec![
            (false, true),
            (false, true),
            (false, true),
            (true, false),
            (true, false)
        ],
        "with the boundary shut, a covered call is marked and hidden"
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
    assert!(
        !group_tail_row(&open).outside_window,
        "the boundary itself is never one of the rows it brackets"
    );
    assert!(
        open.iter().filter(|r| r.outside_window).count() == 3,
        "opening the boundary changes which covered calls are visible, not which are outside"
    );
}

/// A running call the group's window covers stays surfaced through a shut
/// boundary, exactly as a live run does under the response's — otherwise the
/// axis would hide the one call the reader is waiting on.
#[test]
fn a_live_covered_call_stays_surfaced_through_a_shut_group_boundary() {
    let mut items = turn_of_one_group(4, false);
    items[2] = tool("g0", ToolStatusView::InProgress);
    let rows = project_open_group(&items, TailWindow::Last(1));
    let surfaced: Vec<(bool, bool)> = rows
        .iter()
        .filter(
            |r| matches!(r.kind, RowKind::AgentItem(ix) if matches!(items[ix], ChatItem::ToolCall(_))),
        )
        .map(|r| (!r.hidden, r.outside_window))
        .collect();
    assert_eq!(
        surfaced,
        vec![(true, true), (false, true), (false, true), (true, false)],
        "the live covered call is on screen and marked; the kept call is neither"
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
        TailWindow::Last(2),
        &only_reads(),
    );
    assert_eq!(
        group_tail_counts(&rows),
        (1, 2),
        "three calls survive the filter and two are kept, so one is behind the boundary"
    );
}

/// Deliberate scope: the axis counts work steps, and a stretch of thoughts is
/// one step's reasoning rather than a run of them. A reasoning group is folded
/// by its own rule, never trimmed by this one.
#[test]
fn a_reasoning_group_is_not_divided_by_the_step_axis() {
    let mut items = vec![ChatItem::UserText("q".into())];
    for i in 0..4 {
        items.push(ChatItem::Thinking {
            text: format!("thought {i}"),
            streaming: false,
            message_id: None,
        });
    }
    items.push(asst("done"));
    let rows = project_open_group(&items, TailWindow::Last(1));
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::ToolGroupTailMore { .. })),
        "a reasoning group has no step boundary of its own"
    );
    let thoughts: Vec<bool> = rows
        .iter()
        .filter(
            |r| matches!(r.kind, RowKind::AgentItem(ix) if matches!(items[ix], ChatItem::Thinking { .. })),
        )
        .map(|r| !r.hidden)
        .collect();
    assert_eq!(thoughts, vec![true; 4], "every thought stays on screen");
}

#[test]
fn a_response_without_tool_calls_gets_no_tail_row() {
    let items = [ChatItem::UserText("hi".into()), asst("hello")];
    let rows = project_tail(&items, TailWindow::Last(1));
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
    let rows = project_tail(&items, TailWindow::Last(1));
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
fn revealing_the_tail_shows_every_run_again() {
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
fn a_collapsed_response_hides_its_tail_row_too() {
    let items = turn_of_cycles(6);
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(1), FoldContext::past(true));
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
fn a_covered_run_with_a_running_tool_stays_surfaced() {
    let mut items = turn_of_cycles(4);
    items[2] = tool("t0", ToolStatusView::InProgress);
    let rows = project_tail(&items, TailWindow::Last(1));
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

    let rows = project_tail(&items, TailWindow::Last(1));
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
        TailWindow::Last(1),
        &DisplayFilter::default(),
    );
    assert!(
        !covered_prose(&revealed),
        "clicking the row unfolds the range it covers"
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
        TailWindow::All,
        &only_reads(),
    );
    assert!(filtered_count(&filtered) > 0, "the filter still takes rows");
    assert!(
        filtered_away(&filtered).revealable > 0,
        "the chip offers the reveal"
    );
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
    assert!(!a.same_slot(&RenderRow::at(RowKind::AgentItem(1), false, 1)));
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

/// The run's tally, read off the bar that carries it.
fn filtered_away(rows: &[RenderRow]) -> FilteredAway {
    rows.iter()
        .find_map(|r| match r.kind {
            RowKind::ResponseHeader { filtered, .. } => Some(filtered),
            _ => None,
        })
        .expect("an answered run has a bar")
}

fn filtered_count(rows: &[RenderRow]) -> usize {
    filtered_away(rows).revealable
}

fn one_run_turn() -> Vec<ChatItem> {
    vec![
        ChatItem::UserText("q".into()),
        asst("looking"),
        tool("a", ToolStatusView::Completed),
        tool("b", ToolStatusView::Completed),
        asst("done"),
    ]
}

fn live_run_turn() -> Vec<ChatItem> {
    let mut items = one_run_turn();
    items[3] = tool("b", ToolStatusView::InProgress);
    items
}

fn only_reads() -> DisplayFilter {
    DisplayFilter::from_tokens(["tool_read"])
}

/// One turn that delegates: a subagent launch followed by the children the
/// adapter flattened under it, none of which earns a row.
fn turn_with_subagent(children: usize, running: bool) -> Vec<ChatItem> {
    let mut items = vec![ChatItem::UserText("q".into()), asst("delegating")];
    items.push(tool("task", ToolStatusView::Completed));
    for i in 0..children {
        let last = i + 1 == children;
        let status = if last && running {
            ToolStatusView::InProgress
        } else {
            ToolStatusView::Completed
        };
        items.push(child_of(&format!("c{i}"), "task", status));
    }
    items.push(asst("done"));
    items
}

/// The row boundary both narrowing axes observe — see [`top_level_tool`]. A
/// nested child owns no row, so no combination of the step window, the display
/// filter and the fold mode can give it one, and a card's children cannot move
/// the row layer at all.
///
/// Stated as a test because the rule lives in two places (`top_level_tool`'s
/// exclusion and [`FilterMatchIndex::build`]'s descendant keeping) and a change
/// that narrows one axis into a card while the other stays out is silent.
#[test]
fn no_axis_narrows_inside_a_tool_card() {
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
        for tail in [TailWindow::All, TailWindow::Last(1)] {
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
            kinds(&project_tail(&one, tail)),
            kinds(&project_tail(&many, tail)),
            "a card's children never move the row layer: {tail:?}"
        );
    }
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

    let edits = DisplayFilter::from_tokens(["tool_edit"]);
    let edits = FilterMatchIndex::of(&items, edits);
    assert!(edits.keeps_tool(parent), "the Edit parent matches directly");
    assert!(
        edits.keeps_tool(child),
        "a nested child owns no row, so it renders with whatever card survives"
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
        TailWindow::All,
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
        TailWindow::All,
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

/// Collapsing the response is the reader's own gesture and says nothing about
/// what the filter took, so the tally must read the same on both sides of it.
#[test]
fn collapsing_the_response_does_not_move_the_tally() {
    let items = one_run_turn();
    let only_tools = DisplayFilter::from_tokens(["tools"]);
    let mut collapsed = FoldState::default();
    collapsed.toggle(FoldKey::Response(1), FoldContext::past(true));
    let tally = |fold: &FoldState| {
        filtered_count(&project(
            &items,
            fold,
            false,
            &LiveSubagentUnits::of(&items),
            TailWindow::All,
            &only_tools,
        ))
    };
    assert_eq!(tally(&FoldState::default()), tally(&collapsed));
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

/// A read-kind call, so a filter aimed at edits leaves it on screen.
fn read_tool(id: &str, status: ToolStatusView) -> ChatItem {
    let mut item = tool(id, status);
    if let ChatItem::ToolCall(tc) = &mut item {
        tc.kind = ToolKindView::Read;
    }
    item
}

/// One run: prose, then a single group of `calls` edit calls, the last of which
/// is still running when `running`.
fn turn_of_one_group(calls: usize, running: bool) -> Vec<ChatItem> {
    let mut items = vec![ChatItem::UserText("q".into()), asst("looking")];
    for i in 0..calls {
        let last = i + 1 == calls;
        let status = if last && running {
            ToolStatusView::InProgress
        } else {
            ToolStatusView::Completed
        };
        items.push(tool(&format!("g{i}"), status));
    }
    items
}

fn hides_edits() -> DisplayFilter {
    DisplayFilter::default().toggled(FilterFacet::ToolEdit)
}

/// The tally answers "what did the filter take", which is a question about the
/// filter alone. A group's fold flips on its own as the last call settles, and
/// letting that move the number made it climb to the group's size and then drop
/// back mid-turn — the same cut reported two different ways seconds apart.
#[test]
fn a_group_settling_does_not_move_the_tally() {
    let running = project_filtered(&turn_of_one_group(5, true), &hides_edits());
    let settled = project_filtered(&turn_of_one_group(5, false), &hides_edits());
    assert_eq!(
        filtered_count(&running),
        filtered_count(&settled),
        "the fold moved but the filter's cut did not"
    );
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

/// The bar keeps its slot as its tally changes, so toggling a filter facet
/// re-labels the chip in place instead of splicing the list and drifting the
/// scroll.
#[test]
fn the_bar_keeps_its_slot_as_its_tally_changes() {
    let bar = |run_start: usize, filtered: FilteredAway, collapsed: bool| {
        RenderRow::at(
            RowKind::ResponseHeader {
                run_start,
                collapsed,
                filtered,
            },
            false,
            0,
        )
    };
    let full = FilteredAway { revealable: 12 };
    let a = bar(1, full, true);
    assert!(
        a.same_slot(&bar(1, FilteredAway::default(), false)),
        "same run start → same slot"
    );
    assert!(!a.same_slot(&bar(9, full, true)));
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

/// The tally rides on the response bar, so a prompt with no answer yet has
/// nothing to carry one — and no bar either.
#[test]
fn an_unanswered_prompt_gets_no_bar() {
    let items = [ChatItem::UserText("q".into())];
    let rows = project_filtered(&items, &DisplayFilter::default());
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::ResponseHeader { .. }))
    );
}

/// One tally per run, and it lives on that run's own bar.
#[test]
fn each_run_carries_its_own_tally() {
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
            RowKind::ResponseHeader { run_start, .. } => Some(run_start),
            _ => None,
        })
        .collect();
    assert_eq!(
        starts,
        vec![1, 4],
        "one bar per run, keyed by the run's first item"
    );
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
    assert_eq!(kinds(&rows), vec![("user", false)]);
}

/// A bodyless message renders nothing, so it must not change the layout of the
/// items that do.
#[test]
fn an_empty_message_projects_the_same_rows_as_no_message() {
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
/// slot leaves a blank row pinned over a collapsed response — and buries the
/// real reply that should have held it.
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
        vec![1, 2],
        "the real reply and the call it introduced"
    );
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r.kind, RowKind::ConclusionItem(_))),
        "the empty message cannot be the conclusion, and nothing else claims it"
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
        vec![("user", false), ("response", false), ("item", false)]
    );
}

fn preamble(text: &str) -> ChatItem {
    ChatItem::AssistantText {
        text: text.to_owned(),
        streaming: false,
        message_id: None,
        phase: daruda_acp::MessagePhase::Commentary,
    }
}

/// The captured-codex case: every call in the run is a kind the filter hides,
/// and the preamble the agent wrote ahead of them is the survivor holding the
/// turn on screen. Hiding preambles as their own kind takes that row too.
#[test]
fn hiding_preambles_takes_their_rows_with_them() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        preamble("worktree는 제거됐습니다. 이제"),
        kinded_tool("a", ToolKindView::Execute, Completed),
        kinded_tool("b", ToolKindView::Execute, Completed),
        asst("done"),
    ];
    let preamble_shown = |filter: &DisplayFilter| {
        project(
            &items,
            &FoldState::default(),
            false,
            &LiveSubagentUnits::of(&items),
            TailWindow::All,
            filter,
        )
        .iter()
        .any(|r| matches!(r.kind, RowKind::AgentItem(1)) && !r.hidden)
    };

    // Replies + Edits: the preamble is prose, so it survives even though both
    // of its commands are hidden.
    let with_preambles = DisplayFilter::from_tokens(["prose", "tool_edit"]);
    assert!(preamble_shown(&with_preambles), "the preamble survives");

    // The same, with preambles their own hidden kind.
    let without = DisplayFilter::from_tokens(["prose", "prose_answer", "tool_edit"]);
    assert!(!preamble_shown(&without), "and now it does not");
}

// ── Thinking groups ─────────────────────────────────────────────────────────

fn think_group_headers(rows: &[RenderRow]) -> Vec<&RenderRow> {
    rows.iter()
        .filter(|r| matches!(r.kind, RowKind::ThinkingGroupHeader { .. }))
        .collect()
}

/// Every thinking group's `(first_ix, count)`, in projection order.
fn think_group_spans(rows: &[RenderRow]) -> Vec<(usize, usize)> {
    rows.iter()
        .filter_map(|r| match r.kind {
            RowKind::ThinkingGroupHeader {
                first_ix, count, ..
            } => Some((first_ix, count)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_thinking_run_gets_one_group_header_and_indents_its_members() {
    let items = [
        ChatItem::UserText("q".into()),
        think("first"),
        think("second"),
        asst("done"),
    ];
    let rows = project_all(&items);
    let headers = think_group_headers(&rows);
    assert_eq!(headers.len(), 1, "one run, one header");
    let RowKind::ThinkingGroupHeader {
        first_ix,
        count,
        collapsed: _,
    } = headers[0].kind
    else {
        unreachable!()
    };
    assert_eq!((first_ix, count), (1, 2));
    let header_indent = headers[0].indent;
    for ix in [1usize, 2] {
        let child = rows
            .iter()
            .find(|r| matches!(r.kind, RowKind::AgentItem(j) if j == ix))
            .expect("a row per thought");
        assert_eq!(
            child.indent,
            header_indent + 1,
            "thought {ix} nests under its group bar"
        );
    }
}

#[test]
fn a_lone_thought_is_not_grouped() {
    let items = [
        ChatItem::UserText("q".into()),
        think("just one"),
        asst("done"),
    ];
    let rows = project_all(&items);
    assert!(think_group_headers(&rows).is_empty());
    assert!(
        rows.iter()
            .any(|r| matches!(r.kind, RowKind::AgentItem(1)) && !r.hidden),
        "the thought keeps its own plain row"
    );
}

#[test]
fn prose_between_two_runs_splits_them_into_separate_groups() {
    let items = [
        ChatItem::UserText("q".into()),
        think("a"),
        think("b"),
        asst("out loud"),
        think("c"),
        think("d"),
        asst("done"),
    ];
    assert_eq!(
        think_group_spans(&project_all(&items)),
        vec![(1, 2), (4, 2)]
    );
}

#[test]
fn a_tool_call_between_two_runs_splits_them_into_separate_groups() {
    let items = [
        ChatItem::UserText("q".into()),
        think("a"),
        think("b"),
        tool("x", ToolStatusView::Completed),
        think("c"),
        think("d"),
        asst("done"),
    ];
    assert_eq!(
        think_group_spans(&project_all(&items)),
        vec![(1, 2), (4, 2)]
    );
}

/// The thinking run grows only while the item is `Thinking` *and* not bodyless,
/// so an empty streaming chunk stops the run rather than being skipped through:
/// it earns no row, and letting it join a group would render a blank child and
/// inflate the count. The cost is cutting a run the reader sees as contiguous.
/// Both real captures (`acp-wire-codex-acp.log`, `acp-wire-claude.log`) hold
/// zero bodyless thinking items, so the stricter reading costs nothing today.
#[test]
fn an_empty_thought_inside_a_run_splits_it_rather_than_being_skipped_through() {
    let items = [
        ChatItem::UserText("q".into()),
        think("a"),
        think(""),
        think("b"),
        think("c"),
        asst("done"),
    ];
    let rows = project_all(&items);
    assert_eq!(
        think_group_spans(&rows),
        vec![(3, 2)],
        "b + c earn the only group; a is left alone below RUN_GROUP_MIN"
    );
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("item", false),       // a, a lone thought with no bar of its own
            ("thinkgroup", false), // b + c
            ("item", true),
            ("item", true),
            ("item", false), // the conclusion
        ]
    );

    // The same three thoughts with the empty chunk gone are one run of three.
    let contiguous = [
        ChatItem::UserText("q".into()),
        think("a"),
        think("b"),
        think("c"),
        asst("done"),
    ];
    assert_eq!(think_group_spans(&project_all(&contiguous)), vec![(1, 3)]);
}

#[test]
fn hiding_reasoning_takes_the_group_bar_with_it() {
    let items = [
        ChatItem::UserText("q".into()),
        think("a"),
        think("b"),
        asst("done"),
    ];
    let no_reasoning = DisplayFilter::from_tokens(["prose", "tools"]);
    let rows = project_filtered(&items, &no_reasoning);
    let headers = think_group_headers(&rows);
    assert_eq!(headers.len(), 1, "the bar keeps its slot");
    assert!(headers[0].hidden, "nothing left for it to summarize");
    for ix in [1usize, 2] {
        assert!(
            rows.iter()
                .any(|r| matches!(r.kind, RowKind::AgentItem(j) if j == ix) && r.hidden),
            "thought {ix} is filtered away"
        );
    }
    assert!(
        filtered_away(&rows).offers_reveal(),
        "the bar carries the reveal that brings them back"
    );
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
            TailWindow::All,
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

#[test]
fn a_thinking_group_folds_independently_of_the_tool_group_beside_it() {
    let items = [
        ChatItem::UserText("q".into()),
        think("a"),
        think("b"),
        tool("x", ToolStatusView::Completed),
        tool("y", ToolStatusView::Completed),
        asst("done"),
    ];
    // Auto opens the newest response and leaves both settled groups collapsed,
    // which is what makes the two toggles observable apart.
    let visible = |fold: &FoldState| -> Vec<usize> {
        project_under(&items, fold)
            .into_iter()
            .filter(|r| !r.hidden)
            .filter_map(|r| match r.kind {
                RowKind::AgentItem(ix) => Some(ix),
                _ => None,
            })
            .collect()
    };
    let mut fold = FoldState::with_mode(FoldPreset::Auto.mode());
    assert_eq!(visible(&fold), Vec::<usize>::new(), "both groups collapsed");

    fold.toggle(FoldKey::ThinkingGroup(1), FoldContext::last(false));
    assert_eq!(
        visible(&fold),
        vec![1, 2],
        "the thoughts open without touching the tools"
    );

    fold.toggle(FoldKey::ToolGroup("x".into()), FoldContext::last(false));
    assert_eq!(visible(&fold), vec![1, 2, 3, 4], "now both are open");
}

#[test]
fn thinking_groups_share_a_slot_by_their_first_index() {
    let group = |first_ix, count, collapsed| {
        RenderRow::at(
            RowKind::ThinkingGroupHeader {
                first_ix,
                count,
                collapsed,
            },
            false,
            1,
        )
    };
    let a = group(3, 2, false);
    assert!(a.same_slot(&group(3, 5, true)), "same first_ix → same slot");
    assert!(!a.same_slot(&group(4, 2, false)));
    assert!(!a.same_slot(&RenderRow::at(RowKind::AgentItem(3), false, 1)));
}

/// TEMP DIAGNOSTIC — remove.
#[test]
fn diag_tail_population() {
    let Some(path) = std::env::var_os("DIAG_LOG") else {
        return;
    };
    let agent = std::env::var("DIAG_AGENT").unwrap_or_else(|_| "claude".into());
    let preset = match std::env::var("DIAG_PRESET")
        .unwrap_or_else(|_| "auto".into())
        .as_str()
    {
        "expanded" => FoldPreset::Expanded,
        "summary" => FoldPreset::Summary,
        _ => FoldPreset::Auto,
    };
    let replay = daruda_acp::replay_log(std::path::Path::new(&path), &agent).expect("replay");
    let all = replay.items;
    // Last turn, as it looked while newest.
    let items = match std::env::var("DIAG_END") {
        Ok(v) => all[..v.parse::<usize>().unwrap()].to_vec(),
        Err(_) => all,
    };
    let live = LiveSubagentUnits::of(&items);
    for tail in [TailWindow::All, TailWindow::Last(5)] {
        let rows = project(
            &items,
            &FoldState::with_mode(preset.mode()),
            false,
            &live,
            tail,
            &DisplayFilter::default(),
        );
        println!("=== tail {tail:?} preset {preset:?} ===");
        for r in &rows {
            if r.hidden {
                continue;
            }
            let k = match &r.kind {
                RowKind::User(i) => format!("User({i})"),
                RowKind::Interrupted(i) => format!("Interrupted({i})"),
                RowKind::ResponseHeader {
                    run_start,
                    collapsed,
                    ..
                } => format!("Response@{run_start} collapsed={collapsed}"),
                RowKind::AgentItem(i) => format!("Item({i}:{})", label(&items[*i])),
                RowKind::TailMore {
                    run_start,
                    hidden_steps,
                    kept_steps,
                    collapsed,
                } => format!(
                    "TailMore@{run_start} hidden={hidden_steps} kept={kept_steps} collapsed={collapsed}"
                ),
                RowKind::ToolGroupTailMore {
                    gid,
                    hidden_calls,
                    kept_calls,
                    collapsed,
                } => format!(
                    "GroupTailMore@{gid} hidden={hidden_calls} kept={kept_calls} collapsed={collapsed}"
                ),
                RowKind::ToolGroupHeader {
                    first_ix,
                    count,
                    collapsed,
                    ..
                } => format!("ToolGroup@{first_ix} x{count} collapsed={collapsed}"),
                RowKind::ThinkingGroupHeader {
                    first_ix,
                    count,
                    collapsed,
                } => format!("ThinkGroup@{first_ix} x{count} collapsed={collapsed}"),
                RowKind::ConclusionItem(i) => format!("Conclusion({i})"),
                RowKind::WorkingIndicator => "Working".into(),
            };
            println!(
                "  {}{} outside={}",
                "  ".repeat(r.indent as usize),
                k,
                r.outside_window
            );
        }
    }
}

fn label(it: &ChatItem) -> &'static str {
    match it {
        ChatItem::UserText(_) => "user",
        ChatItem::AssistantText { .. } => "text",
        ChatItem::Thinking { .. } => "think",
        ChatItem::ToolCall(_) => "tool",
        ChatItem::Permission(_) => "perm",
        _ => "other",
    }
}

/// A Stop leaves the marker as the last item, but the pane can still be busy —
/// a trailing background subagent keeps `activity_state()` on `Working` for the
/// quiescence window after the cut. The progress row must survive that, or the
/// transcript contradicts every other activity readout in the pane.
#[test]
fn the_working_indicator_survives_a_trailing_stop_marker() {
    let items = [
        ChatItem::UserText("q".into()),
        tool("a", ToolStatusView::Completed),
        ChatItem::Interrupted,
    ];
    let rows = project(
        &items,
        &FoldState::default(),
        true,
        &LiveSubagentUnits::of(&items),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    let projected = kinds(&rows);
    assert!(
        projected.iter().any(|(k, _)| *k == "working"),
        "the marker must not swallow the progress row: {projected:?}"
    );
    assert_eq!(
        projected.last().map(|(k, _)| *k),
        Some("working"),
        "and it stays pinned to the tail, below the marker: {projected:?}"
    );

    // The emission is one post-loop site, so it no longer depends on a run
    // having been walked. Not reachable today (a prompt is echoed into `items`
    // before the pane reports Working), but pin it so a change at either end
    // is a test failure rather than a surprise.
    let rows = project(
        &[],
        &FoldState::default(),
        true,
        &LiveSubagentUnits::default(),
        TailWindow::All,
        &DisplayFilter::default(),
    );
    assert_eq!(kinds(&rows), vec![("working", false)]);
}

/// The marker never sits inside a run, at any position.
#[test]
fn the_stop_marker_is_always_a_top_level_row() {
    for items in [
        vec![ChatItem::Interrupted],
        vec![ChatItem::UserText("q".into()), ChatItem::Interrupted],
        vec![ChatItem::Interrupted, ChatItem::Interrupted],
        vec![
            ChatItem::UserText("q".into()),
            ChatItem::Interrupted,
            ChatItem::UserText("again".into()),
        ],
    ] {
        let rows = project(
            &items,
            &FoldState::default(),
            false,
            &LiveSubagentUnits::of(&items),
            TailWindow::All,
            &DisplayFilter::default(),
        );
        let markers: Vec<_> = rows
            .iter()
            .filter(|r| matches!(r.kind, RowKind::Interrupted(_)))
            .collect();
        assert_eq!(
            markers.len(),
            items
                .iter()
                .filter(|i| matches!(i, ChatItem::Interrupted))
                .count(),
            "one row per marker, no more and no fewer"
        );
        assert!(
            markers.iter().all(|r| r.indent == 0 && !r.hidden),
            "a marker is never nested and never folded away"
        );
    }
}

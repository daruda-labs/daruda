use super::*;
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
    })
}
/// A permission card — `resolved=false` makes it pending (actionable).
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
    }
}
fn kinds(rows: &[RenderRow]) -> Vec<(&'static str, bool)> {
    rows.iter()
        .map(|r| {
            let k = match r.kind {
                RowKind::User(_) => "user",
                RowKind::ResponseHeader { .. } => "response",
                // The conclusion is still an item row for visibility tests;
                // its distinct variant is asserted directly where it matters.
                RowKind::AgentItem(_) | RowKind::ConclusionItem(_) => "item",
                RowKind::SoloResponse(_) => "solo",
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
    let rows = project(&items, &FoldState::default(), false);
    // user anchor, response header (last turn → expanded), then the run at
    // indent 1: assistant, tool-group header, 3 settled tool members
    // (group collapsed → hidden), trailing assistant.
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("item", false),
            ("group", false),
            ("item", true),
            ("item", true),
            ("item", true),
            ("item", false),
        ]
    );
    // indents: user 0, response 0, run items 1, group members 2.
    let indents: Vec<u8> = rows.iter().map(|r| r.indent).collect();
    assert_eq!(indents, vec![0, 0, 1, 1, 2, 2, 2, 1]);
}

#[test]
fn trivial_response_has_no_bar() {
    // One short assistant reply, no tools → inline (no response header). The
    // block stands for the whole response, so it is a `SoloResponse` and carries
    // the rollup glyph the absent bar would have shown.
    let items = [ChatItem::UserText("hi".into()), asst("hello")];
    let rows = project(&items, &FoldState::default(), false);
    assert_eq!(kinds(&rows), vec![("user", false), ("solo", false)]);
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
    let rows = project(&items, &FoldState::default(), false);
    // Past response (first turn, settled, not last) collapses → its process
    // hides but its conclusion (a1, the run's last assistant text) stays
    // visible; current response (last turn) expands → its run visible.
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),     // first
            ("response", false), // header always shown
            ("item", false),     // a1 = conclusion, stays visible
            ("group", true),
            ("item", true),
            ("item", true),
            ("user", false), // second
            ("response", false),
            ("item", false),  // a2 visible (current expanded)
            ("group", false), // settled group → collapsed members
            ("item", true),
            ("item", true),
        ]
    );
}

#[test]
fn working_indicator_fills_gap_after_tool_group_settles() {
    use ToolStatusView::Completed;
    // A turn whose tools have all settled but the next assistant text has
    // not arrived yet, with a turn still in flight → trailing indicator.
    let items = [
        ChatItem::UserText("q".into()),
        asst("planning"),
        tool("a", Completed),
        tool("b", Completed),
    ];
    let rows = project(&items, &FoldState::default(), true);
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("item", false),  // assistant prose
            ("group", false), // tool group header
            ("item", true),   // settled members collapsed
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
        },
    ];
    let rows = project(&items, &FoldState::default(), true);
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
    let rows = project(&items, &FoldState::default(), false);
    assert!(!kinds(&rows).iter().any(|(k, _)| *k == "working"));
}

#[test]
fn working_indicator_on_first_token_wait() {
    // Prompt sent, no agent output yet, turn in flight → indicator under the
    // user message at top level (no response bar for an empty run).
    let items = [ChatItem::UserText("q".into())];
    let rows = project(&items, &FoldState::default(), true);
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
    fold.toggle(FoldKey::Response(0), true);
    let rows = project(&items, &fold, true);
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
    fold.toggle(FoldKey::Response(0), true); // collapse the response
    let rows = project(&items, &fold, false);
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
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
    let rows = project(&items, &FoldState::default(), false);
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
    let rows = project(&items, &FoldState::default(), false);
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
    fold.toggle(FoldKey::Response(0), true);
    let rows = project(&items, &fold, false);
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
    fold.toggle(FoldKey::Response(0), true);
    let rows = project(&items, &fold, false);
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("item", false), // "answer" = last assistant text → visible
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
    fold.toggle(FoldKey::Response(0), true);
    let rows = project(&items, &fold, false);
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("group", true), // no assistant text → nothing stays visible
            ("item", true),
            ("item", true),
        ]
    );
}

#[test]
fn pending_permission_stays_visible_when_response_collapsed() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        tool("a", Completed),
        tool("b", Completed),
        perm(false), // pending → actionable
    ];
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(0), true); // collapse the response
    let rows = project(&items, &fold, false);
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
}

#[test]
fn resolved_permission_folds_with_the_response() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        tool("a", Completed),
        tool("b", Completed),
        perm(true), // resolved → no longer actionable
    ];
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(0), true);
    let rows = project(&items, &fold, false);
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
    let rows = project(&items, &FoldState::default(), false);
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
    let rows = project(&items, &FoldState::default(), false);
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
    let rows = project(&items, &FoldState::default(), false);
    assert!(
        rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(1))),
        "an orphan child (no present parent) still renders as a row"
    );
}

fn child_of(id: &str, parent: &str, status: ToolStatusView) -> ChatItem {
    let mut c = tool(id, status);
    if let ChatItem::ToolCall(tc) = &mut c {
        tc.parent_tool_id = Some(parent.to_owned());
    }
    c
}

#[test]
fn subagent_parent_reads_live_while_a_child_runs() {
    use ToolStatusView::{Completed, InProgress};
    // The adapter marks the parent Task `Completed` (its SDK call returned)
    // while a flattened child keeps running — the unit is still working,
    // so the subtree must read live.
    let items = [
        tool("task", Completed),
        child_of("child", "task", InProgress),
    ];
    assert!(
        subagent_subtree_live(&items, "task", 8),
        "a live child keeps the subagent parent reading as running"
    );
}

#[test]
fn subagent_parent_settles_when_all_children_terminal() {
    use ToolStatusView::{Completed, Failed};
    let items = [
        tool("task", Completed),
        child_of("a", "task", Completed),
        child_of("b", "task", Failed),
    ];
    assert!(
        !subagent_subtree_live(&items, "task", 8),
        "no live descendant → the parent unit is done"
    );
}

#[test]
fn subagent_liveness_is_transitive() {
    use ToolStatusView::{Completed, InProgress};
    // parent → child(done) → grandchild(running): the running grandchild
    // still makes the whole unit live.
    let items = [
        tool("task", Completed),
        child_of("child", "task", Completed),
        child_of("grand", "child", InProgress),
    ];
    assert!(subagent_subtree_live(&items, "task", 8));
}

#[test]
fn subagent_liveness_no_children_is_false() {
    let items = [tool("task", ToolStatusView::Completed)];
    assert!(!subagent_subtree_live(&items, "task", 8));
}

#[test]
fn subagent_liveness_depth_cap_terminates_on_cycle() {
    use ToolStatusView::Completed;
    // A malformed self-parent (id == parent_tool_id) with a terminal status
    // must not recurse without bound; the depth budget forces `false`.
    let items = [child_of("x", "x", Completed)];
    assert!(!subagent_subtree_live(&items, "x", 8));
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
    let rows = project(&items, &fold, false);
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
    let rows = project(&items, &fold, false);
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
    let rows = project(&items, &FoldState::default(), false);
    assert_eq!(
        kinds(&rows),
        vec![("item", false), ("item", false), ("item", false)],
        "a single tool call renders as a plain item, no group header"
    );
}

#[test]
fn in_progress_group_defaults_expanded() {
    use ToolStatusView::{Completed, InProgress};
    let items = [tool("a", Completed), tool("b", InProgress)];
    let rows = project(&items, &FoldState::default(), false);
    // group active (one tool in progress) → members visible.
    assert_eq!(
        kinds(&rows),
        vec![("group", false), ("item", false), ("item", false)]
    );
}

#[test]
fn group_member_visibility_follows_fold_override() {
    use ToolStatusView::Completed;
    let items = [tool("a", Completed), tool("b", Completed)];
    let mut fold = FoldState::default();
    // Force-expand the (otherwise collapsed) settled group.
    fold.toggle(FoldKey::ToolGroup("a".into()), false);
    let rows = project(&items, &fold, false);
    assert_eq!(
        kinds(&rows),
        vec![("group", false), ("item", false), ("item", false)],
        "user-expanded group shows its members"
    );
}

#[test]
fn same_slot_compares_key_not_hidden_or_payload() {
    let a = RenderRow {
        kind: RowKind::ToolGroupHeader {
            gid: "g".into(),
            first_ix: 1,
            count: 2,
            collapsed: false,
        },
        hidden: false,
        indent: 0,
    };
    let b = RenderRow {
        kind: RowKind::ToolGroupHeader {
            gid: "g".into(),
            first_ix: 5,
            count: 3,
            collapsed: true,
        },
        hidden: true,
        indent: 0,
    };
    assert!(
        a.same_slot(&b),
        "same gid → same slot regardless of count/hidden"
    );

    let u0 = RenderRow {
        kind: RowKind::User(0),
        hidden: false,
        indent: 0,
    };
    let u1 = RenderRow {
        kind: RowKind::User(1),
        hidden: false,
        indent: 0,
    };
    assert!(!u0.same_slot(&u1));
    assert!(!u0.same_slot(&RenderRow {
        kind: RowKind::AgentItem(0),
        hidden: false,
        indent: 0
    }));
}

#[test]
fn collapsed_response_survivors_all_sit_at_the_run_indent() {
    use ToolStatusView::{Completed, InProgress};
    // A collapsed response leaves three kinds of row visible: its own bar, the
    // conclusion, a still-live tool group, and the working indicator. The bar is
    // the parent at indent 0 and every survivor stays one level in, so a folded
    // turn still reads as "bar ⊃ what survived" rather than a flat list. The
    // trivial-response case has no bar, hence no parent and no indent — that
    // difference is hierarchy, not drift.
    let items = [
        ChatItem::UserText("q".into()),
        asst("planning"),
        tool("a", Completed),
        tool("b", InProgress),
        asst("here is the result"),
    ];
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(0), true);
    let rows = project(&items, &fold, true);

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
    // The set of survivors is what the invariant above is worth asserting over.
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
fn leading_run_without_a_user_anchor_gets_no_solo_response() {
    use ToolStatusView::Failed;
    // Reachable on session restore: `append_user_chunk` drops a replayed
    // `<task-notification>` user turn (see daruda_acp::mapping), so a restored
    // pane can open with agent items and no `UserText` anchor. Such a run gets no
    // response bar, and its blocks must stay plain `AgentItem`s — a `SoloResponse`
    // here would let the assistant header report ✓ over itself while the sibling
    // tool call sitting right next to it has failed.
    let items = [asst("here is what I found"), tool("c1", Failed)];
    let rows = project(&items, &FoldState::default(), false);

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
    assert_eq!(kinds(&rows), vec![("item", false), ("item", false)]);
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
    let rows = project(&items, &FoldState::default(), false);
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

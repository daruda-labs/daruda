//! Fold defaults and collapse: what each bar's own state hides.

use super::*;

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
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::default(),
    );
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("tail", true),
            ("item", false),
            ("group", true),
            ("grouptail", true),
            ("item", true),
            ("item", true),
            ("user", false),
            ("response", false),
            ("tail", true),
            ("item", false),
            ("group", false),
            ("grouptail", true),
            ("item", true),
            ("item", true),
        ]
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
            ("item", false),
            ("group", false),
            ("grouptail", true),
            ("item", true),
            ("item", true),
            ("user", false),
            ("response", false),
            ("tail", true),
            ("item", false),
            ("group", false),
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
        StepWindow::uniform(TailWindow::All),
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
        StepWindow::uniform(TailWindow::All),
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
        StepWindow::uniform(TailWindow::All),
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
fn sole_reply_earns_the_conclusions_fold() {
    // A lone reply is the whole response, and the response bar cannot fold it:
    // the conclusion escape keeps it on screen through that fold. The bare
    // chevron is the only control left, so it is the one the reply must get --
    // without it a prose-only turn (`/usage`) has no fold at all.
    let items = [ChatItem::UserText("hi".into()), asst("hello")];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::default(),
    );
    assert!(
        rows.iter()
            .any(|r| matches!(r.kind, RowKind::ConclusionItem(1)) && !r.hidden)
    );
    assert!(!rows.iter().any(|r| matches!(r.kind, RowKind::AgentItem(1))));
}

#[test]
fn sole_reply_stays_visible_when_its_response_is_collapsed() {
    // The conclusion escape is what makes the bar unable to fold it, so the
    // bare chevron has to be a live second control rather than a duplicate of
    // one that already works.
    let items = [ChatItem::UserText("hi".into()), asst("hello")];
    let mut fold = FoldState::default();
    fold.toggle(FoldKey::Response(1), FoldContext::past(false));
    let rows = project(
        &items,
        &fold,
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::default(),
    );
    assert!(
        rows.iter()
            .any(|r| matches!(r.kind, RowKind::ConclusionItem(1)) && !r.hidden)
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
        StepWindow::uniform(TailWindow::All),
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
        StepWindow::uniform(TailWindow::All),
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
    let rows = project_tail(&items, StepWindow::uniform(TailWindow::Last(1)));
    assert!(visible(&rows, 2), "the prompt survives a covered run");
    assert!(!visible(&rows, 1), "its neighbouring prose is covered");
    assert!(!visible(&rows, 3), "so is the call the window left out");
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
        StepWindow::uniform(TailWindow::All),
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
        StepWindow::uniform(TailWindow::All),
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
fn in_progress_group_defaults_expanded() {
    use ToolStatusView::{Completed, InProgress};
    let items = [tool("a", Completed), tool("b", InProgress)];
    let rows = project(
        &items,
        &FoldState::default(),
        false,
        &LiveSubagentUnits::of(&items),
        StepWindow::uniform(TailWindow::All),
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
        StepWindow::uniform(TailWindow::All),
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
        StepWindow::uniform(TailWindow::All),
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

/// A group of one takes the same default as a group of many: the fold setting
/// decides, and nothing about the member count overrides it. Auto shuts a
/// settled group; the Expanded setting opens the newest turn's.
#[test]
fn a_group_of_one_follows_the_fold_setting_like_any_other() {
    use ToolStatusView::Completed;
    let items = [ChatItem::UserText("q".into()), tool("a", Completed)];
    let collapsed_under = |fold: &FoldState| {
        project_under(&items, fold)
            .into_iter()
            .find_map(|r| match r.kind {
                RowKind::ToolGroupHeader { collapsed, .. } => Some(collapsed),
                _ => None,
            })
            .expect("the run earns a bar")
    };
    assert!(collapsed_under(&FoldState::default()), "auto shuts it");
    assert!(
        !collapsed_under(&FoldState::with_mode(FoldPreset::Expanded.mode())),
        "the expanded setting opens it"
    );
    assert!(
        project_under(&items, &FoldState::default())
            .iter()
            .any(|r| matches!(r.kind, RowKind::AgentItem(1)) && r.hidden),
        "the only call goes behind the bar with it"
    );
}

/// A subagent launch settles the moment its SDK call returns, while the work it
/// delegated keeps running inside its card. A shut bar must not take those
/// launches off screen mid-run.
#[test]
fn a_running_member_stays_on_screen_under_its_collapsed_group() {
    use ToolStatusView::{Completed, InProgress};
    let items = vec![
        ChatItem::UserText("q".into()),
        asst("delegating"),
        subagent_launch("A", Completed),
        child_of("a1", "A", InProgress),
        subagent_launch("B", Completed),
        child_of("b1", "B", InProgress),
    ];
    let visible: Vec<usize> = project_all(&items)
        .iter()
        .filter(|r| !r.hidden)
        .filter_map(|r| match r.kind {
            RowKind::AgentItem(ix) => Some(ix),
            _ => None,
        })
        .collect();
    assert_eq!(
        visible,
        vec![1, 2, 4],
        "the prose and both working launches"
    );
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
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::default(),
    );
    let collapsed = |first: usize| {
        rows.iter()
            .find_map(|r| match &r.kind {
                RowKind::ToolGroupHeader {
                    calls, collapsed, ..
                } if calls[0] == first => Some(*collapsed),
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
            StepWindow::uniform(TailWindow::All),
            &only_tools,
        ))
    };
    assert_eq!(tally(&FoldState::default()), tally(&collapsed));
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

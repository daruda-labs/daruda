//! Run and group structure: where a run begins and ends, and what its bar
//! speaks for.

use super::*;

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
        StepWindow::uniform(TailWindow::All),
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
            ("item", true), // three settled members, folded under their bar
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
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::default(),
    );
    assert_eq!(
        kinds(&rows),
        vec![("user", false), ("response", false), ("item", false)]
    );
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
            ("group", false),
            ("grouptail", true),
            ("item", true), // settled members collapsed
            ("item", true),
            ("working", false), // gap indicator at the run tail
        ]
    );
    // The indicator nests inside the response (indent 1), not at top level.
    assert_eq!(rows.last().unwrap().indent, 1);
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
        StepWindow::uniform(TailWindow::All),
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
        StepWindow::uniform(TailWindow::All),
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
fn a_lone_tool_call_gets_its_own_group() {
    let items = [asst("x"), tool("a", ToolStatusView::Completed), asst("y")];
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
            ("response", false),
            ("tail", true),
            ("item", false),     // "x"
            ("group", false),    // one call is still a run, so it earns a bar
            ("grouptail", true), // nothing behind the bar
            ("item", true),      // and the bar's fold shuts over it, as on any run
            ("item", false),     // "y" = conclusion, never folded away
        ],
        "one call earns the bar that carries its fold, like any other run"
    );
    assert!(
        rows.iter()
            .any(|r| matches!(r.kind, RowKind::ToolGroupHeader { .. }))
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
        StepWindow::uniform(TailWindow::All),
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
            ("group", false),
            ("grouptail", true),
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
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::default(),
    );
    assert!(
        rows.iter()
            .any(|r| matches!(r.kind, RowKind::ResponseHeader { .. }))
    );
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
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::default(),
    );
    assert!(
        rows.iter()
            .filter(|r| matches!(r.kind, RowKind::AgentItem(2)))
            .all(|r| !r.hidden),
        "the prose stays a plain row the response's own fold governs"
    );
    assert_eq!(
        think_group_spans(&rows),
        vec![(1, 1)],
        "the lone thought earns a bar of its own and folds under it"
    );
    let group = rows
        .iter()
        .find_map(|r| match &r.kind {
            RowKind::ToolGroupHeader { calls, .. } => Some((calls[0], calls.len())),
            _ => None,
        })
        .expect("two consecutive calls earn a group");
    assert_eq!(group, (3, 2), "the group starts at the first call");
    let indents: Vec<u8> = rows
        .iter()
        .filter(|r| matches!(r.kind, RowKind::AgentItem(2)))
        .map(|r| r.indent)
        .collect();
    assert_eq!(indents, vec![1], "no wrapper indents the prose");
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
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::default(),
    );
    let starts: Vec<usize> = rows
        .iter()
        .filter_map(|r| match &r.kind {
            RowKind::ToolGroupHeader { calls, .. } => Some(calls[0]),
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
        StepWindow::uniform(TailWindow::All),
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

/// `RUN_GROUP_MIN` is the whole rule, and it is one: every run gets the bar
/// that carries its fold and its summary, so a turn's shape does not change
/// with how many calls happened to land next to each other.
#[test]
fn every_run_earns_a_group_however_short() {
    use ToolStatusView::Completed;
    let groups = |items: &[ChatItem]| {
        project(
            items,
            &FoldState::default(),
            false,
            &LiveSubagentUnits::of(items),
            StepWindow::uniform(TailWindow::All),
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
        1
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

/// The bar speaks for its calls, so it has to name them. A span cannot: the run
/// covers items that own no row, so `first..first + count` would pick up a
/// nested child and drop the call the group actually ends on.
#[test]
fn a_group_header_names_the_calls_it_speaks_for() {
    use ToolStatusView::Completed;
    let items = [
        ChatItem::UserText("q".into()),
        tool("a", Completed),
        child_of("mid", "a", Completed),
        tool("b", Completed),
    ];
    let header = project_all(&items)
        .into_iter()
        .find_map(|r| match r.kind {
            RowKind::ToolGroupHeader { calls, .. } => Some(calls),
            _ => None,
        })
        .expect("the run earns a bar");
    assert_eq!(header, vec![1, 3], "the two top-level calls, not the child");
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
    let rows = project_open_group(&items, StepWindow::uniform(TailWindow::Last(1)));
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

/// The bar keeps its slot as its tally changes, so toggling a filter facet
/// re-labels the chip in place instead of splicing the list and drifting the
/// scroll.
#[test]
fn the_bar_keeps_its_slot_as_its_tally_changes() {
    let bar = |run_start: usize, filtered: FilteredAway, collapsed: bool| {
        RenderRow::at(
            RowKind::ResponseHeader {
                run_start,
                categories: Vec::new(),
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

/// Every run earns a header, however short — a lone thought that rendered bare
/// while a pair sat under a bar read as two different kinds of step.
#[test]
fn a_lone_thought_earns_its_own_group() {
    let items = [
        ChatItem::UserText("q".into()),
        think("just one"),
        asst("done"),
    ];
    let rows = project_all(&items);
    assert_eq!(think_group_spans(&rows), vec![(1, 1)]);
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
        vec![(1, 1), (3, 2)],
        "the empty thought splits the run in two; each side earns its own bar"
    );
    assert_eq!(
        kinds(&rows),
        vec![
            ("user", false),
            ("response", false),
            ("thinkgroup", false), // a, alone but still a run
            ("item", true),
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

#[test]
fn prose_never_renders_outside_a_response_bar() {
    // Every run that puts anything on screen earns a response bar and projects
    // its blocks at indent 1, which is what lets `render_item` render prose
    // inline with no speaker label of its own. The shapes below are the ones
    // that could reach `base_indent = 0`; none emits an item row at all.
    let shapes: Vec<(&str, Vec<ChatItem>)> = vec![
        (
            "only bodyless blocks, so the run renders nothing",
            vec![ChatItem::UserText("u".into()), asst(""), think("")],
        ),
        (
            "a sole reply",
            vec![ChatItem::UserText("u".into()), asst("a")],
        ),
        ("a run with no user prompt before it", vec![asst("a")]),
        (
            "a failure as the run's only block",
            vec![
                ChatItem::UserText("u".into()),
                ChatItem::Failure(daruda_acp::AcpFailure::unclassified("boom")),
            ],
        ),
        (
            "a permission as the run's only block",
            vec![ChatItem::UserText("u".into()), perm(false)],
        ),
        (
            "a stop marker, which owns a top-level row",
            vec![ChatItem::UserText("u".into()), ChatItem::Interrupted],
        ),
    ];
    for (name, items) in shapes {
        let rows = project(
            &items,
            &FoldState::default(),
            false,
            &LiveSubagentUnits::of(&items),
            StepWindow::uniform(TailWindow::All),
            &DisplayFilter::default(),
        );
        assert!(
            !rows.iter().any(|r| matches!(
                r.kind,
                RowKind::AgentItem(_) | RowKind::ConclusionItem(_)
            ) && r.indent == 0),
            "{name}: an item row at indent 0 would be prose with no bar above it"
        );
    }
}

/// A turn whose work is one run gets no tally on its bar: the group bar
/// directly below says the same thing, and two bars repeating one summary read
/// as a fault rather than as a hierarchy. Two runs and the turn bar earns it
/// back, because no single bar below covers both.
#[test]
fn the_turn_tally_is_withheld_when_one_bar_below_already_says_it() {
    use ToolStatusView::Completed;
    let tally_of = |items: &[ChatItem]| {
        project_all(items)
            .iter()
            .find_map(|r| match &r.kind {
                RowKind::ResponseHeader { categories, .. } => Some(categories.clone()),
                _ => None,
            })
            .expect("the turn has a bar")
    };
    let mut child = tool("child", Completed);
    if let ChatItem::ToolCall(tc) = &mut child {
        tc.parent_tool_id = Some("parent".to_owned());
    }
    assert!(
        tally_of(&[
            ChatItem::UserText("q".into()),
            tool("parent", Completed),
            child,
            asst("done"),
        ])
        .is_empty(),
        "one run — a subagent's children are nested, not a second run"
    );
    assert!(
        !tally_of(&[
            ChatItem::UserText("q".into()),
            tool("a", Completed),
            asst("between"),
            tool("b", Completed),
            asst("done"),
        ])
        .is_empty(),
        "prose split them into two runs, so the bar summarizes across both"
    );
}

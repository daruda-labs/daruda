//! The row vocabulary itself: which items earn a row, which slot it takes,
//! and what the conclusion and the working indicator do.

use super::*;

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
        StepWindow::uniform(TailWindow::All),
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
        StepWindow::uniform(TailWindow::All),
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
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::default(),
    );
    assert_eq!(kinds(&rows), vec![("user", false), ("working", false)]);
    assert_eq!(rows.last().unwrap().indent, 0);
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
        StepWindow::uniform(TailWindow::All),
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
        StepWindow::uniform(TailWindow::All),
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
            categories: Vec::new(),
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
            calls: Vec::new(),
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
            calls: vec![1, 2],
            collapsed: false,
        },
        false,
        0,
    );
    let b = RenderRow::at(
        RowKind::ToolGroupHeader {
            gid: "g".into(),
            calls: vec![5, 6, 7],
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

/// The two levels are one axis but two windows: narrowing the steps must not
/// trim a kept run's calls, and narrowing the calls must not hide a run. One
/// value drove both before, so either narrowing did both at once.
#[test]
fn the_two_levels_narrow_independently() {
    // Four runs of two calls each: enough for both levels to have something to
    // hold back, and few enough to assert every row by hand.
    let items = turn_of_cycles(4);

    let calls_only = project_open_group(
        &items,
        StepWindow {
            steps: TailWindow::All,
            calls: TailWindow::Last(1),
        },
    );
    assert_eq!(
        group_visibility(&calls_only),
        vec![true; 4],
        "a window on the calls hides no run"
    );
    assert_eq!(
        call_visibility(&items, &calls_only),
        vec![false, true, false, true, false, true, false, true],
        "every group keeps only its last call"
    );
    assert_eq!(
        tail_counts(&calls_only).0,
        0,
        "the response's own boundary has nothing to offer"
    );
    assert_eq!(group_tail_counts(&calls_only), (1, 1));

    let steps_only = project_open_group(
        &items,
        StepWindow {
            steps: TailWindow::Last(2),
            calls: TailWindow::All,
        },
    );
    assert_eq!(
        group_visibility(&steps_only),
        vec![false, false, true, true],
        "a window on the steps hides the covered runs"
    );
    assert_eq!(
        call_visibility(&items, &steps_only),
        vec![false, false, false, false, true, true, true, true],
        "and a kept run shows every call it holds"
    );
    assert_eq!(tail_counts(&steps_only).0, 2);
    assert_eq!(
        group_tail_counts(&steps_only),
        (0, 2),
        "no group boundary has anything to offer"
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
        StepWindow::uniform(TailWindow::All),
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
            StepWindow::uniform(TailWindow::All),
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
        StepWindow::uniform(TailWindow::All),
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
        "the real reply — the call it introduced sits behind its own group bar"
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
        StepWindow::uniform(TailWindow::All),
        &DisplayFilter::default(),
    );
    assert_eq!(
        kinds(&rows),
        vec![("user", false), ("response", false), ("item", false)]
    );
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
            StepWindow::uniform(TailWindow::All),
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
        StepWindow::uniform(TailWindow::All),
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
        StepWindow::uniform(TailWindow::All),
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
            StepWindow::uniform(TailWindow::All),
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

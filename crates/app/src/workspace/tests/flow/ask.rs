//! Permission questions: the queue, the modal, and answering.
//!
//! `flow_runs.rs`'s own tests cover the queue's rules without a window. These
//! are the surface half — whether a modal goes up, for which lane, and whether
//! a click on a stale frame does anything.

use super::*;

/// A question has to survive being painted more than once, and an answer
/// has to name the question it is answering: a surface can still be showing
/// a resolved question, and that click must do nothing rather than answer
/// whatever came next.
#[gpui::test]
async fn an_answer_only_lands_on_the_question_it_names(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);

    // `cx.update_window`, not `wh.update`: the latter leases the window's
    // root view, and parking a question opens a dialog — which has to lease
    // that same root. The real path arrives through the workspace entity,
    // where nothing is holding it.
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let here = ws.active;
            let run_dir = lane.path().join("run");
            ws.seed_flow_run_for_test(here, run_dir.clone());
            let (reply_tx, reply_rx) = smol::channel::bounded(1);
            ws.park_flow_ask_for_test(
                here,
                &run_dir,
                daruda_flow::runner::PendingAsk {
                    node: "design".to_string(),
                    attempt: 1,
                    ask_id: 7,
                    request: daruda_flow::runner::AskRequest {
                        tool: "Bash".to_string(),
                        detail: Some("rm -rf build".to_string()),
                        options: Vec::new(),
                    },
                    reply: reply_tx,
                },
                window,
                cx,
            );

            // The panel projects the question, and the projection is what a
            // click quotes back.
            let row = ws
                .flow_rows_for_active_lane()
                .into_iter()
                .next()
                .expect("a row for the parked run");
            let asking = row.asking.expect("the row carries the question");
            assert_eq!(asking.ask_id, 7);
            assert_eq!(asking.tool.as_ref(), "Bash");

            // A stale click — right lane, wrong question.
            ws.answer_flow_ask(
                here,
                6,
                daruda_acp::PermissionDecision::Allow {
                    option_id: "once".to_string(),
                },
                cx,
            );
            assert!(
                reply_rx.try_recv().is_err(),
                "a click on a resolved question answered the live one"
            );

            ws.answer_flow_ask(
                here,
                7,
                daruda_acp::PermissionDecision::Allow {
                    option_id: "once".to_string(),
                },
                cx,
            );
            assert!(
                matches!(
                    reply_rx.try_recv(),
                    Ok(daruda_acp::PermissionDecision::Allow { .. })
                ),
                "the answer never reached the run"
            );

            // And the question is gone at once. The agent goes back to work for
            // as long as it likes, so waiting for the run's next event leaves
            // the buttons up with no sign the click did anything — which reads
            // as a dead button, and gets answered again.
            let row = ws
                .flow_rows_for_active_lane()
                .into_iter()
                .next()
                .expect("the run is still there");
            assert!(
                row.asking.is_none(),
                "the answered question stayed on screen"
            );
        });
    })
    .expect("the test window is live");
}

/// Dismissing the modal is not an answer.
///
/// The question lives on the run, and the modal is one of two views onto
/// it — so closing it must leave the question in the panel, where it can
/// wait as long as it likes. If ownership ever moved into the modal, Esc
/// would silently throw the question away and the run would park forever
/// with nothing on screen.
///
/// What the dialog *looks* like is checked with
/// `--screenshot-scenario flow-asking`; that it is up is asserted here.
#[gpui::test]
async fn closing_the_modal_leaves_the_question_standing(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let here = ws.active;
            let run_dir = lane.path().join("run");
            ws.seed_flow_run_for_test(here, run_dir.clone());
            let (reply_tx, reply_rx) = smol::channel::bounded(1);
            ws.park_flow_ask_for_test(
                here,
                &run_dir,
                daruda_flow::runner::PendingAsk {
                    node: "design".to_string(),
                    attempt: 1,
                    ask_id: 3,
                    request: daruda_flow::runner::AskRequest {
                        tool: "Bash".to_string(),
                        detail: None,
                        options: Vec::new(),
                    },
                    reply: reply_tx,
                },
                window,
                cx,
            );

            assert!(
                crate::ui::WindowExt::has_active_dialog(window, cx),
                "a question in the lane in view did not reach the front"
            );

            // What Escape does: closes the dialog and nothing else.
            crate::ui::WindowExt::close_dialog(window, cx);

            let row = ws
                .flow_rows_for_active_lane()
                .into_iter()
                .next()
                .expect("the run is still there");
            assert_eq!(
                row.asking.map(|a| a.ask_id),
                Some(3),
                "dismissing the modal took the question with it"
            );
            assert!(
                reply_rx.try_recv().is_err(),
                "dismissing the modal answered on the user's behalf"
            );
        });
    })
    .expect("the test window is live");
}

/// A run in another lane must not take the window away from what someone
/// is doing. The question still parks — the chip and the panel are how it
/// is reachable — but nothing is raised over the top of the lane in view.
#[gpui::test]
async fn a_question_from_a_lane_out_of_view_does_not_take_the_window(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let elsewhere = daruda_store::project::LaneRef {
                project: ws.active.project,
                lane: ws.active.lane + 1,
            };
            assert_ne!(elsewhere, ws.active, "the fixture lane is the one in view");
            let run_dir = lane.path().join("run");
            ws.seed_flow_run_for_test(elsewhere, run_dir.clone());
            let (reply_tx, _reply_rx) = smol::channel::bounded(1);
            ws.park_flow_ask_for_test(
                elsewhere,
                &run_dir,
                daruda_flow::runner::PendingAsk {
                    node: "design".to_string(),
                    attempt: 1,
                    ask_id: 4,
                    request: daruda_flow::runner::AskRequest {
                        tool: "Bash".to_string(),
                        detail: None,
                        options: Vec::new(),
                    },
                    reply: reply_tx,
                },
                window,
                cx,
            );

            assert!(
                !crate::ui::WindowExt::has_active_dialog(window, cx),
                "a run in another lane took the window"
            );
            // And it is still a question somebody can answer: the chip is
            // workspace-wide, so it carries the lanes out of view.
            assert!(
                ws.flow_status_rows()
                    .iter()
                    .any(|row| row.asking.as_ref().is_some_and(|a| a.ask_id == 4)),
                "the parked question is not reachable anywhere"
            );
        });
    })
    .expect("the test window is live");
}

/// A channel drains before it closes, so the last question a stopping run
/// queued can arrive after the next run in that lane has taken the slot.
/// `ask_id` restarts at 1 per run, so without the run it came from being
/// checked it lands on the newcomer looking exactly like its own — and
/// the person answers a question nothing is waiting for.
#[gpui::test]
async fn a_question_left_over_from_the_previous_run_is_not_shown_as_this_one_s(
    cx: &mut TestAppContext,
) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let here = ws.active;
            let previous = lane.path().join("runs/1");
            let current = lane.path().join("runs/2");
            ws.seed_flow_run_for_test(here, current);

            let (reply_tx, _reply_rx) = smol::channel::bounded(1);
            ws.park_flow_ask_for_test(
                here,
                &previous,
                daruda_flow::runner::PendingAsk {
                    node: "design".to_string(),
                    attempt: 1,
                    ask_id: 1,
                    request: daruda_flow::runner::AskRequest {
                        tool: "Bash".to_string(),
                        detail: None,
                        options: Vec::new(),
                    },
                    reply: reply_tx,
                },
                window,
                cx,
            );

            let row = ws
                .flow_rows_for_active_lane()
                .into_iter()
                .next()
                .expect("the run is still there");
            assert!(
                row.asking.is_none(),
                "a question from the run before landed on this one"
            );
            assert!(
                !crate::ui::WindowExt::has_active_dialog(window, cx),
                "a question from the run before took the window"
            );
        });
    })
    .expect("the test window is live");
}

/// Stop is the other way out of a question, and the modal's backdrop is
/// why it has to be reachable from there: with a dialog up, the panel's
/// Stop cannot be clicked at all. Stopping settles the question on its way
/// out — a stopped run has no question anyone should still be answering,
/// and leaving the buttons up is how "the button does nothing" was met the
/// first time.
#[gpui::test]
async fn stopping_a_run_takes_its_question_down_with_it(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let here = ws.active;
            let run_dir = lane.path().join("run");
            ws.seed_flow_run_for_test(here, run_dir.clone());
            let (reply_tx, _reply_rx) = smol::channel::bounded(1);
            ws.park_flow_ask_for_test(
                here,
                &run_dir,
                daruda_flow::runner::PendingAsk {
                    node: "design".to_string(),
                    attempt: 2,
                    ask_id: 9,
                    request: daruda_flow::runner::AskRequest {
                        tool: "Bash".to_string(),
                        detail: None,
                        options: Vec::new(),
                    },
                    reply: reply_tx,
                },
                window,
                cx,
            );

            ws.stop_flow_run_in(here, cx);

            let row = ws
                .flow_rows_for_active_lane()
                .into_iter()
                .next()
                .expect("the run is still listed until it ends");
            assert!(
                row.asking.is_none(),
                "a stopped run still offers its question to answer"
            );
            // And it says what it was doing, not a blank stage.
            assert!(row.doing.contains("design"), "{}", row.doing);
        });
    })
    .expect("the test window is live");
}

/// Nodes running together ask independently, so a second question can
/// arrive while the first is up. It waits behind it.
///
/// Replacing the first is what the single-question stage used to do, and
/// the cost is not cosmetic: the replaced question's node stays parked on a
/// reply nobody can send any more, and the run waits forever on something
/// no longer on screen.
#[gpui::test]
async fn a_second_question_waits_behind_the_first(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.update(&mut cx, |ws, _| ws.active);
    let run_dir = lane.path().join("run");
    ws.update(&mut cx, |ws, _| {
        ws.seed_flow_run_for_test(here, run_dir.clone())
    });

    let first = park_question(&ws, &mut cx, here, &run_dir, 1);
    let _second = park_question(&ws, &mut cx, here, &run_dir, 2);

    let row = ws.update(&mut cx, |ws, _| {
        ws.flow_rows_for_active_lane()
            .into_iter()
            .next()
            .expect("the run is listed")
    });
    assert_eq!(
        row.asking.as_ref().map(|a| a.ask_id),
        Some(1),
        "the second question replaced the first"
    );
    assert_eq!(row.also_waiting, 1, "the second question was dropped");
    assert!(
        first.try_recv().is_err(),
        "the first question was answered by the second one arriving"
    );
}

/// Answering promotes the next. Otherwise the run reads as working while a
/// question nobody can see is still holding a node.
#[gpui::test]
async fn answering_brings_the_next_question_forward(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.update(&mut cx, |ws, _| ws.active);
    let run_dir = lane.path().join("run");
    ws.update(&mut cx, |ws, _| {
        ws.seed_flow_run_for_test(here, run_dir.clone())
    });
    let _first = park_question(&ws, &mut cx, here, &run_dir, 1);
    let _second = park_question(&ws, &mut cx, here, &run_dir, 2);

    let row = ws.update(&mut cx, |ws, cx| {
        ws.answer_flow_ask(
            here,
            1,
            daruda_acp::PermissionDecision::Reject {
                option_id: "no".to_string(),
            },
            cx,
        );
        ws.flow_rows_for_active_lane()
            .into_iter()
            .next()
            .expect("the run is listed")
    });
    assert_eq!(
        row.asking.as_ref().map(|a| a.ask_id),
        Some(2),
        "answering hid a question that is still waiting"
    );
    assert_eq!(row.also_waiting, 0);
}

/// A question that arrives after the last one was answered raises the
/// modal, same as the first did.
///
/// Written to settle an observation from a real run: five questions, one
/// modal. Either the person answered the rest in the panel before the
/// modal reached them, or later questions do not raise one at all — and
/// those two look identical from the outside.
#[gpui::test]
async fn a_question_arriving_after_the_last_was_answered_still_raises_the_modal(
    cx: &mut TestAppContext,
) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.update(&mut cx, |ws, _| ws.active);
    let run_dir = lane.path().join("run");
    ws.update(&mut cx, |ws, _| {
        ws.seed_flow_run_for_test(here, run_dir.clone())
    });

    let _first = park_question(&ws, &mut cx, here, &run_dir, 1);
    cx.update(|window, cx| {
        assert!(
            crate::ui::WindowExt::has_active_dialog(window, cx),
            "the first question did not reach the front"
        );
        ws.update(cx, |ws, cx| {
            ws.answer_flow_ask(
                here,
                1,
                daruda_acp::PermissionDecision::Allow {
                    option_id: "once".to_string(),
                },
                cx,
            );
        });
        crate::ui::WindowExt::close_dialog(window, cx);
    });

    let _second = park_question(&ws, &mut cx, here, &run_dir, 2);
    cx.update(|window, cx| {
        assert!(
            crate::ui::WindowExt::has_active_dialog(window, cx),
            "the next question never reached the front"
        );
    });
}

/// A question that arrives *while* one is up must not raise a second
/// modal. It is behind the first, and a dialog stacked over the one being
/// answered would show the same question twice — the front one — with the
/// person's click landing on whichever copy is on top.
#[gpui::test]
async fn a_queued_question_does_not_stack_a_second_modal(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.update(&mut cx, |ws, _| ws.active);
    let run_dir = lane.path().join("run");
    ws.update(&mut cx, |ws, _| {
        ws.seed_flow_run_for_test(here, run_dir.clone())
    });

    let _first = park_question(&ws, &mut cx, here, &run_dir, 1);
    let _second = park_question(&ws, &mut cx, here, &run_dir, 2);

    // One close is enough to clear the front dialog. A second one behind
    // it would still be up.
    cx.update(crate::ui::WindowExt::close_dialog);
    cx.update(|window, cx| {
        assert!(
            !crate::ui::WindowExt::has_active_dialog(window, cx),
            "a queued question stacked a modal of its own"
        );
    });
}

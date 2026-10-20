//! Permission questions: the set a run holds open, the modal, and answering.
//!
//! `runs/flows`'s own tests cover the set's rules without a window. These
//! are the surface half — whether a modal goes up, for which lane, and whether
//! a click on a stale frame does anything.

use super::*;

fn allow() -> daruda_acp::PermissionDecision {
    daruda_acp::PermissionDecision::Allow {
        option_id: "once".to_string(),
    }
}

fn asking_ids(row: &crate::workspace::flow_rows::FlowRunRow) -> Vec<u64> {
    row.asking.iter().map(|ask| ask.ask_id).collect()
}

#[gpui::test]
async fn a_pending_flow_question_marks_the_orchestrator_chip(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let unrelated = crate::test_support::workspace_for_control(cx);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.update(&mut cx, |ws, _| ws.active);
    let run_dir = lane.path().join("run");
    seed_run(&ws, &mut cx, here, &run_dir);
    let reply = park_question(&ws, &mut cx, here, &run_dir, 1);
    ws.update(&mut cx, |ws, cx| {
        assert_eq!(
            ws.orchestrator_chip_state(cx),
            Some(crate::workspace::status_bar::orchestrator_chip::OrchestratorChipState::AwaitingPermission)
        );
    });
    unrelated.workspace.update(&mut cx, |ws, cx| {
        assert_eq!(
            ws.orchestrator_chip_state(cx),
            Some(crate::workspace::status_bar::orchestrator_chip::OrchestratorChipState::AwaitingPermission)
        );
    });
    drop(reply);
    ws.update(&mut cx, |ws, cx| {
        assert_ne!(
            ws.orchestrator_chip_state(cx),
            Some(crate::workspace::status_bar::orchestrator_chip::OrchestratorChipState::AwaitingPermission)
        );
    });
}

#[gpui::test]
async fn stale_desktop_answers_and_modal_stop_do_not_touch_a_replacement_run(
    cx: &mut TestAppContext,
) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.read_with(&cx, |ws, _| ws.active);
    let old_run = lane.path().join("old");
    seed_run(&ws, &mut cx, here, &old_run);
    let _old = park_question(&ws, &mut cx, here, &old_run, 1);
    let stale_modal = ws.read_with(&cx, |ws, _| {
        ws.flows.ask_modal.as_ref().unwrap().upgrade().unwrap()
    });
    let new_run = lane.path().join("new");
    seed_run(&ws, &mut cx, here, &new_run);
    let answer = park_question(&ws, &mut cx, here, &new_run, 1);
    let (modal_lane, modal_run, ask_id) = stale_modal.read_with(&cx, |modal, _| {
        let (lane, run_dir, ask_id) = modal.question();
        (lane, run_dir.to_path_buf(), ask_id)
    });
    ws.update(&mut cx, |ws, cx| {
        ws.answer_flow_ask(modal_lane, &modal_run, ask_id, allow(), cx);
        ws.answer_flow_ask(here, &old_run, 1, allow(), cx);
        ws.stop_flow_ask(modal_lane, &modal_run, ask_id, cx);
        assert!(!ws.flow_run_in(here, cx).unwrap().is_stopping());
    });
    assert!(answer.is_empty());
    assert!(
        row_of(&ws, &mut cx, here)
            .unwrap()
            .asking
            .iter()
            .any(|ask| ask.ask_id == 1)
    );
    seed_run(&ws, &mut cx, here, &old_run);
    let resumed = park_question(&ws, &mut cx, here, &old_run, 2);
    ws.update(&mut cx, |ws, cx| {
        ws.stop_flow_ask(modal_lane, &modal_run, ask_id, cx);
        assert!(!ws.flow_run_in(here, cx).unwrap().is_stopping());
    });
    assert!(resumed.is_empty());
}

#[gpui::test]
async fn a_stale_stop_prompt_does_not_cancel_a_replacement_run(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let here = ws.read_with(cx, |ws, _| ws.active);
    let old_run = lane.path().join("old");
    let new_run = lane.path().join("new");
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.seed_flow_run_for_test(here, old_run, cx);
            ws.open_flow_picker(
                here,
                crate::workspace::command::flow_picker::FlowPurpose::Run,
                cx,
            );
            assert!(matches!(
                ws.overlays.flow_picker,
                crate::workspace::command::flow_picker::FlowPicker::Stopping { .. }
            ));
            ws.seed_flow_run_for_test(here, new_run.clone(), cx);
            ws.execute_flow_picker_selection(window, cx);
            let run = ws.flow_run_in(here, cx).unwrap();
            assert_eq!(run.view().run_dir(), new_run);
            assert!(
                !run.is_stopping(),
                "a stale stop prompt canceled the replacement"
            );
            assert!(!ws.overlays.flow_picker.is_open());
            ws.open_flow_picker(
                here,
                crate::workspace::command::flow_picker::FlowPurpose::Run,
                cx,
            );
            ws.execute_flow_picker_selection(window, cx);
            assert!(ws.flow_run_in(here, cx).unwrap().is_stopping());
        });
    })
    .expect("the test window is live");
}

/// A question has to survive being painted more than once, and an answer
/// has to name the question it is answering: a surface can still be showing
/// a resolved question, and that click must do nothing rather than answer
/// whatever came next.
#[gpui::test]
async fn an_answer_only_lands_on_the_question_it_names(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.update(&mut cx, |ws, _| ws.active);
    let run_dir = lane.path().join("run");
    seed_run(&ws, &mut cx, here, &run_dir);
    let reply = park_question(&ws, &mut cx, here, &run_dir, 7);

    // The panel projects the question, and the projection is what a click
    // quotes back.
    let row = row_of(&ws, &mut cx, here).expect("a row for the parked run");
    assert_eq!(asking_ids(&row), [7]);
    assert_eq!(row.asking[0].tool.as_ref(), "Bash");

    // A stale click — right lane, wrong question.
    ws.update(&mut cx, |ws, cx| {
        ws.answer_flow_ask(here, &run_dir, 6, allow(), cx)
    });
    assert!(
        reply.try_recv().is_err(),
        "a click on a resolved question answered the live one"
    );

    ws.update(&mut cx, |ws, cx| {
        ws.answer_flow_ask(here, &run_dir, 7, allow(), cx)
    });
    assert!(
        matches!(
            reply.try_recv(),
            Ok(daruda_acp::PermissionDecision::Allow { .. })
        ),
        "the answer never reached the run"
    );

    // And the question is gone at once. The agent goes back to work for as
    // long as it likes, so waiting for the run's next event leaves the
    // buttons up with no sign the click did anything.
    let row = row_of(&ws, &mut cx, here).expect("the run is still there");
    assert!(
        row.asking.is_empty(),
        "the answered question stayed on screen"
    );
}

/// Dismissing the modal is not an answer.
///
/// The question lives on the run, and the modal is one of two views onto
/// it — so closing it must leave the question in the panel, where it can
/// wait as long as it likes.
///
/// What the dialog *looks* like is checked with
/// `--screenshot-scenario flow-asking`; that it is up is asserted here.
#[gpui::test]
async fn closing_the_modal_leaves_the_question_standing(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.update(&mut cx, |ws, _| ws.active);
    let run_dir = lane.path().join("run");
    seed_run(&ws, &mut cx, here, &run_dir);
    let reply = park_question(&ws, &mut cx, here, &run_dir, 3);

    assert!(
        has_dialog(&mut cx),
        "a question in the lane in view did not reach the front"
    );
    // What Escape does: closes the dialog and nothing else.
    cx.update(crate::ui::WindowExt::close_dialog);

    let row = row_of(&ws, &mut cx, here).expect("the run is still there");
    assert_eq!(
        asking_ids(&row),
        [3],
        "dismissing the modal took the question with it"
    );
    assert!(
        reply.try_recv().is_err(),
        "dismissing the modal answered on the user's behalf"
    );
}

/// A run in another lane must not take the window away from what someone
/// is doing. The question still parks — the chip and the panel are how it
/// is reachable — but nothing is raised over the top of the lane in view.
#[gpui::test]
async fn a_question_from_a_lane_out_of_view_does_not_take_the_window(cx: &mut TestAppContext) {
    let (_lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let (other_dir, elsewhere) = add_lane_with_a_flow(&ws, wh, cx, "deploy.yaml", ONE_AGENT);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    assert_ne!(elsewhere, ws.update(&mut cx, |ws, _| ws.active));
    let run_dir = other_dir.path().join("run");
    seed_run(&ws, &mut cx, elsewhere, &run_dir);
    let _reply = park_question(&ws, &mut cx, elsewhere, &run_dir, 4);

    assert!(
        !has_dialog(&mut cx),
        "a run in another lane took the window"
    );
    // And it is still a question somebody can answer: the chip is
    // workspace-wide, so it carries the lanes out of view.
    let row = row_of(&ws, &mut cx, elsewhere).expect("the run is listed");
    assert_eq!(
        asking_ids(&row),
        [4],
        "the parked question is not reachable anywhere"
    );
}

/// A channel drains before it closes, so the last question a stopping run
/// queued can arrive after the next run in that lane has taken the slot.
/// `ask_id` restarts at 1 per run, so without the run it came from being
/// checked it lands on the newcomer looking exactly like its own.
#[gpui::test]
async fn a_question_left_over_from_the_previous_run_is_not_shown_as_this_one_s(
    cx: &mut TestAppContext,
) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.update(&mut cx, |ws, _| ws.active);
    seed_run(&ws, &mut cx, here, &lane.path().join("runs/2"));
    let reply = park_question(&ws, &mut cx, here, &lane.path().join("runs/1"), 1);

    let row = row_of(&ws, &mut cx, here).expect("the run is still there");
    assert!(
        row.asking.is_empty(),
        "a question from the run before landed on this one"
    );
    assert!(
        !has_dialog(&mut cx),
        "a question from the run before took the window"
    );
    assert!(reply.is_closed(), "the old run's engine is released");
}

/// Stop is the other way out of a question. Stopping settles every question
/// on its way out — a stopped run has none anyone should still be answering.
#[gpui::test]
async fn stopping_a_run_takes_its_questions_down_with_it(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.update(&mut cx, |ws, _| ws.active);
    let run_dir = lane.path().join("run");
    seed_run(&ws, &mut cx, here, &run_dir);
    ws.update(&mut cx, |ws, cx| {
        ws.apply_flow_event_for_test(
            here,
            &daruda_flow::event::FlowEvent::NodeStarted {
                node: "design".into(),
                attempt: 2,
            },
            cx,
        )
    });
    let _first = park_question(&ws, &mut cx, here, &run_dir, 9);
    let _second = park_question(&ws, &mut cx, here, &run_dir, 10);

    ws.update(&mut cx, |ws, cx| ws.stop_flow_run_in(here, cx));

    let row = row_of(&ws, &mut cx, here).expect("the run is listed until it ends");
    assert!(
        row.asking.is_empty(),
        "a stopped run still offers its questions to answer"
    );
    // And it says what it was doing, not a blank stage.
    assert!(row.doing.contains("design"), "{}", row.doing);
}

/// Nodes running together ask independently, so a second question can
/// arrive while the first is up. Both stay up, each with its own answers.
#[gpui::test]
async fn a_second_question_stands_beside_the_first(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.update(&mut cx, |ws, _| ws.active);
    let run_dir = lane.path().join("run");
    seed_run(&ws, &mut cx, here, &run_dir);

    let first = park_question(&ws, &mut cx, here, &run_dir, 1);
    let _second = park_question(&ws, &mut cx, here, &run_dir, 2);

    let row = row_of(&ws, &mut cx, here).expect("the run is listed");
    assert_eq!(asking_ids(&row), [1, 2], "a question was dropped");
    assert!(
        first.try_recv().is_err(),
        "the first question was answered by the second one arriving"
    );
}

/// Answering one leaves the others up. Otherwise the run reads as working
/// while a question nobody can see is still holding a node.
#[gpui::test]
async fn answering_one_question_leaves_the_others_up(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.update(&mut cx, |ws, _| ws.active);
    let run_dir = lane.path().join("run");
    seed_run(&ws, &mut cx, here, &run_dir);
    let _first = park_question(&ws, &mut cx, here, &run_dir, 1);
    let _second = park_question(&ws, &mut cx, here, &run_dir, 2);

    ws.update(&mut cx, |ws, cx| {
        ws.answer_flow_ask(here, &run_dir, 1, allow(), cx)
    });
    let row = row_of(&ws, &mut cx, here).expect("the run is listed");
    assert_eq!(
        asking_ids(&row),
        [2],
        "answering hid a question that is still waiting"
    );
}

/// An event from a sibling node leaves the question standing; taking it
/// down would leave the asking node parked on a reply nobody could send.
#[gpui::test]
async fn a_sibling_event_never_drops_an_open_question(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.update(&mut cx, |ws, _| ws.active);
    let run_dir = lane.path().join("run");
    seed_run(&ws, &mut cx, here, &run_dir);
    let reply = park_question(&ws, &mut cx, here, &run_dir, 1);

    for event in [
        daruda_flow::event::FlowEvent::NodeStarted {
            node: "sibling".into(),
            attempt: 1,
        },
        daruda_flow::event::FlowEvent::NodePassed {
            node: "sibling".into(),
            attempt: 1,
        },
    ] {
        ws.update(&mut cx, |ws, cx| {
            ws.apply_flow_event_for_test(here, &event, cx)
        });
    }

    let row = row_of(&ws, &mut cx, here).expect("the run is listed");
    assert_eq!(asking_ids(&row), [1], "a sibling's progress took it down");
    ws.update(&mut cx, |ws, cx| {
        ws.answer_flow_ask(here, &run_dir, 1, allow(), cx)
    });
    assert!(
        reply.try_recv().is_ok(),
        "the question could not be answered"
    );
}

/// A question the engine already abandoned — its node was stopped or
/// timed out — cannot take an answer, and the click says so rather than
/// looking like it landed.
#[gpui::test]
async fn answering_an_abandoned_question_reports_failure(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.update(&mut cx, |ws, _| ws.active);
    let run_dir = lane.path().join("run");
    seed_run(&ws, &mut cx, here, &run_dir);
    let reply = park_question(&ws, &mut cx, here, &run_dir, 5);
    // The engine dropping its end is what abandoning a question is.
    drop(reply);

    ws.update(&mut cx, |ws, cx| {
        ws.answer_flow_ask(here, &run_dir, 5, allow(), cx)
    });

    let told = ws.read_with(&cx, |ws, _| {
        ws.error_history()
            .iter()
            .any(|report| report.title == crate::surface::strings::flow::ask_closed())
    });
    assert!(told, "an answer nobody received looked like a success");
    let row = row_of(&ws, &mut cx, here).expect("the run is listed");
    assert!(row.asking.is_empty(), "a closed question stayed up");
}

/// A question that arrives after the last one was answered raises the
/// modal, same as the first did.
#[gpui::test]
async fn a_question_arriving_after_the_last_was_answered_still_raises_the_modal(
    cx: &mut TestAppContext,
) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.update(&mut cx, |ws, _| ws.active);
    let run_dir = lane.path().join("run");
    seed_run(&ws, &mut cx, here, &run_dir);

    let _first = park_question(&ws, &mut cx, here, &run_dir, 1);
    assert!(
        has_dialog(&mut cx),
        "the first question did not reach the front"
    );
    ws.update(&mut cx, |ws, cx| {
        ws.answer_flow_ask(here, &run_dir, 1, allow(), cx)
    });
    cx.update(crate::ui::WindowExt::close_dialog);

    let _second = park_question(&ws, &mut cx, here, &run_dir, 2);
    assert!(
        has_dialog(&mut cx),
        "the next question never reached the front"
    );
}

/// A question that arrives *while* one is up must not raise a second
/// modal: a dialog stacked over the one being answered would take the
/// click meant for it.
#[gpui::test]
async fn a_question_beside_another_does_not_stack_a_second_modal(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let mut cx = gpui::VisualTestContext::from_window(wh.into(), cx);
    let here = ws.update(&mut cx, |ws, _| ws.active);
    let run_dir = lane.path().join("run");
    seed_run(&ws, &mut cx, here, &run_dir);

    let _first = park_question(&ws, &mut cx, here, &run_dir, 1);
    let _second = park_question(&ws, &mut cx, here, &run_dir, 2);

    // One close is enough to clear the front dialog. A second one behind
    // it would still be up.
    cx.update(crate::ui::WindowExt::close_dialog);
    assert!(
        !has_dialog(&mut cx),
        "a second question stacked a modal of its own"
    );
}

/// Two windows on one worktree, each showing the same run's question.
struct TwoWindows {
    _lane: tempfile::TempDir,
    first: gpui::Entity<Workspace>,
    first_wh: gpui::WindowHandle<gpui_component::Root>,
    second: gpui::Entity<Workspace>,
    second_wh: gpui::WindowHandle<gpui_component::Root>,
    _reply: smol::channel::Receiver<daruda_acp::PermissionDecision>,
}

fn two_windows_asked(cx: &mut TestAppContext) -> TwoWindows {
    let (lane, first, _flow_path, first_wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let built = build_workspace_with(
        cx,
        &daruda_config::Config::default(),
        Some(daruda_store::project::Project::from_path(lane.path())),
    );
    let (second_wh, second) = registered(cx, built);
    let run_dir = lane.path().join("run");
    let here = first.read_with(cx, |ws, _| ws.active);
    let seeded = run_dir.clone();
    first.update(cx, |ws, cx| ws.seed_flow_run_for_test(here, seeded, cx));
    let (reply, rx) = smol::channel::bounded(1);
    let pending = question(1, "design", reply);
    first.update(cx, |ws, cx| {
        ws.park_flow_ask_for_test(here, &run_dir, pending, cx)
    });
    cx.run_until_parked();
    let windows = TwoWindows {
        _lane: lane,
        first,
        first_wh,
        second,
        second_wh,
        _reply: rx,
    };
    assert!(dialog_up(cx, windows.first_wh), "no question in the first");
    assert!(
        dialog_up(cx, windows.second_wh),
        "no question in the second"
    );
    windows
}

fn dialog_up(cx: &mut TestAppContext, wh: gpui::WindowHandle<gpui_component::Root>) -> bool {
    cx.update_window(wh.into(), |_, window, cx| {
        crate::ui::WindowExt::has_active_dialog(window, cx)
    })
    .expect("window is live")
}

/// A question answered in one window comes down in the other, rather than
/// leaving buttons there that answer nothing.
#[gpui::test]
async fn a_question_answered_in_one_window_closes_in_the_other(cx: &mut TestAppContext) {
    let w = two_windows_asked(cx);
    let here = w.first.read_with(cx, |ws, _| ws.active);
    w.first.update(cx, |ws, cx| {
        ws.answer_flow_ask(here, &w._lane.path().join("run"), 1, allow(), cx)
    });
    cx.run_until_parked();
    assert!(
        !dialog_up(cx, w.second_wh),
        "the other window kept a question already answered"
    );
}

/// Stopping the run from one window takes its question down in every one.
#[gpui::test]
async fn a_run_stopped_in_one_window_closes_its_question_in_the_other(cx: &mut TestAppContext) {
    let w = two_windows_asked(cx);
    let here = w.first.read_with(cx, |ws, _| ws.active);
    w.first.update(cx, |ws, cx| ws.stop_flow_run_in(here, cx));
    cx.run_until_parked();
    assert!(!dialog_up(cx, w.first_wh), "the stopping window kept it");
    assert!(!dialog_up(cx, w.second_wh), "the other window kept it");
}

/// A run that ends leaves no question up anywhere.
#[gpui::test]
async fn a_run_that_ends_closes_its_question_in_every_window(cx: &mut TestAppContext) {
    let w = two_windows_asked(cx);
    let here = w.second.read_with(cx, |ws, _| ws.active);
    w.second.update(cx, |ws, cx| {
        ws.apply_flow_event_for_test(
            here,
            &daruda_flow::event::FlowEvent::RunEnded {
                end: daruda_flow::event::RunEnd::Canceled { node: None },
            },
            cx,
        )
    });
    cx.run_until_parked();
    assert!(!dialog_up(cx, w.first_wh), "the first window kept it");
    assert!(!dialog_up(cx, w.second_wh), "the second window kept it");
}

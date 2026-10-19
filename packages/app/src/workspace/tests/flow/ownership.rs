//! Which run a window reaches, and which window speaks for it: one folder
//! under two spellings is one run, a worktree holds one run at a time, and
//! a run's end is said even where no window holds its worktree.

use super::*;

/// Long enough that only a stop ends it within a test's patience.
const LONG_COMMAND: &str = "\
version: 1
nodes:
  - id: wait
    kind: command
    run: \"sleep 30\"
";

const COMMAND_ONLY: &str = "\
version: 1
nodes:
  - id: check
    kind: command
    run: \"true\"
";

fn allow() -> daruda_acp::PermissionDecision {
    daruda_acp::PermissionDecision::Allow {
        option_id: "once".to_string(),
    }
}

/// The run `ws` sees in its active lane: its directory, and whether it was
/// told to stop.
fn seen(
    ws: &gpui::Entity<Workspace>,
    cx: &mut TestAppContext,
) -> Option<(std::path::PathBuf, bool)> {
    ws.read_with(cx, |ws, cx| {
        let run = ws.flow_run_in(ws.active, cx)?;
        Some((run.view().run_dir().to_path_buf(), run.is_stopping()))
    })
}

/// Two windows that opened one folder under two spellings — one of them
/// through a symlinked prefix — hold one run: each sees it, either answers
/// its question, and either stops it.
#[cfg(unix)]
#[gpui::test]
async fn windows_spelling_one_worktree_two_ways_share_its_run(cx: &mut TestAppContext) {
    let (lane, first, flow_path, first_wh) = workspace_with_a_flow(cx, LONG_COMMAND);
    let links = tempfile::tempdir().expect("tempdir");
    let link = links.path().join("lane");
    std::os::unix::fs::symlink(lane.path(), &link).expect("symlink");
    let config = daruda_config::Config::default();
    let built = build_workspace_with(
        cx,
        &config,
        Some(daruda_store::project::Project::from_path(&link)),
    );
    let (_second_wh, second) = registered(cx, built);
    cx.run_until_parked();
    let spelled = second.read_with(cx, |ws, _| {
        ws.lane_for(ws.active).expect("a lane").path.clone()
    });
    assert_eq!(spelled, link, "the second window resolved the link itself");
    // A real run: its log thread wakes the app from outside the scheduler.
    cx.executor().allow_parking();

    cx.update_window(first_wh.into(), |_, window, cx| {
        first.update(cx, |ws, cx| {
            assert!(ws.run_flow_at(
                ws.active,
                &flow_path,
                crate::workspace::command::flow_picker::FlowPurpose::Run,
                crate::workspace::flow_request::FlowSelection::default(),
                window,
                cx,
            ));
        });
    })
    .expect("window is live");
    cx.run_until_parked();

    let (run_dir, _) = seen(&first, cx).expect("the starting window sees its run");
    assert_eq!(
        seen(&second, cx).map(|(dir, _)| dir),
        Some(run_dir.clone()),
        "the window with the other spelling does not see the run"
    );
    let rows = second.read_with(cx, |ws, cx| ws.flow_rows_for_active_lane(cx));
    assert_eq!(rows.len(), 1, "no row for the run in the other window");

    // A question parked through the first spelling is answered through the
    // second.
    let here = first.read_with(cx, |ws, _| ws.active);
    let (reply, answered) = smol::channel::bounded(1);
    let pending = question(1, "wait", reply);
    first.update(cx, |ws, cx| {
        ws.park_flow_ask_for_test(here, &run_dir, pending, cx)
    });
    let there = second.read_with(cx, |ws, _| ws.active);
    second.update(cx, |ws, cx| ws.answer_flow_ask(there, 1, allow(), cx));
    assert!(
        matches!(
            answered.try_recv(),
            Ok(daruda_acp::PermissionDecision::Allow { .. })
        ),
        "the answer from the other spelling never reached the run"
    );

    second.update(cx, |ws, cx| ws.stop_flow_run_in(there, cx));
    assert_eq!(
        seen(&first, cx).map(|(_, stopping)| stopping),
        Some(true),
        "the stop from the other spelling did not reach the run"
    );
    assert!(
        wait_for_run(cx, |cx| seen(&first, cx).is_none()
            && seen(&second, cx).is_none()),
        "the run did not end"
    );
}

/// A run this process holds blocks a second start in its worktree even
/// before its thread takes the lock: the host is asked first, and the
/// answer is the offer to stop it — never a run started over it.
#[gpui::test]
async fn a_held_run_blocks_a_second_start_before_its_lock_exists(cx: &mut TestAppContext) {
    let (lane, ws, flow_path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let run_dir = crate::workspace::flow_paths::runs_dir(lane.path()).join("first");

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let here = ws.active;
            ws.seed_flow_run_for_test(here, run_dir.clone(), cx);
            assert!(
                !ws.run_flow_at(
                    here,
                    &flow_path,
                    crate::workspace::command::flow_picker::FlowPurpose::Run,
                    crate::workspace::flow_request::FlowSelection::default(),
                    window,
                    cx,
                ),
                "a second run was let through"
            );
            assert!(matches!(
                ws.overlays.flow_picker,
                crate::workspace::command::flow_picker::FlowPicker::Stopping { .. }
            ));
        });
    })
    .expect("window is live");
    assert_eq!(seen(&ws, cx).map(|(dir, _)| dir), Some(run_dir));
}

/// The picker's guard ran when it opened; a run started in the worktree
/// after that is not replaced by the pick. The start is refused and offers
/// to stop the run that is there.
#[gpui::test]
async fn a_run_started_while_the_picker_was_open_is_not_replaced(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let run_dir = crate::workspace::flow_paths::runs_dir(lane.path()).join("elsewhere");
    ws.update(cx, |ws, cx| {
        ws.open_flow_picker(
            ws.active,
            crate::workspace::command::flow_picker::FlowPurpose::Run,
            cx,
        );
    });
    // Another window's start, landing while this picker is up.
    let here = ws.read_with(cx, |ws, _| ws.active);
    ws.update(cx, |ws, cx| {
        ws.seed_flow_run_for_test(here, run_dir.clone(), cx)
    });

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.pick_flow_row(0, window, cx));
    })
    .expect("window is live");

    assert_eq!(
        seen(&ws, cx).map(|(dir, _)| dir),
        Some(run_dir),
        "the pick replaced the run already going"
    );
    ws.read_with(cx, |ws, _| {
        assert!(matches!(
            ws.overlays.flow_picker,
            crate::workspace::command::flow_picker::FlowPicker::Stopping { .. }
        ));
    });
}

/// A run whose worktree no window holds any more still says how it ended:
/// the window that speaks for it toasts the outcome, not only refusals.
#[gpui::test]
async fn a_run_no_window_holds_still_toasts_its_end(cx: &mut TestAppContext) {
    let (_lane, ws, _flow_path, _wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let gone = tempfile::tempdir().expect("tempdir");
    let run_dir = gone.path().join("run");
    std::fs::create_dir_all(&run_dir).expect("run dir");
    std::fs::write(run_dir.join(daruda_flow::record::RUN_REPORT_FILE), "# Run").expect("report");
    let source = crate::runs::FlowSource::Resumed {
        run_dir: run_dir.clone(),
    };
    cx.update(|cx| {
        crate::runs::RunsHost::seed_flow_run(
            gone.path().to_path_buf(),
            crate::runs::RunView::new(run_dir.clone(), source, Vec::new()),
            cx,
        );
    });
    cx.run_until_parked();

    cx.update(|cx| {
        crate::runs::RunsHost::apply_flow_event_for_test(
            gone.path(),
            &daruda_flow::event::FlowEvent::RunEnded {
                end: daruda_flow::event::RunEnd::Done,
            },
            cx,
        );
    });
    cx.run_until_parked();

    let toasted = ws.read_with(cx, |ws, _| {
        ws.error_history()
            .iter()
            .any(|r| r.title == crate::surface::strings::flow::run_finished_toast())
    });
    assert!(toasted, "the end of a run no window holds went unsaid");
}

/// A window can hold one folder under two lanes — a worktree of one project
/// opened as another project's root. Its run is still one row, its question
/// counted once, and the lane in view is the one that answers for it.
#[gpui::test]
async fn a_folder_held_twice_in_one_window_lists_its_run_once(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, _wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    let (here, twin) = ws.update(cx, |ws, _| {
        let here = ws.active;
        let lane_id = ws.alloc_id();
        let mut twin = crate::lane::Lane::default_for_project(lane_id, lane.path().to_path_buf());
        twin.tab_order = 1;
        ws.project_for_mut(here.project)
            .expect("the fixture's project")
            .lanes
            .push(twin);
        let twin = daruda_store::project::LaneRef {
            project: here.project,
            lane: lane_id,
        };
        (here, twin)
    });
    let run_dir = lane.path().join("run");
    ws.update(cx, |ws, cx| {
        ws.seed_flow_run_for_test(here, run_dir.clone(), cx)
    });
    let (reply, _answered) = smol::channel::bounded(1);
    let pending = question(1, "check", reply);
    ws.update(cx, |ws, cx| {
        ws.park_flow_ask_for_test(twin, &run_dir, pending, cx)
    });
    cx.run_until_parked();

    let rows = ws.read_with(cx, |ws, cx| ws.flow_status_rows(cx));
    assert_eq!(rows.len(), 1, "the run was listed once per lane: {rows:?}");
    assert_eq!(rows[0].lane, here, "the row named a lane out of view");
    assert_eq!(rows[0].asking.len(), 1);
    let tree = daruda_flow::lock::CanonicalTree::resolve(lane.path()).expect("resolves");
    ws.read_with(cx, |ws, cx| {
        assert_eq!(ws.lanes_at(&tree, cx), vec![here, twin]);
    });
}

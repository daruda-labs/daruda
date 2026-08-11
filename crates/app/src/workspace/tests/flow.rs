//! What the app hands `daruda_flow` when a user picks a flow.
//!
//! The unit tests next to `flow_request.rs` cover the pure pieces; these
//! build a real workspace, because the failure this guards against is the
//! one the engine already caught once at runtime — a request assembled from
//! live workspace state whose paths were not what the engine requires.

use super::*;

/// A workspace whose active lane is a real directory holding one flow.
fn workspace_with_a_flow(
    cx: &mut TestAppContext,
    flow: &str,
) -> (
    tempfile::TempDir,
    gpui::Entity<Workspace>,
    std::path::PathBuf,
    gpui::WindowHandle<gpui_component::Root>,
) {
    let lane = tempfile::tempdir().expect("tempdir");
    let flows = crate::workspace::flow_paths::flows_dir(lane.path());
    std::fs::create_dir_all(&flows).expect("create flows dir");
    let flow_path = flows.join("ship.yaml");
    std::fs::write(&flow_path, flow).expect("write flow");

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(lane.path());
    let (wh, ws) = build_workspace_with(cx, &config, Some(project));
    (lane, ws, flow_path, wh)
}

const ONE_AGENT: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write a line
";

/// The engine refuses a relative path *by name*, and it does so after
/// taking the lock and writing the run directory is no longer possible —
/// but only because it now checks first. A request the app builds must
/// never be one of those in the first place.
#[gpui::test]
async fn every_path_the_app_puts_in_a_request_is_absolute(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);

    let (lane_ref, issues) = ws.update(cx, |ws, cx| {
        let submission = ws
            .build_flow_request(&flow_path, None, cx)
            .unwrap_or_else(|_| panic!("a local lane with a valid flow builds a request"));
        (
            submission.lane,
            daruda_flow::request::validate_request(&submission.request),
        )
    });
    assert_eq!(lane_ref, ws.update(cx, |ws, _cx| ws.active_ref()));

    let relative: Vec<_> = issues
        .iter()
        .filter(|i| {
            matches!(
                i.kind,
                daruda_flow::error::ValidationKind::RelativeRequestPath { .. }
            )
        })
        .collect();
    assert!(relative.is_empty(), "{relative:?}");
}

/// Everything about one request the app built that the engine would
/// otherwise have to discover at runtime. One workspace rather than three,
/// because building it is what costs — the assertions are free.
///
/// The runtime must not land in the user's working tree: the `.gitignore`
/// the engine writes reaches `flow-runs/` only, so a runtime unpacked
/// beside it turns up in `git status`. Found the hard way with a real
/// agent in P2c. And design §6's ceilings are only a last line of defence
/// if they are actually installed — a `Budget::unlimited()` slipping in
/// here is invisible until a flow runs all night.
#[gpui::test]
async fn a_submitted_request_is_whole(cx: &mut TestAppContext) {
    let (lane, ws, flow_path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);

    let request = ws
        .update(cx, |ws, cx| ws.build_flow_request(&flow_path, None, cx))
        .expect("a local lane with a valid flow builds a request")
        .request;

    assert!(
        !request.node_install_dir.starts_with(lane.path()),
        "the managed runtime is inside the lane: {:?}",
        request.node_install_dir
    );
    assert!(
        request.run_dir.starts_with(lane.path()),
        "{:?}",
        request.run_dir
    );
    assert!(request.budget.deadline.is_some(), "no wall-clock ceiling");
    assert_eq!(
        request.budget.max_node_runs,
        Some(daruda_config::flow::DEFAULT_MAX_NODE_RUNS)
    );
}

/// A flow that does not load is refused before anything is taken — no run
/// directory, and nothing in the lane to clean up afterwards.
#[gpui::test]
async fn a_flow_that_does_not_load_leaves_nothing_behind(cx: &mut TestAppContext) {
    let (lane, ws, flow_path, _wh) = workspace_with_a_flow(
        cx,
        "version: 1\nnodes:\n  - id: a\n    kind: command\n    deps: [ghost]\n    run: \"true\"\n",
    );

    let refused = ws.update(cx, |ws, cx| {
        ws.build_flow_request(&flow_path, None, cx).is_err()
    });
    assert!(refused, "a node depending on nothing should not run");
    assert!(
        !crate::workspace::flow_paths::runs_dir(lane.path()).exists(),
        "a refused submission created a run directory"
    );
}

/// The picker lists the lane's flows, and only its flows. This is the one
/// step between the palette entry and everything above it.
#[gpui::test]
async fn the_picker_offers_the_flows_in_the_active_lane(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);
    std::fs::write(
        crate::workspace::flow_paths::flows_dir(lane.path()).join("notes.md"),
        "not a flow",
    )
    .expect("write");

    let labels = ws.update(cx, |ws, cx| {
        ws.open_flow_picker(
            crate::workspace::command::flow_picker::FlowPurpose::Validate,
            cx,
        );
        ws.flow_picker
            .choosing()
            .map(|c| {
                c.filtered()
                    .into_iter()
                    .filter_map(|i| c.stage.row(i))
                    .map(|r| r.label.to_string())
                    .collect()
            })
            .unwrap_or_else(Vec::new)
    });
    assert_eq!(labels, vec!["ship.yaml".to_string()]);
}

/// The rules that need the request's own context — here, a node naming an
/// agent the catalog does not have.
///
/// The engine catches this too, and refuses *by returning*: a run that
/// never started emits neither `RunStarted` nor `RunEnded`, so a host that
/// only watched the stream would leave the user staring at a palette that
/// did nothing. Submission has to check it as well.
#[gpui::test]
async fn a_node_naming_an_agent_the_catalog_lacks_is_refused_at_submission(
    cx: &mut TestAppContext,
) {
    let (_lane, ws, flow_path, _wh) = workspace_with_a_flow(
        cx,
        "\
version: 1
nodes:
  - id: design
    kind: agent
    agent:
      id: nonesuch
      mode: bypassPermissions
    output: design.md
    prompt: write a line
",
    );

    let refused = ws.update(cx, |ws, cx| {
        match ws.build_flow_request(&flow_path, None, cx) {
            Err(crate::workspace::flow_request::FlowSubmitError::Invalid(issues)) => issues,
            Err(_) => panic!("refused, but not for the reason under test"),
            Ok(_) => panic!("a flow naming an unconfigured agent was accepted"),
        }
    });
    assert!(
        refused.iter().any(|i| matches!(
            i.kind,
            daruda_flow::error::ValidationKind::UnknownAgent { .. }
        )),
        "{refused:?}"
    );
}

/// Design §14 derives "a run is going" from the lock so a run this app did
/// not start is still recognised. But the stop switch is a `CancelToken`
/// this process holds — there is no way to reach another process's. The
/// picker must not offer a button that cannot work.
#[gpui::test]
async fn a_run_owned_by_another_process_is_not_offered_a_stop_button(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let runs = crate::workspace::flow_paths::runs_dir(lane.path());
    std::fs::create_dir_all(&runs).expect("create runs dir");
    // pid 1 is alive on every unix and is emphatically not this process.
    std::fs::write(
        runs.join(".lock"),
        "pid: 1\nrun_id: someone-elses\nstarted_unix_secs: 1\n",
    )
    .expect("plant a lock");

    let state = ws.update(cx, |ws, cx| {
        ws.open_flow_picker(crate::workspace::command::flow_picker::FlowPurpose::Run, cx);
        format!("{:?}", ws.flow_picker)
    });
    assert!(
        state.starts_with("Closed"),
        "offered to stop a run it cannot reach: {state}"
    );
}

/// The whole point of the app is lanes running in parallel, so two flows
/// can be in flight at once. A single run handle meant the second submit
/// displaced the first one's cancel token, and a run ending then settled —
/// and opened the report of — whichever lane happened to be active.
#[gpui::test]
async fn a_run_ending_settles_its_own_lane_and_leaves_the_others_alone(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let dir_here = lane.path().join("run-here");
    let dir_elsewhere = lane.path().join("run-elsewhere");
    for dir in [&dir_here, &dir_elsewhere] {
        std::fs::create_dir_all(dir).expect("run dir");
        std::fs::write(dir.join(daruda_flow::record::RUN_REPORT_FILE), "# Run").expect("report");
    }

    ws.update(cx, |ws, cx| {
        let here = ws.active;
        let elsewhere = daruda_store::project::LaneRef {
            project: here.project,
            lane: here.lane + 1,
        };
        ws.seed_flow_run_for_test(here, dir_here.clone());
        ws.seed_flow_run_for_test(elsewhere, dir_elsewhere.clone());

        // The run in the *other* lane ends while this one is active.
        let report = ws.settle_flow_run(elsewhere, &daruda_flow::event::RunEnd::Done, cx);

        assert_eq!(
            report.as_deref(),
            Some(
                dir_elsewhere
                    .join(daruda_flow::record::RUN_REPORT_FILE)
                    .as_path()
            ),
            "opened the active lane's report instead of the one that ended"
        );
        assert!(
            ws.flow_runs.contains_key(&here),
            "settling one lane dropped another lane's cancel token"
        );
        assert!(!ws.flow_runs.contains_key(&elsewhere));
    });
}

/// The chip spans lanes and the panel does not — that split is the whole
/// reason both exist. A panel showing another lane's run would offer to
/// answer a question beside a run history that has nothing to do with it.
#[gpui::test]
async fn the_panel_lists_only_the_active_lane_while_the_chip_lists_every_one(
    cx: &mut TestAppContext,
) {
    let (lane, ws, _flow_path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);

    ws.update(cx, |ws, _cx| {
        let here = ws.active;
        let elsewhere = daruda_store::project::LaneRef {
            project: here.project,
            lane: here.lane + 1,
        };
        ws.seed_flow_run_for_test(here, lane.path().join("run-here"));
        ws.seed_flow_run_for_test(elsewhere, lane.path().join("run-elsewhere"));

        let panel: Vec<_> = ws
            .flow_rows_for_active_lane()
            .into_iter()
            .map(|row| row.lane)
            .collect();
        assert_eq!(panel, vec![here], "the panel reached into another lane");

        assert_eq!(
            ws.flow_status_rows().len(),
            2,
            "the chip must still see both — it is the cross-lane surface"
        );
    });
}

/// The right dock repaints only when `content_differs` says something
/// changed, and that method is a hand-written list. A field added without
/// a line there is invisible: the panel simply never updates, with no
/// error and no failing test anywhere else.
#[gpui::test]
async fn a_started_run_makes_the_right_dock_snapshot_differ(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);

    let (before, after) = ws.update(cx, |ws, cx| {
        let before = ws.prepare_right_dock_snapshot(cx);
        let here = ws.active;
        ws.seed_flow_run_for_test(here, lane.path().join("run-here"));
        let after = ws.prepare_right_dock_snapshot(cx);
        (before, after)
    });

    assert!(
        after.content_differs(&before),
        "the panel would show a stale run list"
    );
    assert!(!after.content_differs(&after), "nothing changed");
}

/// A finished run left on disk by an earlier session is the only way a
/// crash is visible at all — v1 computed the status and had no caller, so
/// `kill -9` could only be seen with `ls`.
#[gpui::test]
async fn the_panel_reads_past_runs_off_disk_when_its_tab_is_showing(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let runs = crate::workspace::flow_paths::runs_dir(lane.path());
    // One that said how it ended, and one that never got to — which is
    // exactly what a crash leaves behind.
    let done = runs.join(crate::workspace::flow_request::run_id(
        1_786_000_000_000,
        42,
        1,
    ));
    let crashed = runs.join(crate::workspace::flow_request::run_id(
        1_786_000_001_000,
        42,
        1,
    ));
    for dir in [&done, &crashed] {
        std::fs::create_dir_all(dir).expect("run dir");
    }
    std::fs::write(done.join("DONE"), "").expect("marker");

    ws.update(cx, |ws, _cx| {
        // The tab has to be showing: a lane nobody is looking at must not
        // cost a directory listing.
        assert!(
            ws.flow_history_for_panel().is_none(),
            "read the disk for a tab that is not open"
        );
        ws.right_dock_view = daruda_store::project::RightDockView::Flows;

        let history = ws.flow_history_for_panel().expect("read");
        let statuses: Vec<_> = history.runs().iter().map(|r| r.status).collect();
        assert_eq!(
            statuses,
            vec![
                daruda_flow::marker::RunStatus::Unknown,
                daruda_flow::marker::RunStatus::Done
            ],
            "newest first; the unmarked one is the crash evidence"
        );
    });
}

/// Retention runs *during* start-up — after the run announces itself and
/// before its first node — so the history is re-read when a run leaves
/// `Starting`, and **only** then.
///
/// Both halves matter. Refresh too early (on the announcement) and the
/// pre-sweep listing sticks for the length of the run; refresh on every
/// node and a twenty-node flow lists the directory twenty times, which is
/// the per-frame disk read this cache exists to avoid, only slower to
/// notice.
#[gpui::test]
async fn only_a_run_leaving_setup_refreshes_the_history(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);
    std::fs::create_dir_all(crate::workspace::flow_paths::runs_dir(lane.path())).expect("runs dir");

    ws.update(cx, |ws, cx| {
        let here = ws.active;
        ws.right_dock_view = daruda_store::project::RightDockView::Flows;
        ws.seed_flow_run_for_test(here, lane.path().join("run-here"));
        ws.flow_history_for_panel().expect("primed");

        let started = |node: &str| daruda_flow::event::FlowEvent::NodeStarted {
            node: node.to_string(),
            attempt: 1,
        };

        // Leaving `Starting`: the sweep has happened, so the list is stale.
        ws.apply_flow_event_for_test(here, &started("design"), cx);
        assert!(
            ws.flow_history.is_none(),
            "a swept run would stay on screen for the length of the run"
        );

        ws.flow_history_for_panel().expect("re-read");
        // Every node after the first: the directory has not changed, and
        // re-reading it per node is the cost this cache exists to avoid.
        for node in ["test", "review"] {
            ws.apply_flow_event_for_test(here, &started(node), cx);
            assert!(
                ws.flow_history.is_some(),
                "re-read the directory at node `{node}`"
            );
        }
    });
}

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

/// The panel is lane-scoped, so a question raised in a lane you are not
/// looking at is unanswerable until you get there. The chip is the only
/// surface that spans lanes, and this is the move it exists to offer —
/// all three parts of it: the lane, the dock, and the tab.
#[gpui::test]
async fn revealing_a_run_lands_on_its_lane_with_the_panel_open(cx: &mut TestAppContext) {
    let lane = tempfile::tempdir().expect("tempdir");
    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(lane.path());
    let (wh, ws) = build_workspace_with(cx, &config, Some(project));

    let here = ws.update(cx, |ws, _cx| ws.active);
    ws.update(cx, |ws, _cx| {
        ws.seed_flow_run_for_test(here, lane.path().join("run"));
    });

    wh.update(cx, |_, window, cx| {
        ws.update(cx, |ws, cx| {
            // Start from a closed dock on another tab, so both of the
            // non-lane parts of the move have something to do.
            ws.right_dock.update(cx, |d, _| d.is_open = false);
            ws.set_right_dock_view(daruda_store::project::RightDockView::Usage, cx);

            ws.reveal_flow_run(here, window, cx);

            assert!(
                ws.right_dock.read(cx).is_open,
                "revealed a run behind a closed dock"
            );
            assert_eq!(
                ws.right_dock_view,
                daruda_store::project::RightDockView::Flows,
                "landed on the wrong tab"
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

/// `load` alone is not the whole of stage 1. A `prompt_file` that is not
/// there is only knowable with the request's own context, so checking less
/// here than `Run Flow…` checks means "no problems found" followed by a
/// refusal — the split design §12 exists to prevent.
#[gpui::test]
async fn checking_a_flow_finds_what_only_the_request_can_see(cx: &mut TestAppContext) {
    let (lane, ws, flow_path, _wh) = workspace_with_a_flow(
        cx,
        "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt_file: ./missing.md
",
    );

    let issues = ws.update(cx, |ws, cx| {
        ws.check_flow(&flow_path, None, cx).expect("checked")
    });
    assert!(
        issues.iter().any(|i| matches!(
            i.kind,
            daruda_flow::error::ValidationKind::MissingPromptFile { .. }
        )),
        "{issues:?}"
    );
    // Stage 1 costs nothing and changes nothing.
    assert!(!crate::workspace::flow_paths::runs_dir(lane.path()).exists());
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

/// The panel has a tab, so it needs the rest of the G3 chain: an action,
/// a handler wired to it, and a palette entry that reaches it. Driven
/// through the palette because that is the point furthest from the code
/// under test — a broken link anywhere along the four leaves the entry
/// visible and inert, which is exactly how a user meets it.
#[gpui::test]
async fn the_palette_can_reach_the_flows_panel(cx: &mut TestAppContext) {
    let (_lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.set_right_dock_view(daruda_store::project::RightDockView::Tasks, cx);
            ws.command_palette.open();
            for ch in "Right Panel: Flows".chars() {
                ws.command_palette.append(ch);
            }
            assert_eq!(
                ws.command_palette.filtered_entries().len(),
                1,
                "the palette does not offer the Flows panel"
            );
            ws.execute_palette_action(window, cx);
            assert_eq!(
                ws.right_dock_view,
                daruda_store::project::RightDockView::Flows,
                "the palette entry did not reach the panel"
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

const WITH_PROFILES: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
profiles:
  cheap:
    agent:
      model: haiku
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write a line
";

/// A flow that declares profiles asks a second question before it runs,
/// and the file as written stays on the menu — declaring a profile must
/// not take away the base run.
#[gpui::test]
async fn a_flow_with_profiles_asks_which_one_before_running(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, wh) = workspace_with_a_flow(cx, WITH_PROFILES);

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_flow_picker(crate::workspace::command::flow_picker::FlowPurpose::Run, cx);
            ws.execute_flow_picker_selection(window, cx);

            assert!(
                ws.flow_picker.is_open(),
                "the picker closed before asking which profile"
            );
            let rows = picker_rows(ws);
            assert_eq!(
                rows.len(),
                2,
                "expected the file's own defaults and one profile: {rows:?}"
            );
            assert_eq!(rows[1], "cheap");

            // The second Enter: the half that actually starts the run under
            // the chosen name. Asserted through `focused_pick` rather than
            // by executing, because executing submits a real run.
            ws.flow_picker.move_down();
            assert_eq!(
                ws.flow_picker.focused_pick(),
                Some(crate::workspace::command::flow_picker::FlowPick::Profile(
                    crate::workspace::command::flow_picker::FlowPurpose::Run,
                    flow_path.clone(),
                    Some("cheap".to_string()),
                )),
                "the second question answered with something other than the profile"
            );
        });
    })
    .expect("the test window is live");
}

/// And the answer is carried all the way into the run. The pick and the
/// request are wired by one `match` arm, and dropping the name there leaves
/// every profiled run silently running as plain `defaults`.
#[gpui::test]
async fn answering_the_second_question_runs_under_that_profile(cx: &mut TestAppContext) {
    let (_lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, WITH_PROFILES);

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            // `Validate` rather than `Run`: it walks the same two stages and
            // the same wiring, and takes no lock and starts no session.
            ws.open_flow_picker(
                crate::workspace::command::flow_picker::FlowPurpose::Validate,
                cx,
            );
            ws.execute_flow_picker_selection(window, cx);
            ws.flow_picker.move_down();
            ws.execute_flow_picker_selection(window, cx);
            assert!(!ws.flow_picker.is_open(), "the picker is still asking");
        });
    })
    .expect("the test window is live");
}

/// A flow declaring none runs the moment it is picked. The second question
/// costs the flows that do not use profiles nothing.
#[gpui::test]
async fn a_flow_without_profiles_is_never_asked_about_one(cx: &mut TestAppContext) {
    let (_lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);

    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            ws.open_flow_picker(
                crate::workspace::command::flow_picker::FlowPurpose::Validate,
                cx,
            );
            ws.execute_flow_picker_selection(window, cx);
            assert!(
                !ws.flow_picker.is_open(),
                "a flow with no profiles was asked about one"
            );
        });
    })
    .expect("the test window is live");
}

/// The chosen profile reaches the engine. Checked through the request the
/// app assembles, because that is the only place the two meet.
#[gpui::test]
async fn the_chosen_profile_reaches_the_request(cx: &mut TestAppContext) {
    let (_lane, ws, flow_path, _wh) = workspace_with_a_flow(cx, WITH_PROFILES);

    let (plain, chosen) = ws.update(cx, |ws, cx| {
        let plain = ws
            .build_flow_request(&flow_path, None, cx)
            .expect("builds")
            .request;
        let chosen = ws
            .build_flow_request(&flow_path, Some("cheap"), cx)
            .expect("builds")
            .request;
        (
            model_of_first_node(&plain),
            (
                model_of_first_node(&chosen),
                chosen.loaded.flow().profile.clone(),
            ),
        )
    });
    assert_eq!(plain, None, "the base run picked up a profile's model");
    assert_eq!(
        chosen.0.as_deref(),
        Some("haiku"),
        "the profile did not reach the run"
    );
    assert_eq!(
        chosen.1.as_deref(),
        Some("cheap"),
        "the run does not know its profile"
    );
}

fn model_of_first_node(request: &daruda_flow::request::RunRequest) -> Option<String> {
    match &request.loaded.flow().nodes[0].kind {
        daruda_flow::model::NodeKind::Agent { agent, .. } => agent.model.clone(),
        daruda_flow::model::NodeKind::Command { .. } => None,
    }
}

fn picker_rows(ws: &crate::workspace::Workspace) -> Vec<String> {
    ws.flow_picker
        .choosing()
        .map(|c| {
            c.filtered()
                .into_iter()
                .filter_map(|i| c.stage.row(i))
                .map(|r| r.label.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// The picker offers both places at once. Stated at this level and not
/// only against `flow_paths` because what the window looks in is a field
/// it holds — the two can drift, and the way that showed up first was a
/// test reading the developer's own flows.
#[gpui::test]
async fn the_picker_offers_the_person_s_own_flows_beside_the_lane_s(cx: &mut TestAppContext) {
    let (_lane, ws, _flow_path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);
    // The window's own data directory, which the test fixture already
    // points at a temp dir — the same one production resolves the global
    // flows under.
    let global = ws.update(cx, |ws, _| {
        crate::workspace::flow_paths::global_flows_dir(&ws.data_dir)
    });
    std::fs::create_dir_all(&global).expect("mkdir");
    std::fs::write(global.join("tidy.yaml"), "version: 1\n").expect("write");

    let rows = ws.update(cx, |ws, cx| {
        ws.open_flow_picker(
            crate::workspace::command::flow_picker::FlowPurpose::Validate,
            cx,
        );
        picker_rows(ws)
    });
    assert_eq!(rows, vec!["ship.yaml".to_string(), "tidy.yaml".to_string()]);
}

/// A run directory that a killed process left: a journal, a spec, and no
/// marker, with a lock naming a pid that is gone.
fn killed_run_in(lane: &std::path::Path) -> std::path::PathBuf {
    let runs = crate::workspace::flow_paths::runs_dir(lane);
    let run_dir = runs.join("0000000000000001-00000001-0001");
    std::fs::create_dir_all(&run_dir).expect("mkdir");
    std::fs::write(
        run_dir.join(daruda_flow::resume::RUN_SPEC_FILE),
        ONE_AGENT_RESOLVED,
    )
    .expect("spec");
    std::fs::write(
        run_dir.join(daruda_flow::journal::JOURNAL_FILE),
        "{\"kind\":\"started\",\"v\":1,\"profile\":null}\n",
    )
    .expect("journal");
    // pid 0 is never a live process, which is what makes this a crash
    // rather than a run still going.
    std::fs::write(
        runs.join(".lock"),
        "pid: 0\nrun_id: 0000000000000001-00000001-0001\nstarted_unix_secs: 1\n",
    )
    .expect("lock");
    run_dir
}

/// What `run.yaml` holds: every node stating what it resolved to.
const ONE_AGENT_RESOLVED: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
    permission: deny
nodes:
  - id: design
    timeout: 10m
    kind: agent
    agent:
      id: claude
      mode: bypassPermissions
      permission: deny
    prompt: write a line
    output: design.md
    on_fail: halt
";

/// A killed run is picked up from its own directory: the same run id, the
/// spec it recorded, and the journal saying what it had finished. Never the
/// flow file on disk today — editing that between the crash and the resume
/// must not change what the run is, halfway through.
#[gpui::test]
async fn a_killed_run_is_continued_from_its_own_directory(cx: &mut TestAppContext) {
    let (lane, ws, flow_path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let run_dir = killed_run_in(lane.path());
    // The flow file says something else entirely by now.
    std::fs::write(&flow_path, "version: 1\nnodes: []\n").expect("rewrite");

    let (lane_ref, request) = ws.update(cx, |ws, cx| {
        let lane_ref = ws.active_ref();
        let submission = ws
            .build_resume_request(lane_ref, &run_dir, cx)
            .unwrap_or_else(|e| panic!("a killed run should be resumable: {e:?}"));
        (submission.lane, submission.request)
    });

    assert_eq!(lane_ref, ws.update(cx, |ws, _cx| ws.active_ref()));
    assert_eq!(request.run_dir, run_dir, "a resume took a new directory");
    assert!(
        request.resume.is_some(),
        "the request is not a continuation"
    );
    assert_eq!(
        request.loaded.flow().nodes.len(),
        1,
        "the resume read today's flow file instead of the run's own spec"
    );
}

/// The row's lane, not the lane active when the confirmation is answered,
/// owns the continuation. Otherwise a lane switch while the dialog is open
/// pairs one run directory with another lane's cwd and agent resolution.
#[gpui::test]
async fn a_resume_request_uses_the_lane_that_owned_the_row(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let run_dir = killed_run_in(lane.path());
    let other = tempfile::tempdir().expect("tempdir");

    let (original, other_ref) = ws.update(cx, |ws, _cx| {
        let original = ws.active_ref();
        let other_id = ws.projects[0]
            .lanes
            .iter()
            .map(|lane| lane.id)
            .max()
            .unwrap_or_default()
            + 1;
        ws.projects[0]
            .lanes
            .push(crate::lane::Lane::default_for_project(
                other_id,
                other.path().to_path_buf(),
            ));
        (
            original,
            daruda_store::project::LaneRef {
                project: original.project,
                lane: other_id,
            },
        )
    });
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| ws.activate_lane(other_ref, window, cx));
    })
    .expect("window exists");

    let submission = ws.update(cx, |ws, cx| {
        ws.build_resume_request(original, &run_dir, cx)
            .unwrap_or_else(|e| panic!("a killed run should be resumable: {e:?}"))
    });

    assert_eq!(submission.lane, original);
    assert_eq!(submission.request.cwd, lane.path().to_path_buf());
    assert_ne!(ws.update(cx, |ws, _cx| ws.active_ref()), original);
}

/// Only a killed run. A finished one ended the way its policy said to, and
/// continuing it is a different verb — the engine decides that, and the app
/// asks rather than deciding again.
#[gpui::test]
async fn a_run_that_ended_on_purpose_is_not_continued(cx: &mut TestAppContext) {
    let (lane, ws, _flow_path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let run_dir = killed_run_in(lane.path());
    std::fs::write(run_dir.join("DONE"), "").expect("marker");

    let refused = ws.update(cx, |ws, cx| {
        ws.build_resume_request(ws.active_ref(), &run_dir, cx).err()
    });
    assert!(
        matches!(
            refused,
            Some(crate::workspace::flow_request::FlowSubmitError::Resume(
                daruda_flow::resume::ResumeError::NotResumable(_)
            ))
        ),
        "{refused:?}"
    );
}

/// One predicate answers both halves of the same question.
///
/// Whether the earlier process is gone decides *both* that the run may be
/// continued and that its stale lock may be reclaimed. Two predicates can
/// drift, and then the panel offers a Continue the lock refuses as held —
/// which is exactly what happened the first time this was wired.
#[gpui::test]
async fn the_resumed_run_judges_the_stale_lock_the_same_way_it_judged_the_crash(
    cx: &mut TestAppContext,
) {
    let (lane, ws, _flow_path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);
    let run_dir = killed_run_in(lane.path());

    let request = ws.update(cx, |ws, cx| {
        ws.build_resume_request(ws.active_ref(), &run_dir, cx)
            .unwrap_or_else(|e| panic!("a killed run should be resumable: {e:?}"))
            .request
    });

    let holder = daruda_flow::lock::read_holder(run_dir.parent().expect("runs dir"))
        .expect("the killed run left its lock");
    assert!(
        !(request.is_alive)(holder.pid),
        "the request would find the lock held by a process the resume just called gone"
    );
}

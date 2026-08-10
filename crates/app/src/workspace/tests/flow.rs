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
) {
    let lane = tempfile::tempdir().expect("tempdir");
    let flows = crate::workspace::flow_paths::flows_dir(lane.path());
    std::fs::create_dir_all(&flows).expect("create flows dir");
    let flow_path = flows.join("ship.yaml");
    std::fs::write(&flow_path, flow).expect("write flow");

    let config = daruda_config::Config::default();
    let project = daruda_store::project::Project::from_path(lane.path());
    let (_wh, ws) = build_workspace_with(cx, &config, Some(project));
    (lane, ws, flow_path)
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
    let (_lane, ws, flow_path) = workspace_with_a_flow(cx, ONE_AGENT);

    let issues = ws.update(cx, |ws, cx| {
        let submission = ws
            .build_flow_request(&flow_path, cx)
            .unwrap_or_else(|_| panic!("a local lane with a valid flow builds a request"));
        daruda_flow::request::validate_request(&submission.request)
    });

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
    let (lane, ws, flow_path) = workspace_with_a_flow(cx, ONE_AGENT);

    let request = ws
        .update(cx, |ws, cx| ws.build_flow_request(&flow_path, cx))
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
    let (lane, ws, flow_path) = workspace_with_a_flow(
        cx,
        "version: 1\nnodes:\n  - id: a\n    kind: command\n    deps: [ghost]\n    run: \"true\"\n",
    );

    let refused = ws.update(cx, |ws, cx| ws.build_flow_request(&flow_path, cx).is_err());
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
    let (lane, ws, _flow_path) = workspace_with_a_flow(cx, ONE_AGENT);
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
            .map(|c| c.candidates.iter().map(|f| f.label.clone()).collect())
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
    let (_lane, ws, flow_path) = workspace_with_a_flow(
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

    let refused = ws.update(cx, |ws, cx| match ws.build_flow_request(&flow_path, cx) {
        Err(crate::workspace::flow_request::FlowSubmitError::Invalid(issues)) => issues,
        Err(_) => panic!("refused, but not for the reason under test"),
        Ok(_) => panic!("a flow naming an unconfigured agent was accepted"),
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
    let (lane, ws, _flow_path) = workspace_with_a_flow(cx, ONE_AGENT);
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
    let (lane, ws, _flow_path) = workspace_with_a_flow(cx, ONE_AGENT);
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
    let (lane, ws, _flow_path) = workspace_with_a_flow(cx, ONE_AGENT);

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
    let (lane, ws, _flow_path) = workspace_with_a_flow(cx, ONE_AGENT);

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
    let (lane, ws, _flow_path) = workspace_with_a_flow(cx, ONE_AGENT);
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
    let (lane, ws, _flow_path) = workspace_with_a_flow(cx, ONE_AGENT);
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
    let (lane, ws, _flow_path) = workspace_with_a_flow(cx, ONE_AGENT);

    ws.update(cx, |ws, cx| {
        let here = ws.active;
        ws.seed_flow_run_for_test(here, lane.path().join("run"));
        let (reply_tx, reply_rx) = smol::channel::bounded(1);
        ws.park_flow_ask_for_test(
            here,
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

/// `load` alone is not the whole of stage 1. A `prompt_file` that is not
/// there is only knowable with the request's own context, so checking less
/// here than `Run Flow…` checks means "no problems found" followed by a
/// refusal — the split design §12 exists to prevent.
#[gpui::test]
async fn checking_a_flow_finds_what_only_the_request_can_see(cx: &mut TestAppContext) {
    let (lane, ws, flow_path) = workspace_with_a_flow(
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

    let issues = ws.update(cx, |ws, cx| ws.check_flow(&flow_path, cx).expect("checked"));
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

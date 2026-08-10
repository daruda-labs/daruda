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

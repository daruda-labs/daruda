//! Flow tests, split by what they are about.
//!
//! One file per surface rather than one per type: a run's submission, a
//! question's queue, the graph pane, the inspector, and the files themselves
//! fail for different reasons and want different fixtures. What they share —
//! a workspace with a flow in it, and the flow texts — is here.

mod ask;
mod files;
mod graph;
mod inspector;
mod partial;
mod run;

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

/// A node declaring a shape for its output. The inspector has no box for one —
/// it is written in the YAML editor — so this fixture is what proves a save
/// does not take it away.
const WITH_A_SCHEMA: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: design
    kind: agent
    output: design.json
    output_schema:
      type: object
      required: [verdict]
      properties:
        verdict: { type: string, enum: [pass, fail] }
    prompt: write a line
";

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

/// Park a question on `lane`, with the reply channel handed back so a test
/// can see whether it was ever answered.
fn park_question(
    ws: &gpui::Entity<Workspace>,
    cx: &mut gpui::VisualTestContext,
    lane: daruda_store::project::LaneRef,
    run_dir: &std::path::Path,
    ask_id: u64,
) -> smol::channel::Receiver<daruda_acp::PermissionDecision> {
    let (reply, rx) = smol::channel::bounded(1);
    cx.update(|window, cx| {
        ws.update(cx, |ws, cx| {
            ws.park_flow_ask_for_test(
                lane,
                run_dir,
                daruda_flow::runner::PendingAsk {
                    node: format!("node-{ask_id}").into(),
                    attempt: 1,
                    ask_id,
                    request: daruda_flow::runner::AskRequest {
                        tool: "Bash".to_string(),
                        detail: None,
                        options: Vec::new(),
                    },
                    reply,
                },
                window,
                cx,
            );
        });
    });
    rx
}

/// A chain long enough that, unfitted, its tail sits past the right edge of
/// the drawable — layout spaces columns a full node-width-plus-120 apart, so
/// this is the shape that proves the graph is framed to the pane rather than
/// merely drawn at the origin.
///
/// Six nodes, not more: the canvas will not zoom out past `ZOOM_MIN` (0.7),
/// so a chain past roughly seven nodes cannot be framed whole at all and
/// would make this test assert something the canvas never promised.
const LONG_CHAIN: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: bypassPermissions
nodes:
  - id: n1
    kind: agent
    output: n1.md
    prompt: one
  - id: n2
    kind: agent
    deps: [n1]
    output: n2.md
    prompt: two
  - id: n3
    kind: agent
    deps: [n2]
    output: n3.md
    prompt: three
  - id: n4
    kind: agent
    deps: [n3]
    output: n4.md
    prompt: four
  - id: n5
    kind: agent
    deps: [n4]
    output: n5.md
    prompt: five
  - id: n6
    kind: agent
    deps: [n5]
    output: n6.md
    prompt: six
";

const TWO_NODE_CHAIN: &str = "\
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
  - id: build
    kind: agent
    deps: [design]
    output: build.md
    prompt: build it
";

const OVERRIDDEN: &str = "\
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
    agent:
      id: codex
      mode: danger-full-access
      model: gpt
";

/// Whether the workspace has said that unsaved typing was dropped.
///
/// A toast rather than a banner in the inspector: the two cases below replace
/// or remove the very form a banner would have sat on.
fn told_about_dropped_typing(
    ws: &gpui::Entity<crate::workspace::Workspace>,
    vcx: &gpui::VisualTestContext,
) -> bool {
    ws.read_with(vcx, |ws, _| {
        ws.error_history()
            .iter()
            .any(|report| report.dedup_key.as_deref() == Some("flow.edit_dropped_typing"))
    })
}

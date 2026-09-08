//! `/flow` from the external control surface: which flows it lists, and the
//! three refusals that precede a run.
//!
//! The refusals are what these are really about. A run started from the
//! desktop reports a problem in a toast; a run started from a phone has no
//! toast to read, so anything the run would refuse *after* dispatch has to
//! become an answer before it.

use super::*;

use crate::control::result::{ControlError, FlowOriginKind};
use crate::workspace::flow_paths;

/// A flow that stops to ask a person — the exact shape a phone cannot answer.
const ASKS_A_PERSON: &str = "\
version: 1
defaults:
  agent:
    id: claude
    mode: acceptEdits
    permission: ask
nodes:
  - id: design
    kind: agent
    output: design.md
    prompt: write a line
";

/// A flow that needs no agent and no person: the one shape whose *successful*
/// start a test can assert without a live ACP session.
const COMMAND_ONLY: &str = "\
version: 1
nodes:
  - id: check
    kind: command
    run: \"true\"
";

/// Names a dependency no node declares — a graph the engine refuses to build.
const DANGLING_DEP: &str = "\
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
    deps:
      - nowhere
";

fn write_global_flow(ws: &Workspace, name: &str, body: &str) {
    let dir = flow_paths::global_flows_dir(&ws.data_dir);
    std::fs::create_dir_all(&dir).expect("create global flows dir");
    std::fs::write(dir.join(name), body).expect("write global flow");
}

#[gpui::test]
async fn the_listing_names_every_flow_with_the_scope_it_came_from(cx: &mut TestAppContext) {
    let (_lane, ws, _path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);
    ws.update(cx, |ws, _| write_global_flow(ws, "mine.yaml", ONE_AGENT));

    ws.read_with(cx, |ws, _| {
        let entries = ws.control_flow_list();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"ship.yaml"), "the repo's own: {names:?}");
        assert!(names.contains(&"mine.yaml"), "the person's own: {names:?}");
        let ship = entries
            .iter()
            .find(|e| e.name == "ship.yaml")
            .expect("ship");
        assert_eq!(ship.origin, FlowOriginKind::Repo);
        let mine = entries
            .iter()
            .find(|e| e.name == "mine.yaml")
            .expect("mine");
        assert_eq!(mine.origin, FlowOriginKind::Global);
    });
}

/// The repo's copy shadows the person's, so a name resolves to exactly one
/// file and there is no ambiguity for the caller to disambiguate.
#[gpui::test]
async fn a_name_held_by_two_scopes_resolves_to_the_narrower_one(cx: &mut TestAppContext) {
    let (_lane, ws, _path, _wh) = workspace_with_a_flow(cx, ONE_AGENT);
    ws.update(cx, |ws, _| write_global_flow(ws, "ship.yaml", ONE_AGENT));

    ws.read_with(cx, |ws, _| {
        let ship: Vec<_> = ws
            .control_flow_list()
            .into_iter()
            .filter(|e| e.name == "ship.yaml")
            .collect();
        assert_eq!(ship.len(), 1, "one entry per name: {ship:?}");
        assert_eq!(ship[0].origin, FlowOriginKind::Repo);
    });
}

#[gpui::test]
async fn an_unknown_flow_name_is_refused(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, ONE_AGENT);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(
                ws.control_flow_run("nope", window, cx),
                Err(ControlError::FlowNotFound {
                    name: "nope".into()
                })
            );
        });
    })
    .expect("window is live");
}

/// A phone should be able to type `deploy`, not `deploy.yaml`.
#[gpui::test]
async fn a_name_resolves_with_or_without_its_extension(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, ASKS_A_PERSON);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            // Both spellings reach the same file, and so hit the same refusal
            // — which is what proves resolution happened at all.
            for name in ["ship", "ship.yaml"] {
                assert_eq!(
                    ws.control_flow_run(name, window, cx),
                    Err(ControlError::FlowNeedsInteraction {
                        name: "ship.yaml".into()
                    }),
                    "{name} must resolve"
                );
            }
        });
    })
    .expect("window is live");
}

#[gpui::test]
async fn a_flow_that_asks_a_person_is_refused_before_it_can_hang(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, ASKS_A_PERSON);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(
                ws.control_flow_run("ship", window, cx),
                Err(ControlError::FlowNeedsInteraction {
                    name: "ship.yaml".into()
                })
            );
        });
    })
    .expect("window is live");
}

/// A file declaring profiles opens the picker's second question, which is a
/// desktop dialog like any other.
#[gpui::test]
async fn a_flow_declaring_profiles_is_refused_for_the_same_reason(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, WITH_PROFILES);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(
                ws.control_flow_run("ship", window, cx),
                Err(ControlError::FlowNeedsInteraction {
                    name: "ship.yaml".into()
                })
            );
        });
    })
    .expect("window is live");
}

/// The regression this guards: reporting a run as started that the desktop
/// then refuses in a toast the phone never sees.
#[gpui::test]
async fn a_flow_that_will_not_run_is_answered_not_reported_as_started(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, DANGLING_DEP);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(
                ws.control_flow_run("ship", window, cx),
                Err(ControlError::FlowRefused {
                    name: "ship.yaml".into()
                })
            );
        });
    })
    .expect("window is live");
}

#[gpui::test]
async fn running_a_flow_without_an_active_lane_is_refused(cx: &mut TestAppContext) {
    let config = daruda_config::Config::default();
    let (wh, ws) = build_workspace_with(cx, &config, None);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            assert_eq!(
                ws.control_flow_run("ship", window, cx),
                Err(ControlError::NoActiveLane)
            );
        });
    })
    .expect("window is live");
}

/// The headline path, and the one every refusal test above is the negative of:
/// a runnable flow is dispatched, is reported under the file that resolved,
/// and — the invariant the whole `needs_desktop_answer` / profile machinery
/// exists for — raises no desktop dialog on the way.
#[gpui::test]
async fn a_runnable_flow_starts_and_names_the_file_that_ran(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let entry = ws
                .control_flow_run("ship", window, cx)
                .expect("a command-only flow needs neither an agent nor a person");
            assert_eq!(entry.name, "ship.yaml", "named by the file, not the input");
            assert_eq!(entry.origin, FlowOriginKind::Repo);
            assert!(
                !ws.flow_picker.is_open(),
                "a phone command must never raise a desktop dialog"
            );
        });
    })
    .expect("window is live");
}

/// A lane already running a flow refuses the next one — and, critically,
/// refuses it *as an answer* rather than by opening the desktop "stop it?"
/// picker the guard would otherwise show.
#[gpui::test]
async fn a_lane_already_running_a_flow_refuses_the_next(cx: &mut TestAppContext) {
    let (_lane, ws, _path, wh) = workspace_with_a_flow(cx, COMMAND_ONLY);
    cx.update_window(wh.into(), |_, window, cx| {
        ws.update(cx, |ws, cx| {
            let lane_ref = ws.active_ref();
            ws.seed_flow_run_for_test(lane_ref, std::path::PathBuf::from("/tmp/seeded-run"));
            assert_eq!(
                ws.control_flow_run("ship", window, cx),
                Err(ControlError::FlowLocked {
                    name: "ship.yaml".into()
                })
            );
            assert!(
                !ws.flow_picker.is_open(),
                "the refusal is the answer, not a dialog"
            );
        });
    })
    .expect("window is live");
}
